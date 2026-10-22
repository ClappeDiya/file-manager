//! `ufop-server`: headless backup server with an authenticated, versioned
//! REST API (`/api/v1`), the browser UI (or API-only), HTTPS, CORS,
//! reverse-proxy pre-authentication and an allowed-hosts check.
//!
//! The same `BackupManager` the desktop app uses runs here, so jobs,
//! scheduling, history and maintenance behave identically.

pub mod api_tokens;
pub mod auth;
pub mod oauth;
pub mod openapi;
pub mod shares;
pub mod tls;
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
const REFRESH_COOKIE: &str = "ufop_refresh";

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

async fn require_auth(State(st): State<ServerState>, mut req: Request, next: Next) -> Response {
    let h = req.headers();
    if let Some(t) = h.get(PREAUTH_HEADER).and_then(|v| v.to_str().ok()) {
        if !t.is_empty() && st.cfg.preauth_tokens.iter().any(|p| constant_eq(p, t)) {
            req.extensions_mut().insert(Principal::Session);
            return next.run(req).await;
        }
    }
    let bearer = h.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(str::to_string);
    let principal = match bearer {
        Some(t) if t.starts_with(api_tokens::PREFIX) => match st.auth.api_tokens().validate(&t).await {
            Ok(tok) => {
                let need = api_tokens::required_scope(req.method(), req.uri().path());
                if tok.scope < need {
                    return ApiError(AppError::PermissionDenied {
                        message: format!("The API token \"{}\" has the {} scope; this needs {}", tok.name, tok.scope.as_str(), need.as_str()),
                        advice: "Use a token with a wider scope.".into(),
                    })
                    .into_response();
                }
                Principal::ApiToken { id: tok.id, scope: tok.scope }
            }
            Err(e) => return ApiError(e).into_response(),
        },
        Some(t) => match st.auth.validate(&t).await {
            Ok(_) => Principal::Session,
            Err(e) => return ApiError(e).into_response(),
        },
        None => return ApiError(AppError::PermissionDenied { message: "Authentication required".into(), advice: "Sign in first.".into() }).into_response(),
    };
    req.extensions_mut().insert(principal);
    next.run(req).await
}

fn constant_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
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

// ── Auth routes ──

#[derive(Deserialize)]
struct LoginBody {
    password: String,
}

async fn session_response(st: &ServerState) -> Result<Response, ApiError> {
    let access = st.auth.issue_access().await?;
    let refresh = st.auth.issue_refresh().await?;
    let mut r = Json(json!({"access_token": access.token, "expires_at": access.expires_at})).into_response();
    r.headers_mut().insert(header::SET_COOKIE, refresh_cookie(st, &refresh.token, auth::REFRESH_TTL_DAYS));
    r.headers_mut().append(header::SET_COOKIE, session_marker(auth::REFRESH_TTL_DAYS));
    Ok(r)
}

async fn login(State(st): State<ServerState>, Json(b): Json<LoginBody>) -> Result<Response, ApiError> {
    match st.auth.login(&b.password).await? {
        None => Ok((StatusCode::TOO_MANY_REQUESTS, Json(json!({"error_type":"throttled","message":"Too many sign-in attempts","advice":"Wait a moment and try again."}))).into_response()),
        Some(false) => Err(AppError::PermissionDenied { message: "Wrong password".into(), advice: "Check the server password.".into() }.into()),
        Some(true) => session_response(&st).await,
    }
}

async fn refresh(State(st): State<ServerState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let raw = cookie_value(&headers, REFRESH_COOKIE).ok_or_else(|| AppError::PermissionDenied { message: "No session".into(), advice: "Sign in.".into() })?;
    let (access, refresh) = st.auth.refresh(&raw).await?;
    let mut r = Json(json!({"access_token": access.token, "expires_at": access.expires_at})).into_response();
    r.headers_mut().insert(header::SET_COOKIE, refresh_cookie(&st, &refresh.token, auth::REFRESH_TTL_DAYS));
    Ok(r)
}

async fn logout(State(st): State<ServerState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(raw) = cookie_value(&headers, REFRESH_COOKIE) {
        st.auth.revoke_refresh(&raw).await?;
    }
    let mut r = Json(json!({"ok": true})).into_response();
    r.headers_mut().insert(header::SET_COOKIE, refresh_cookie(&st, "", 0));
    r.headers_mut().append(header::SET_COOKIE, session_marker(0));
    Ok(r)
}

#[derive(Deserialize)]
struct TokenBody {
    token: String,
}

async fn redeem_signin(State(st): State<ServerState>, Json(b): Json<TokenBody>) -> Result<Response, ApiError> {
    st.auth.redeem_signin(&b.token).await?;
    session_response(&st).await
}

async fn signin_token(State(st): State<ServerState>) -> ApiResult {
    ok(st.auth.issue_signin().await?)
}

async fn automation_token(State(st): State<ServerState>) -> ApiResult {
    ok(st.auth.issue_automation().await?)
}

#[derive(Deserialize)]
struct ApiTokenBody {
    name: String,
    scope: api_tokens::Scope,
    #[serde(default = "default_token_days")]
    expires_in_days: i64,
}

fn default_token_days() -> i64 {
    90
}

async fn list_api_tokens(State(st): State<ServerState>) -> ApiResult {
    ok(json!({"tokens": st.auth.api_tokens().list().await?}))
}

/// Minting needs an interactive sign-in: an API token cannot create more tokens.
async fn create_api_token(State(st): State<ServerState>, Extension(who): Extension<Principal>, Json(b): Json<ApiTokenBody>) -> ApiResult {
    if who != Principal::Session {
        return Err(AppError::PermissionDenied { message: "API tokens cannot create API tokens".into(), advice: "Sign in with the server password.".into() }.into());
    }
    let (t, raw) = st.auth.api_tokens().create(&b.name, b.scope, b.expires_in_days).await?;
    tracing::info!("Created API token {} ({}, {} scope)", t.id, t.name, t.scope.as_str());
    ok(json!({"token": raw, "info": t}))
}

async fn revoke_api_token(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
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

async fn change_password(State(st): State<ServerState>, Json(b): Json<PasswordBody>) -> ApiResult {
    if st.auth.login(&b.current_password).await? != Some(true) {
        return Err(AppError::PermissionDenied { message: "Wrong current password".into(), advice: "Check it and try again.".into() }.into());
    }
    st.auth.set_password(&b.new_password).await?;
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

async fn status(State(st): State<ServerState>) -> ApiResult {
    ok(st.mgr.status().await)
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

async fn rpc(State(st): State<ServerState>, AxPath(cmd): AxPath<String>, body: Option<Json<Value>>) -> ApiResult {
    let args = body.map(|b| b.0).unwrap_or(json!({}));
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

async fn list_jobs(State(st): State<ServerState>) -> ApiResult {
    ok(st.mgr.list_jobs().await)
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

async fn job_operation(State(st): State<ServerState>, AxPath(id): AxPath<String>, Json(op): Json<BackupOperation>) -> ApiResult {
    if matches!(op, BackupOperation::Backup { .. }) {
        return Err(AppError::validation("Use POST /jobs/{id}/run to start a backup").into());
    }
    ok(st.mgr.execute(&id, op).await?)
}

async fn versions(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(st.mgr.list_versions(&id).await?)
}

#[derive(Deserialize)]
struct FilesQuery {
    #[serde(default)]
    version: usize,
    prefix: Option<String>,
}

async fn files(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<FilesQuery>) -> ApiResult {
    ok(st.mgr.list_files(&id, q.version, q.prefix).await?)
}

#[derive(Deserialize)]
struct SearchQuery {
    q: String,
    version: Option<usize>,
}

async fn search(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<SearchQuery>) -> ApiResult {
    ok(st.mgr.search(&id, q.q, q.version).await?)
}

#[derive(Deserialize)]
struct CompareQuery {
    base: usize,
    compare: usize,
}

async fn compare(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(q): Query<CompareQuery>) -> ApiResult {
    ok(st.mgr.compare(&id, q.base, q.compare).await?)
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

async fn cancel_task(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> ApiResult {
    ok(json!({"cancelled": st.mgr.cancel(&id)}))
}

async fn tasks(State(st): State<ServerState>) -> ApiResult {
    ok(st.mgr.status().await.tasks)
}

#[derive(Deserialize)]
struct RunsQuery {
    job_id: Option<String>,
    limit: Option<u32>,
}

async fn runs(State(st): State<ServerState>, Query(q): Query<RunsQuery>) -> ApiResult {
    ok(st.mgr.run_history(q.job_id, q.limit.unwrap_or(50)).await?)
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
            return ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], inject_server_flag(&idx)).into_response();
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

/// Build the full router.
pub fn router(st: ServerState) -> Router {
    let public = Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/openapi.json", get(|| async { Json(openapi::document()) }))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/refresh", post(refresh))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/signin", post(redeem_signin))
        .merge(oauth::public_routes())
        .merge(shares::public_routes());
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
        .route("/api/v1/jobs/{id}/run", post(run_job))
        .route("/api/v1/jobs/{id}/operations", post(job_operation))
        .route("/api/v1/jobs/{id}/versions", get(versions))
        .route("/api/v1/jobs/{id}/files", get(files))
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
        .merge(oauth::protected_routes())
        .merge(shares::protected_routes())
        .route_layer(middleware::from_fn_with_state(st.clone(), require_auth));
    let mut app = public.merge(protected);
    if !st.cfg.api_only && st.cfg.webroot.is_some() {
        app = app.fallback(static_files);
    }
    if !st.cfg.cors_origins.is_empty() {
        let origins: Vec<HeaderValue> = st.cfg.cors_origins.iter().filter_map(|o| o.parse().ok()).collect();
        app = app.layer(
            tower_http::cors::CorsLayer::new()
                .allow_origin(origins)
                .allow_credentials(true)
                .allow_methods([Method::GET, Method::POST, Method::PUT, Method::DELETE])
                .allow_headers([header::AUTHORIZATION, header::CONTENT_TYPE, header::HeaderName::from_static(PREAUTH_HEADER)]),
        );
    }
    app.layer(middleware::from_fn_with_state(st.clone(), host_check))
        .layer(middleware::from_fn_with_state(st.clone(), ip_check))
        .layer(middleware::from_fn_with_state(st.clone(), security_headers))
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

/// Serve until the process is stopped.
pub async fn serve(st: ServerState) -> Result<(), AppError> {
    // Several TLS backends are linked (reqwest, axum-server); pick one explicitly.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let addr = st.cfg.bind_addr()?;
    let tls = st.cfg.tls.clone();
    let min_tls = st.cfg.min_tls;
    let app = router(st).into_make_service_with_connect_info::<SocketAddr>();
    let io = |e: std::io::Error| AppError::Configuration { message: format!("Server failed: {e}"), advice: "Check the port and TLS settings.".into() };
    match tls {
        TlsMode::None => {
            tracing::info!("ufop-server listening on http://{addr}");
            axum_server::bind(addr).serve(app).await.map_err(io)
        }
        TlsMode::Generated { dir, hostnames } => {
            let (chain, key) = tls::pem_to_der(&tls::GeneratedCerts::new(dir).load_or_create(&hostnames)?)?;
            let cfg = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls::server_config(chain, key, min_tls)?));
            tracing::info!("ufop-server listening on https://{addr} (TLS {:?}+)", min_tls);
            axum_server::bind_rustls(addr, cfg).serve(app).await.map_err(io)
        }
        TlsMode::Pfx { path, password } => {
            let (chain, key) = tls::load_pfx(&path, &password)?;
            let cfg = axum_server::tls_rustls::RustlsConfig::from_config(Arc::new(tls::server_config(chain, key, min_tls)?));
            tracing::info!("ufop-server listening on https://{addr} (imported certificate, TLS {:?}+)", min_tls);
            axum_server::bind_rustls(addr, cfg).serve(app).await.map_err(io)
        }
    }
}

#[cfg(test)]
mod tests;
