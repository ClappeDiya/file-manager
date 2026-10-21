//! Bug reports: a zip a user can attach to an issue without exposing their
//! file names, credentials or addresses. Contains a sanitized copy of the job
//! database, the job configuration with secrets and paths removed, recent run
//! logs (paths redacted) and system information.

use super::{notify, BackupJob, BackupRun};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::io::Write;
use std::path::Path;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BugReport {
    pub path: String,
    pub size: u64,
    pub files: Vec<String>,
    pub paths_obfuscated: u64,
    pub runs_included: usize,
}

fn is_secret_key(k: &str) -> bool {
    let k = k.to_ascii_lowercase();
    ["pass", "secret", "token", "api_key", "apikey", "authorization", "headers", "private", "cookie", "chat_id", "body_template"].iter().any(|s| k.contains(s))
}

/// Keep only scheme and host of a URL (query strings and paths often carry tokens).
fn redact_url(s: &str) -> String {
    match url::Url::parse(s) {
        Ok(u) if u.host_str().is_some() => format!("{}://{}/…", u.scheme(), u.host_str().unwrap_or_default()),
        _ => s.to_string(),
    }
}

/// Recursively blank secrets, shorten URLs and hide addresses and paths.
pub fn sanitize_json(v: &mut Value, roots: &[String]) {
    let email = regex::Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").expect("valid regex");
    fn walk(v: &mut Value, key: &str, roots: &[String], email: &regex::Regex) {
        match v {
            Value::Object(m) => {
                for (k, x) in m.iter_mut() {
                    walk(x, k, roots, email);
                }
            }
            Value::Array(a) => a.iter_mut().for_each(|x| walk(x, key, roots, email)),
            Value::String(s) if !s.is_empty() => {
                *s = if is_secret_key(key) {
                    "***".into()
                } else if s.contains("://") {
                    redact_url(s)
                } else {
                    email.replace_all(&notify::redact_paths(s, roots), "***@***").into_owned()
                };
            }
            _ => {}
        }
    }
    walk(v, "", roots, &email);
}

fn json_bytes<T: Serialize>(v: &T, roots: &[String]) -> Result<Vec<u8>, AppError> {
    let mut j = serde_json::to_value(v)?;
    sanitize_json(&mut j, roots);
    Ok(serde_json::to_vec_pretty(&j)?)
}

/// Build the zip at `out`. `db_path` is the job's local database (may be missing).
pub fn create(out: &Path, job: &BackupJob, db_path: &Path, runs: &[BackupRun], log_file: Option<&Path>) -> Result<BugReport, AppError> {
    if let Some(d) = out.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let roots = job.sources.clone();
    let tmp = tempfile_path(out);
    let mut rep = BugReport { path: out.to_string_lossy().into_owned(), ..Default::default() };
    let file = std::fs::File::create(out)?;
    let mut z = zip::ZipWriter::new(file);
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let add = |z: &mut zip::ZipWriter<std::fs::File>, name: &str, data: &[u8], rep: &mut BugReport| -> Result<(), AppError> {
        z.start_file(name, opts).map_err(|e| AppError::internal(format!("bug report zip: {e}")))?;
        z.write_all(data)?;
        rep.files.push(name.to_string());
        Ok(())
    };

    let mut job_v = serde_json::to_value(job)?;
    // The stored statistics' last error can quote paths; sanitize like the rest.
    sanitize_json(&mut job_v, &roots);
    add(&mut z, "job.json", &serde_json::to_vec_pretty(&job_v)?, &mut rep)?;
    add(&mut z, "runs.json", &json_bytes(&runs, &roots)?, &mut rep)?;
    rep.runs_included = runs.len();

    let mut system = serde_json::json!({
        "app_version": env!("CARGO_PKG_VERSION"),
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "family": std::env::consts::FAMILY,
        "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "created_at": chrono::Utc::now().to_rfc3339(),
        "destination_kind": match &job.destination {
            super::BackupDestination::Local { .. } => "local",
            super::BackupDestination::Connection { .. } => "connection",
            super::BackupDestination::Library { .. } => "library",
            super::BackupDestination::Cloud { config, .. } => config.provider(),
        },
    });
    if db_path.is_file() {
        let db = ufop_core::backup::db::BackupDb::open(db_path)?;
        let s = db.export_sanitized(&tmp)?;
        rep.paths_obfuscated = s.paths_obfuscated;
        system["database"] = serde_json::to_value(ufop_core::backup::db::BackupDb::inspect(db_path)?)?;
        system["database"]["path"] = Value::String("(hidden)".into());
        system["database_integrity"] = serde_json::to_value(db.integrity_problems()?)?;
        drop(db);
        let data = std::fs::read(&tmp)?;
        let _ = std::fs::remove_file(&tmp);
        add(&mut z, "database.sqlite", &data, &mut rep)?;
    }
    add(&mut z, "system.json", &serde_json::to_vec_pretty(&system)?, &mut rep)?;
    if let Some(lf) = log_file.filter(|p| p.is_file()) {
        // Last 1 MiB of the job's log file, paths redacted.
        let data = std::fs::read(lf)?;
        let tail = String::from_utf8_lossy(&data[data.len().saturating_sub(1 << 20)..]).into_owned();
        add(&mut z, "job.log", notify::redact_paths(&tail, &roots).as_bytes(), &mut rep)?;
    }
    z.finish().map_err(|e| AppError::internal(format!("bug report zip: {e}")))?;
    rep.size = std::fs::metadata(out)?.len();
    Ok(rep)
}

fn tempfile_path(out: &Path) -> std::path::PathBuf {
    let mut p = out.as_os_str().to_owned();
    p.push(format!(".{}.db.tmp", uuid::Uuid::new_v4().simple()));
    p.into()
}
