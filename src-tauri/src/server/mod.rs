//! `ufop-server`: headless backup server with an authenticated, versioned
//! REST API (`/api/v1`), the browser UI (or API-only), HTTPS, CORS,
//! reverse-proxy pre-authentication and an allowed-hosts check.
//!
//! The same `BackupManager` the desktop app uses runs here, so jobs,
//! scheduling, history and maintenance behave identically.

pub mod access;
pub mod accounts;
pub mod api_tokens;
pub mod auth;
pub mod branding;
pub mod csrf;
pub mod dav;
pub mod folders;
pub mod gateway;
pub mod lifecycle;
pub mod oauth;
pub mod observe;
pub mod mcp;
pub mod mcp_oauth;
pub mod metrics;
pub mod openapi;
pub mod otlp;
pub mod sdk;
pub mod shares;
pub mod tls;
pub mod users;
/// Server settings live in `security::settings_store` (shared with the backup engine).
pub use crate::security::settings_store as settings;

use crate::backup_engine::{api, BackupManager, BackupOperation};
use crate::connectors::ConnectionManager;
use crate::core::error::AppError;
use auth::AuthService;
use axum::extract::{Path as AxPath, Query, Request, State};
use axum::Extension;
use api_tokens::Principal;
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

pub const PREAUTH_HEADER: &str = "x-ufop-preauth";
pub(crate) const REFRESH_COOKIE: &str = "ufop_refresh";

#[derive(Debug, Clone)]
pub enum TlsMode {
    None,
    /// Local CA + server certificate in `dir`, auto-renewed.
    Generated { dir: PathBuf, hostnames: Vec<String> },
    /// Imported PFX/P12.
    Pfx { path: PathBuf, password: String },
}

#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub port: u16,
    /// `loopback` (default), `any`, or an IP address.
    pub interface: String,
    /// Host header values accepted (`*` = any). Loopback names are always allowed.
    pub allowed_hostnames: Vec<String>,
    pub webroot: Option<PathBuf>,
    pub api_only: bool,
    /// Path prefixes that fall back to index.html (single-page app routes).
    pub spa_paths: Vec<String>,
    pub cors_origins: Vec<String>,
    /// Tokens a trusted reverse proxy sends in `X-UFOP-PreAuth`.
    pub preauth_tokens: Vec<String>,
    pub tls: TlsMode,
    /// Client addresses allowed to connect (TCP peer; empty = any).
    pub allowed_clients: ufop_core::netacl::IpAllowlist,
    /// Lowest TLS version accepted when HTTPS is on.
    pub min_tls: tls::MinTls,
    /// Strict-Transport-Security max-age in seconds when HTTPS is on (0 = no header).
    pub hsts_max_age: u64,
    /// Add `includeSubDomains` to the HSTS header.
    pub hsts_include_subdomains: bool,
    /// Folders whose contents may be shared by link (`/s/…`); empty = sharing off.
    pub share_roots: Vec<PathBuf>,
    /// Folders served read-only over WebDAV at `/dav/<name>/`; empty = off.
    pub dav_roots: Vec<PathBuf>,
    /// Client address ranges allowed to use WebDAV (on top of `allowed_clients`; empty = any).
    pub dav_allowed_clients: ufop_core::netacl::IpAllowlist,
    /// Folders the `/mcp` endpoint's file tools may use; empty = MCP off.
    pub mcp_roots: Vec<PathBuf>,
    /// Offer MCP tools that change files (to tokens with the write scope).
    pub mcp_allow_write: bool,
    /// Public origin (`https://files.example.com`) for OAuth metadata; default: from the request.
    pub public_url: Option<String>,
    /// How long running backups get to stop at a safe point on shutdown.
    pub shutdown_grace_secs: u64,
    /// One log line per request (method, route template, status, latency, request id).
    pub request_log: bool,
    /// Separate listener for Prometheus `/metrics` (off when `None`).
    pub metrics_listen: Option<SocketAddr>,
    /// `user:password` required (HTTP Basic) on the metrics listener.
    pub metrics_basic_auth: Option<String>,
    /// Push `/metrics` to a Prometheus push gateway.
    pub pushgateway: Option<otlp::PushConfig>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: 8200,
            interface: "loopback".into(),
            allowed_hostnames: Vec::new(),
            webroot: None,
            api_only: false,
            spa_paths: vec!["/".into()],
            cors_origins: Vec::new(),
            preauth_tokens: Vec::new(),
            tls: TlsMode::None,
            allowed_clients: Default::default(),
            min_tls: tls::MinTls::Tls12,
            hsts_max_age: 31_536_000,
            hsts_include_subdomains: false,
            share_roots: Vec::new(),
            dav_roots: Vec::new(),
            mcp_roots: Vec::new(),
            mcp_allow_write: false,
            public_url: None,
            dav_allowed_clients: Default::default(),
            shutdown_grace_secs: crate::backup_engine::shutdown::DEFAULT_GRACE.as_secs(),
            request_log: true,
            metrics_listen: None,
            metrics_basic_auth: None,
            pushgateway: None,
        }
    }
}

impl ServerConfig {
    pub fn bind_addr(&self) -> Result<SocketAddr, AppError> {
        let ip: IpAddr = match self.interface.as_str() {
            "loopback" | "" => IpAddr::from([127, 0, 0, 1]),
            "any" | "*" => IpAddr::from([0, 0, 0, 0]),
            other => other.parse().map_err(|_| AppError::validation(format!("Invalid interface '{other}'")))?,
        };
        Ok(SocketAddr::new(ip, self.port))
    }

    fn host_allowed(&self, host: &str) -> bool {
        let name = host.rsplit_once(':').filter(|(h, p)| !h.is_empty() && p.chars().all(|c| c.is_ascii_digit())).map(|(h, _)| h).unwrap_or(host);
        let name = name.trim_start_matches('[').trim_end_matches(']').to_ascii_lowercase();
        matches!(name.as_str(), "localhost" | "127.0.0.1" | "::1")
            || self.allowed_hostnames.iter().any(|h| h == "*" || h.eq_ignore_ascii_case(&name))
            || (self.interface != "loopback" && self.interface == name)
    }
}

#[derive(Clone)]
pub struct ServerState {
    pub mgr: BackupManager,
    pub conns: Option<ConnectionManager>,
    pub auth: AuthService,
    pub cfg: Arc<ServerConfig>,
    pub started: Instant,
    pub oauth: Option<oauth::OAuthBroker>,
    /// Management console agent (`fleet_agent_*` RPC calls of the web UI).
    pub agent: Option<crate::fleet_agent::FleetAgent>,
    /// File gateway relay (on-premises agents and their jobs).
    pub gateways: gateway::Hub,
    /// Graceful-shutdown switch (signals, service stop, `/server/shutdown`).
    pub life: lifecycle::Lifecycle,
    /// HTTP request counters and latency histograms (`/metrics`).
    pub metrics: metrics::Metrics,
    /// OTLP trace exporter (flushed on shutdown).
    pub otlp: Option<otlp::OtlpHandle>,
    /// Web UI branding (command line, saved, console layers).
    pub branding: branding::BrandingState,
}

impl ServerState {
    /// State with no OAuth broker, console agent or gateways (set the fields after).
    pub fn new(mgr: BackupManager, conns: Option<ConnectionManager>, auth: AuthService, cfg: ServerConfig) -> Self {
        let life = lifecycle::Lifecycle::new(std::time::Duration::from_secs(cfg.shutdown_grace_secs));
        Self { mgr, conns, auth, cfg: Arc::new(cfg), started: Instant::now(), oauth: None, agent: None, gateways: Default::default(), life, metrics: Default::default(), otlp: None, branding: Default::default() }
    }
}

/// HTTP error wrapper: AppError → status + sanitized `{error_type, message, advice}`.
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(e: AppError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let msg = self.0.user_message();
        let code = match &self.0 {
            AppError::PermissionDenied { message, .. } if message.contains("not allowed") => StatusCode::FORBIDDEN,
            AppError::PermissionDenied { .. } => StatusCode::UNAUTHORIZED,
            AppError::FileOperation { .. } if msg.contains("not found") => StatusCode::NOT_FOUND,
            AppError::FileOperation { .. } | AppError::Encryption { .. } => StatusCode::BAD_REQUEST,
            AppError::State { .. } => StatusCode::CONFLICT,
            AppError::Internal { .. } => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        };
        (code, Json(serde_json::to_value(&self.0).unwrap_or_default())).into_response()
    }
}

type ApiResult = Result<Json<Value>, ApiError>;

fn ok<T: serde::Serialize>(v: T) -> ApiResult {
    Ok(Json(serde_json::to_value(v).map_err(AppError::from)?))
}

// ── Middleware ──

/// Refuse clients outside `--allowed-cidrs` (by TCP peer address, so a
/// forwarded header cannot fake it; list the reverse proxy behind one).
async fn ip_check(State(st): State<ServerState>, req: Request, next: Next) -> Response {
    if !st.cfg.allowed_clients.is_empty() {
        let peer = req.extensions().get::<axum::extract::ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
        if !peer.is_some_and(|ip| st.cfg.allowed_clients.allows(ip)) {
            let who = peer.map(|p| p.to_string()).unwrap_or_else(|| "unknown".into());
            tracing::warn!("Refused a connection from {who} (not in --allowed-cidrs)");
            return (StatusCode::FORBIDDEN, Json(json!({"error_type":"permission_denied","message":format!("Connections from {who} are not allowed"),"advice":"Add the address range with --allowed-cidrs."}))).into_response();
        }
    }
    next.run(req).await
}

/// Security headers on every response; HSTS only when served over HTTPS.
async fn security_headers(State(st): State<ServerState>, req: Request, next: Next) -> Response {
    let mut res = next.run(req).await;
    let h = res.headers_mut();
    h.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    h.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    h.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    if !matches!(st.cfg.tls, TlsMode::None) && st.cfg.hsts_max_age > 0 {
        let v = format!("max-age={}{}", st.cfg.hsts_max_age, if st.cfg.hsts_include_subdomains { "; includeSubDomains" } else { "" });
        if let Ok(v) = HeaderValue::from_str(&v) {
            h.insert(header::STRICT_TRANSPORT_SECURITY, v);
        }
    }
    res
}

async fn host_check(State(st): State<ServerState>, req: Request, next: Next) -> Response {
    let host = req.headers().get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("");
    if !st.cfg.host_allowed(host) {
        return (StatusCode::MISDIRECTED_REQUEST, Json(json!({"error_type":"host","message":format!("Host '{host}' is not allowed"),"advice":"Add it with --allowed-hostnames."}))).into_response();
    }
    next.run(req).await
}

/// Who is calling — the pre-authentication header, an API token (with
/// enough scope for `method path`), or a session/automation token — and
/// with which rights (see `access`).
pub(crate) async fn authenticate_access(st: &ServerState, h: &HeaderMap, method: &Method, path: &str) -> Result<(Principal, access::Access), AppError> {
    if let Some(t) = h.get(PREAUTH_HEADER).and_then(|v| v.to_str().ok()) {
        if !t.is_empty() && st.cfg.preauth_tokens.iter().any(|p| constant_eq(p, t)) {
            return Ok((Principal::Session, access::Access::admin()));
        }
    }
    let bearer = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::to_string);
    match bearer {
        Some(t) if t.starts_with(api_tokens::PREFIX) => {
            let tok = st.auth.api_tokens().validate(&t).await?;
            let need = api_tokens::required_scope(method, path);
            if tok.scope < need {
                return Err(AppError::PermissionDenied {
                    message: format!("The API token \"{}\" has the {} scope; this needs {}", tok.name, tok.scope.as_str(), need.as_str()),
                    advice: "Use a token with a wider scope.".into(),
                });
            }
            let limit = access::TokenLimit { id: tok.id.clone(), scope: tok.scope, job_ids: tok.job_ids.clone() };
            let acc = account_access(st, &tok.owner, Some(limit)).await?;
            Ok((Principal::ApiToken { id: tok.id, scope: tok.scope }, acc))
        }
        Some(t) => {
            let claims = st.auth.validate(&t).await?;
            Ok((Principal::Session, account_access(st, &claims.sub, None).await?))
        }
        None => Err(AppError::PermissionDenied { message: "Authentication required".into(), advice: "Sign in first.".into() }),
    }
}

/// The rights of an (enabled) account, optionally through a token.
async fn account_access(st: &ServerState, username: &str, token: Option<access::TokenLimit>) -> Result<access::Access, AppError> {
    let user = st.auth.account(username).await?;
    let rules = st.auth.users().rules_for(&user).await?;
    Ok(access::Access { user: user.username, role: user.role, rules, token })
}

async fn require_auth(State(st): State<ServerState>, mut req: Request, next: Next) -> Response {
    let (method, path) = (req.method().clone(), req.uri().path().to_string());
    match authenticate_access(&st, req.headers(), &method, &path).await {
        Ok((principal, acc)) => {
            req.extensions_mut().insert(principal);
            req.extensions_mut().insert(acc);
            next.run(req).await
        }
        Err(e) => ApiError(e).into_response(),
    }
}

pub(crate) fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub(crate) fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then(|| v.to_string())
    })
}

/// Script-visible marker that a session cookie exists, so the web UI only
/// attempts a refresh when there is something to refresh.
fn session_marker(max_age_days: i64) -> HeaderValue {
    HeaderValue::from_str(&format!("ufop_session=1; SameSite=Strict; Path=/; Max-Age={}", max_age_days * 86400)).unwrap_or(HeaderValue::from_static(""))
}

fn refresh_cookie(st: &ServerState, token: &str, max_age_days: i64) -> HeaderValue {
    let secure = if matches!(st.cfg.tls, TlsMode::None) { "" } else { "; Secure" };
    HeaderValue::from_str(&format!(
        "{REFRESH_COOKIE}={token}; HttpOnly; SameSite=Strict; Path=/api/v1/auth; Max-Age={}{secure}",
        max_age_days * 86400
    ))
    .unwrap_or(HeaderValue::from_static(""))
}

/// Script-readable double-submit CSRF cookie (see `csrf`).
fn csrf_cookie(st: &ServerState, token: &str, max_age_days: i64) -> HeaderValue {
    let secure = if matches!(st.cfg.tls, TlsMode::None) { "" } else { "; Secure" };
    HeaderValue::from_str(&format!("{}={token}; SameSite=Strict; Path=/; Max-Age={}{secure}", csrf::CSRF_COOKIE, max_age_days * 86400)).unwrap_or(HeaderValue::from_static(""))
}

/// `{access_token, expires_at, csrf_token}` with the refresh and CSRF cookies.
async fn with_session_cookies(st: &ServerState, access: auth::IssuedToken, refresh: auth::IssuedToken) -> Result<Response, ApiError> {
    let csrf = st.auth.csrf_token(&refresh.token).await?;
    let mut r = Json(json!({"access_token": access.token, "expires_at": access.expires_at, "csrf_token": csrf})).into_response();
    r.headers_mut().insert(header::SET_COOKIE, refresh_cookie(st, &refresh.token, auth::REFRESH_TTL_DAYS));
    r.headers_mut().append(header::SET_COOKIE, csrf_cookie(st, &csrf, auth::REFRESH_TTL_DAYS));
    Ok(r)
}

// ── Auth routes ──

#[derive(Deserialize)]
struct LoginBody {
    /// Account name (default `admin`, the original server password).
    #[serde(default)]
    username: Option<String>,
    password: String,
}

async fn session_response(st: &ServerState, user: &str) -> Result<Response, ApiError> {
    let access = st.auth.issue_access_for(user).await?;
    let refresh = st.auth.issue_refresh_for(user).await?;
    let mut r = with_session_cookies(st, access, refresh).await?;
    r.headers_mut().append(header::SET_COOKIE, session_marker(auth::REFRESH_TTL_DAYS));
    Ok(r)
}

async fn login(State(st): State<ServerState>, Json(b): Json<LoginBody>) -> Result<Response, ApiError> {
    let username = b.username.as_deref().map(str::trim).filter(|u| !u.is_empty()).unwrap_or(users::ADMIN);
    match st.auth.login_user(username, &b.password).await? {
        None => Ok((StatusCode::TOO_MANY_REQUESTS, Json(json!({"error_type":"throttled","message":"Too many sign-in attempts","advice":"Wait a moment and try again."}))).into_response()),
        Some(None) => {
            tracing::warn!("Failed sign-in for '{username}'");
            Err(AppError::PermissionDenied { message: "Wrong user name or password".into(), advice: "Check them (the original server password is the 'admin' account's), or ask an administrator.".into() }.into())
        }
        Some(Some(u)) => {
            tracing::info!("'{}' signed in", u.username);
            session_response(&st, &u.username).await
        }
    }
}

async fn refresh(State(st): State<ServerState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let raw = cookie_value(&headers, REFRESH_COOKIE).ok_or_else(|| AppError::PermissionDenied { message: "No session".into(), advice: "Sign in.".into() })?;
    let (access, refresh) = st.auth.refresh(&raw).await?;
    with_session_cookies(&st, access, refresh).await
}

async fn logout(State(st): State<ServerState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(raw) = cookie_value(&headers, REFRESH_COOKIE) {
        st.auth.revoke_refresh(&raw).await?;
    }
    let mut r = Json(json!({"ok": true})).into_response();
    r.headers_mut().insert(header::SET_COOKIE, refresh_cookie(&st, "", 0));
    r.headers_mut().append(header::SET_COOKIE, csrf_cookie(&st, "", 0));
    r.headers_mut().append(header::SET_COOKIE, session_marker(0));
    Ok(r)
}

#[derive(Deserialize)]
struct TokenBody {
    token: String,
}

async fn redeem_signin(State(st): State<ServerState>, Json(b): Json<TokenBody>) -> Result<Response, ApiError> {
    let user = st.auth.redeem_signin(&b.token).await?;
    st.auth.account(&user).await?;
    session_response(&st, &user).await
}

async fn signin_token(State(st): State<ServerState>, Extension(acc): Extension<access::Access>) -> ApiResult {
    ok(st.auth.issue_signin_for(&acc.user).await?)
}

async fn automation_token(State(st): State<ServerState>, Extension(acc): Extension<access::Access>) -> ApiResult {
    ok(st.auth.issue_automation_for(&acc.user).await?)
}

#[derive(Deserialize)]
struct ApiTokenBody {
    name: String,
    scope: api_tokens::Scope,
    #[serde(default = "default_token_days")]
    expires_in_days: i64,
    /// Limit the token to these jobs (ids; absent or null = every job).
    #[serde(default)]
    job_ids: Option<Vec<String>>,
}

fn default_token_days() -> i64 {
    90
}

/// Administrators see every token; others their own.
async fn list_api_tokens(State(st): State<ServerState>, Extension(acc): Extension<access::Access>) -> ApiResult {
    let mut t = st.auth.api_tokens().list().await?;
    if !acc.is_admin() {
        t.retain(|t| t.owner.eq_ignore_ascii_case(&acc.user));
    }
    ok(json!({"tokens": t}))
}

/// Minting needs an interactive sign-in: an API token cannot create more tokens.
async fn create_api_token(State(st): State<ServerState>, Extension(who): Extension<Principal>, Extension(acc): Extension<access::Access>, Json(b): Json<ApiTokenBody>) -> ApiResult {
    if who != Principal::Session {
        return Err(AppError::PermissionDenied { message: "API tokens cannot create API tokens".into(), advice: "Sign in with the server password.".into() }.into());
    }
    if b.scope == api_tokens::Scope::Admin && acc.role != access::Role::Admin {
        return Err(AppError::PermissionDenied { message: "Admin-scope tokens are not allowed for this account (administrators only)".into(), advice: "Pick the read, run or write scope.".into() }.into());
    }
    if let Some(ids) = &b.job_ids {
        for id in ids.iter().map(|i| i.trim()).filter(|i| !i.is_empty()) {
            st.mgr.get_job(id).await.map_err(|_| AppError::validation(format!("There is no job with id '{id}'")))?;
        }
    }
    let (t, raw) = st.auth.api_tokens().create_owned(&b.name, b.scope, b.expires_in_days, b.job_ids, &acc.user).await?;
    tracing::info!("Created API token {} ({}, {} scope{})", t.id, t.name, t.scope.as_str(), t.job_ids.as_ref().map(|j| format!(", {} job(s)", j.len())).unwrap_or_default());
    ok(json!({"token": raw, "info": t}))
}

async fn revoke_api_token(State(st): State<ServerState>, Extension(acc): Extension<access::Access>, AxPath(id): AxPath<String>) -> ApiResult {
    let owned = st.auth.api_tokens().list().await?.into_iter().any(|t| t.id == id && t.owner.eq_ignore_ascii_case(&acc.user));
    if !acc.is_admin() && !owned {
        return Err(AppError::FileOperation { message: format!("API token {id} not found"), advice: "List your tokens to see their ids.".into() }.into());
    }
    if !st.auth.api_tokens().revoke(&id).await? {
        return Err(AppError::FileOperation { message: format!("API token {id} not found"), advice: "List the tokens to see their ids.".into() }.into());
    }
    tracing::info!("Revoked API token {id}");
    ok(json!({"ok": true}))
}

async fn reset_jwt(State(st): State<ServerState>) -> ApiResult {
    st.auth.reset_jwt().await?;
    ok(json!({"ok": true, "message": "All existing tokens were revoked"}))
}

#[derive(Deserialize)]
struct PasswordBody {
    current_password: String,
    new_password: String,
}

/// Change the signed-in account's own password.
async fn change_password(State(st): State<ServerState>, Extension(acc): Extension<access::Access>, Json(b): Json<PasswordBody>) -> ApiResult {
    if !matches!(st.auth.login_user(&acc.user, &b.current_password).await?, Some(Some(_))) {
        return Err(AppError::PermissionDenied { message: "Wrong current password".into(), advice: "Check it and try again.".into() }.into());
    }
    st.auth.set_user_password(&acc.user, &b.new_password).await?;
    ok(json!({"ok": true}))
}

// ── General routes ──

async fn health(State(st): State<ServerState>) -> ApiResult {
    let s = st.mgr.status().await;
    ok(json!({
        "status": "ok",
        "version": env!("CARGO_PKG_VERSION"),
        "uptime_secs": st.started.elapsed().as_secs(),
        "scheduler_running": s.running,
        "scheduler_paused": s.paused,
        "active_tasks": s.tasks.iter().filter(|t| t.state == "running").count(),
        "queued_tasks": s.tasks.iter().filter(|t| t.state == "queued").count(),
    }))
}

async fn status(State(st): State<ServerState>, Extension(acc): Extension<access::Access>) -> ApiResult {
    ok(visible_status(&st, &acc).await)
}

async fn modules(State(st): State<ServerState>) -> ApiResult {
    ok(st.mgr.module_policy().modules())
}

async fn get_settings(State(st): State<ServerState>) -> ApiResult {
    ok(json!({"allow_automation_tokens": st.auth.automation_allowed().await?}))
}

#[derive(Deserialize)]
struct SettingsBody {
    allow_automation_tokens: Option<bool>,
}

async fn put_settings(State(st): State<ServerState>, Json(b): Json<SettingsBody>) -> ApiResult {
    if let Some(a) = b.allow_automation_tokens {
        st.auth.set_automation_allowed(a).await?;
    }
    get_settings(State(st)).await
}

async fn rpc(State(st): State<ServerState>, Extension(acc): Extension<access::Access>, AxPath(cmd): AxPath<String>, body: Option<Json<Value>>) -> ApiResult {
    let args = body.map(|b| b.0).unwrap_or(json!({}));
    // Administrators (without a job-limited token) may call any command.
    if !acc.is_admin() || acc.job_limited() {
        match access::rpc_need(&cmd, &args) {
            access::RpcNeed::Job { id, perm } => acc.require_job(&st.mgr, &id, perm).await?,
            access::RpcNeed::Global(g) => acc.require_global(g)?,
            access::RpcNeed::Filtered => {
                return match cmd.as_str() {
                    "backup_list_jobs" => ok(visible_jobs(&st, &acc).await),
                    "backup_status" => ok(visible_status(&st, &acc).await),
                    "backup_run_history" => ok(runs_for(&st, &acc, args.get("jobId").and_then(|v| v.as_str()).map(String::from), args.get("limit").and_then(|v| v.as_u64()).unwrap_or(50) as u32).await?),
                    "backup_cancel" => ok(cancel_for(&st, &acc, args.get("id").and_then(|v| v.as_str()).unwrap_or("")).await?),
                    _ => ok(run_all_for(&st, &acc, args.get("filter").cloned().map(serde_json::from_value).transpose().map_err(AppError::from)?.unwrap_or_default()).await?),
                };
            }
        }
    }
    if cmd == "server_list_directory" {
        let q = FsQuery { path: args.get("path").and_then(|v| v.as_str()).map(String::from), folders_only: true };
        return filesystem(Query(q)).await;
    }
    if cmd == "list_connections" {
        return match &st.conns {
            Some(c) => ok(c.list_connections().await?),
            None => ok(Vec::<Value>::new()),
        };
    }
    if cmd.starts_with("fleet_agent_") {
        return match &st.agent {
            Some(a) => Ok(Json(crate::fleet_agent::dispatch(a, &cmd, &args).await?)),
            None => Err(AppError::state("The management console agent is not available on this server", "Start ufop-server without --no-console-agent.").into()),
        };
    }
    Ok(Json(api::dispatch(&st.mgr, &cmd, &args).await?))
}

async fn list_jobs(State(st): State<ServerState>, Extension(acc): Extension<access::Access>) -> ApiResult {
    ok(visible_jobs(&st, &acc).await)
}

/// The jobs the caller may read.
async fn visible_jobs(st: &ServerState, acc: &access::Access) -> Vec<crate::backup_engine::BackupJob> {
    st.mgr.list_jobs().await.into_iter().filter(|j| acc.job_permission(&j.id, &j.name) >= access::Permission::Read).collect()
}

/// Scheduler status with only the tasks and upcoming runs of readable jobs.
async fn visible_status(st: &ServerState, acc: &access::Access) -> crate::backup_engine::SchedulerStatus {
    let mut s = st.mgr.status().await;
    if !acc.is_admin() || acc.job_limited() {
        let ids: std::collections::HashSet<String> = visible_jobs(st, acc).await.into_iter().map(|j| j.id).collect();
        s.tasks.retain(|t| ids.contains(&t.job_id));
        s.next_runs.retain(|t| ids.contains(&t.job_id));
        s.next_maintenance.retain(|t| ids.contains(&t.job_id));
    }
    s
}

/// Run all, limited to the jobs the caller may run.
async fn run_all_for(st: &ServerState, acc: &access::Access, mut filter: crate::backup_engine::runs::RunAllFilter) -> Result<crate::backup_engine::runs::RunAllReport, AppError> {
    let allowed = acc.jobs_with(&st.mgr, access::Permission::Run).await;
    let all = st.mgr.list_jobs().await;
    let mut refused = Vec::new();
    if filter.job_ids.is_empty() {
        if allowed.len() < all.len() {
            filter.job_ids = allowed.clone();
        }
    } else {
        let (ok, no): (Vec<String>, Vec<String>) = filter.job_ids.into_iter().partition(|id| allowed.contains(id) || !all.iter().any(|j| &j.id == id));
        filter.job_ids = ok;
        refused = no;
    }
    let mut report = if filter.job_ids.is_empty() && allowed.len() < all.len() { Default::default() } else { st.mgr.run_all(filter).await? };
    for id in refused {
        let name = all.iter().find(|j| j.id == id).map(|j| j.name.clone()).unwrap_or_default();
        report.skipped.push(crate::backup_engine::runs::SkippedRun { job_id: id, job_name: name, reason: "not allowed".into() });
    }
    Ok(report)
}

/// Cancel a task (or a job's tasks) the caller may run.
async fn cancel_for(st: &ServerState, acc: &access::Access, id: &str) -> Result<bool, AppError> {
    let tasks = st.mgr.status().await.tasks;
    for t in tasks.iter().filter(|t| t.task_id == id || t.job_id == id) {
        acc.require_job(&st.mgr, &t.job_id, access::Permission::Run).await?;
    }
    if tasks.iter().all(|t| t.task_id != id && t.job_id != id) {
        // A job id without tasks: still only for jobs the caller may run.
        if st.mgr.get_job(id).await.is_ok() {
            acc.require_job(&st.mgr, id, access::Permission::Run).await?;
        }
    }
    Ok(st.mgr.cancel(id))
}

#[derive(Deserialize)]
struct SaveJobBody {
    job: crate::backup_engine::BackupJob,
    passphrase: Option<String>,
}

async fn create_job(State(st): State<ServerState>, Json(mut b): Json<SaveJobBody>) -> ApiResult {
    b.job.id.clear();
    ok(st.mgr.save_job(b.job, b.passphrase).await?)
}

async fn get_job(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(st.mgr.get_job(&id).await?)
}

async fn update_job(State(st): State<ServerState>, AxPath(id): AxPath<String>, Json(mut b): Json<SaveJobBody>) -> ApiResult {
    st.mgr.get_job(&id).await?;
    b.job.id = id;
    ok(st.mgr.save_job(b.job, b.passphrase).await?)
}

#[derive(Deserialize)]
struct DeleteQuery {
    #[serde(default)]
    delete_local_db: bool,
    #[serde(default)]
    delete_remote: bool,
}

async fn delete_job(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<DeleteQuery>) -> ApiResult {
    st.mgr.delete_job(&id, q.delete_local_db, q.delete_remote).await?;
    ok(json!({"ok": true}))
}

#[derive(Deserialize, Default)]
struct RunBody {
    #[serde(default)]
    dry_run: bool,
}

async fn run_job(State(st): State<ServerState>, AxPath(id): AxPath<String>, body: Option<Json<RunBody>>) -> ApiResult {
    let dry = body.map(|b| b.0.dry_run).unwrap_or(false);
    ok(json!({"task_id": st.mgr.start_backup(&id, dry).await?}))
}

async fn run_all_jobs(State(st): State<ServerState>, Extension(acc): Extension<access::Access>, body: Option<Json<crate::backup_engine::runs::RunAllFilter>>) -> ApiResult {
    ok(run_all_for(&st, &acc, body.map(|b| b.0).unwrap_or_default()).await?)
}

#[derive(Deserialize)]
struct EstimateQuery {
    assumed_bytes_per_sec: Option<u64>,
}

async fn estimate_job(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<EstimateQuery>) -> ApiResult {
    ok(st.mgr.estimate(&id, q.assumed_bytes_per_sec).await?)
}

async fn job_operation(State(st): State<ServerState>, Extension(acc): Extension<access::Access>, AxPath(id): AxPath<String>, Json(op): Json<BackupOperation>) -> ApiResult {
    if matches!(op, BackupOperation::Backup { .. }) {
        return Err(AppError::validation("Use POST /jobs/{id}/run to start a backup").into());
    }
    acc.require_job(&st.mgr, &id, access::operation_need(&op)).await?;
    ok(st.mgr.execute(&id, op).await?)
}

#[derive(Deserialize)]
struct PassphraseBody {
    #[serde(default)]
    current: Option<String>,
    new_passphrase: String,
    #[serde(default)]
    kdf_preset: Option<String>,
}

/// Change an encrypted job's passphrase (format 2 repositories: header only).
async fn change_job_passphrase(State(st): State<ServerState>, AxPath(id): AxPath<String>, Json(b): Json<PassphraseBody>) -> ApiResult {
    let kdf = match b.kdf_preset.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => Some(ufop_core::backup::crypto::KdfParams::preset(p).ok_or_else(|| AppError::validation(format!("Unknown key-derivation preset '{p}'")))?),
        None => None,
    };
    ok(st.mgr.change_passphrase(&id, b.current, b.new_passphrase, kdf).await?)
}

#[derive(Deserialize)]
struct PointInTimeBody {
    at: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    name: Option<String>,
}

/// A new read-only job reading this job's versioned S3 bucket as of `at`.
async fn open_point_in_time(State(st): State<ServerState>, AxPath(id): AxPath<String>, Json(b): Json<PointInTimeBody>) -> ApiResult {
    ok(st.mgr.open_point_in_time(&id, b.at, b.name).await?)
}

/// Ransomware-hardening checklist (from the job's settings and the last probe).
async fn job_hardening(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(st.mgr.hardening_report(&id, false).await?)
}

/// Run the destination probe ("Run checks"): writes a probe object and tries to delete it.
async fn job_hardening_probe(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(st.mgr.hardening_report(&id, true).await?)
}

/// Who holds the job's repository lock(s).
async fn job_lock_status(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(st.mgr.lock_status(&id).await?)
}

#[derive(Deserialize)]
struct BreakLockQuery {
    #[serde(default)]
    confirm: bool,
    owner_id: Option<String>,
}

/// Force-unlock (`?confirm=true`, optionally `&owner_id=` of the lock shown).
async fn break_job_lock(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<BreakLockQuery>) -> ApiResult {
    ok(st.mgr.break_lock(&id, q.confirm, q.owner_id).await?)
}

#[derive(Deserialize)]
struct DbEncryptionBody {
    enabled: bool,
}

/// Encrypt (or decrypt) the job's local database now.
async fn set_job_database_encryption(State(st): State<ServerState>, AxPath(id): AxPath<String>, Json(b): Json<DbEncryptionBody>) -> ApiResult {
    ok(st.mgr.set_database_encryption(&id, b.enabled).await?)
}

#[derive(Deserialize)]
struct VersionsQuery {
    /// Another source id, or `*` for every source.
    source: Option<String>,
    /// Comma-separated `key=value` (or `key`) tags a version must carry.
    tags: Option<String>,
}

async fn versions(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<VersionsQuery>) -> ApiResult {
    let tags: Vec<String> = q.tags.unwrap_or_default().split(',').map(str::trim).filter(|t| !t.is_empty()).map(String::from).collect();
    ok(st.mgr.list_versions_in(&id, q.source, tags).await?)
}

#[derive(Deserialize)]
struct VersionPatchBody {
    #[serde(default)]
    source: Option<String>,
    #[serde(flatten)]
    patch: ufop_core::backup::versions::MetadataPatch,
}

/// Change a version's note, tags or pin.
async fn patch_version(State(st): State<ServerState>, AxPath((id, version)): AxPath<(String, usize)>, Json(b): Json<VersionPatchBody>) -> ApiResult {
    ok(st.mgr.execute(&id, BackupOperation::SetVersionMetadata { version, source: b.source, patch: b.patch }).await?)
}

async fn sources(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(st.mgr.list_sources(&id).await?)
}

#[derive(Deserialize, Default)]
struct RetentionPreviewBody {
    #[serde(default)]
    policy: Option<ufop_core::backup::RetentionPolicy>,
    #[serde(default)]
    source: Option<String>,
}

/// Keep/delete per version for the job's policy or an unsaved one.
async fn retention_preview(State(st): State<ServerState>, AxPath(id): AxPath<String>, body: Option<Json<RetentionPreviewBody>>) -> ApiResult {
    let b = body.map(|b| b.0).unwrap_or_default();
    ok(st.mgr.preview_retention(&id, b.policy, b.source).await?)
}

#[derive(Deserialize)]
struct FilesQuery {
    /// Version number (0 = newest), `latest`, `oldest` or a version id.
    version: Option<String>,
    /// "As of" this time (RFC 3339, or `2026-10-01 09:00` in the server's time zone).
    time: Option<String>,
    #[serde(default)]
    allow_partial: bool,
    prefix: Option<String>,
    source: Option<String>,
}

async fn files(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<FilesQuery>) -> ApiResult {
    let sel = crate::backup_engine::access::query_selector(q.version.as_deref(), q.time.as_deref(), q.allow_partial)?;
    let version = st.mgr.resolve_version(&id, sel, q.source.clone()).await?.index;
    ok(st.mgr.list_files_in(&id, version, q.prefix, q.source).await?)
}

#[derive(Deserialize)]
struct ResolveQuery {
    version: Option<String>,
    time: Option<String>,
    #[serde(default)]
    allow_partial: bool,
    source: Option<String>,
}

/// Which version a time (or latest / oldest / number / id) picks.
async fn resolve_version(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<ResolveQuery>) -> ApiResult {
    let sel = crate::backup_engine::access::query_selector(q.version.as_deref(), q.time.as_deref(), q.allow_partial)?;
    ok(st.mgr.resolve_version(&id, sel, q.source).await?)
}

#[derive(Deserialize)]
struct FileQuery {
    path: String,
    version: Option<String>,
    time: Option<String>,
    #[serde(default)]
    allow_partial: bool,
    source: Option<String>,
}

/// One backed-up file's bytes, streamed, with single `Range` support.
async fn file_content(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<FileQuery>, headers: HeaderMap) -> Result<Response, ApiError> {
    let sel = crate::backup_engine::access::query_selector(q.version.as_deref(), q.time.as_deref(), q.allow_partial)?;
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok()).map(String::from);
    let fs = match st.mgr.stream_file(&id, sel, q.path, range, q.source).await {
        Ok(f) => f,
        Err(e) if e.user_message().contains("Range not satisfiable") => {
            let mut r = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            r.headers_mut().insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
            return Ok(r);
        }
        Err(e) => return Err(e.into()),
    };
    let (status, count) = match fs.range {
        Some((a, b)) => (StatusCode::PARTIAL_CONTENT, b - a + 1),
        None => (StatusCode::OK, fs.size),
    };
    let stream = futures_util::stream::unfold(fs.data, |mut rx| async move { rx.recv().await.map(|item| (item, rx)) });
    let mut r = Response::new(axum::body::Body::from_stream(stream));
    *r.status_mut() = status;
    let h = r.headers_mut();
    let name = std::path::Path::new(&fs.path.replace('\\', "/")).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static(ufop_core::webdav::content_type(&name)));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(count));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Ok(v) = HeaderValue::from_str(&fs.etag) {
        h.insert(header::ETAG, v);
    }
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename*=UTF-8''{}", ufop_core::webdav::encode_segment(&name))) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    if let Some(v) = fs.mtime.and_then(|m| chrono::DateTime::from_timestamp(m, 0)).and_then(|t| HeaderValue::from_str(&ufop_core::webdav::http_date(t.into())).ok()) {
        h.insert(header::LAST_MODIFIED, v);
    }
    if let Some((a, b)) = fs.range {
        if let Ok(v) = HeaderValue::from_str(&format!("bytes {a}-{b}/{}", fs.size)) {
            h.insert(header::CONTENT_RANGE, v);
        }
    }
    Ok(r)
}

async fn cache_stats(State(st): State<ServerState>) -> ApiResult {
    ok(st.mgr.cache_stats().await?)
}

async fn clear_cache(State(st): State<ServerState>) -> ApiResult {
    ok(json!({ "bytes_freed": st.mgr.clear_cache().await? }))
}

async fn get_cache_settings(State(st): State<ServerState>) -> ApiResult {
    ok(st.mgr.cache_settings().await?)
}

async fn put_cache_settings(State(st): State<ServerState>, Json(s): Json<ufop_core::backup::cache::CacheSettings>) -> ApiResult {
    ok(st.mgr.set_cache_settings(s).await?)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    version: Option<usize>,
    source: Option<String>,
}

async fn search(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<SearchQuery>) -> ApiResult {
    ok(st.mgr.search_in(&id, q.q, q.version, q.source).await?)
}

#[derive(Deserialize)]
struct CompareQuery {
    base: usize,
    compare: usize,
    source: Option<String>,
}

async fn compare(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<CompareQuery>) -> ApiResult {
    ok(st.mgr.compare_in(&id, q.base, q.compare, q.source).await?)
}

#[derive(Deserialize)]
struct ExportQuery {
    #[serde(default)]
    include_passphrase: bool,
    export_passphrase: Option<String>,
}

async fn export_job(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<ExportQuery>) -> Result<Response, ApiError> {
    let body = api::export_config(&st.mgr, &id, q.include_passphrase, q.export_passphrase.as_deref(), true).await?;
    Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
}

async fn export_cli(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(json!({"command": crate::backup_engine::portability::cli_command(&st.mgr.get_job(&id).await?)}))
}

#[derive(Deserialize)]
struct ImportBody {
    config: String,
    export_passphrase: Option<String>,
    #[serde(default)]
    import_stats: bool,
    #[serde(default = "yes")]
    save: bool,
}
fn yes() -> bool {
    true
}

async fn import_job(State(st): State<ServerState>, Json(b): Json<ImportBody>) -> ApiResult {
    ok(api::import_config(&st.mgr, &b.config, b.export_passphrase.as_deref(), b.import_stats, b.save).await?)
}

async fn cancel_task(State(st): State<ServerState>, Extension(acc): Extension<access::Access>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(json!({"cancelled": cancel_for(&st, &acc, &id).await?}))
}

async fn tasks(State(st): State<ServerState>, Extension(acc): Extension<access::Access>) -> ApiResult {
    ok(visible_status(&st, &acc).await.tasks)
}

#[derive(Deserialize)]
struct RunsQuery {
    job_id: Option<String>,
    limit: Option<u32>,
}

async fn runs(State(st): State<ServerState>, Extension(acc): Extension<access::Access>, Query(q): Query<RunsQuery>) -> ApiResult {
    ok(runs_for(&st, &acc, q.job_id, q.limit.unwrap_or(50)).await?)
}

/// Run history of the jobs the caller may read.
async fn runs_for(st: &ServerState, acc: &access::Access, job_id: Option<String>, limit: u32) -> Result<Vec<crate::backup_engine::BackupRun>, AppError> {
    if let Some(id) = &job_id {
        acc.require_job(&st.mgr, id, access::Permission::Read).await?;
        return st.mgr.run_history(job_id, limit).await;
    }
    if acc.is_admin() && !acc.job_limited() {
        return st.mgr.run_history(None, limit).await;
    }
    let ids: std::collections::HashSet<String> = visible_jobs(st, acc).await.into_iter().map(|j| j.id).collect();
    // History of deleted jobs and of other jobs is left out.
    let mut out = st.mgr.run_history(None, limit.saturating_mul(4).max(200)).await?;
    out.retain(|r| ids.contains(&r.job_id));
    out.truncate(limit as usize);
    Ok(out)
}

#[derive(Deserialize)]
struct DestTestBody {
    destination: crate::backup_engine::BackupDestination,
    #[serde(default)]
    create: bool,
}

async fn test_destination(State(st): State<ServerState>, Json(b): Json<DestTestBody>) -> ApiResult {
    ok(json!({"files": st.mgr.test_destination(&b.destination, b.create).await?}))
}

#[derive(Deserialize)]
struct PauseBody {
    minutes: Option<u32>,
}

async fn pause(State(st): State<ServerState>, body: Option<Json<PauseBody>>) -> ApiResult {
    st.mgr.pause(body.and_then(|b| b.0.minutes));
    ok(st.mgr.status().await)
}

// ── Notification profiles, previews and webhook presets ──

type Profile = crate::backup_engine::notify::profiles::NotificationProfile;

async fn list_notification_profiles(State(st): State<ServerState>) -> ApiResult {
    ok(st.mgr.list_notification_profiles().await?)
}

async fn create_notification_profile(State(st): State<ServerState>, Json(mut p): Json<Profile>) -> ApiResult {
    p.id.clear();
    ok(st.mgr.save_notification_profile(p).await?)
}

async fn get_notification_profile(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(st.mgr.get_notification_profile(&id).await?)
}

async fn update_notification_profile(State(st): State<ServerState>, AxPath(id): AxPath<String>, Json(mut p): Json<Profile>) -> ApiResult {
    st.mgr.get_notification_profile(&id).await?;
    p.id = id;
    ok(st.mgr.save_notification_profile(p).await?)
}

#[derive(Deserialize, Default)]
struct ForceQuery {
    #[serde(default)]
    force: bool,
}

async fn delete_notification_profile(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<ForceQuery>) -> ApiResult {
    ok(st.mgr.delete_notification_profile(&id, q.force).await?)
}

async fn test_notification_profile(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    st.mgr.test_notification_profile(&id).await?;
    ok(json!({"ok": true}))
}

#[derive(Deserialize)]
struct PreviewBody {
    notification: crate::backup_engine::notify::Notification,
    #[serde(default)]
    result: Option<String>,
}

async fn preview_notification(State(st): State<ServerState>, Json(b): Json<PreviewBody>) -> ApiResult {
    ok(st.mgr.preview_notification(&b.notification, b.result.as_deref().unwrap_or("success")))
}

#[derive(Deserialize)]
struct PresetBody {
    preset: crate::backup_engine::notify::webhook::WebhookPreset,
    #[serde(default)]
    params: crate::backup_engine::notify::webhook::PresetParams,
}

async fn webhook_preset(Json(b): Json<PresetBody>) -> ApiResult {
    ok(crate::backup_engine::notify::webhook::preset(b.preset, &b.params)?)
}

// ── Policy templates, effective settings, schedule preview ──

async fn list_templates(State(st): State<ServerState>) -> ApiResult {
    ok(st.mgr.list_templates().await?)
}

async fn create_template(State(st): State<ServerState>, Json(mut t): Json<crate::backup_engine::templates::PolicyTemplate>) -> ApiResult {
    t.id.clear();
    ok(st.mgr.save_template(t).await?)
}

async fn get_template(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(st.mgr.get_template(&id).await?)
}

async fn update_template(State(st): State<ServerState>, AxPath(id): AxPath<String>, Json(mut t): Json<crate::backup_engine::templates::PolicyTemplate>) -> ApiResult {
    st.mgr.get_template(&id).await?;
    t.id = id;
    ok(st.mgr.save_template(t).await?)
}

async fn delete_template(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    st.mgr.delete_template(&id).await?;
    ok(json!({"ok": true}))
}

#[derive(Deserialize)]
struct ApplyTemplateBody {
    job_id: String,
}

async fn apply_template(State(st): State<ServerState>, Extension(acc): Extension<access::Access>, AxPath(id): AxPath<String>, Json(b): Json<ApplyTemplateBody>) -> ApiResult {
    acc.require_job(&st.mgr, &b.job_id, access::Permission::Write).await?;
    ok(st.mgr.apply_template(&id, &b.job_id).await?)
}

#[derive(Deserialize, Default)]
struct ExportTemplatesQuery {
    /// Comma-separated ids (empty = all).
    #[serde(default)]
    ids: String,
}

async fn export_templates(State(st): State<ServerState>, Query(q): Query<ExportTemplatesQuery>) -> Result<Response, ApiError> {
    let ids: Vec<String> = q.ids.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    let body = st.mgr.export_templates(&ids).await?;
    Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
}

async fn import_templates(State(st): State<ServerState>, body: String) -> ApiResult {
    ok(st.mgr.import_templates(&body).await?)
}

async fn effective_settings(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(st.mgr.effective_settings(Some(&id), None).await?)
}

#[derive(Deserialize)]
struct SchedulePreviewBody {
    schedule: crate::backup_engine::schedule::BackupSchedule,
    #[serde(default)]
    count: Option<usize>,
}

async fn schedule_preview(Json(b): Json<SchedulePreviewBody>) -> ApiResult {
    ok(b.schedule.preview(chrono::Utc::now(), b.count.unwrap_or(5))?)
}

async fn resume(State(st): State<ServerState>) -> ApiResult {
    st.mgr.resume();
    ok(st.mgr.status().await)
}

#[derive(Deserialize)]
struct FsQuery {
    path: Option<String>,
    #[serde(default)]
    folders_only: bool,
}

/// Filesystem browsing for the source picker.
async fn filesystem(Query(q): Query<FsQuery>) -> ApiResult {
    let path = q.path.filter(|p| !p.is_empty()).map(PathBuf::from).or_else(dirs::home_dir).unwrap_or_else(|| PathBuf::from("/"));
    let mut out = Vec::new();
    for e in std::fs::read_dir(&path).map_err(AppError::from)? {
        let Ok(e) = e else { continue };
        let Ok(md) = e.metadata() else { continue };
        if q.folders_only && !md.is_dir() {
            continue;
        }
        out.push(json!({"name": e.file_name().to_string_lossy(), "path": e.path().to_string_lossy(), "is_dir": md.is_dir(), "size": if md.is_dir() {0} else {md.len()}}));
    }
    out.sort_by(|a, b| (!a["is_dir"].as_bool().unwrap_or(false), a["name"].as_str().unwrap_or("").to_lowercase()).cmp(&(!b["is_dir"].as_bool().unwrap_or(false), b["name"].as_str().unwrap_or("").to_lowercase())));
    ok(json!({"path": path.to_string_lossy(), "parent": path.parent().map(|p| p.to_string_lossy().into_owned()), "entries": out}))
}

// ── Static UI ──

fn inject_server_flag(index: &str) -> String {
    let tag = r#"<script>window.__UFOP_SERVER__={api:"/api/v1"}</script>"#;
    match index.find("</head>") {
        Some(i) => format!("{}{tag}{}", &index[..i], &index[i..]),
        None => format!("{tag}{index}"),
    }
}

async fn static_files(State(st): State<ServerState>, req: Request) -> Response {
    if let Some(slot) = req.extensions().get::<observe::RouteSlot>() {
        let _ = slot.0.set("static".into());
    }
    let Some(root) = st.cfg.webroot.clone() else { return StatusCode::NOT_FOUND.into_response() };
    let rel = req.uri().path().trim_start_matches('/');
    if rel.split('/').any(|seg| seg == "..") {
        return StatusCode::BAD_REQUEST.into_response();
    }
    let file = root.join(rel);
    if !rel.is_empty() && file.is_file() {
        let mime = mime_for(&file);
        return match tokio::fs::read(&file).await {
            Ok(d) => ([(header::CONTENT_TYPE, mime)], d).into_response(),
            Err(_) => StatusCode::NOT_FOUND.into_response(),
        };
    }
    let path = format!("/{rel}");
    if st.cfg.spa_paths.iter().any(|p| path.starts_with(p.as_str())) {
        if let Ok(idx) = tokio::fs::read_to_string(root.join("index.html")).await {
            let html = branding::brand_index(&inject_server_flag(&idx), &st.branding.effective().await);
            return ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response();
        }
    }
    StatusCode::NOT_FOUND.into_response()
}

fn mime_for(p: &std::path::Path) -> &'static str {
    match p.extension().and_then(|e| e.to_str()).unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "json" => "application/json",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "ico" => "image/x-icon",
        "woff2" => "font/woff2",
        "wasm" => "application/wasm",
        _ => "application/octet-stream",
    }
}

/// `GET /api/v1/sdk/{language}`: the generated client library (public, like the OpenAPI document).
async fn sdk_download(axum::extract::Path(language): axum::extract::Path<String>) -> Response {
    let doc = openapi::document();
    let (text, file) = match language.as_str() {
        "typescript" | "ts" => (sdk::typescript(&doc), "ufop-client.ts"),
        "python" | "py" => (sdk::python(&doc), "ufop_client.py"),
        _ => return (StatusCode::NOT_FOUND, Json(json!({ "error_type": "not_found", "message": format!("No client for '{language}'"), "advice": "Use typescript or python." }))).into_response(),
    };
    ([(header::CONTENT_TYPE, "text/plain; charset=utf-8".to_string()), (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{file}\""))], text).into_response()
}

/// Build the full router.
pub fn router(st: ServerState) -> Router {
    let public = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/openapi.json", get(|| async { Json(openapi::document()) }))
        .route("/api/v1/sdk/{language}", get(sdk_download))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/signin", post(redeem_signin))
        .route("/api/v1/branding", get(branding::public_branding))
        .route("/branding/logo", get(branding::logo))
        .route("/branding/custom.css", get(branding::custom_css))
        .merge(oauth::public_routes())
        .merge(shares::public_routes())
        .merge(mcp_oauth::routes())
        .merge(mcp::routes());
    let protected = Router::new()
        .route("/api/v1/auth/signin-token", post(signin_token))
        .route("/api/v1/auth/automation-token", post(automation_token))
        .route("/api/v1/auth/api-tokens", get(list_api_tokens).post(create_api_token))
        .route("/api/v1/auth/api-tokens/{id}", axum::routing::delete(revoke_api_token))
        .route("/api/v1/auth/reset", post(reset_jwt))
        .route("/api/v1/auth/password", axum::routing::put(change_password))
        .route("/api/v1/status", get(status))
        .route("/api/v1/modules", get(modules))
        .route("/api/v1/settings", get(get_settings).put(put_settings))
        .route("/api/v1/rpc/{command}", post(rpc))
        .route("/api/v1/jobs", get(list_jobs).post(create_job))
        .route("/api/v1/jobs/import", post(import_job))
        .route("/api/v1/jobs/{id}", get(get_job).put(update_job).delete(delete_job))
        .route("/api/v1/jobs/run-all", post(run_all_jobs))
        .route("/api/v1/jobs/{id}/run", post(run_job))
        .route("/api/v1/jobs/{id}/estimate", get(estimate_job))
        .route("/api/v1/jobs/{id}/operations", post(job_operation))
        .route("/api/v1/jobs/{id}/passphrase", axum::routing::put(change_job_passphrase))
        .route("/api/v1/jobs/{id}/point-in-time", post(open_point_in_time))
        .route("/api/v1/jobs/{id}/lock", get(job_lock_status).delete(break_job_lock))
        .route("/api/v1/jobs/{id}/hardening", get(job_hardening))
        .route("/api/v1/jobs/{id}/hardening/probe", post(job_hardening_probe))
        .route("/api/v1/jobs/{id}/database-encryption", axum::routing::put(set_job_database_encryption))
        .route("/api/v1/jobs/{id}/versions", get(versions))
        .route("/api/v1/jobs/{id}/versions/{version}", axum::routing::patch(patch_version))
        .route("/api/v1/jobs/{id}/sources", get(sources))
        .route("/api/v1/jobs/{id}/retention/preview", post(retention_preview))
        .route("/api/v1/jobs/{id}/files", get(files))
        .route("/api/v1/jobs/{id}/file", get(file_content))
        .route("/api/v1/jobs/{id}/versions/resolve", get(resolve_version))
        .route("/api/v1/backup-cache", get(cache_stats).delete(clear_cache))
        .route("/api/v1/backup-cache/settings", get(get_cache_settings).put(put_cache_settings))
        .route("/api/v1/jobs/{id}/search", get(search))
        .route("/api/v1/jobs/{id}/compare", get(compare))
        .route("/api/v1/jobs/{id}/export", get(export_job))
        .route("/api/v1/jobs/{id}/cli", get(export_cli))
        .route("/api/v1/tasks", get(tasks))
        .route("/api/v1/tasks/{id}/cancel", post(cancel_task))
        .route("/api/v1/runs", get(runs))
        .route("/api/v1/destinations/test", post(test_destination))
        .route("/api/v1/filesystem", get(filesystem))
        .route("/api/v1/scheduler/pause", post(pause))
        .route("/api/v1/scheduler/resume", post(resume))
        .route("/api/v1/notification-profiles", get(list_notification_profiles).post(create_notification_profile))
        .route("/api/v1/notification-profiles/{id}", get(get_notification_profile).put(update_notification_profile).delete(delete_notification_profile))
        .route("/api/v1/notification-profiles/{id}/test", post(test_notification_profile))
        .route("/api/v1/notifications/preview", post(preview_notification))
        .route("/api/v1/notifications/webhook-preset", post(webhook_preset))
        .route("/api/v1/backup-templates", get(list_templates).post(create_template))
        .route("/api/v1/backup-templates/export", get(export_templates))
        .route("/api/v1/backup-templates/import", post(import_templates))
        .route("/api/v1/backup-templates/{id}", get(get_template).put(update_template).delete(delete_template))
        .route("/api/v1/backup-templates/{id}/apply", post(apply_template))
        .route("/api/v1/jobs/{id}/effective-settings", get(effective_settings))
        .route("/api/v1/backup-schedule/preview", post(schedule_preview))
        .merge(oauth::protected_routes())
        .merge(shares::protected_routes())
        .merge(folders::protected_routes())
        .route("/api/v1/server/shutdown", post(lifecycle::shutdown_route))
        .route("/api/v1/server/branding", get(branding::get_admin).put(branding::put_admin))
        .route("/api/v1/metrics", get(metrics::metrics_route))
        .merge(gateway::protected_routes())
        .merge(accounts::protected_routes())
        .route_layer(middleware::from_fn_with_state(st.clone(), access::authorize))
        .route_layer(middleware::from_fn_with_state(st.clone(), require_auth));
    let mut app = public.merge(protected).merge(dav::routes()).route_layer(middleware::from_fn(observe::record_route));
    if !st.cfg.api_only && st.cfg.webroot.is_some() {
        app = app.fallback(static_files);
    }
    if !st.cfg.cors_origins.is_empty() {
        let origins: Vec<HeaderValue> = st.cfg.cors_origins.iter().filter_map(|o| o.parse().ok()).collect();
        app = app.layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(origins)
                .allow_credentials(true)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::PATCH, Method::DELETE])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::HeaderName::from_static(PREAUTH_HEADER), header::HeaderName::from_static(csrf::CSRF_HEADER), observe::REQUEST_ID])
                .expose_headers([observe::REQUEST_ID]),
        );
    }
    app.layer(middleware::from_fn_with_state(st.clone(), csrf::csrf_guard))
        .layer(middleware::from_fn_with_state(st.clone(), lifecycle::drain_guard))
        .layer(middleware::from_fn_with_state(st.clone(), host_check))
        .layer(middleware::from_fn_with_state(st.clone(), ip_check))
        .layer(middleware::from_fn_with_state(st.clone(), security_headers))
        .layer(
            tower_http::trace::TraceLayer::new_for_http()
                .make_span_with(observe::make_span)
                .on_request(observe::on_request)
                .on_response(observe::LogResponse { enabled: st.cfg.request_log })
                .on_failure(()),
        )
        .layer(middleware::from_fn_with_state(st.clone(), observe::request_context))
        .with_state(st)
}

/// Restrict the data directory to the current user (0700 on Unix).
pub fn enforce_data_dir_permissions(dir: &std::path::Path) -> Result<(), AppError> {
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir)?.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            tracing::warn!("Tightening permissions on {} from {:o} to 700", dir.display(), mode);
            std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(())
}

/// Serve until stopped: a stop request (signal, service stop or
/// `POST /server/shutdown`) drains running backups, then open connections.
pub async fn serve(st: ServerState) -> Result<(), AppError> {
    // Several TLS backends are linked (reqwest, axum-server); pick one explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let addr = st.cfg.bind_addr()?;
    let tls = st.cfg.tls.clone();
    let min_tls = st.cfg.min_tls;
    st.life.listen_for_signals();
    if let Some(maddr) = st.cfg.metrics_listen {
        let (mst, auth) = (st.clone(), st.cfg.metrics_basic_auth.clone());
        tokio::spawn(async move {
            if let Err(e) = metrics::serve_standalone(mst, maddr, auth).await {
                tracing::error!("{}", e.user_message());
            }
        });
    }
    let pusher = st.cfg.pushgateway.clone().map(|p| otlp::spawn_pushgateway(st.clone(), p));
    let handle = axum_server::Handle::new();
    let drain = {
        let (st, handle) = (st.clone(), handle.clone());
        tokio::spawn(async move {
            let report = lifecycle::drain_when_requested(st.clone()).await;
            tracing::info!("Backups stopped ({} cancelled, {} still running after {} ms); closing connections", report.cancelled, report.still_running, report.waited_ms);
            if st.life.is_immediate() {
                handle.shutdown();
            } else {
                handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
            }
            report
        })
    };
    let app = router(st.clone()).into_make_service_with_connect_info::<SocketAddr>();
    let io = |e: std::io::Error| AppError::Configuration { message: format!("Server failed: {e}"), advice: "Check the port and TLS settings.".into() };
    let served = match tls {
        TlsMode::None => {
            tracing::info!("ufop-server listening on http://{addr}");
            axum_server::bind(addr).handle(handle).serve(app).await.map_err(io)
        }
        TlsMode::Generated { dir, hostnames } => {
            let (chain, key) = tls::pem_to_der(&tls::GeneratedCerts::new(dir).load_or_create(&hostnames)?)?;
            let cfg = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls::server_config(chain, key, min_tls)?));
            tracing::info!("ufop-server listening on https://{addr} (TLS {:?}+)", min_tls);
            axum_server::bind_rustls(addr, cfg).handle(handle).serve(app).await.map_err(io)
        }
        TlsMode::Pfx { path, password } => {
            let (chain, key) = tls::load_pfx(&path, &password)?;
            let cfg = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls::server_config(chain, key, min_tls)?));
            tracing::info!("ufop-server listening on https://{addr} (imported certificate, TLS {:?}+)", min_tls);
            axum_server::bind_rustls(addr, cfg).handle(handle).serve(app).await.map_err(io)
        }
    };
    served?;
    // The listener only stops after a stop request, so the drain is done or ending.
    if !drain.is_finished() {
        st.mgr.begin_shutdown();
        st.mgr.cancel_all();
    }
    drain.abort();
    if let Some(p) = pusher {
        // The final push runs once the drain is over.
        let _ = tokio::time::timeout(std::time::Duration::from_secs(20), p).await;
    }
    if let Some(o) = &st.otlp {
        if !o.flush(std::time::Duration::from_secs(10)).await {
            tracing::warn!("Could not send the last trace spans");
        }
    }
    tracing::info!("ufop-server stopped");
    Ok(())
}

#[cfg(test)]
mod tests;
