//! Inbound webhook trigger for automation rules.
//!
//! A small HTTP listener (loopback by default) accepts
//! `POST /hooks/<rule-id>` with the rule's secret as a bearer token (or a
//! `token` query parameter for tools that can't set headers). Top-level
//! scalar fields of a JSON body become template values (`{customer}`), and
//! `{payload}` holds the raw body. The secret itself is never stored: the
//! rule keeps its SHA-256, and a new one is issued by rotating.

use super::{FireContext, RuleTrigger};
use axum::extract::{Path as AxPath, Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Default port for the local listener.
pub const DEFAULT_PORT: u16 = 47321;
/// Largest request body accepted.
const MAX_BODY: usize = 64 * 1024;
/// Requests per rule per minute.
const RATE_PER_MINUTE: usize = 60;

pub fn hash_secret(secret: &str) -> String {
    hex::encode(Sha256::digest(secret.as_bytes()))
}

/// A fresh random secret (256 bits, URL-safe).
pub fn new_secret() -> String {
    use rand::RngCore;
    let mut b = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut b);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, b)
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Template values from a JSON object body: scalar top-level fields.
pub fn payload_values(body: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    out.insert("payload".to_string(), body.chars().take(MAX_BODY).collect());
    if let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(body) {
        for (k, v) in map {
            let valid = !k.is_empty() && k.len() <= 64 && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
            if !valid {
                continue;
            }
            let s = match v {
                serde_json::Value::String(s) => s,
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            out.insert(k, s.chars().take(4096).collect());
        }
    }
    out
}

#[derive(Clone)]
struct HookState {
    ctx: FireContext,
    recent: Arc<Mutex<HashMap<String, Vec<Instant>>>>,
}

fn reply(status: StatusCode, msg: &str) -> Response {
    (status, Json(serde_json::json!({ "error": msg }))).into_response()
}

async fn hook(
    State(st): State<HookState>,
    AxPath(rule_id): AxPath<String>,
    Query(q): Query<HashMap<String, String>>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if body.len() > MAX_BODY {
        return reply(StatusCode::PAYLOAD_TOO_LARGE, "Body too large");
    }
    let presented = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_string)
        .or_else(|| q.get("token").cloned());
    let Some(rule) = st.ctx.rule(&rule_id).await else {
        return reply(StatusCode::NOT_FOUND, "Unknown rule");
    };
    // Same answer for "no secret configured" and "wrong secret".
    let authorized = match (&presented, &rule.options.webhook_secret_hash) {
        (Some(p), Some(h)) => constant_eq(&hash_secret(p), h),
        _ => false,
    };
    if !authorized {
        return reply(StatusCode::UNAUTHORIZED, "Missing or wrong token");
    }
    if rule.trigger != RuleTrigger::Webhook {
        return reply(StatusCode::CONFLICT, "This rule is not triggered by webhooks");
    }
    if !rule.enabled {
        return reply(StatusCode::CONFLICT, "This rule is disabled");
    }
    {
        let mut recent = st.recent.lock().unwrap_or_else(|e| e.into_inner());
        let hits = recent.entry(rule_id.clone()).or_default();
        hits.retain(|t| t.elapsed() < Duration::from_secs(60));
        if hits.len() >= RATE_PER_MINUTE {
            return reply(StatusCode::TOO_MANY_REQUESTS, "Too many requests for this rule");
        }
        hits.push(Instant::now());
    }
    let values = payload_values(&String::from_utf8_lossy(&body));
    let ctx = st.ctx.clone();
    tokio::spawn(async move {
        if let Err(e) = ctx.fire_with(&rule_id, None, "webhook", None, values).await {
            tracing::warn!("Webhook-triggered automation failed to start: {e}");
        }
    });
    (StatusCode::ACCEPTED, Json(serde_json::json!({ "accepted": true }))).into_response()
}

pub(crate) fn router(ctx: FireContext) -> Router {
    let st = HookState { ctx, recent: Default::default() };
    Router::new()
        .route("/hooks/{rule_id}", post(hook).get(hook))
        .layer(axum::extract::DefaultBodyLimit::max(MAX_BODY))
        .with_state(st)
}

/// Bind and serve the listener; returns the bound address.
pub(crate) async fn serve(ctx: FireContext, addr: SocketAddr) -> std::io::Result<SocketAddr> {
    let listener = tokio::net::TcpListener::bind(addr).await?;
    let bound = listener.local_addr()?;
    let app = router(ctx);
    tokio::spawn(async move {
        if let Err(e) = axum::serve(listener, app).await {
            tracing::error!("Automation webhook listener stopped: {e}");
        }
    });
    Ok(bound)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_fields_become_values() {
        let v = payload_values(r#"{"customer":"acme","count":3,"ok":true,"nested":{"x":1},"bad key":"x"}"#);
        assert_eq!(v["customer"], "acme");
        assert_eq!(v["count"], "3");
        assert_eq!(v["ok"], "true");
        assert!(!v.contains_key("nested") && !v.contains_key("bad key"));
        assert!(v["payload"].starts_with('{'));
        assert_eq!(payload_values("not json").len(), 1);
    }

    #[test]
    fn secrets_are_random_and_hashed() {
        let (a, b) = (new_secret(), new_secret());
        assert_ne!(a, b);
        assert!(a.len() >= 43);
        assert_eq!(hash_secret(&a).len(), 64);
        assert!(constant_eq(&hash_secret(&a), &hash_secret(&a)));
        assert!(!constant_eq(&hash_secret(&a), &hash_secret(&b)));
    }
}
