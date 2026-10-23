//! Share portal: links to files and folders under `--share-root` that people
//! open in a browser without an account (Files.com-style share links and
//! file drops).
//!
//! - Kinds: `download` (a file, or a folder to browse and download from),
//!   `upload` (a drop box: files go into the folder, nothing is listed) and
//!   `both`.
//! - Options: password (argon2), expiry, download limit, upload size limit.
//!   Only a SHA-256 of each link's token is stored; the link is shown once.
//! - Paths are confined to the share's folder after resolving symlinks
//!   (`ufop_core::confine`); dot-files are neither listed nor served.
//! - Downloads stream with `Content-Disposition: attachment`; uploads stream
//!   to a temporary file and are renamed into place without overwriting.
//! - Every view, download, upload, refusal and wrong password is logged per
//!   share; wrong passwords are throttled.

use super::oauth::html_escape;
use super::{ApiError, ServerState};
use crate::core::error::AppError;
use crate::storage::pool::DbPool;
use axum::body::Body;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path as AxPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use axum::{Form, Json, Router};
use base64::Engine;
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use tokio::io::AsyncWriteExt;
use ufop_core::confine::Roots;

pub const DEFAULT_UPLOAD_LIMIT: i64 = 1024 * 1024 * 1024;
const MAX_LISTING: usize = 2_000;
const UNLOCK_HOURS: i64 = 12;
const MAX_FAILURES: u32 = 10;
const FAILURE_WINDOW_SECS: i64 = 15 * 60;
const CSP: &str = "default-src 'none'; style-src 'unsafe-inline'; img-src 'self' data:; form-action 'self'; base-uri 'none'; frame-ancestors 'none'";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShareKind {
    Download,
    Upload,
    Both,
}

impl ShareKind {
    fn as_str(self) -> &'static str {
        match self {
            ShareKind::Download => "download",
            ShareKind::Upload => "upload",
            ShareKind::Both => "both",
        }
    }
    fn parse(s: &str) -> Self {
        match s {
            "upload" => ShareKind::Upload,
            "both" => ShareKind::Both,
            _ => ShareKind::Download,
        }
    }
    fn lists(self) -> bool {
        self != ShareKind::Upload
    }
    fn uploads(self) -> bool {
        self != ShareKind::Download
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Share {
    pub id: String,
    pub name: String,
    pub path: String,
    pub kind: ShareKind,
    /// First characters of the token, to tell links apart.
    pub token_prefix: String,
    pub has_password: bool,
    pub expires_at: Option<String>,
    pub max_downloads: Option<i64>,
    pub downloads: i64,
    pub max_upload_bytes: Option<i64>,
    pub created_at: String,
    pub revoked_at: Option<String>,
    pub last_access_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NewShare {
    pub name: String,
    pub path: String,
    #[serde(default = "default_kind")]
    pub kind: ShareKind,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub expires_in_days: Option<i64>,
    #[serde(default)]
    pub max_downloads: Option<i64>,
    #[serde(default)]
    pub max_upload_mb: Option<i64>,
}

fn default_kind() -> ShareKind {
    ShareKind::Download
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ShareEvent {
    pub at: String,
    pub ip: String,
    pub action: String,
    pub detail: String,
}

/// Why a link does not work (any more).
fn unavailable(s: &Share, now: DateTime<Utc>) -> Option<&'static str> {
    if s.revoked_at.is_some() {
        return Some("This link has been turned off.");
    }
    if s.expires_at.as_deref().and_then(|e| DateTime::parse_from_rfc3339(e).ok()).is_some_and(|e| e < now) {
        return Some("This link has expired.");
    }
    None
}

fn sha256_hex(s: &str) -> String {
    hex::encode(Sha256::digest(s.as_bytes()))
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::validation(msg.into())
}

const COLS: &str = "id, name, path, kind, token_prefix, password_hash, expires_at, max_downloads, downloads, max_upload_bytes, created_at, revoked_at, last_access_at";

fn from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<(Share, Option<String>)> {
    let kind: String = r.get(3)?;
    let pw: Option<String> = r.get(5)?;
    Ok((
        Share {
            id: r.get(0)?,
            name: r.get(1)?,
            path: r.get(2)?,
            kind: ShareKind::parse(&kind),
            token_prefix: r.get(4)?,
            has_password: pw.is_some(),
            expires_at: r.get(6)?,
            max_downloads: r.get(7)?,
            downloads: r.get(8)?,
            max_upload_bytes: r.get(9)?,
            created_at: r.get(10)?,
            revoked_at: r.get(11)?,
            last_access_at: r.get(12)?,
        },
        pw,
    ))
}

#[derive(Clone)]
pub struct ShareStore {
    pool: DbPool,
}

impl ShareStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Create a link to `n.path`, which must lie inside `roots`. Returns the share and its token (shown once).
    pub async fn create(&self, n: NewShare, roots: &Roots) -> Result<(Share, String), AppError> {
        let name = n.name.trim().to_string();
        if name.is_empty() || name.chars().count() > 120 {
            return Err(bad("Give the link a name (up to 120 characters)"));
        }
        let path = roots.resolve(&n.path, true).map_err(bad)?;
        if n.kind.uploads() && !path.is_dir() {
            return Err(bad("Upload links need a folder"));
        }
        if path.file_name().is_some_and(|f| f.to_string_lossy().starts_with('.')) {
            return Err(bad("Hidden files and folders cannot be shared"));
        }
        let password_hash = match n.password.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => Some(super::auth::hash_password(p).map_err(|_| bad("Link passwords need at least 8 characters"))?),
            None => None,
        };
        let expires_at = match n.expires_in_days {
            Some(d) if !(1..=3650).contains(&d) => return Err(bad("Expiry must be 1–3650 days")),
            Some(d) => Some((Utc::now() + ChronoDuration::days(d)).to_rfc3339()),
            None => None,
        };
        if n.max_downloads.is_some_and(|m| m < 1) {
            return Err(bad("The download limit must be at least 1"));
        }
        let max_upload_bytes = match n.max_upload_mb {
            Some(m) if !(1..=1024 * 1024).contains(&m) => return Err(bad("The upload limit must be 1 MB–1 TB")),
            Some(m) => Some(m * 1024 * 1024),
            None if n.kind.uploads() => Some(DEFAULT_UPLOAD_LIMIT),
            None => None,
        };
        let mut raw = [0u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut raw);
        let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        let share = Share {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            path: path.to_string_lossy().into_owned(),
            kind: n.kind,
            token_prefix: token[..6].to_string(),
            has_password: password_hash.is_some(),
            expires_at,
            max_downloads: n.max_downloads,
            downloads: 0,
            max_upload_bytes,
            created_at: Utc::now().to_rfc3339(),
            revoked_at: None,
            last_access_at: None,
        };
        let (s, hash) = (share.clone(), sha256_hex(&token));
        self.pool
            .execute(move |c| {
                c.execute(
                    &format!("INSERT INTO server_shares ({COLS}, token_hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 0, ?9, ?10, NULL, NULL, ?11)"),
                    params![s.id, s.name, s.path, s.kind.as_str(), s.token_prefix, password_hash, s.expires_at, s.max_downloads, s.max_upload_bytes, s.created_at, hash],
                )?;
                Ok(())
            })
            .await?;
        Ok((share, token))
    }

    pub async fn list(&self) -> Result<Vec<Share>, AppError> {
        self.pool
            .execute(|c| {
                let mut st = c.prepare(&format!("SELECT {COLS} FROM server_shares ORDER BY created_at DESC"))?;
                let v = st.query_map([], from_row)?.map(|r| r.map(|(s, _)| s)).collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })
            .await
    }

    async fn by_hash(&self, hash: String) -> Result<Option<(Share, Option<String>)>, AppError> {
        self.pool.execute(move |c| Ok(c.query_row(&format!("SELECT {COLS} FROM server_shares WHERE token_hash = ?1"), params![hash], from_row).optional()?)).await
    }

    pub async fn by_token(&self, token: &str) -> Result<Option<(Share, Option<String>)>, AppError> {
        if token.len() > 128 {
            return Ok(None);
        }
        self.by_hash(sha256_hex(token)).await
    }

    pub async fn revoke(&self, id: &str) -> Result<bool, AppError> {
        let id = id.to_string();
        self.pool.execute(move |c| Ok(c.execute("UPDATE server_shares SET revoked_at = ?2 WHERE id = ?1 AND revoked_at IS NULL", params![id, Utc::now().to_rfc3339()])? == 1)).await
    }

    pub async fn events(&self, id: &str, limit: i64) -> Result<Vec<ShareEvent>, AppError> {
        let id = id.to_string();
        self.pool
            .execute(move |c| {
                let mut st = c.prepare("SELECT at, ip, action, detail FROM server_share_events WHERE share_id = ?1 ORDER BY id DESC LIMIT ?2")?;
                let v = st.query_map(params![id, limit.clamp(1, 1000)], |r| Ok(ShareEvent { at: r.get(0)?, ip: r.get(1)?, action: r.get(2)?, detail: r.get(3)? }))?.collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })
            .await
    }

    pub async fn record(&self, id: &str, ip: &str, action: &str, detail: &str) {
        let (id, ip, action, detail) = (id.to_string(), ip.to_string(), action.to_string(), detail.chars().take(500).collect::<String>());
        let r = self
            .pool
            .execute(move |c| {
                let now = Utc::now().to_rfc3339();
                c.execute("INSERT INTO server_share_events (share_id, at, ip, action, detail) VALUES (?1, ?2, ?3, ?4, ?5)", params![id, now, ip, action, detail])?;
                c.execute("UPDATE server_shares SET last_access_at = ?2 WHERE id = ?1", params![id, now])?;
                // Keep the newest 5,000 events per link.
                c.execute("DELETE FROM server_share_events WHERE share_id = ?1 AND id <= (SELECT id FROM server_share_events WHERE share_id = ?1 ORDER BY id DESC LIMIT 1 OFFSET 5000)", params![id])?;
                Ok(())
            })
            .await;
        if let Err(e) = r {
            tracing::warn!("share event not recorded: {e}");
        }
    }

    /// Count a download if the limit allows it.
    async fn take_download(&self, id: &str) -> Result<bool, AppError> {
        let id = id.to_string();
        self.pool
            .execute(move |c| Ok(c.execute("UPDATE server_shares SET downloads = downloads + 1 WHERE id = ?1 AND (max_downloads IS NULL OR downloads < max_downloads)", params![id])? == 1))
            .await
    }
}

// ── Unlock cookies and password throttling ──

fn unlock_key() -> &'static [u8; 32] {
    static K: OnceLock<[u8; 32]> = OnceLock::new();
    K.get_or_init(|| {
        let mut k = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut k);
        k
    })
}

/// `<expiry>.<HMAC(share id, expiry, password hash)>`: changing the password ends unlocks.
fn unlock_value(share_id: &str, pw_hash: &str, exp: i64) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(unlock_key()).expect("HMAC key");
    mac.update(format!("{share_id}|{exp}|{pw_hash}").as_bytes());
    format!("{exp}.{}", hex::encode(mac.finalize().into_bytes()))
}

fn cookie_name(share_id: &str) -> String {
    format!("ufop_share_{}", &share_id[..8.min(share_id.len())])
}

fn unlocked(headers: &HeaderMap, share: &Share, pw_hash: &str) -> bool {
    let name = cookie_name(&share.id);
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|kv| kv.trim().split_once('='))
        .filter(|(k, _)| *k == name)
        .any(|(_, v)| {
            let exp: i64 = v.split('.').next().and_then(|e| e.parse().ok()).unwrap_or(0);
            exp > Utc::now().timestamp() && {
                let want = unlock_value(&share.id, pw_hash, exp);
                want.len() == v.len() && want.bytes().zip(v.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
            }
        })
}

fn failures() -> &'static Mutex<HashMap<String, (u32, i64)>> {
    static F: OnceLock<Mutex<HashMap<String, (u32, i64)>>> = OnceLock::new();
    F.get_or_init(|| Mutex::new(HashMap::new()))
}

fn throttled(share_id: &str) -> bool {
    let now = Utc::now().timestamp();
    let f = failures().lock().unwrap_or_else(|e| e.into_inner());
    f.get(share_id).is_some_and(|(n, since)| *n >= MAX_FAILURES && now - since < FAILURE_WINDOW_SECS)
}

fn note_failure(share_id: &str, ok: bool) {
    let now = Utc::now().timestamp();
    let mut f = failures().lock().unwrap_or_else(|e| e.into_inner());
    if ok {
        f.remove(share_id);
        return;
    }
    let e = f.entry(share_id.to_string()).or_insert((0, now));
    if now - e.1 >= FAILURE_WINDOW_SECS {
        *e = (0, now);
    }
    e.0 += 1;
}

// ── Pages ──

fn page(status: StatusCode, title: &str, body: &str) -> Response {
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="robots" content="noindex,nofollow"><title>{t}</title>
<style>body{{font:15px/1.5 system-ui,sans-serif;margin:0;background:#f6f7f9;color:#1f2328}}main{{max-width:760px;margin:32px auto;padding:0 16px}}.card{{background:#fff;border:1px solid #d8dee4;border-radius:10px;padding:20px}}h1{{font-size:20px;margin:0 0 12px}}table{{width:100%;border-collapse:collapse}}td{{padding:6px 4px;border-top:1px solid #eef1f4}}td.s{{text-align:right;color:#59636e;white-space:nowrap}}a{{color:#0969da;text-decoration:none}}a:hover{{text-decoration:underline}}.muted{{color:#59636e;font-size:13px}}.btn{{display:inline-block;background:#1f6feb;color:#fff;border:0;border-radius:6px;padding:7px 14px;font:inherit;cursor:pointer}}input[type=password]{{font:inherit;padding:6px;border:1px solid #d0d7de;border-radius:6px}}.ok{{color:#1a7f37}}.err{{color:#cf222e}}@media(prefers-color-scheme:dark){{body{{background:#0d1117;color:#e6edf3}}.card{{background:#161b22;border-color:#30363d}}td{{border-color:#21262d}}a{{color:#4493f8}}}}</style></head>
<body><main><div class="card">{body}</div><p class="muted">Shared with UFOP</p></main></body></html>"#,
        t = html_escape(title),
    );
    let mut r = (status, [(header::CONTENT_TYPE, "text/html; charset=utf-8")], html).into_response();
    let h = r.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CSP));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    r
}

fn message(status: StatusCode, title: &str, text: &str) -> Response {
    page(status, title, &format!("<h1>{}</h1><p>{}</p>", html_escape(title), html_escape(text)))
}

fn human(n: u64) -> String {
    let u = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < u.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} B")
    } else {
        format!("{v:.1} {}", u[i])
    }
}

/// Percent-encode a query value.
fn q(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

#[derive(Deserialize, Default)]
pub struct PathQuery {
    #[serde(default)]
    p: String,
    #[serde(default)]
    uploaded: Option<u32>,
    #[serde(default)]
    refused: Option<u32>,
}

/// The share for `token` when it is usable, else the page to show instead.
async fn open(st: &ServerState, token: &str, ip: &str) -> Result<(Share, Option<String>), Response> {
    let store = st.auth.shares();
    let Ok(Some((share, pw))) = store.by_token(token).await else {
        return Err(message(StatusCode::NOT_FOUND, "Link not found", "Check the address, or ask for a new link."));
    };
    if let Some(why) = unavailable(&share, Utc::now()) {
        store.record(&share.id, ip, "refused", why).await;
        return Err(message(StatusCode::GONE, "Link unavailable", why));
    }
    Ok((share, pw))
}

/// The folder (or file) a visitor asks for, inside the share.
fn locate(share: &Share, rel: &str) -> Result<PathBuf, String> {
    let rel = rel.trim_matches(['/', '\\']);
    if rel.split(['/', '\\']).any(|c| c.starts_with('.')) {
        return Err("not found".into());
    }
    let base = PathBuf::from(&share.path);
    if base.is_file() {
        // A one-file link: only the file itself.
        return if rel.is_empty() { base.canonicalize().map_err(|e| e.to_string()) } else { Err("not found".into()) };
    }
    let roots = Roots::new(&[base])?;
    let target = if rel.is_empty() { roots.list()[0].path.clone() } else { roots.resolve(&roots.list()[0].path.join(rel).to_string_lossy(), true)? };
    Ok(target)
}

fn rel_of(share: &Share, p: &Path) -> String {
    let base = Path::new(&share.path).canonicalize().unwrap_or_else(|_| PathBuf::from(&share.path));
    if base.is_file() {
        return String::new();
    }
    p.strip_prefix(&base).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default()
}

async fn view(State(st): State<ServerState>, AxPath(token): AxPath<String>, Query(qr): Query<PathQuery>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    let ip = peer.ip().to_string();
    let (share, pw) = match open(&st, &token, &ip).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let store = st.auth.shares();
    let base = format!("/s/{}", q(&token));
    if let Some(h) = &pw {
        if !unlocked(&headers, &share, h) {
            return page(
                StatusCode::OK,
                &share.name,
                &format!(
                    r#"<h1>{}</h1><p>This link is protected by a password.</p><form method="post" action="{base}/unlock"><input type="password" name="password" aria-label="Password" autofocus required> <button class="btn" type="submit">Open</button></form>"#,
                    html_escape(&share.name)
                ),
            );
        }
    }
    store.record(&share.id, &ip, "view", &qr.p).await;
    let target = match locate(&share, &qr.p) {
        Ok(t) => t,
        Err(_) => return message(StatusCode::NOT_FOUND, "Not found", "That file or folder is not part of this link."),
    };
    let mut body = format!("<h1>{}</h1>", html_escape(&share.name));
    if let Some(n) = qr.uploaded {
        body.push_str(&format!(r#"<p class="ok">Uploaded {n} file{}. Thank you!</p>"#, if n == 1 { "" } else { "s" }));
    }
    if let Some(n) = qr.refused.filter(|n| *n > 0) {
        body.push_str(&format!(r#"<p class="err">{n} file{} not accepted in this folder.</p>"#, if n == 1 { " was" } else { "s were" }));
    }
    if let Some(m) = share.max_downloads {
        if share.downloads >= m && share.kind.lists() {
            body.push_str(r#"<p class="err">The download limit of this link has been reached.</p>"#);
        }
    }
    let rel = rel_of(&share, &target);
    if share.kind.lists() {
        if target.is_file() {
            let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            body.push_str(&format!(r#"<p><b>{}</b> <span class="muted">{}</span></p><p><a class="btn" href="{base}/dl?p={}">Download</a></p>"#, html_escape(&name), human(size), q(&rel)));
        } else {
            if !rel.is_empty() {
                let parent = rel.rsplit_once('/').map(|(p, _)| p).unwrap_or("");
                body.push_str(&format!(r#"<p class="muted"><a href="{base}?p={}">⬆ Up</a> · /{}</p>"#, q(parent), html_escape(&rel)));
            }
            let mut entries: Vec<(bool, String, u64)> = std::fs::read_dir(&target)
                .map(|rd| {
                    rd.filter_map(Result::ok)
                        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
                        .filter_map(|e| {
                            let m = std::fs::metadata(e.path()).ok()?;
                            Some((m.is_dir(), e.file_name().to_string_lossy().into_owned(), m.len()))
                        })
                        .collect()
                })
                .unwrap_or_default();
            entries.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.to_lowercase().cmp(&b.1.to_lowercase())));
            let more = entries.len() > MAX_LISTING;
            entries.truncate(MAX_LISTING);
            body.push_str("<table>");
            for (dir, name, size) in &entries {
                let child = if rel.is_empty() { name.clone() } else { format!("{rel}/{name}") };
                if *dir {
                    body.push_str(&format!(r#"<tr><td>📁 <a href="{base}?p={}">{}</a></td><td class="s"></td></tr>"#, q(&child), html_escape(name)));
                } else {
                    body.push_str(&format!(r#"<tr><td>📄 <a href="{base}/dl?p={}">{}</a></td><td class="s">{}</td></tr>"#, q(&child), html_escape(name), human(*size)));
                }
            }
            if entries.is_empty() {
                body.push_str(r#"<tr><td class="muted">This folder is empty.</td></tr>"#);
            }
            body.push_str("</table>");
            if more {
                body.push_str(&format!(r#"<p class="muted">Showing the first {MAX_LISTING} items.</p>"#));
            }
        }
    }
    if share.kind.uploads() && target.is_dir() {
        let policy = super::folders::effective_for(&st, &target).await.unwrap_or_default();
        if policy.locked {
            body.push_str(r#"<p class="muted" style="margin-top:20px">This folder does not take uploads at the moment.</p>"#);
        } else {
            let limit = (share.max_upload_bytes.unwrap_or(DEFAULT_UPLOAD_LIMIT) as u64).min(policy.max_upload_mb.map_or(u64::MAX, |mb| mb as u64 * 1024 * 1024));
            let types = policy.allowed_extensions.as_ref().map(|a| format!(" Accepted: {}.", a.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(", "))).unwrap_or_default();
            body.push_str(&format!(
                r#"<h2 style="font-size:16px;margin-top:20px">Upload files</h2><form method="post" enctype="multipart/form-data" action="{base}/upload?p={}"><input type="file" name="file" multiple required> <button class="btn" type="submit">Upload</button></form><p class="muted">Up to {} per file.{} Files with the same name are kept side by side.</p>"#,
                q(&rel),
                human(limit),
                html_escape(&types)
            ));
        }
    }
    page(StatusCode::OK, &share.name, &body)
}

#[derive(Deserialize)]
struct UnlockForm {
    password: String,
}

async fn unlock(State(st): State<ServerState>, AxPath(token): AxPath<String>, ConnectInfo(peer): ConnectInfo<SocketAddr>, Form(f): Form<UnlockForm>) -> Response {
    let ip = peer.ip().to_string();
    let (share, pw) = match open(&st, &token, &ip).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let Some(hash) = pw else { return Redirect::to(&format!("/s/{}", q(&token))).into_response() };
    let store = st.auth.shares();
    if throttled(&share.id) {
        store.record(&share.id, &ip, "throttled", "too many wrong passwords").await;
        return message(StatusCode::TOO_MANY_REQUESTS, "Too many attempts", "Wait a few minutes and try again.");
    }
    let ok = {
        let (pw, hash) = (f.password.clone(), hash.clone());
        tokio::task::spawn_blocking(move || super::auth::verify_password_hash(&pw, &hash)).await.unwrap_or(false)
    };
    note_failure(&share.id, ok);
    if !ok {
        store.record(&share.id, &ip, "wrong_password", "").await;
        return message(StatusCode::UNAUTHORIZED, "Wrong password", "Go back and try again.");
    }
    store.record(&share.id, &ip, "unlock", "").await;
    let exp = Utc::now().timestamp() + UNLOCK_HOURS * 3600;
    let secure = if matches!(st.cfg.tls, super::TlsMode::None) { "" } else { "; Secure" };
    let cookie = format!("{}={}; HttpOnly; SameSite=Lax; Path=/s/{}; Max-Age={}{secure}", cookie_name(&share.id), unlock_value(&share.id, &hash, exp), q(&token), UNLOCK_HOURS * 3600);
    let mut r = Redirect::to(&format!("/s/{}", q(&token))).into_response();
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        r.headers_mut().insert(header::SET_COOKIE, v);
    }
    r
}

/// `filename*` per RFC 6266/5987 plus an ASCII fallback.
fn disposition(name: &str) -> String {
    let ascii: String = name.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' || c == ' ' { c } else { '_' }).collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{}", name.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect::<String>())
}

async fn download(State(st): State<ServerState>, AxPath(token): AxPath<String>, Query(qr): Query<PathQuery>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    let ip = peer.ip().to_string();
    let (share, pw) = match open(&st, &token, &ip).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let store = st.auth.shares();
    if pw.as_deref().is_some_and(|h| !unlocked(&headers, &share, h)) {
        return Redirect::to(&format!("/s/{}", q(&token))).into_response();
    }
    if !share.kind.lists() {
        return message(StatusCode::FORBIDDEN, "Upload only", "Files in this folder cannot be downloaded through this link.");
    }
    let target = match locate(&share, &qr.p) {
        Ok(t) if t.is_file() => t,
        _ => return message(StatusCode::NOT_FOUND, "Not found", "That file is not part of this link."),
    };
    let rel = rel_of(&share, &target);
    match store.take_download(&share.id).await {
        Ok(true) => {}
        Ok(false) => {
            store.record(&share.id, &ip, "refused", "download limit reached").await;
            return message(StatusCode::GONE, "Download limit reached", "This link has been used as many times as it allows.");
        }
        Err(e) => return ApiError(e).into_response(),
    }
    let file = match tokio::fs::File::open(&target).await {
        Ok(f) => f,
        Err(_) => return message(StatusCode::NOT_FOUND, "Not found", "That file is no longer there."),
    };
    let len = file.metadata().await.map(|m| m.len()).unwrap_or(0);
    store.record(&share.id, &ip, "download", &format!("{} ({})", if rel.is_empty() { target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default() } else { rel.clone() }, human(len))).await;
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "download".into());
    let mut r = Response::new(Body::from_stream(tokio_util::io::ReaderStream::new(file)));
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    h.insert(header::CONTENT_LENGTH, HeaderValue::from(len));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Ok(v) = HeaderValue::from_str(&disposition(&name)) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    r
}

/// A safe file name for an upload: the last component, no control
/// characters or reserved names, never hidden.
pub fn safe_name(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control() && !"<>:\"|?*".contains(*c)).collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim_end_matches(['.', ' ']).to_string();
    let stem = cleaned.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = ["CON", "PRN", "AUX", "NUL"].contains(&stem.as_str()) || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    (!cleaned.is_empty() && !reserved && cleaned.len() <= 255).then_some(cleaned)
}

/// `name`, or `name (2).ext`, … — the first that does not exist in `dir`.
fn free_name(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let (stem, ext) = match name.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (name.to_string(), String::new()),
    };
    (2..).map(|i| dir.join(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap_or(first)
}

async fn upload(State(st): State<ServerState>, AxPath(token): AxPath<String>, Query(qr): Query<PathQuery>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap, mut form: Multipart) -> Response {
    let ip = peer.ip().to_string();
    let (share, pw) = match open(&st, &token, &ip).await {
        Ok(s) => s,
        Err(r) => return r,
    };
    let store = st.auth.shares();
    if pw.as_deref().is_some_and(|h| !unlocked(&headers, &share, h)) {
        return message(StatusCode::UNAUTHORIZED, "Password required", "Open the link and enter its password first.");
    }
    if !share.kind.uploads() {
        return message(StatusCode::FORBIDDEN, "Download only", "This link does not accept uploads.");
    }
    let dir = match locate(&share, if share.kind.lists() { &qr.p } else { "" }) {
        Ok(d) if d.is_dir() => d,
        _ => return message(StatusCode::NOT_FOUND, "Not found", "That folder is not part of this link."),
    };
    // The folder's policy: lock, accepted types, size cap, intake naming.
    let policy = match super::folders::effective_for(&st, &dir).await {
        Ok(p) => p,
        Err(e) => return ApiError(e).into_response(),
    };
    if policy.locked {
        store.record(&share.id, &ip, "refused", "folder is locked").await;
        return message(StatusCode::LOCKED, "Folder locked", "This folder does not take uploads at the moment.");
    }
    let limit = (share.max_upload_bytes.unwrap_or(DEFAULT_UPLOAD_LIMIT) as u64).min(policy.max_upload_mb.map_or(u64::MAX, |mb| mb as u64 * 1024 * 1024));
    let mut saved = 0u32;
    let mut refused: Vec<String> = Vec::new();
    loop {
        let mut field = match form.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return message(StatusCode::BAD_REQUEST, "Upload failed", &e.body_text()),
        };
        let Some(name) = field.file_name().and_then(safe_name) else { continue };
        if let Err(why) = policy.accepts(&name) {
            // Drain the refused file and go on with the others.
            while let Ok(Some(_)) = field.chunk().await {}
            store.record(&share.id, &ip, "refused", &why).await;
            refused.push(why);
            continue;
        }
        let tmp = dir.join(format!(".ufop-upload-{}", uuid::Uuid::new_v4().simple()));
        let mut out = match tokio::fs::File::create(&tmp).await {
            Ok(f) => f,
            Err(e) => return ApiError(AppError::from(e)).into_response(),
        };
        let mut written = 0u64;
        let failed = loop {
            match field.chunk().await {
                Ok(Some(c)) => {
                    written += c.len() as u64;
                    if written > limit {
                        break Some((StatusCode::PAYLOAD_TOO_LARGE, format!("{name} is larger than {}", human(limit))));
                    }
                    if let Err(e) = out.write_all(&c).await {
                        break Some((StatusCode::INSUFFICIENT_STORAGE, format!("{name}: {e}")));
                    }
                }
                Ok(None) => break None,
                Err(e) => break Some((StatusCode::BAD_REQUEST, format!("{name}: {}", e.body_text()))),
            }
        };
        let flushed = out.flush().await.and(out.sync_all().await);
        drop(out);
        if let Some((code, why)) = failed.or_else(|| flushed.err().map(|e| (StatusCode::INSUFFICIENT_STORAGE, e.to_string()))) {
            let _ = tokio::fs::remove_file(&tmp).await;
            store.record(&share.id, &ip, "refused", &why).await;
            return message(code, "Upload failed", &why);
        }
        // Intake naming and the move into place are serialized, so sequence
        // numbers and timestamps follow arrival order.
        let _intake = intake_lock().lock().await;
        let named = super::folders::intake_name(policy.intake_naming, &name, chrono::Utc::now(), super::folders::next_sequence(&dir));
        let dest = free_name(&dir, &named);
        if let Err(e) = tokio::fs::rename(&tmp, &dest).await {
            let _ = tokio::fs::remove_file(&tmp).await;
            return ApiError(AppError::from(e)).into_response();
        }
        let rel = rel_of(&share, &dest);
        store.record(&share.id, &ip, "upload", &format!("{} ({})", if rel.is_empty() { name.clone() } else { rel }, human(written))).await;
        saved += 1;
    }
    if saved == 0 && !refused.is_empty() {
        return message(StatusCode::UNSUPPORTED_MEDIA_TYPE, "Not accepted", &refused.join(" "));
    }
    Redirect::to(&format!("/s/{}?p={}&uploaded={saved}&refused={}", q(&token), q(&rel_of(&share, &dir)), refused.len())).into_response()
}

fn intake_lock() -> &'static tokio::sync::Mutex<()> {
    static L: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
    L.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Public link routes (`/s/…`): no account, the token is the credential.
pub fn public_routes() -> Router<ServerState> {
    Router::new()
        .route("/s/{token}", get(view))
        .route("/s/{token}/unlock", post(unlock))
        .route("/s/{token}/dl", get(download))
        .route("/s/{token}/upload", post(upload).layer(DefaultBodyLimit::disable()))
}

// ── Admin API ──

fn share_roots(st: &ServerState) -> Result<Roots, ApiError> {
    if st.cfg.share_roots.is_empty() {
        return Err(AppError::state("Sharing is off on this server", "Start ufop-server with --share-root <folder> to allow links to files under it.").into());
    }
    Roots::new(&st.cfg.share_roots).map_err(|e| AppError::state(format!("Share roots: {e}"), "Check the --share-root folders.").into())
}

async fn list_shares(State(st): State<ServerState>) -> Result<Json<serde_json::Value>, ApiError> {
    let roots: Vec<String> = st.cfg.share_roots.iter().map(|p| p.to_string_lossy().into_owned()).collect();
    Ok(Json(serde_json::json!({ "shares": st.auth.shares().list().await?, "roots": roots })))
}

async fn create_share(State(st): State<ServerState>, Json(n): Json<NewShare>) -> Result<Json<serde_json::Value>, ApiError> {
    let roots = share_roots(&st)?;
    // The folder's policy decides which links may exist.
    let policy = super::folders::effective_for(&st, Path::new(&n.path)).await?;
    if policy.locked && n.kind.uploads() {
        return Err(bad("This folder is locked: it takes no upload links").into());
    }
    if policy.require_link_password && n.password.as_deref().map_or(true, str::is_empty) {
        return Err(bad("Links to this folder need a password").into());
    }
    if let Some(max) = policy.max_link_days {
        if n.expires_in_days.map_or(true, |d| d > max) {
            return Err(bad(format!("Links to this folder must expire within {max} days")).into());
        }
    }
    let (share, token) = st.auth.shares().create(n, &roots).await?;
    tracing::info!("Created share link {} ({}) for {}", share.id, share.kind.as_str(), share.path);
    Ok(Json(serde_json::json!({ "share": share, "token": token, "url": format!("/s/{token}") })))
}

async fn revoke_share(State(st): State<ServerState>, AxPath(id): AxPath<String>) -> Result<Json<serde_json::Value>, ApiError> {
    if !st.auth.shares().revoke(&id).await? {
        return Err(AppError::FileOperation { message: format!("Share link {id} not found"), advice: "List the links to see their ids.".into() }.into());
    }
    tracing::info!("Turned off share link {id}");
    Ok(Json(serde_json::json!({ "ok": true })))
}

#[derive(Deserialize)]
struct EventsQuery {
    limit: Option<i64>,
}

async fn share_events(State(st): State<ServerState>, AxPath(id): AxPath<String>, Query(qr): Query<EventsQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!({ "events": st.auth.shares().events(&id, qr.limit.unwrap_or(200)).await? })))
}

pub fn protected_routes() -> Router<ServerState> {
    Router::new()
        .route("/api/v1/shares", get(list_shares).post(create_share))
        .route("/api/v1/shares/{id}", axum::routing::delete(revoke_share))
        .route("/api/v1/shares/{id}/events", get(share_events))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_names_are_made_safe() {
        assert_eq!(safe_name("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(safe_name(r"C:\Users\x\report.pdf").as_deref(), Some("report.pdf"));
        assert_eq!(safe_name(".bashrc").as_deref(), Some("bashrc"));
        assert_eq!(safe_name("a<b>:c|d?.txt").as_deref(), Some("abcd.txt"));
        assert_eq!(safe_name("CON.txt"), None);
        assert_eq!(safe_name("com1"), None);
        assert_eq!(safe_name("..."), None);
        assert_eq!(safe_name("x\u{0}y.txt").as_deref(), Some("xy.txt"));
    }

    #[test]
    fn free_names_and_dispositions() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("a.txt"), "").unwrap();
        std::fs::write(d.path().join("a (2).txt"), "").unwrap();
        assert_eq!(free_name(d.path(), "a.txt"), d.path().join("a (3).txt"));
        assert_eq!(free_name(d.path(), "b"), d.path().join("b"));
        assert_eq!(disposition("Rapport été.pdf"), "attachment; filename=\"Rapport _t_.pdf\"; filename*=UTF-8''Rapport%20%C3%A9t%C3%A9.pdf");
        assert_eq!(q("a b/c&d"), "a%20b/c%26d");
    }
}
