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

/// (method, route template, status) → latency histogram.
type HttpSeries = BTreeMap<(String, String, u16), Histogram>;

/// HTTP request counters and histograms (cheap to clone).
#[derive(Clone, Default)]
pub struct Metrics {
    http: Arc<Mutex<HttpSeries>>,
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

pub use crate::backup_engine::metrics::esc;
use crate::backup_engine::metrics::Exposition as Out;

/// The whole exposition.
pub async fn render(st: &ServerState) -> String {
    let mut o = Out::default();
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

    crate::backup_engine::metrics::render_backup(&st.mgr, &mut o).await;
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
