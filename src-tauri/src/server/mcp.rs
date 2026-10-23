//! The MCP endpoint (`POST /mcp`, Streamable HTTP with JSON responses):
//! the same file tools as `ufop mcp` (`ufop_core::mcp`), confined to
//! `--mcp-root` folders. Clients authenticate with a bearer token — an
//! OAuth access token from `mcp_oauth`, an API token or a session; a
//! request without one gets 401 with a pointer to the OAuth metadata.
//! Write tools exist only with `--mcp-allow-write` and a token that may
//! write. Requests from browser pages of other origins are refused (DNS
//! rebinding).

use super::api_tokens::{Principal, Scope};
use super::{mcp_oauth, ServerState};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde_json::{json, Value};
use ufop_core::mcp::{Roots, Server};

const MAX_READ_BYTES: u64 = 4 * 1024 * 1024;

pub fn routes() -> Router<ServerState> {
    Router::new().route("/mcp", post(handle).get(no_stream).delete(no_stream))
}

/// No server-initiated stream and no sessions to end.
async fn no_stream(State(st): State<ServerState>) -> Response {
    if st.cfg.mcp_roots.is_empty() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut r = StatusCode::METHOD_NOT_ALLOWED.into_response();
    r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("POST"));
    r
}

fn unauthorized(st: &ServerState, headers: &HeaderMap, description: &str) -> Response {
    let challenge = format!(
        "Bearer resource_metadata=\"{}\", scope=\"{}\", error=\"invalid_token\", error_description=\"{}\"",
        mcp_oauth::metadata_url(st, headers),
        if st.cfg.mcp_allow_write { format!("{} {}", mcp_oauth::SCOPE_READ, mcp_oauth::SCOPE_WRITE) } else { mcp_oauth::SCOPE_READ.to_string() },
        description.replace('"', "'"),
    );
    let mut r = (StatusCode::UNAUTHORIZED, Json(json!({ "error": "invalid_token", "error_description": description }))).into_response();
    if let Ok(v) = HeaderValue::from_str(&challenge) {
        r.headers_mut().insert(header::WWW_AUTHENTICATE, v);
    }
    r
}

async fn handle(State(st): State<ServerState>, headers: HeaderMap, body: Bytes) -> Response {
    if st.cfg.mcp_roots.is_empty() {
        return (StatusCode::NOT_FOUND, Json(json!({ "error": "MCP is not enabled on this server (--mcp-root)" }))).into_response();
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|o| o.to_str().ok()) {
        let own = mcp_oauth::base_url(&st, &headers);
        if origin != own && !st.cfg.cors_origins.iter().any(|o| o == origin) {
            return (StatusCode::FORBIDDEN, Json(json!({ "error": format!("Origin {origin} may not use this MCP endpoint") }))).into_response();
        }
    }
    let principal = match super::authenticate(&st, &headers, &Method::POST, "/mcp").await {
        Ok(p) => p,
        Err(e) => return unauthorized(&st, &headers, &e.user_message()),
    };
    let may_write = match &principal {
        Principal::Session => true,
        Principal::ApiToken { scope, .. } => *scope >= Scope::Write,
    };
    let roots = match Roots::new(&st.cfg.mcp_roots) {
        Ok(r) => r,
        Err(e) => return (StatusCode::SERVICE_UNAVAILABLE, Json(json!({ "error": format!("MCP roots: {e}") }))).into_response(),
    };
    let server = Server { roots, allow_write: st.cfg.mcp_allow_write && may_write, max_read_bytes: MAX_READ_BYTES };
    let msg: Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return Json(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } })).into_response(),
    };
    let reply = tokio::task::spawn_blocking(move || match msg {
        Value::Array(batch) => {
            let replies: Vec<Value> = batch.iter().filter_map(|m| server.handle(m)).collect();
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        m => server.handle(&m),
    })
    .await
    .unwrap_or_else(|e| Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32603, "message": format!("internal error: {e}") } })));
    match reply {
        Some(v) => Json(v).into_response(),
        // Notifications and responses: accepted, nothing to say.
        None => StatusCode::ACCEPTED.into_response(),
    }
}
