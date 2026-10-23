//! OAuth 2.1 authorization for the `/mcp` endpoint, as the MCP
//! authorization spec asks: protected-resource metadata (RFC 9728),
//! authorization-server metadata (RFC 8414), dynamic client registration
//! for public clients (RFC 7591), the authorization-code grant with PKCE
//! (S256 only), rotating refresh tokens, and revocation (RFC 7009).
//!
//! The person approving signs in with the server password on the consent
//! page. Access tokens are ordinary scoped API tokens (`MCP: <client>`,
//! one day, `read` or `write`), so they show up — and can be revoked — with
//! the server's other API tokens. Everything here is off (404) unless MCP
//! roots are configured.

use super::api_tokens::{self, Scope};
use super::{ApiError, ServerState, TlsMode};
use crate::core::error::AppError;
use axum::extract::{Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

pub const SCOPE_READ: &str = "mcp:read";
pub const SCOPE_WRITE: &str = "mcp:write";
const CODE_TTL: Duration = Duration::from_secs(120);
const ACCESS_DAYS: i64 = 1;
const REFRESH_DAYS: i64 = 30;
const REFRESH_PREFIX: &str = "ufop_rt_";
const MAX_CLIENTS: i64 = 200;

/// Pending authorization codes (single use, two minutes).
struct Pending {
    client_id: String,
    redirect_uri: String,
    challenge: String,
    scope: Scope,
    created: Instant,
}

fn codes() -> &'static Mutex<HashMap<String, Pending>> {
    static CODES: OnceLock<Mutex<HashMap<String, Pending>>> = OnceLock::new();
    CODES.get_or_init(Default::default)
}

fn random(prefix: &str) -> String {
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    format!("{prefix}{}", hex::encode(b))
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

/// The server's public origin: `--public-url`, else from the request.
pub(crate) fn base_url(st: &ServerState, headers: &HeaderMap) -> String {
    if let Some(u) = st.cfg.public_url.as_deref().filter(|u| !u.trim().is_empty()) {
        return u.trim().trim_end_matches('/').to_string();
    }
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
    let https = !matches!(st.cfg.tls, TlsMode::None) || headers.get("x-forwarded-proto").and_then(|v| v.to_str().ok()) == Some("https");
    format!("{}://{host}", if https { "https" } else { "http" })
}

pub(crate) fn resource_url(st: &ServerState, headers: &HeaderMap) -> String {
    format!("{}/mcp", base_url(st, headers))
}

pub(crate) fn metadata_url(st: &ServerState, headers: &HeaderMap) -> String {
    format!("{}/.well-known/oauth-protected-resource", base_url(st, headers))
}

/// `Some(404)` when MCP is off.
fn disabled(st: &ServerState) -> Option<Response> {
    st.cfg.mcp_roots.is_empty().then(|| (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found", "error_description": "MCP is not enabled on this server (--mcp-root)" }))).into_response())
}

fn scopes_supported(st: &ServerState) -> Vec<&'static str> {
    if st.cfg.mcp_allow_write { vec![SCOPE_READ, SCOPE_WRITE] } else { vec![SCOPE_READ] }
}

pub fn routes() -> Router<ServerState> {
    Router::new()
        .route("/.well-known/oauth-protected-resource", get(protected_resource))
        .route("/.well-known/oauth-protected-resource/mcp", get(protected_resource))
        .route("/.well-known/oauth-authorization-server", get(authorization_server))
        .route("/oauth2/register", post(register))
        .route("/oauth2/authorize", get(authorize_page).post(authorize_submit))
        .route("/oauth2/token", post(token))
        .route("/oauth2/revoke", post(revoke))
}

async fn protected_resource(State(st): State<ServerState>, headers: HeaderMap) -> Response {
    if let Some(r) = disabled(&st) {
        return r;
    }
    Json(json!({
        "resource": resource_url(&st, &headers),
        "authorization_servers": [base_url(&st, &headers)],
        "scopes_supported": scopes_supported(&st),
        "bearer_methods_supported": ["header"],
        "resource_name": "UFOP files (MCP)",
    }))
    .into_response()
}

async fn authorization_server(State(st): State<ServerState>, headers: HeaderMap) -> Response {
    if let Some(r) = disabled(&st) {
        return r;
    }
    let base = base_url(&st, &headers);
    Json(json!({
        "issuer": base,
        "authorization_endpoint": format!("{base}/oauth2/authorize"),
        "token_endpoint": format!("{base}/oauth2/token"),
        "registration_endpoint": format!("{base}/oauth2/register"),
        "revocation_endpoint": format!("{base}/oauth2/revoke"),
        "scopes_supported": scopes_supported(&st),
        "response_types_supported": ["code"],
        "grant_types_supported": ["authorization_code", "refresh_token"],
        "code_challenge_methods_supported": ["S256"],
        "token_endpoint_auth_methods_supported": ["none"],
        "revocation_endpoint_auth_methods_supported": ["none"],
        "authorization_response_iss_parameter_supported": true,
    }))
    .into_response()
}

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    let mut r = (status, Json(json!({ "error": error, "error_description": description }))).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// https URIs, or http on a loopback host (native apps, RFC 8252).
fn redirect_allowed(uri: &str) -> bool {
    let Ok(u) = url::Url::parse(uri) else { return false };
    if u.fragment().is_some() {
        return false;
    }
    match u.scheme() {
        "https" => u.host_str().is_some(),
        "http" => matches!(u.host_str(), Some("localhost" | "127.0.0.1" | "[::1]")),
        _ => false,
    }
}

#[derive(Debug, Clone)]
struct Client {
    id: String,
    name: String,
    redirect_uris: Vec<String>,
}

async fn find_client(st: &ServerState, id: &str) -> Result<Option<Client>, AppError> {
    let id = id.to_string();
    st.auth
        .settings_pool()
        .execute(move |c| {
            use rusqlite::OptionalExtension;
            let row = c
                .query_row("SELECT client_id, client_name, redirect_uris FROM server_oauth_clients WHERE client_id = ?1", [id], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))
                .optional()?;
            Ok(row.map(|(id, name, uris)| Client { id, name, redirect_uris: serde_json::from_str(&uris).unwrap_or_default() }))
        })
        .await
}

async fn register(State(st): State<ServerState>, Json(b): Json<Value>) -> Response {
    if let Some(r) = disabled(&st) {
        return r;
    }
    let uris: Vec<String> = b["redirect_uris"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if uris.is_empty() || uris.len() > 10 || !uris.iter().all(|u| redirect_allowed(u)) {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri", "Give 1–10 redirect URIs: https, or http on localhost/127.0.0.1");
    }
    if b["token_endpoint_auth_method"].as_str().is_some_and(|m| m != "none") {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", "Only public clients (token_endpoint_auth_method none) with PKCE are supported");
    }
    let name: String = b["client_name"].as_str().unwrap_or("MCP client").trim().chars().filter(|c| !c.is_control()).take(60).collect();
    let name = if name.is_empty() { "MCP client".to_string() } else { name };
    let id = format!("mcp_{}", &uuid::Uuid::new_v4().simple().to_string()[..20]);
    let now = chrono::Utc::now();
    let (row_id, row_name, row_uris) = (id.clone(), name.clone(), serde_json::to_string(&uris).unwrap_or_default());
    let saved = st
        .auth
        .settings_pool()
        .execute(move |c| {
            let n: i64 = c.query_row("SELECT COUNT(*) FROM server_oauth_clients", [], |r| r.get(0))?;
            if n >= MAX_CLIENTS {
                return Err(AppError::validation("Too many registered MCP clients"));
            }
            c.execute("INSERT INTO server_oauth_clients (client_id, client_name, redirect_uris, created_at) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![row_id, row_name, row_uris, now.to_rfc3339()])?;
            Ok(())
        })
        .await;
    if let Err(e) = saved {
        return oauth_error(StatusCode::BAD_REQUEST, "invalid_client_metadata", &e.user_message());
    }
    tracing::info!("Registered MCP OAuth client \"{name}\" ({id})");
    (
        StatusCode::CREATED,
        Json(json!({
            "client_id": id,
            "client_name": name,
            "redirect_uris": uris,
            "token_endpoint_auth_method": "none",
            "grant_types": ["authorization_code", "refresh_token"],
            "response_types": ["code"],
            "client_id_issued_at": now.timestamp(),
        })),
    )
        .into_response()
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct AuthorizeParams {
    #[serde(default)]
    response_type: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    code_challenge: String,
    #[serde(default)]
    code_challenge_method: String,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    resource: Option<String>,
    /// Consent form fields.
    #[serde(default)]
    password: Option<String>,
    #[serde(default)]
    decision: Option<String>,
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}

fn page(status: StatusCode, body: &str) -> Response {
    let html = format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\"><title>Authorize — UFOP</title><style>body{{font:15px system-ui,sans-serif;max-width:28rem;margin:3rem auto;padding:0 1rem;color:#222}}input{{width:100%;padding:.5rem;margin:.5rem 0;box-sizing:border-box}}button{{padding:.5rem 1rem;margin-right:.5rem}}.err{{color:#b00}}code{{background:#eee;padding:0 .2rem}}</style></head><body>{body}</body></html>"
    );
    let mut r = (status, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; frame-ancestors 'none'"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// Check an authorization request; `Err` is the page to show (never a redirect to an unverified URI).
async fn check_request(st: &ServerState, headers: &HeaderMap, p: &AuthorizeParams) -> Result<(Client, Scope), Box<Response>> {
    let bad = |m: &str| Box::new(page(StatusCode::BAD_REQUEST, &format!("<h1>Can't authorize</h1><p class=err>{}</p>", esc(m))));
    let client = find_client(st, &p.client_id).await.map_err(|e| bad(&e.user_message()))?.ok_or_else(|| bad("Unknown client: register it first."))?;
    if !client.redirect_uris.contains(&p.redirect_uri) {
        return Err(bad("The redirect URI is not registered for this client."));
    }
    if p.response_type != "code" {
        return Err(bad("Only response_type=code is supported."));
    }
    if p.code_challenge_method != "S256" || !(43..=128).contains(&p.code_challenge.len()) {
        return Err(bad("PKCE with code_challenge_method=S256 is required."));
    }
    if let Some(r) = p.resource.as_deref().filter(|r| !r.is_empty()) {
        if r.trim_end_matches('/') != resource_url(st, headers) {
            return Err(bad("The resource is not this server's MCP endpoint."));
        }
    }
    let requested: Vec<&str> = p.scope.as_deref().unwrap_or(SCOPE_READ).split_whitespace().collect();
    if let Some(u) = requested.iter().find(|s| **s != SCOPE_READ && **s != SCOPE_WRITE) {
        return Err(bad(&format!("Unknown scope {u}.")));
    }
    // Writing is granted only when asked for and the server allows it.
    let scope = if requested.contains(&SCOPE_WRITE) && st.cfg.mcp_allow_write { Scope::Write } else { Scope::Read };
    Ok((client, scope))
}

fn consent(p: &AuthorizeParams, client: &Client, scope: Scope, error: Option<&str>) -> String {
    let hidden: String = [
        ("response_type", p.response_type.as_str()),
        ("client_id", p.client_id.as_str()),
        ("redirect_uri", p.redirect_uri.as_str()),
        ("code_challenge", p.code_challenge.as_str()),
        ("code_challenge_method", p.code_challenge_method.as_str()),
        ("state", p.state.as_deref().unwrap_or("")),
        ("scope", p.scope.as_deref().unwrap_or(SCOPE_READ)),
        ("resource", p.resource.as_deref().unwrap_or("")),
    ]
    .iter()
    .map(|(k, v)| format!("<input type=hidden name={k} value=\"{}\">", esc(v)))
    .collect();
    let host = url::Url::parse(&p.redirect_uri).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_default();
    format!(
        "<h1>Allow <b>{}</b>?</h1><p>It wants to use this server's file tools through MCP: <b>{}</b>.</p><p>After approval you return to <code>{}</code>.</p>{}<form method=post action=/oauth2/authorize>{hidden}<label>Server password<input type=password name=password autocomplete=current-password autofocus></label><button name=decision value=allow>Allow</button><button name=decision value=deny>Deny</button></form>",
        esc(&client.name),
        if scope == Scope::Write { "read and change files in the shared folders" } else { "read files in the shared folders" },
        esc(&host),
        error.map(|e| format!("<p class=err>{}</p>", esc(e))).unwrap_or_default(),
    )
}

async fn authorize_page(State(st): State<ServerState>, headers: HeaderMap, Query(p): Query<AuthorizeParams>) -> Response {
    if let Some(r) = disabled(&st) {
        return r;
    }
    match check_request(&st, &headers, &p).await {
        Ok((client, scope)) => page(StatusCode::OK, &consent(&p, &client, scope, None)),
        Err(r) => *r,
    }
}

fn redirect_with(uri: &str, params: &[(&str, &str)]) -> Response {
    let mut u = match url::Url::parse(uri) {
        Ok(u) => u,
        Err(_) => return page(StatusCode::BAD_REQUEST, "<p class=err>Invalid redirect URI</p>"),
    };
    {
        let mut q = u.query_pairs_mut();
        for (k, v) in params {
            q.append_pair(k, v);
        }
    }
    let mut r = StatusCode::SEE_OTHER.into_response();
    if let Ok(v) = HeaderValue::from_str(u.as_str()) {
        r.headers_mut().insert(header::LOCATION, v);
    }
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

async fn authorize_submit(State(st): State<ServerState>, headers: HeaderMap, Form(p): Form<AuthorizeParams>) -> Response {
    if let Some(r) = disabled(&st) {
        return r;
    }
    let (client, scope) = match check_request(&st, &headers, &p).await {
        Ok(v) => v,
        Err(r) => return *r,
    };
    let state = p.state.clone().unwrap_or_default();
    let iss = base_url(&st, &headers);
    let mut back: Vec<(&str, &str)> = vec![("iss", iss.as_str())];
    if !state.is_empty() {
        back.push(("state", state.as_str()));
    }
    if p.decision.as_deref() != Some("allow") {
        back.insert(0, ("error", "access_denied"));
        return redirect_with(&p.redirect_uri, &back);
    }
    match st.auth.login(p.password.as_deref().unwrap_or("")).await {
        Ok(Some(true)) => {}
        Ok(Some(false)) => return page(StatusCode::UNAUTHORIZED, &consent(&p, &client, scope, Some("Wrong password."))),
        Ok(None) => return page(StatusCode::TOO_MANY_REQUESTS, &consent(&p, &client, scope, Some("Too many attempts; wait a moment."))),
        Err(e) => return page(StatusCode::CONFLICT, &format!("<p class=err>{}</p>", esc(&e.user_message()))),
    }
    let code = random("");
    {
        let mut c = codes().lock().unwrap_or_else(|e| e.into_inner());
        c.retain(|_, v| v.created.elapsed() < CODE_TTL);
        c.insert(code.clone(), Pending { client_id: client.id.clone(), redirect_uri: p.redirect_uri.clone(), challenge: p.code_challenge.clone(), scope, created: Instant::now() });
    }
    tracing::info!("Authorized MCP client \"{}\" ({}) for {}", client.name, client.id, scope.as_str());
    back.insert(0, ("code", code.as_str()));
    redirect_with(&p.redirect_uri, &back)
}

#[derive(Debug, Deserialize)]
struct TokenForm {
    grant_type: String,
    #[serde(default)]
    code: String,
    #[serde(default)]
    redirect_uri: String,
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    code_verifier: String,
    #[serde(default)]
    refresh_token: String,
}

/// A new access token (API token) and refresh token for `client`.
async fn issue(st: &ServerState, client: &Client, scope: Scope) -> Result<Value, AppError> {
    let (access, raw) = st.auth.api_tokens().create(&format!("MCP: {}", client.name), scope, ACCESS_DAYS).await?;
    let refresh = random(REFRESH_PREFIX);
    let (hash, cid, sc, aid) = (sha256_hex(&refresh), client.id.clone(), scope.as_str().to_string(), access.id.clone());
    let expires = (chrono::Utc::now() + chrono::Duration::days(REFRESH_DAYS)).to_rfc3339();
    st.auth
        .settings_pool()
        .execute(move |c| {
            c.execute("INSERT INTO server_oauth_refresh (token_hash, client_id, scope, access_token_id, expires_at) VALUES (?1, ?2, ?3, ?4, ?5)", rusqlite::params![hash, cid, sc, aid, expires])?;
            Ok(())
        })
        .await?;
    Ok(json!({
        "access_token": raw,
        "token_type": "Bearer",
        "expires_in": ACCESS_DAYS * 86_400,
        "refresh_token": refresh,
        "scope": if scope == Scope::Write { format!("{SCOPE_READ} {SCOPE_WRITE}") } else { SCOPE_READ.to_string() },
    }))
}

fn token_ok(v: Value) -> Response {
    let mut r = Json(v).into_response();
    r.headers_mut().insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

/// Revoke a refresh token (by hash) and the access token it issued; returns its client and scope.
async fn take_refresh(st: &ServerState, hash: String) -> Result<Option<(String, String, bool)>, AppError> {
    let found = st
        .auth
        .settings_pool()
        .execute(move |c| {
            use rusqlite::OptionalExtension;
            let row = c
                .query_row("SELECT client_id, scope, access_token_id, expires_at, revoked FROM server_oauth_refresh WHERE token_hash = ?1", [&hash], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, r.get::<_, String>(3)?, r.get::<_, i64>(4)? != 0))
                })
                .optional()?;
            if row.is_some() {
                c.execute("UPDATE server_oauth_refresh SET revoked = 1 WHERE token_hash = ?1", [&hash])?;
            }
            Ok(row)
        })
        .await?;
    let Some((client, scope, access_id, expires, revoked)) = found else { return Ok(None) };
    st.auth.api_tokens().revoke(&access_id).await?;
    let live = !revoked && chrono::DateTime::parse_from_rfc3339(&expires).is_ok_and(|e| e > chrono::Utc::now());
    Ok(Some((client, scope, live)))
}

async fn token(State(st): State<ServerState>, Form(f): Form<TokenForm>) -> Response {
    if let Some(r) = disabled(&st) {
        return r;
    }
    let client = match find_client(&st, &f.client_id).await {
        Ok(Some(c)) => c,
        Ok(None) => return oauth_error(StatusCode::UNAUTHORIZED, "invalid_client", "Unknown client_id"),
        Err(e) => return ApiError(e).into_response(),
    };
    let scope = match f.grant_type.as_str() {
        "authorization_code" => {
            let pending = codes().lock().unwrap_or_else(|e| e.into_inner()).remove(&f.code);
            let Some(p) = pending.filter(|p| p.created.elapsed() < CODE_TTL) else {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "The code is unknown, used or expired");
            };
            if p.client_id != client.id || p.redirect_uri != f.redirect_uri {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "The code was issued to another client or redirect URI");
            }
            let computed = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(Sha256::digest(f.code_verifier.as_bytes()));
            if f.code_verifier.len() < 43 || computed != p.challenge {
                return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "The PKCE code_verifier does not match");
            }
            p.scope
        }
        "refresh_token" => match take_refresh(&st, sha256_hex(&f.refresh_token)).await {
            Ok(Some((cid, scope, true))) if cid == client.id => if scope == "write" { Scope::Write } else { Scope::Read },
            Ok(_) => return oauth_error(StatusCode::BAD_REQUEST, "invalid_grant", "The refresh token is unknown, used, expired or for another client"),
            Err(e) => return ApiError(e).into_response(),
        },
        _ => return oauth_error(StatusCode::BAD_REQUEST, "unsupported_grant_type", "Use authorization_code or refresh_token"),
    };
    // The server may have stopped allowing writes since.
    let scope = if scope == Scope::Write && !st.cfg.mcp_allow_write { Scope::Read } else { scope };
    match issue(&st, &client, scope).await {
        Ok(v) => token_ok(v),
        Err(e) => oauth_error(StatusCode::BAD_REQUEST, "invalid_request", &e.user_message()),
    }
}

#[derive(Debug, Deserialize)]
struct RevokeForm {
    token: String,
}

/// RFC 7009: revoke a refresh token (and its access token) or an access token. Always 200.
async fn revoke(State(st): State<ServerState>, Form(f): Form<RevokeForm>) -> Response {
    if let Some(r) = disabled(&st) {
        return r;
    }
    if f.token.starts_with(REFRESH_PREFIX) {
        let _ = take_refresh(&st, sha256_hex(&f.token)).await;
    } else if f.token.starts_with(api_tokens::PREFIX) {
        if let Ok(t) = st.auth.api_tokens().validate(&f.token).await {
            if t.name.starts_with("MCP: ") {
                let _ = st.auth.api_tokens().revoke(&t.id).await;
            }
        }
    }
    StatusCode::OK.into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_uris_are_https_or_loopback() {
        assert!(redirect_allowed("https://claude.ai/api/mcp/auth_callback"));
        assert!(redirect_allowed("http://127.0.0.1:33418/callback"));
        assert!(redirect_allowed("http://localhost:6274/oauth/callback"));
        assert!(!redirect_allowed("http://evil.example.com/cb"));
        assert!(!redirect_allowed("myapp://cb"));
        assert!(!redirect_allowed("https://a.example.com/cb#frag"));
        assert!(!redirect_allowed("not a url"));
        assert_eq!(esc("<a href=\"x\">'"), "&lt;a href=&quot;x&quot;&gt;&#39;");
    }
}
