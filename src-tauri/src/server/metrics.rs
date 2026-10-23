//! Prometheus metrics (text exposition format 0.0.4), written by hand.
//!
//! Served at `GET /api/v1/metrics` on the main listener (any signed-in
//! user or API token with the `read` scope) and, with `--metrics-listen
//! <addr>`, at `/metrics` on a separate listener (no sign-in, or HTTP Basic
//! with `--metrics-basic-auth user:password`). The same text is what the
//! push-gateway client sends (see `otlp::spawn_pushgateway`).

use super::ServerState;
use axum::extract::State;
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Request-duration histogram bounds (seconds).
pub const BUCKETS: &[f64] = &[0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0, 60.0];
/// Distinct (method, route, status) series kept; later ones are folded into route "other".
const MAX_SERIES: usize = 2000;

#[derive(Default, Clone)]
struct Histogram {
    counts: Vec<u64>,
    sum: f64,
    total: u64,
}

/// HTTP request counters and histograms (cheap to clone).
#[derive(Clone, Default)]
pub struct Metrics {
    http: Arc<Mutex<BTreeMap<(String, String, u16), Histogram>>>,
}

impl Metrics {
    pub fn observe_http(&self, method: &str, route: &str, status: u16, latency: Duration) {
        let mut m = self.http.lock().unwrap_or_else(|e| e.into_inner());
        let mut key = (method.to_string(), route.to_string(), status);
        if !m.contains_key(&key) && m.len() >= MAX_SERIES {
            key.1 = "other".into();
        }
        let h = m.entry(key).or_insert_with(|| Histogram { counts: vec![0; BUCKETS.len()], ..Default::default() });
        let secs = latency.as_secs_f64();
        for (i, b) in BUCKETS.iter().enumerate() {
            if secs <= *b {
                h.counts[i] += 1;
            }
        }
        h.sum += secs;
        h.total += 1;
    }
}

/// A label value escaped for the text format.
pub fn esc(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn labels(pairs: &[(&str, &str)]) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let inner: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}=\"{}\"", esc(v))).collect();
    format!("{{{}}}", inner.join(","))
}

struct Out(String);

impl Out {
    fn head(&mut self, name: &str, kind: &str, help: &str) {
        let _ = writeln!(self.0, "# HELP {name} {help}\n# TYPE {name} {kind}");
    }
    fn sample(&mut self, name: &str, l: &[(&str, &str)], v: impl std::fmt::Display) {
        let _ = writeln!(self.0, "{name}{} {v}", labels(l));
    }
}

fn ts_secs(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp())
}

/// The whole exposition.
pub async fn render(st: &ServerState) -> String {
    let mut o = Out(String::new());
    o.head("ufop_build_info", "gauge", "Version of this ufop-server (always 1).");
    o.sample("ufop_build_info", &[("version", env!("CARGO_PKG_VERSION")), ("os", std::env::consts::OS), ("arch", std::env::consts::ARCH)], 1);
    o.head("ufop_uptime_seconds", "gauge", "Seconds since the server started.");
    o.sample("ufop_uptime_seconds", &[], st.started.elapsed().as_secs());
    o.head("ufop_shutting_down", "gauge", "1 while the server drains for shutdown.");
    o.sample("ufop_shutting_down", &[], u8::from(st.life.is_draining()));

    // HTTP.
    let http = st.metrics.http.lock().unwrap_or_else(|e| e.into_inner()).clone();
    o.head("ufop_http_requests_total", "counter", "HTTP requests by method, route template and status.");
    for ((m, r, s), h) in &http {
        o.sample("ufop_http_requests_total", &[("method", m), ("route", r), ("status", &s.to_string())], h.total);
    }
    o.head("ufop_http_request_duration_seconds", "histogram", "HTTP request latency by method, route template and status.");
    for ((m, r, s), h) in &http {
        let status = s.to_string();
        for (i, b) in BUCKETS.iter().enumerate() {
            o.sample("ufop_http_request_duration_seconds_bucket", &[("method", m), ("route", r), ("status", &status), ("le", &b.to_string())], h.counts[i]);
        }
        o.sample("ufop_http_request_duration_seconds_bucket", &[("method", m), ("route", r), ("status", &status), ("le", "+Inf")], h.total);
        o.sample("ufop_http_request_duration_seconds_sum", &[("method", m), ("route", r), ("status", &status)], h.sum);
        o.sample("ufop_http_request_duration_seconds_count", &[("method", m), ("route", r), ("status", &status)], h.total);
    }

    // Jobs and runs.
    let jobs = st.mgr.list_jobs().await;
    o.head("ufop_backup_job_info", "gauge", "One series per backup job, carrying its name.");
    for j in &jobs {
        o.sample("ufop_backup_job_info", &[("job_id", &j.id), ("job_name", &j.name), ("read_only", if j.read_only { "true" } else { "false" })], 1);
    }
    o.head("ufop_backup_versions", "gauge", "Versions in the job's repository (as of its last run).");
    for j in &jobs {
        o.sample("ufop_backup_versions", &[("job_id", &j.id)], j.stats.version_count);
    }
    o.head("ufop_backup_source_bytes", "gauge", "Size of the job's sources at its last backup.");
    for j in &jobs {
        o.sample("ufop_backup_source_bytes", &[("job_id", &j.id)], j.stats.source_size);
    }
    o.head("ufop_backup_destination_bytes", "gauge", "Size of the job's repository at its last run.");
    for j in &jobs {
        o.sample("ufop_backup_destination_bytes", &[("job_id", &j.id)], j.stats.backend_size);
    }
    match st.mgr.run_metrics().await {
        Ok(rm) => {
            o.head("ufop_backup_runs_total", "counter", "Finished operations by job, operation and result (from the run history).");
            for c in &rm.counts {
                o.sample("ufop_backup_runs_total", &[("job_id", &c.job_id), ("operation", &c.operation), ("result", &c.status)], c.count);
            }
            o.head("ufop_backup_last_success_timestamp_seconds", "gauge", "Unix time the job's newest successful backup finished.");
            for l in rm.last.iter().filter(|l| l.last_success_unix.is_some()) {
                o.sample("ufop_backup_last_success_timestamp_seconds", &[("job_id", &l.job_id)], l.last_success_unix.unwrap_or(0));
            }
            o.head("ufop_backup_last_duration_seconds", "gauge", "Duration of the job's newest backup run.");
            for l in &rm.last {
                o.sample("ufop_backup_last_duration_seconds", &[("job_id", &l.job_id), ("result", &l.status)], l.duration_secs);
            }
            o.head("ufop_backup_last_uploaded_bytes", "gauge", "Bytes uploaded by the job's newest backup run.");
            for l in &rm.last {
                o.sample("ufop_backup_last_uploaded_bytes", &[("job_id", &l.job_id)], l.bytes_uploaded);
            }
            o.head("ufop_backup_last_files", "gauge", "Files examined by the job's newest backup run.");
            for l in &rm.last {
                o.sample("ufop_backup_last_files", &[("job_id", &l.job_id)], l.files_examined);
            }
        }
        Err(e) => tracing::warn!("Run metrics unavailable: {}", e.user_message()),
    }

    // Scheduler, tasks, locks.
    let status = st.mgr.status().await;
    let mut running: BTreeMap<String, u64> = BTreeMap::new();
    let mut locked: BTreeMap<String, u64> = BTreeMap::new();
    for t in status.tasks.iter().filter(|t| t.state == "running") {
        *running.entry(t.operation.clone()).or_default() += 1;
        locked.insert(t.job_id.clone(), 1);
    }
    o.head("ufop_operations_running", "gauge", "Operations running now, by operation.");
    if running.is_empty() {
        o.sample("ufop_operations_running", &[("operation", "backup")], 0);
    }
    for (op, n) in &running {
        o.sample("ufop_operations_running", &[("operation", op)], n);
    }
    o.head("ufop_operations_queued", "gauge", "Operations waiting for a free slot (queue length).");
    o.sample("ufop_operations_queued", &[], status.tasks.iter().filter(|t| t.state == "queued").count());
    o.head("ufop_repository_lock_held", "gauge", "1 while an operation of this job runs and holds its repository lock.");
    for j in &jobs {
        o.sample("ufop_repository_lock_held", &[("job_id", &j.id)], locked.get(&j.id).copied().unwrap_or(0));
    }
    o.head("ufop_scheduler_running", "gauge", "1 when the scheduler loop runs.");
    o.sample("ufop_scheduler_running", &[], u8::from(status.running));
    o.head("ufop_scheduler_paused", "gauge", "1 while scheduled work is paused.");
    o.sample("ufop_scheduler_paused", &[], u8::from(status.paused));
    o.head("ufop_scheduler_next_run_timestamp_seconds", "gauge", "Unix time of the next scheduled run, by job and operation.");
    for n in status.next_runs.iter().chain(status.next_maintenance.iter()) {
        if let Some(t) = ts_secs(&n.next_run_at) {
            o.sample("ufop_scheduler_next_run_timestamp_seconds", &[("job_id", &n.job_id), ("operation", &n.operation)], t);
        }
    }

    // Volume cache.
    if let Ok(c) = st.mgr.cache_stats().await {
        o.head("ufop_volume_cache_bytes", "gauge", "Bytes in the local volume cache.");
        o.sample("ufop_volume_cache_bytes", &[], c.bytes);
        o.head("ufop_volume_cache_max_bytes", "gauge", "Volume cache limit (0 = off).");
        o.sample("ufop_volume_cache_max_bytes", &[], c.max_bytes);
        o.head("ufop_volume_cache_entries", "gauge", "Volumes in the cache.");
        o.sample("ufop_volume_cache_entries", &[], c.entries);
        o.head("ufop_volume_cache_hits_total", "counter", "Cache hits since start.");
        o.sample("ufop_volume_cache_hits_total", &[], c.hits);
        o.head("ufop_volume_cache_misses_total", "counter", "Cache misses since start.");
        o.sample("ufop_volume_cache_misses_total", &[], c.misses);
        o.head("ufop_volume_cache_evictions_total", "counter", "Volumes evicted since start.");
        o.sample("ufop_volume_cache_evictions_total", &[], c.evictions);
    }
    o.0
}

pub const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// `GET /api/v1/metrics` (signed in, `read` scope).
pub(crate) async fn metrics_route(State(st): State<ServerState>) -> Response {
    ([(header::CONTENT_TYPE, CONTENT_TYPE)], render(&st).await).into_response()
}

/// `GET /metrics` on `--metrics-listen`, optionally behind HTTP Basic.
async fn standalone(State((st, basic)): State<(ServerState, Option<String>)>, h: HeaderMap) -> Response {
    if let Some(expected) = &basic {
        use base64::Engine;
        let want = format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(expected));
        let got = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).unwrap_or("");
        if !super::constant_eq(got, &want) {
            return (StatusCode::UNAUTHORIZED, [(header::WWW_AUTHENTICATE, "Basic realm=\"ufop-metrics\"")], "Unauthorized\n").into_response();
        }
    }
    ([(header::CONTENT_TYPE, CONTENT_TYPE)], render(&st).await).into_response()
}

/// The separate metrics listener's router.
pub fn standalone_router(st: ServerState, basic_auth: Option<String>) -> axum::Router {
    axum::Router::new().route("/metrics", axum::routing::get(standalone)).with_state((st, basic_auth))
}

/// Serve `/metrics` on `addr` until the server stops.
pub async fn serve_standalone(st: ServerState, addr: std::net::SocketAddr, basic_auth: Option<String>) -> Result<(), crate::core::error::AppError> {
    let listener = tokio::net::TcpListener::bind(addr).await.map_err(|e| crate::core::error::AppError::Configuration { message: format!("Cannot listen for metrics on {addr}: {e}"), advice: "Pick another --metrics-listen address.".into() })?;
    tracing::info!("Prometheus metrics on http://{addr}/metrics{}", if basic_auth.is_some() { " (basic auth)" } else { "" });
    let life = st.life.clone();
    axum::serve(listener, standalone_router(st, basic_auth))
        .with_graceful_shutdown(async move { life.requested().await })
        .await
        .map_err(|e| crate::core::error::AppError::Configuration { message: format!("Metrics listener failed: {e}"), advice: String::new() })
}

#[cfg(test)]
pub(crate) mod parse {
    //! A small parser for the text format, for tests.
    use std::collections::BTreeMap;

    #[derive(Debug, Clone, PartialEq)]
    pub struct Sample {
        pub name: String,
        pub labels: BTreeMap<String, String>,
        pub value: f64,
    }

    pub fn parse(text: &str) -> (BTreeMap<String, String>, Vec<Sample>) {
        let mut types = BTreeMap::new();
        let mut out = Vec::new();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("# TYPE ") {
                let (n, t) = rest.split_once(' ').expect("TYPE line");
                types.insert(n.to_string(), t.to_string());
                continue;
            }
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            let (head, value) = line.rsplit_once(' ').expect("sample line");
            let (name, labels) = match head.split_once('{') {
                Some((n, l)) => (n.to_string(), parse_labels(l.strip_suffix('}').expect("closing brace"))),
                None => (head.to_string(), BTreeMap::new()),
            };
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':'), "bad metric name {name}");
            out.push(Sample { name, labels, value: value.parse().unwrap_or_else(|_| panic!("bad value in {line}")) });
        }
        (types, out)
    }

    fn parse_labels(s: &str) -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        let mut chars = s.chars().peekable();
        while chars.peek().is_some() {
            let key: String = chars.by_ref().take_while(|c| *c != '=').collect();
            assert_eq!(chars.next(), Some('"'));
            let mut val = String::new();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => match chars.next() {
                        Some('n') => val.push('\n'),
                        Some(o) => val.push(o),
                        None => {}
                    },
                    '"' => break,
                    o => val.push(o),
                }
            }
            m.insert(key, val);
            if chars.peek() == Some(&',') {
                chars.next();
            }
        }
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_and_histogram_buckets() {
        assert_eq!(esc("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
        let m = Metrics::default();
        m.observe_http("GET", "/api/v1/jobs", 200, Duration::from_millis(3));
        m.observe_http("GET", "/api/v1/jobs", 200, Duration::from_millis(300));
        let h = m.http.lock().unwrap();
        let h = &h[&("GET".to_string(), "/api/v1/jobs".to_string(), 200)];
        assert_eq!((h.total, h.counts[0], h.counts[BUCKETS.len() - 1]), (2, 1, 2));
        assert_eq!(h.counts[BUCKETS.iter().position(|b| *b == 0.25).unwrap()], 1);
        let (_, samples) = parse::parse("x{a=\"q\\\"uote\",b=\"2\"} 1.5\ny 3\n");
        assert_eq!(samples[0].labels["a"], "q\"uote");
        assert_eq!(samples[1].value, 3.0);
    }

    #[test]
    fn series_are_bounded() {
        let m = Metrics::default();
        for i in 0..MAX_SERIES + 10 {
            m.observe_http("GET", &format!("/r{i}"), 200, Duration::ZERO);
        }
        let h = m.http.lock().unwrap();
        assert_eq!(h.len(), MAX_SERIES + 1);
        assert_eq!(h[&("GET".to_string(), "other".to_string(), 200)].total, 10);
    }
}
