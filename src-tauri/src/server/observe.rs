//! Request tracing for `ufop-server`.
//!
//! Every request gets a request id: a safe client-supplied `X-Request-Id`
//! (1–128 characters of `A-Z a-z 0-9 . _ : / + = -`) or a new UUIDv4. It is
//! echoed in the response, added to JSON error bodies (`request_id`) and
//! carried by the request's tracing span (`http.request`, from tower-http's
//! `TraceLayer`) together with the method, the **route template** (never the
//! raw path, which can hold share tokens), the status and the latency.
//!
//! Logged lines never contain credentials: headers appear only at debug
//! level with `Authorization`, cookies, pre-auth and CSRF values redacted,
//! and query strings are logged with sensitive parameters replaced.
//! `--request-log off` turns the per-request line off (spans stay).

use super::ServerState;
use axum::body::Body;
use axum::extract::{MatchedPath, Request, State};
use axum::http::{header, HeaderMap, HeaderName, HeaderValue};
use axum::middleware::Next;
use axum::response::Response;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

pub const REQUEST_ID: HeaderName = HeaderName::from_static("x-request-id");

/// The request's id (request extension).
#[derive(Debug, Clone, PartialEq)]
pub struct RequestId(pub String);

/// Filled with the matched route template once routing is done.
#[derive(Debug, Clone, Default)]
pub struct RouteSlot(pub Arc<OnceLock<String>>);

impl RouteSlot {
    pub fn get(&self) -> &str {
        self.0.get().map(String::as_str).unwrap_or("unmatched")
    }
}

/// A client-supplied id we can safely log and echo.
pub fn valid_request_id(s: &str) -> bool {
    (1..=128).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b"._:/+=-".contains(&b))
}

pub fn request_id_of(h: &HeaderMap) -> String {
    h.get(&REQUEST_ID).and_then(|v| v.to_str().ok()).filter(|v| valid_request_id(v)).map(str::to_string).unwrap_or_else(|| uuid::Uuid::new_v4().to_string())
}

/// Query parameters whose values are never logged.
const SECRET_PARAMS: &[&str] = &[
    "token", "access_token", "refresh_token", "id_token", "authid", "code", "password", "pass", "passphrase", "secret", "client_secret", "key", "api_key", "apikey",
    "signature", "sig", "x-amz-signature", "x-amz-credential", "x-amz-security-token", "se", "sp", "sv", "st", "state", "csrf", "session",
];

fn secret_param(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    SECRET_PARAMS.contains(&n.as_str()) || n.ends_with("token") || n.ends_with("secret") || n.ends_with("password")
}

/// `a=1&token=abc` → `a=1&token=REDACTED`.
pub fn redact_query(q: &str) -> String {
    q.split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| match kv.split_once('=') {
            Some((k, _)) if secret_param(k) => format!("{k}=REDACTED"),
            _ if secret_param(kv) => format!("{kv}=REDACTED"),
            _ => kv.to_string(),
        })
        .collect::<Vec<_>>()
        .join("&")
}

/// Headers for debug logs, credentials replaced.
pub fn redacted_headers(h: &HeaderMap) -> String {
    let secret = |n: &str| matches!(n, "authorization" | "proxy-authorization" | "cookie" | "set-cookie" | "x-csrf-token" | "x-api-key") || n == super::PREAUTH_HEADER;
    h.iter()
        .map(|(k, v)| {
            let n = k.as_str();
            let val = if secret(n) { "REDACTED".to_string() } else { v.to_str().unwrap_or("<binary>").to_string() };
            format!("{n}: {val}")
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Outermost: assign the request id, record the route, echo the id and put
/// it into JSON error bodies.
pub(crate) async fn request_context(State(_st): State<ServerState>, mut req: Request, next: Next) -> Response {
    let id = request_id_of(req.headers());
    let slot = RouteSlot::default();
    req.extensions_mut().insert(RequestId(id.clone()));
    req.extensions_mut().insert(slot.clone());
    let res = next.run(req).await;
    let mut res = if res.status().as_u16() >= 400 { with_request_id_in_body(res, &id).await } else { res };
    if let Ok(v) = HeaderValue::from_str(&id) {
        res.headers_mut().insert(REQUEST_ID, v);
    }
    res
}

/// Inside routing: the matched route template goes to the slot and the span.
pub(crate) async fn record_route(req: Request, next: Next) -> Response {
    if let Some(p) = req.extensions().get::<MatchedPath>().map(|p| p.as_str().to_string()) {
        tracing::Span::current().record("http.route", p.as_str());
        if let Some(slot) = req.extensions().get::<RouteSlot>() {
            let _ = slot.0.set(p);
        }
    }
    next.run(req).await
}

async fn with_request_id_in_body(res: Response, id: &str) -> Response {
    let is_json = res.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|c| c.starts_with("application/json"));
    let small = axum::body::HttpBody::size_hint(res.body()).upper().is_some_and(|n| n <= 64 * 1024);
    if !is_json || !small {
        return res;
    }
    let (mut parts, body) = res.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, 64 * 1024).await else { return Response::from_parts(parts, Body::empty()) };
    let out = match serde_json::from_slice::<serde_json::Value>(&bytes) {
        Ok(serde_json::Value::Object(mut m)) => {
            m.insert("request_id".into(), serde_json::Value::String(id.to_string()));
            let b = serde_json::to_vec(&m).unwrap_or_else(|_| bytes.to_vec());
            parts.headers.remove(header::CONTENT_LENGTH);
            b
        }
        _ => bytes.to_vec(),
    };
    Response::from_parts(parts, Body::from(out))
}

/// The request span (tower-http `TraceLayer` `make_span_with`).
pub(crate) fn make_span(req: &Request) -> tracing::Span {
    let id = req.extensions().get::<RequestId>().map(|r| r.0.clone()).unwrap_or_default();
    tracing::info_span!(
        "http.request",
        otel.kind = "server",
        http.method = %req.method(),
        http.route = tracing::field::Empty,
        http.status_code = tracing::field::Empty,
        request_id = %id,
    )
}

/// Debug-level request line with the query and headers redacted.
pub(crate) fn on_request(req: &Request, _span: &tracing::Span) {
    tracing::debug!(query = %req.uri().query().map(redact_query).unwrap_or_default(), headers = %redacted_headers(req.headers()), "request received");
}

/// Records the status on the span and writes the per-request log line.
#[derive(Clone, Copy)]
pub(crate) struct LogResponse {
    pub enabled: bool,
}

impl<B> tower_http::trace::OnResponse<B> for LogResponse {
    fn on_response(self, res: &axum::http::Response<B>, latency: Duration, span: &tracing::Span) {
        let status = res.status().as_u16();
        span.record("http.status_code", status);
        if !self.enabled {
            return;
        }
        let ms = latency.as_secs_f64() * 1000.0;
        if status >= 500 {
            tracing::warn!(target: "ufop_server::http", status, latency_ms = format_args!("{ms:.1}"), "request failed");
        } else {
            tracing::info!(target: "ufop_server::http", status, latency_ms = format_args!("{ms:.1}"), "request");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_ids_are_validated() {
        assert!(valid_request_id("abc-123_x.y:z/9+="));
        assert!(!valid_request_id(""));
        assert!(!valid_request_id(&"a".repeat(129)));
        assert!(!valid_request_id("has space"));
        assert!(!valid_request_id("new\nline"));
        assert!(!valid_request_id("<script>"));
        let mut h = HeaderMap::new();
        h.insert(&REQUEST_ID, HeaderValue::from_static("bad id"));
        let generated = request_id_of(&h);
        assert!(uuid::Uuid::parse_str(&generated).is_ok_and(|u| u.get_version_num() == 4));
    }

    #[test]
    fn queries_and_headers_are_redacted() {
        assert_eq!(redact_query("path=/a&token=abc&limit=5"), "path=/a&token=REDACTED&limit=5");
        assert_eq!(redact_query("authid=xyz&X-Amz-Signature=deadbeef&refresh_token&ok=1"), "authid=REDACTED&X-Amz-Signature=REDACTED&refresh_token=REDACTED&ok=1");
        assert_eq!(redact_query("my_api_token=1&db_password=2&client_secret=3"), "my_api_token=REDACTED&db_password=REDACTED&client_secret=REDACTED");
        let mut h = HeaderMap::new();
        h.insert(header::AUTHORIZATION, HeaderValue::from_static("Bearer ufop_pat_secret"));
        h.insert(header::COOKIE, HeaderValue::from_static("ufop_refresh=r; ufop_csrf=c"));
        h.insert(HeaderName::from_static(super::super::PREAUTH_HEADER), HeaderValue::from_static("proxy-secret"));
        h.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        let s = redacted_headers(&h);
        assert!(!s.contains("ufop_pat_secret") && !s.contains("ufop_refresh=r") && !s.contains("proxy-secret"), "{s}");
        assert!(s.contains("accept: application/json"));
    }
}
