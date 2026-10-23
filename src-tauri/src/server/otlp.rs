//! OpenTelemetry trace export (OTLP/HTTP JSON) and the Prometheus
//! push-gateway client, without extra crates.
//!
//! **Traces.** [`OtlpLayer`] is a `tracing-subscriber` layer that turns the
//! server's `http.request` spans and the backup manager's `backup.operation`
//! spans into OTLP spans (trace and span ids, parent, kind, start and end
//! times, attributes from the span fields, error status) and hands them to
//! a bounded queue. A background task posts them in batches to
//! `{endpoint}/v1/traces` as `resourceSpans → scopeSpans → spans`, with the
//! configured headers. A full queue drops spans (counted) instead of
//! slowing requests down. Incoming W3C `traceparent` headers make the
//! request span a child of the caller's span; otherwise a new trace starts
//! and is kept with probability `sample_ratio` (children follow their
//! parent's decision).
//!
//! **Push gateway.** [`spawn_pushgateway`] PUTs the `/metrics` text to
//! `{url}/metrics/job/{job}/instance/{instance}` every interval, with HTTP
//! Basic or a bearer token, and once more when the server stops.

use super::ServerState;
use axum::http::HeaderMap;
use rand::RngCore;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::sync::{mpsc, oneshot};
use tracing::span::{Attributes, Id, Record};
use tracing::Subscriber;
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

/// Spans waiting for export; more are dropped.
pub const QUEUE: usize = 4096;
/// Spans per request to the collector.
pub const BATCH: usize = 512;

#[derive(Debug, Clone)]
pub struct OtlpConfig {
    /// Collector base URL (`http://otel:4318`) or the full `/v1/traces` URL.
    pub endpoint: String,
    /// Extra request headers (`api-key`, `authorization`, …).
    pub headers: Vec<(String, String)>,
    pub service_name: String,
    /// Share of new traces kept, 0.0–1.0.
    pub sample_ratio: f64,
    /// Longest wait before a partial batch is sent.
    pub flush_interval: Duration,
}

impl Default for OtlpConfig {
    fn default() -> Self {
        Self { endpoint: String::new(), headers: Vec::new(), service_name: "ufop-server".into(), sample_ratio: 1.0, flush_interval: Duration::from_secs(2) }
    }
}

/// `k=v,k2=v2` (the `OTEL_EXPORTER_OTLP_HEADERS` syntax; values may be %-encoded).
pub fn parse_headers(s: &str) -> Result<Vec<(String, String)>, String> {
    s.split(',')
        .map(str::trim)
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').ok_or_else(|| format!("'{kv}' is not name=value"))?;
            let v = percent_decode(v.trim());
            let k = k.trim().to_string();
            if k.is_empty() || axum::http::HeaderName::from_bytes(k.as_bytes()).is_err() || axum::http::HeaderValue::from_str(&v).is_err() {
                return Err(format!("'{k}' is not a valid header"));
            }
            Ok((k, v))
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 3 <= b.len() {
            if let Some(x) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(x);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A valid W3C `traceparent`: `(trace id, parent span id, sampled)`.
pub fn parse_traceparent(v: &str) -> Option<([u8; 16], [u8; 8], bool)> {
    let parts: Vec<&str> = v.trim().split('-').collect();
    if parts.len() < 4 || parts[0].len() != 2 || parts[0] == "ff" || parts[1].len() != 32 || parts[2].len() != 16 || parts[3].len() != 2 {
        return None;
    }
    if parts[0] == "00" && parts.len() != 4 {
        return None;
    }
    let mut t = [0u8; 16];
    let mut p = [0u8; 8];
    hex::decode_to_slice(parts[1], &mut t).ok()?;
    hex::decode_to_slice(parts[2], &mut p).ok()?;
    let flags = u8::from_str_radix(parts[3], 16).ok()?;
    if t == [0; 16] || p == [0; 8] || parts[1].chars().any(|c| c.is_ascii_uppercase()) {
        return None;
    }
    Some((t, p, flags & 1 == 1))
}

/// The request's `traceparent`, when valid (recorded on the request span).
pub fn remote_parent(h: &HeaderMap) -> Option<String> {
    let v = h.get("traceparent")?.to_str().ok()?;
    parse_traceparent(v).map(|_| v.trim().to_string())
}

/// One finished span, ready for the wire.
#[derive(Debug, Clone)]
pub struct SpanRecord {
    pub trace_id: [u8; 16],
    pub span_id: [u8; 8],
    pub parent_span_id: Option<[u8; 8]>,
    pub name: String,
    /// 1 internal, 2 server.
    pub kind: u8,
    pub start_ns: u128,
    pub end_ns: u128,
    pub attributes: Vec<(String, Value)>,
    pub error: Option<String>,
}

/// Per-span state kept in the registry.
struct SpanState {
    trace_id: [u8; 16],
    span_id: [u8; 8],
    parent: Option<[u8; 8]>,
    sampled: bool,
    start_ns: u128,
    attributes: Vec<(String, Value)>,
}

fn now_ns() -> u128 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0)
}

fn random<const N: usize>() -> [u8; N] {
    let mut b = [0u8; N];
    while b == [0u8; N] {
        rand::rngs::OsRng.fill_bytes(&mut b);
    }
    b
}

/// Field visitor: span fields become OTLP attributes (`otel.*` and the propagation field are not).
struct Fields<'a>(&'a mut Vec<(String, Value)>);

impl Fields<'_> {
    fn put(&mut self, k: &str, v: Value) {
        if k == "traceparent" && v.as_str().is_some_and(str::is_empty) {
            return;
        }
        match self.0.iter_mut().find(|(n, _)| n == k) {
            Some(slot) => slot.1 = v,
            None => self.0.push((k.to_string(), v)),
        }
    }
}

impl tracing::field::Visit for Fields<'_> {
    fn record_str(&mut self, f: &tracing::field::Field, v: &str) {
        self.put(f.name(), json!(v));
    }
    fn record_i64(&mut self, f: &tracing::field::Field, v: i64) {
        self.put(f.name(), json!(v));
    }
    fn record_u64(&mut self, f: &tracing::field::Field, v: u64) {
        self.put(f.name(), json!(v));
    }
    fn record_bool(&mut self, f: &tracing::field::Field, v: bool) {
        self.put(f.name(), json!(v));
    }
    fn record_f64(&mut self, f: &tracing::field::Field, v: f64) {
        self.put(f.name(), json!(v));
    }
    fn record_debug(&mut self, f: &tracing::field::Field, v: &dyn std::fmt::Debug) {
        self.put(f.name(), json!(format!("{v:?}")));
    }
}

/// Which spans are exported.
fn exported(name: &str) -> bool {
    name == "http.request" || name.starts_with("backup.")
}

/// The layer (install it on the global subscriber).
pub struct OtlpLayer {
    tx: mpsc::Sender<Msg>,
    sample_ratio: f64,
    dropped: Arc<AtomicU64>,
}

impl<S> Layer<S> for OtlpLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        if !exported(span.name()) {
            return;
        }
        let mut attributes = Vec::new();
        attrs.record(&mut Fields(&mut attributes));
        // Nearest exported ancestor in this process, else the remote caller.
        let local = span.scope().skip(1).find_map(|a| a.extensions().get::<SpanState>().map(|p| (p.trace_id, p.span_id, p.sampled)));
        let remote = attributes.iter().find(|(k, _)| k == "traceparent").and_then(|(_, v)| v.as_str()).and_then(parse_traceparent);
        let (trace_id, parent, sampled) = match (local, remote) {
            (Some((t, p, s)), _) => (t, Some(p), s),
            (None, Some((t, p, s))) => (t, Some(p), s),
            (None, None) => {
                let t: [u8; 16] = random();
                let r = u64::from_be_bytes(t[8..16].try_into().unwrap_or([0; 8])) as f64 / u64::MAX as f64;
                (t, None, r < self.sample_ratio)
            }
        };
        attributes.retain(|(k, _)| !k.starts_with("otel.") && k != "traceparent");
        span.extensions_mut().insert(SpanState { trace_id, span_id: random(), parent, sampled, start_ns: now_ns(), attributes });
    }

    fn on_record(&self, id: &Id, values: &Record<'_>, ctx: Context<'_, S>) {
        if let Some(span) = ctx.span(id) {
            if let Some(st) = span.extensions_mut().get_mut::<SpanState>() {
                values.record(&mut Fields(&mut st.attributes));
                st.attributes.retain(|(k, _)| !k.starts_with("otel.") && k != "traceparent");
            }
        }
    }

    fn on_close(&self, id: Id, ctx: Context<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let Some(st) = span.extensions_mut().remove::<SpanState>() else { return };
        if !st.sampled {
            return;
        }
        let name = span.name();
        let attr = |k: &str| st.attributes.iter().find(|(n, _)| n == k).map(|(_, v)| v.clone());
        let error = match name {
            "http.request" => attr("http.status_code").and_then(|v| v.as_u64()).filter(|c| *c >= 500).map(|c| format!("HTTP {c}")),
            _ => attr("backup.result").and_then(|v| v.as_str().map(String::from)).filter(|r| r == "error").map(|_| attr("backup.error").and_then(|e| e.as_str().map(String::from)).unwrap_or_else(|| "failed".into())),
        };
        let display = match (name, attr("http.method"), attr("http.route")) {
            ("http.request", Some(m), Some(r)) => format!("{} {}", m.as_str().unwrap_or(""), r.as_str().unwrap_or("")),
            ("http.request", Some(m), None) => m.as_str().unwrap_or("HTTP").to_string(),
            _ => name.to_string(),
        };
        let rec = SpanRecord {
            trace_id: st.trace_id,
            span_id: st.span_id,
            parent_span_id: st.parent,
            name: display,
            kind: if name == "http.request" { 2 } else { 1 },
            start_ns: st.start_ns,
            end_ns: now_ns(),
            attributes: st.attributes,
            error,
        };
        if self.tx.try_send(Msg::Span(Box::new(rec))).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

enum Msg {
    Span(Box<SpanRecord>),
    Flush(oneshot::Sender<()>),
}

/// Flushes the exporter (shutdown) and reports its counters.
#[derive(Clone)]
pub struct OtlpHandle {
    tx: mpsc::Sender<Msg>,
    pub dropped: Arc<AtomicU64>,
    pub exported: Arc<AtomicU64>,
    pub failed: Arc<AtomicU64>,
}

impl OtlpHandle {
    /// Send everything queued; waits at most `timeout`.
    pub async fn flush(&self, timeout: Duration) -> bool {
        let (tx, rx) = oneshot::channel();
        if self.tx.send(Msg::Flush(tx)).await.is_err() {
            return false;
        }
        tokio::time::timeout(timeout, rx).await.is_ok_and(|r| r.is_ok())
    }
}

/// OTLP JSON value of an attribute.
fn any_value(v: &Value) -> Value {
    match v {
        Value::Bool(b) => json!({"boolValue": b}),
        Value::Number(n) if n.is_i64() || n.is_u64() => json!({"intValue": n.to_string()}),
        Value::Number(n) => json!({"doubleValue": n.as_f64().unwrap_or(0.0)}),
        Value::String(s) => json!({"stringValue": s}),
        other => json!({"stringValue": other.to_string()}),
    }
}

fn kv(k: &str, v: &Value) -> Value {
    json!({"key": k, "value": any_value(v)})
}

/// The `ExportTraceServiceRequest` body for a batch.
pub fn export_body(service_name: &str, spans: &[SpanRecord]) -> Value {
    let host = hostname::get().map(|h| h.to_string_lossy().into_owned()).unwrap_or_default();
    let resource = [("service.name", json!(service_name)), ("service.version", json!(env!("CARGO_PKG_VERSION"))), ("host.name", json!(host)), ("telemetry.sdk.name", json!("ufop")), ("telemetry.sdk.language", json!("rust"))];
    let spans: Vec<Value> = spans
        .iter()
        .map(|s| {
            let mut o = json!({
                "traceId": hex::encode(s.trace_id),
                "spanId": hex::encode(s.span_id),
                "name": s.name,
                "kind": s.kind,
                "startTimeUnixNano": s.start_ns.to_string(),
                "endTimeUnixNano": s.end_ns.to_string(),
                "attributes": s.attributes.iter().map(|(k, v)| kv(k, v)).collect::<Vec<_>>(),
                "status": match &s.error { Some(m) => json!({"code": 2, "message": m}), None => json!({"code": 0}) },
            });
            if let Some(p) = s.parent_span_id {
                o["parentSpanId"] = json!(hex::encode(p));
            }
            o
        })
        .collect();
    json!({"resourceSpans": [{
        "resource": {"attributes": resource.iter().map(|(k, v)| kv(k, v)).collect::<Vec<_>>()},
        "scopeSpans": [{"scope": {"name": "ufop-server", "version": env!("CARGO_PKG_VERSION")}, "spans": spans}]
    }]})
}

fn traces_url(endpoint: &str) -> String {
    let e = endpoint.trim_end_matches('/');
    if e.ends_with("/v1/traces") {
        e.to_string()
    } else {
        format!("{e}/v1/traces")
    }
}

/// The layer and its handle; spawns the export task on the current runtime.
pub fn install(cfg: OtlpConfig) -> (OtlpLayer, OtlpHandle) {
    let (tx, mut rx) = mpsc::channel::<Msg>(QUEUE);
    let dropped = Arc::new(AtomicU64::new(0));
    let handle = OtlpHandle { tx: tx.clone(), dropped: dropped.clone(), exported: Arc::default(), failed: Arc::default() };
    let layer = OtlpLayer { tx, sample_ratio: cfg.sample_ratio.clamp(0.0, 1.0), dropped };
    let (exported, failed) = (handle.exported.clone(), handle.failed.clone());
    let url = traces_url(&cfg.endpoint);
    tokio::spawn(async move {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(10)).build().unwrap_or_default();
        let mut batch: Vec<SpanRecord> = Vec::new();
        let send = |batch: Vec<SpanRecord>| {
            let (http, url, cfg, exported, failed) = (http.clone(), url.clone(), cfg.clone(), exported.clone(), failed.clone());
            async move {
                if batch.is_empty() {
                    return;
                }
                let mut req = http.post(&url).json(&export_body(&cfg.service_name, &batch));
                for (k, v) in &cfg.headers {
                    req = req.header(k.as_str(), v.as_str());
                }
                match req.send().await {
                    Ok(r) if r.status().is_success() => {
                        exported.fetch_add(batch.len() as u64, Ordering::Relaxed);
                    }
                    Ok(r) => {
                        failed.fetch_add(batch.len() as u64, Ordering::Relaxed);
                        tracing::debug!("OTLP export refused: HTTP {}", r.status());
                    }
                    Err(e) => {
                        failed.fetch_add(batch.len() as u64, Ordering::Relaxed);
                        tracing::debug!("OTLP export failed: {e}");
                    }
                }
            }
        };
        let mut tick = tokio::time::interval(cfg.flush_interval);
        loop {
            tokio::select! {
                m = rx.recv() => match m {
                    Some(Msg::Span(s)) => {
                        batch.push(*s);
                        if batch.len() >= BATCH {
                            send(std::mem::take(&mut batch)).await;
                        }
                    }
                    Some(Msg::Flush(ack)) => {
                        while let Ok(Msg::Span(s)) = rx.try_recv() {
                            batch.push(*s);
                        }
                        for chunk in std::mem::take(&mut batch).chunks(BATCH) {
                            send(chunk.to_vec()).await;
                        }
                        let _ = ack.send(());
                    }
                    None => {
                        send(std::mem::take(&mut batch)).await;
                        return;
                    }
                },
                _ = tick.tick() => send(std::mem::take(&mut batch)).await,
            }
        }
    });
    (layer, handle)
}

// ── Prometheus push gateway ──

#[derive(Debug, Clone)]
pub struct PushConfig {
    pub url: String,
    pub job: String,
    pub instance: String,
    pub interval: Duration,
    /// `user:password` for HTTP Basic.
    pub basic_auth: Option<String>,
    pub bearer: Option<String>,
}

/// A grouping-key path segment (`@base64` form when the value has a `/` or is empty).
fn group_segment(name: &str, value: &str) -> String {
    use base64::Engine;
    if value.is_empty() {
        return format!("{name}@base64/=");
    }
    if value.contains('/') {
        return format!("{name}@base64/{}", base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(value));
    }
    let enc: String = value.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect();
    format!("{name}/{enc}")
}

pub fn push_url(cfg: &PushConfig) -> String {
    format!("{}/metrics/{}/{}", cfg.url.trim_end_matches('/'), group_segment("job", &cfg.job), group_segment("instance", &cfg.instance))
}

/// One push of the current metrics.
pub async fn push_once(st: &ServerState, cfg: &PushConfig, http: &reqwest::Client) -> Result<(), String> {
    let body = super::metrics::render(st).await;
    let mut req = http.put(push_url(cfg)).header("content-type", super::metrics::CONTENT_TYPE).body(body);
    if let Some((u, p)) = cfg.basic_auth.as_deref().and_then(|b| b.split_once(':')) {
        req = req.basic_auth(u, Some(p));
    } else if let Some(t) = &cfg.bearer {
        req = req.bearer_auth(t);
    }
    let r = req.send().await.map_err(|e| e.to_string())?;
    if r.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}", r.status()))
    }
}

/// Push every interval until the server stops, then push once more.
pub fn spawn_pushgateway(st: ServerState, cfg: PushConfig) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let http = reqwest::Client::builder().timeout(Duration::from_secs(15)).build().unwrap_or_default();
        tracing::info!("Pushing metrics to {} every {}s", push_url(&cfg), cfg.interval.as_secs());
        let failures: Mutex<HashMap<&str, u32>> = Mutex::default();
        let mut tick = tokio::time::interval(cfg.interval.max(Duration::from_secs(1)));
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if let Err(e) = push_once(&st, &cfg, &http).await {
                        let n = { let mut f = failures.lock().unwrap_or_else(|e| e.into_inner()); let n = f.entry("push").or_default(); *n += 1; *n };
                        if n == 1 || n % 10 == 0 {
                            tracing::warn!("Metrics push failed ({n}x): {e}");
                        }
                    }
                }
                _ = st.life.requested() => break,
            }
        }
        // Wait for the drain, so the last push shows the final state.
        while st.mgr.active_task_count() > 0 && !st.life.is_immediate() {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        match push_once(&st, &cfg, &http).await {
            Ok(()) => tracing::info!("Final metrics push sent"),
            Err(e) => tracing::warn!("Final metrics push failed: {e}"),
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn traceparent_parsing() {
        let v = "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01";
        let (t, p, s) = parse_traceparent(v).unwrap();
        assert_eq!((hex::encode(t).as_str(), hex::encode(p).as_str(), s), ("4bf92f3577b34da6a3ce929d0e0e4736", "00f067aa0ba902b7", true));
        assert!(!parse_traceparent("00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-00").unwrap().2);
        for bad in ["", "00-00000000000000000000000000000000-00f067aa0ba902b7-01", "00-4bf92f3577b34da6a3ce929d0e0e4736-0000000000000000-01", "ff-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01", "00-4BF92F3577B34DA6A3CE929D0E0E4736-00f067aa0ba902b7-01", "00-xyz-00f067aa0ba902b7-01", "00-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-extra"] {
            assert!(parse_traceparent(bad).is_none(), "{bad}");
        }
        // Future versions may append fields.
        assert!(parse_traceparent("01-4bf92f3577b34da6a3ce929d0e0e4736-00f067aa0ba902b7-01-what").is_some());
    }

    #[test]
    fn header_list_and_urls() {
        assert_eq!(parse_headers("api-key=abc, x-tenant = t%201").unwrap(), vec![("api-key".into(), "abc".into()), ("x-tenant".into(), "t 1".into())]);
        assert!(parse_headers("novalue").is_err());
        assert!(parse_headers("bad header=1").is_err());
        assert_eq!(traces_url("http://c:4318/"), "http://c:4318/v1/traces");
        assert_eq!(traces_url("https://c/otlp/v1/traces"), "https://c/otlp/v1/traces");
        let cfg = PushConfig { url: "http://pg:9091/".into(), job: "ufop".into(), instance: "nas/1".into(), interval: Duration::from_secs(60), basic_auth: None, bearer: None };
        assert_eq!(push_url(&cfg), "http://pg:9091/metrics/job/ufop/instance@base64/bmFzLzE");
    }
}
