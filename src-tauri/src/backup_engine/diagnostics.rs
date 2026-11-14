//! Job diagnostics: local-database integrity and consistency with the
//! destination, network timings (DNS, TCP, TLS) and a small upload/download
//! throughput test.

use super::notify::{tls_config, CertPolicy};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::{Duration, Instant};
use ufop_core::backup::backend::Backend;
use ufop_core::backup::db::{BackupDb, DbInfo};
use ufop_core::backup::volume;

fn ms(d: Duration) -> f64 {
    (d.as_secs_f64() * 1e6).round() / 1e3
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseCheck {
    pub info: Option<DbInfo>,
    /// SQLite integrity and reference problems.
    pub problems: Vec<String>,
    /// Volumes the database lists that the destination does not have.
    pub missing_remote: Vec<String>,
    /// Volumes whose remote size differs from the recorded size.
    pub size_mismatch: Vec<String>,
    /// Files with the backup prefix that the database does not track.
    pub untracked_remote: Vec<String>,
    pub remote_files: usize,
    pub tracked_volumes: usize,
    pub check_ms: f64,
}

impl DatabaseCheck {
    pub fn ok(&self) -> bool {
        self.problems.is_empty() && self.missing_remote.is_empty() && self.size_mismatch.is_empty()
    }
}

/// Compare a job database with the destination listing.
pub fn check_database(db: &BackupDb, info: Option<DbInfo>, backend: &dyn Backend, prefix: &str) -> Result<DatabaseCheck, AppError> {
    let t = Instant::now();
    let mut c = DatabaseCheck { info, problems: db.integrity_problems()?, ..Default::default() };
    // Logical volume names, wherever the repository layout keeps them.
    let view = ufop_core::backup::layout::for_repository(backend, prefix);
    let remote: HashMap<String, u64> = view.list()?.into_iter().map(|f| (f.name, f.size)).collect();
    c.remote_files = remote.len();
    let vols = db.volumes(None)?;
    c.tracked_volumes = vols.len();
    for v in &vols {
        match remote.get(&v.name) {
            None if v.state != "uploading" => c.missing_remote.push(v.name.clone()),
            Some(&sz) if v.size > 0 && sz != v.size => c.size_mismatch.push(format!("{} (recorded {} B, remote {} B)", v.name, v.size, sz)),
            _ => {}
        }
    }
    let known: std::collections::HashSet<&str> = vols.iter().map(|v| v.name.as_str()).collect();
    let mut aux = vec![volume::repo_header_name(prefix), volume::verification_file_name(prefix)];
    aux.extend(ufop_core::backup::layout::header_copies(prefix));
    let mut untracked: Vec<String> = remote
        .keys()
        .filter(|n| n.starts_with(&format!("{prefix}-")) && !known.contains(n.as_str()) && !aux.contains(n) && !n.ends_with(".par2") && !ufop_core::backup::lock::is_lock_file(prefix, n))
        .cloned()
        .collect();
    untracked.sort();
    c.untracked_remote = untracked;
    c.missing_remote.sort();
    c.check_ms = ms(t.elapsed());
    Ok(c)
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkTiming {
    pub host: String,
    pub port: u16,
    pub tls: bool,
    pub addresses: Vec<String>,
    pub dns_ms: Option<f64>,
    pub tcp_ms: Option<f64>,
    pub tls_ms: Option<f64>,
    pub tls_version: Option<String>,
    pub error: Option<String>,
}

/// Time name resolution, the TCP connect and (for TLS services) the handshake.
pub async fn probe(host: &str, port: u16, tls: bool, cert: &CertPolicy, timeout: Duration) -> NetworkTiming {
    let mut n = NetworkTiming { host: host.into(), port, tls, ..Default::default() };
    let t = Instant::now();
    let addrs: Vec<std::net::SocketAddr> = match tokio::time::timeout(timeout, tokio::net::lookup_host((host, port))).await {
        Ok(Ok(a)) => a.collect(),
        Ok(Err(e)) => {
            n.error = Some(format!("DNS lookup failed: {e}"));
            return n;
        }
        Err(_) => {
            n.error = Some("DNS lookup timed out".into());
            return n;
        }
    };
    n.dns_ms = Some(ms(t.elapsed()));
    n.addresses = addrs.iter().map(|a| a.ip().to_string()).collect();
    let Some(addr) = addrs.first().copied() else {
        n.error = Some("The name resolved to no addresses".into());
        return n;
    };
    let t = Instant::now();
    let sock = match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await {
        Ok(Ok(s)) => s,
        Ok(Err(e)) => {
            n.error = Some(format!("TCP connect to {addr} failed: {e}"));
            return n;
        }
        Err(_) => {
            n.error = Some(format!("TCP connect to {addr} timed out"));
            return n;
        }
    };
    n.tcp_ms = Some(ms(t.elapsed()));
    if tls {
        let cfg = match tls_config(cert) {
            Ok(c) => c,
            Err(e) => {
                n.error = Some(e.user_message());
                return n;
            }
        };
        let Ok(name) = rustls::pki_types::ServerName::try_from(host.to_string()) else {
            n.error = Some(format!("{host} is not a valid TLS server name"));
            return n;
        };
        let t = Instant::now();
        match tokio::time::timeout(timeout, tokio_rustls::TlsConnector::from(cfg).connect(name, sock)).await {
            Ok(Ok(s)) => {
                n.tls_ms = Some(ms(t.elapsed()));
                n.tls_version = s.get_ref().1.protocol_version().map(|v| format!("{v:?}"));
            }
            Ok(Err(e)) => n.error = Some(format!("TLS handshake failed: {e}")),
            Err(_) => n.error = Some("TLS handshake timed out".into()),
        }
    }
    n
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Throughput {
    /// Latency of three consecutive listings.
    pub list_ms: Vec<f64>,
    pub test_bytes: u64,
    pub upload_ms: f64,
    pub upload_bps: u64,
    pub download_ms: f64,
    pub download_bps: u64,
    /// The downloaded test file matched what was uploaded.
    pub verified: bool,
    pub error: Option<String>,
}

fn rate(bytes: u64, d: Duration) -> u64 {
    (bytes as f64 / d.as_secs_f64().max(1e-6)) as u64
}

/// Upload, download and delete a random test file of `size` bytes.
pub fn throughput(backend: &dyn Backend, size: usize) -> Throughput {
    let mut r = Throughput { test_bytes: size as u64, ..Default::default() };
    for _ in 0..3 {
        let t = Instant::now();
        if let Err(e) = backend.list() {
            r.error = Some(format!("Listing failed: {}", e.user_message()));
            return r;
        }
        r.list_ms.push(ms(t.elapsed()));
    }
    let data: Vec<u8> = (0..size).map(|_| rand::random::<u8>()).collect();
    let name = format!("ufop-diagnostic-{}.tmp", uuid::Uuid::new_v4().simple());
    let t = Instant::now();
    if let Err(e) = backend.put(&name, &data) {
        r.error = Some(format!("Upload failed: {}", e.user_message()));
        return r;
    }
    r.upload_ms = ms(t.elapsed());
    r.upload_bps = rate(size as u64, t.elapsed());
    let t = Instant::now();
    match backend.get(&name) {
        Ok(back) => {
            r.download_ms = ms(t.elapsed());
            r.download_bps = rate(back.len() as u64, t.elapsed());
            r.verified = back == data;
        }
        Err(e) => r.error = Some(format!("Download failed: {}", e.user_message())),
    }
    if let Err(e) = backend.delete(&name) {
        r.error.get_or_insert(format!("Could not delete the test file {name}: {}", e.user_message()));
    }
    r
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Diagnostics {
    pub database: Option<DatabaseCheck>,
    pub database_error: Option<String>,
    pub network: Option<NetworkTiming>,
    pub throughput: Option<Throughput>,
    /// Snapshot methods of this platform and whether their tools are installed.
    #[serde(default)]
    pub snapshot_providers: Vec<ufop_core::backup::snapshot::ProviderStatus>,
}

/// Largest sample the desktop benchmark accepts (the CLI has no cap).
pub const BENCH_MAX_SAMPLE: usize = 256 << 20;

/// Run `ufop_core::backup::bench` off the async runtime (Settings →
/// Benchmark, `backup_bench`). The sample is capped at [`BENCH_MAX_SAMPLE`].
pub async fn bench(mut opts: ufop_core::backup::bench::BenchOptions) -> Result<ufop_core::backup::bench::BenchReport, AppError> {
    opts.size = opts.size.min(BENCH_MAX_SAMPLE);
    tokio::task::spawn_blocking(move || ufop_core::backup::bench::run(&opts, &ufop_core::backup::OpContext::new()))
        .await
        .map_err(|e| AppError::internal(format!("benchmark task failed: {e}")))?
}
