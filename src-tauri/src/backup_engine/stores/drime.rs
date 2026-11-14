//! Drime Cloud (drime.cloud) REST API, `https://app.drime.cloud/api/v1`.
//!
//! Drime publishes no API reference. The endpoints and request/response
//! shapes follow Duplicati's Drime backend, which is maintained against the
//! live service:
//! <https://github.com/duplicati/duplicati/tree/master/Duplicati/Library/Backend/DrimeCloud>
//! (`DrimeBackend.cs`, `Model/*.cs`), and PyDrime
//! (<https://pypi.org/project/pydrime/>, `pydrime/api.py`) for
//! `user/space-usage`.
//!
//! - auth: `Authorization: Bearer <API token>` (account settings → Developers),
//!   or `POST auth/login {email, password, device_name}` → `user.access_token`
//! - list: `GET drive/file-entries` — the root by page; a folder by its hash,
//!   ordered by `created_at`, checked against `GET folders/{id}/count`, and
//!   restarted from the last `created_at` every 20 pages (Drime does not page
//!   deeper reliably)
//! - put: `POST uploads` (multipart form) below 5 MiB; above, an S3 multipart
//!   upload through `s3/multipart/{create,batch-sign-part-urls,complete,abort}`
//!   and `POST s3/entries`. Drime does not overwrite, so an existing volume of
//!   the same name is deleted first.
//! - get: `GET file-entries/download/{hash}`; delete: `POST file-entries/delete`

use super::http::{self, TlsOptions};
use super::{BoxFut, ObjectStore};
use crate::core::error::AppError;
use base64::Engine;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use tokio::sync::Mutex;
use ufop_core::backup::backend::Quota;
use ufop_core::backup::RemoteFile;

const API: &str = "https://app.drime.cloud/api/v1";
/// Uploads at or above this size use the S3 multipart path, in parts of this size.
pub const PART: usize = 5 * 1024 * 1024;
/// Pages per `created_at` window before the listing restarts from the last timestamp.
const WINDOW_PAGES: u32 = 20;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrimeConfig {
    /// Folder path, e.g. `Backups/laptop` (empty = the drive root).
    pub folder: String,
    /// Workspace id; `0` = the personal drive.
    pub workspace_id: u64,
    /// Entries per listing page (1–100).
    pub page_size: u32,
    /// Move deleted volumes to the Drime trash instead of deleting them for good.
    pub soft_delete: bool,
    /// API base (tests / self-hosted); empty = `https://app.drime.cloud/api/v1`.
    pub api_url: String,
    pub tls: TlsOptions,
}

impl Default for DrimeConfig {
    fn default() -> Self {
        Self { folder: String::new(), workspace_id: 0, page_size: 50, soft_delete: false, api_url: String::new(), tls: TlsOptions::default() }
    }
}

#[derive(Debug, Clone)]
struct Entry {
    id: u64,
    name: String,
    kind: String,
    hash: String,
    size: u64,
    parent: Option<u64>,
    created: String,
    deleted: bool,
}

impl Entry {
    fn parse(v: &Value) -> Option<Self> {
        Some(Self {
            id: v["id"].as_u64().filter(|i| *i > 0)?,
            name: v["name"].as_str()?.to_string(),
            kind: v["type"].as_str().unwrap_or_default().to_string(),
            hash: v["hash"].as_str().unwrap_or_default().to_string(),
            size: v["file_size"].as_u64().unwrap_or(0),
            parent: v["parent_id"].as_u64(),
            created: v["created_at"].as_str().unwrap_or_default().to_string(),
            deleted: v["deleted_at"].as_str().is_some_and(|s| !s.is_empty()),
        })
    }
}

#[derive(Clone)]
struct Folder {
    id: u64,
    hash: String,
}

pub struct DrimeStore {
    cfg: DrimeConfig,
    http: reqwest::Client,
    /// Bearer token: the API token, or one obtained from `auth/login`.
    token: Mutex<String>,
    login: Option<(String, String)>,
    /// `None` inside = the root.
    folder: Mutex<Option<Option<Folder>>>,
    cache: Mutex<Option<HashMap<String, Entry>>>,
}

fn pagination(msg: impl Into<String>) -> AppError {
    AppError::connection(format!("Drime listing is inconsistent: {}", msg.into()), "Retry later; Drime returned pages that do not add up, so no partial listing is used.")
}

fn created(s: &str) -> Result<chrono::DateTime<chrono::Utc>, AppError> {
    chrono::DateTime::parse_from_rfc3339(s).map(|d| d.with_timezone(&chrono::Utc)).map_err(|_| pagination(format!("invalid created_at '{s}'")))
}

impl DrimeStore {
    pub fn new(cfg: DrimeConfig, secret: String) -> Result<Self, AppError> {
        let s = secret.trim();
        let (token, login) = if s.starts_with('{') {
            let v: Value = serde_json::from_str(s).map_err(|e| AppError::validation(format!("Invalid Drime credentials JSON: {e}")))?;
            match (v["email"].as_str(), v["password"].as_str(), v["api_token"].as_str()) {
                (_, _, Some(t)) if !t.is_empty() => (t.to_string(), None),
                (Some(e), Some(p), _) if !e.is_empty() && !p.is_empty() => (String::new(), Some((e.to_string(), p.to_string()))),
                _ => return Err(AppError::validation("Drime credentials JSON needs \"api_token\", or \"email\" and \"password\"")),
            }
        } else {
            (s.to_string(), None)
        };
        Ok(Self { http: http::client(&cfg.tls)?, cfg, token: Mutex::new(token), login, folder: Mutex::new(None), cache: Mutex::new(None) })
    }

    fn api(&self) -> String {
        if self.cfg.api_url.trim().is_empty() { API.into() } else { self.cfg.api_url.trim().trim_end_matches('/').to_string() }
    }

    fn ws(&self) -> String {
        self.cfg.workspace_id.to_string()
    }

    async fn sign_in(&self) -> Result<bool, AppError> {
        let Some((email, password)) = &self.login else { return Ok(false) };
        let body = json!({ "email": email, "password": password, "device_name": "UFOP backup" });
        let r = self.http.post(format!("{}/auth/login", self.api())).header("accept", "application/json").json(&body).send().await.map_err(|e| http::net("Drime sign-in", e))?;
        let v: Value = http::check(r, "Drime sign-in").await?.json().await.map_err(|e| http::net("Drime sign-in", e))?;
        if v["user"]["banned_at"].as_str().is_some_and(|b| !b.is_empty()) {
            return Err(AppError::connection("This Drime account is banned", "Contact Drime support."));
        }
        let t = v["user"]["access_token"].as_str().filter(|t| !t.is_empty() && v["status"] == "success");
        let t = t.ok_or_else(|| AppError::connection(format!("Drime sign-in failed: {}", v["message"].as_str().unwrap_or("no access token")), "Check the e-mail address and password."))?;
        *self.token.lock().await = t.to_string();
        Ok(true)
    }

    /// Send an authenticated API request, signing in (again) once on 401.
    async fn send(&self, build: impl Fn(&str) -> reqwest::RequestBuilder, what: &str) -> Result<reqwest::Response, AppError> {
        if self.token.lock().await.is_empty() {
            self.sign_in().await?;
        }
        let t = self.token.lock().await.clone();
        let r = build(&t).bearer_auth(&t).header("accept", "application/json").send().await.map_err(|e| http::net(what, e))?;
        if r.status().as_u16() == 401 && self.sign_in().await? {
            let t = self.token.lock().await.clone();
            return build(&t).bearer_auth(&t).header("accept", "application/json").send().await.map_err(|e| http::net(what, e));
        }
        Ok(r)
    }

    async fn json(&self, build: impl Fn(&str) -> reqwest::RequestBuilder, what: &str) -> Result<Value, AppError> {
        let r = self.send(build, what).await?;
        if r.status().as_u16() == 422 {
            let v: Value = r.json().await.unwrap_or(Value::Null);
            return Err(AppError::connection(format!("{what}: {}", v["message"].as_str().unwrap_or("rejected")), "Check the destination settings."));
        }
        http::check(r, what).await?.json().await.map_err(|e| http::net(what, e))
    }

    /// One page, retried when Drime answers with a different page than asked for.
    async fn page(&self, query: &[(String, String)], page: u32) -> Result<Value, AppError> {
        let url = format!("{}/drive/file-entries", self.api());
        let mut last = Value::Null;
        for attempt in 1..=3u64 {
            let mut q = query.to_vec();
            q.push(("page".into(), page.to_string()));
            last = self.json(|_| self.http.get(&url).query(&q), "Drime listing").await?;
            if last["current_page"].as_u64() == Some(page as u64) {
                return Ok(last);
            }
            tokio::time::sleep(std::time::Duration::from_millis(250 * attempt)).await;
        }
        Err(pagination(format!("asked for page {page}, got current_page={}", last["current_page"])))
    }

    fn per_page(&self) -> u32 {
        self.cfg.page_size.clamp(1, 100)
    }

    /// The root's entries (no folder hash to scope by): page until `last_page`, checked against `total`.
    async fn list_root(&self, folders_only: bool) -> Result<Vec<Entry>, AppError> {
        let mut q = vec![("perPage".into(), self.per_page().to_string()), ("workspaceId".into(), self.ws()), ("orderBy".into(), "name".into()), ("orderDir".into(), "asc".into())];
        if folders_only {
            q.push(("type".into(), "folder".into()));
        }
        let mut seen = HashSet::new();
        let mut out = Vec::new();
        let mut page = 1;
        loop {
            let v = self.page(&q, page).await?;
            let total = v["total"].as_u64().ok_or_else(|| pagination("missing total"))?;
            let last = v["last_page"].as_u64().ok_or_else(|| pagination("missing last_page"))?;
            let data = v["data"].as_array().ok_or_else(|| pagination("missing data"))?;
            for e in data {
                let e = Entry::parse(e).ok_or_else(|| pagination("entry without id or name"))?;
                if seen.insert(e.id) {
                    out.push(e);
                }
            }
            if page as u64 >= last {
                if seen.len() as u64 != total {
                    return Err(pagination(format!("{} entries listed, total={total}", seen.len())));
                }
                break;
            }
            if data.is_empty() {
                return Err(pagination(format!("empty page {page} before last_page={last}")));
            }
            page += 1;
        }
        // Root entries have no parent.
        Ok(out.into_iter().filter(|e| e.parent.is_none()).collect())
    }

    async fn count(&self, id: u64) -> Result<u64, AppError> {
        let url = format!("{}/folders/{id}/count", self.api());
        let ws = self.ws();
        let v = self.json(|_| self.http.get(&url).query(&[("workspaceId", ws.as_str())]), "Drime folder count").await?;
        v["count"].as_u64().ok_or_else(|| pagination("missing folder count"))
    }

    /// Active entries of a folder, in `created_at` windows, validated against the folder count.
    async fn list_folder(&self, f: &Folder) -> Result<Vec<Entry>, AppError> {
        let expected = self.count(f.id).await?;
        let mut observed: HashSet<u64> = HashSet::new();
        let mut active: Vec<Entry> = Vec::new();
        let mut active_ids: HashSet<u64> = HashSet::new();
        let mut lower: Option<chrono::DateTime<chrono::Utc>> = None;
        let mut last_date: Option<chrono::DateTime<chrono::Utc>> = None;
        let (mut page, mut requests) = (1u32, 0u64);
        let mut fingerprints = HashSet::new();
        let mut window_ids: HashSet<u64> = HashSet::new();
        loop {
            requests += 1;
            if requests > (expected.max(observed.len() as u64) * 4).max(1000) {
                return Err(pagination("request limit reached"));
            }
            let mut q = vec![
                ("pageId".into(), f.hash.clone()),
                ("folderId".into(), f.hash.clone()),
                ("workspaceId".into(), self.ws()),
                ("perPage".into(), self.per_page().to_string()),
                ("orderBy".into(), "created_at".into()),
                ("orderDir".into(), "asc".into()),
            ];
            if let Some(l) = lower {
                // The web app's `encodeDriveFilters`: URI-encoded base64 JSON, then query-encoded again.
                let filter = json!([{ "key": "created_at", "operator": ">=", "value": l.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string() }]).to_string();
                let b64 = base64::engine::general_purpose::STANDARD.encode(filter);
                q.push(("filters".into(), urlencoding::encode(&b64).into_owned()));
            }
            let v = self.page(&q, page).await?;
            let data = v["data"].as_array().ok_or_else(|| pagination("missing data"))?;
            let last_page = v["last_page"].as_u64();
            if !data.is_empty() {
                let mut ids: Vec<u64> = data.iter().filter_map(|e| e["id"].as_u64()).collect();
                ids.sort_unstable();
                if !fingerprints.insert(ids) {
                    return Err(pagination(format!("page {page} repeated")));
                }
            }
            for e in data {
                let e = Entry::parse(e).ok_or_else(|| pagination("entry without id or name"))?;
                if e.parent != Some(f.id) {
                    return Err(pagination(format!("entry {} belongs to another folder", e.id)));
                }
                let d = created(&e.created)?;
                if lower.is_some_and(|l| d < l) || last_date.is_some_and(|l| d < l) {
                    return Err(pagination("created_at out of order"));
                }
                last_date = Some(d);
                window_ids.insert(e.id);
                observed.insert(e.id);
                if !e.deleted && active_ids.insert(e.id) {
                    active.push(e);
                }
            }
            if active.len() as u64 > expected {
                return Err(pagination(format!("{} active entries, folder count {expected}", active.len())));
            }
            if last_page == Some(page as u64) {
                if v["total"].as_u64() != Some(window_ids.len() as u64) {
                    return Err(pagination(format!("terminal page total={} but {} entries in the window", v["total"], window_ids.len())));
                }
                if active.len() as u64 != expected {
                    return Err(pagination(format!("{} active entries, folder count {expected}", active.len())));
                }
                break;
            }
            if active.len() as u64 == expected {
                break;
            }
            if data.is_empty() {
                return Err(pagination(format!("empty page {page} with {} of {expected} entries", active.len())));
            }
            if page >= WINDOW_PAGES && (lower.is_none() || last_date > lower) {
                // Inclusive boundary: entries sharing the timestamp are listed again and deduplicated.
                lower = last_date;
                page = 1;
                fingerprints.clear();
                window_ids.clear();
            } else {
                page += 1;
            }
        }
        if self.count(f.id).await? != expected {
            return Err(pagination("the folder changed while it was listed"));
        }
        Ok(active)
    }

    async fn children(&self, parent: &Option<Folder>, folders_only: bool) -> Result<Vec<Entry>, AppError> {
        match parent {
            None => self.list_root(folders_only).await,
            Some(f) => self.list_folder(f).await,
        }
    }

    async fn find_folder(&self, parent: &Option<Folder>, name: &str) -> Result<Option<Folder>, AppError> {
        Ok(self
            .children(parent, true)
            .await?
            .into_iter()
            .find(|e| e.kind == "folder" && !e.deleted && e.name.to_lowercase() == name.to_lowercase())
            .map(|e| Folder { id: e.id, hash: e.hash }))
    }

    async fn target(&self, create: bool) -> Result<Option<Folder>, AppError> {
        if let Some(f) = self.folder.lock().await.clone() {
            return Ok(f);
        }
        let mut cur: Option<Folder> = None;
        for seg in self.cfg.folder.split('/').filter(|s| !s.is_empty()) {
            cur = match self.find_folder(&cur, seg).await? {
                Some(f) => Some(f),
                None if create => {
                    let url = format!("{}/folders", self.api());
                    let ws = self.ws();
                    let body = json!({ "name": seg, "parentId": cur.as_ref().map(|f| f.id) });
                    let v = self.json(|_| self.http.post(&url).query(&[("workspaceId", ws.as_str())]).json(&body), "Drime folder creation").await?;
                    let e = (v["status"] == "success").then(|| Entry::parse(&v["folder"])).flatten();
                    let e = e.ok_or_else(|| AppError::connection(format!("Drime did not create folder '{seg}': {}", v["message"].as_str().unwrap_or("")), "Retry."))?;
                    Some(Folder { id: e.id, hash: e.hash })
                }
                None => return Err(AppError::not_found(format!("Drime folder '{}' does not exist", self.cfg.folder))),
            };
        }
        *self.folder.lock().await = Some(cur.clone());
        Ok(cur)
    }

    async fn files(&self) -> Result<HashMap<String, Entry>, AppError> {
        let f = self.target(false).await?;
        let m: HashMap<String, Entry> = self.children(&f, false).await?.into_iter().filter(|e| e.kind != "folder" && !e.deleted).map(|e| (e.name.clone(), e)).collect();
        *self.cache.lock().await = Some(m.clone());
        Ok(m)
    }

    async fn lookup(&self, name: &str) -> Result<Option<Entry>, AppError> {
        if let Some(c) = self.cache.lock().await.as_ref() {
            if let Some(e) = c.get(name) {
                return Ok(Some(e.clone()));
            }
        }
        Ok(self.files().await?.remove(name))
    }

    async fn remove(&self, e: &Entry) -> Result<(), AppError> {
        let url = format!("{}/file-entries/delete", self.api());
        let body = json!({ "entryIds": [e.id], "deleteForever": !self.cfg.soft_delete });
        let r = self.send(|_| self.http.post(&url).json(&body), "Drime delete").await?;
        // An id that is already gone is reported as a validation error on entryIds.
        if r.status().as_u16() == 422 || r.status().as_u16() == 404 {
            let v: Value = r.json().await.unwrap_or(Value::Null);
            if v["errors"].get("entryIds").is_some() || v.is_null() {
                return Ok(());
            }
            return Err(AppError::connection(format!("Drime delete: {}", v["message"].as_str().unwrap_or("rejected")), "Retry."));
        }
        http::check(r, "Drime delete").await?;
        Ok(())
    }

    async fn upload_small(&self, name: &str, data: Vec<u8>, parent: &Option<Folder>) -> Result<Value, AppError> {
        let url = format!("{}/uploads", self.api());
        let ws = self.ws();
        let parent_id = parent.as_ref().map(|f| f.id.to_string());
        self.json(
            |_| {
                let mut form = reqwest::multipart::Form::new()
                    .part("file", reqwest::multipart::Part::bytes(data.clone()).file_name(name.to_string()).mime_str("application/octet-stream").expect("static mime"))
                    .text("workspaceId", ws.clone());
                if let Some(p) = &parent_id {
                    form = form.text("parentId", p.clone());
                }
                self.http.post(&url).multipart(form)
            },
            "Drime upload",
        )
        .await
    }

    async fn upload_multipart(&self, name: &str, data: &[u8], parent: &Option<Folder>) -> Result<Value, AppError> {
        let api = self.api();
        let ext = name.rsplit_once('.').map(|(_, e)| e).filter(|e| !e.is_empty()).unwrap_or("bin").to_string();
        let parent_id = parent.as_ref().map(|f| f.id);
        let body = json!({ "filename": name, "mime": "application/octet-stream", "size": data.len(), "extension": ext, "workspaceId": self.cfg.workspace_id, "parentId": parent_id });
        let c = self.json(|_| self.http.post(format!("{api}/s3/multipart/create")).json(&body), "Drime multipart upload").await?;
        let (Some(key), Some(upload_id)) = (c["key"].as_str().map(String::from), c["uploadId"].as_str().map(String::from)) else {
            return Err(AppError::connection("Drime did not start the multipart upload", "Retry."));
        };
        let result = async {
            let parts: Vec<usize> = (1..=data.len().div_ceil(PART)).collect();
            let body = json!({ "key": key, "uploadId": upload_id, "partNumbers": parts });
            let s = self.json(|_| self.http.post(format!("{api}/s3/multipart/batch-sign-part-urls")).json(&body), "Drime multipart signing").await?;
            let urls: HashMap<u64, String> = s["urls"].as_array().into_iter().flatten().filter_map(|u| Some((u["partNumber"].as_u64()?, u["url"].as_str()?.to_string()))).collect();
            let mut done = Vec::new();
            for (i, chunk) in data.chunks(PART).enumerate() {
                let n = i as u64 + 1;
                let url = urls.get(&n).ok_or_else(|| AppError::connection(format!("Drime returned no URL for part {n}"), "Retry."))?;
                // Pre-signed S3 URL: no Authorization or Content-Type header.
                let r = self.http.put(url).body(chunk.to_vec()).send().await.map_err(|e| http::net("Drime part upload", e))?;
                let r = http::check(r, &format!("Drime part {n} upload")).await?;
                let etag = r.headers().get("etag").and_then(|v| v.to_str().ok()).filter(|e| !e.is_empty()).ok_or_else(|| AppError::connection(format!("Drime part {n}: no ETag"), "Retry."))?;
                // `complete` expects PascalCase part fields.
                done.push(json!({ "PartNumber": n, "ETag": etag }));
            }
            let body = json!({ "key": key, "uploadId": upload_id, "parts": done });
            self.json(|_| self.http.post(format!("{api}/s3/multipart/complete")).json(&body), "Drime multipart completion").await?;
            let file = key.rsplit('/').next().unwrap_or(&key).to_string();
            let body = json!({ "filename": file, "size": data.len(), "clientName": name, "clientMime": "application/octet-stream", "clientExtension": ext, "workspaceId": self.cfg.workspace_id, "parentId": parent_id });
            self.json(|_| self.http.post(format!("{api}/s3/entries")).json(&body), "Drime file registration").await
        }
        .await;
        if result.is_err() {
            let body = json!({ "key": key, "uploadId": upload_id });
            let _ = self.send(|_| self.http.post(format!("{api}/s3/multipart/abort")).json(&body), "Drime multipart abort").await;
        }
        result
    }
}

impl ObjectStore for DrimeStore {
    fn describe(&self) -> String {
        format!("drime://{}/{}", self.cfg.workspace_id, self.cfg.folder.trim_matches('/'))
    }

    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move { Ok(self.files().await?.into_values().map(|e| RemoteFile { name: e.name, size: e.size }).collect()) })
    }

    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let parent = self.target(false).await?;
            // Drime does not overwrite: a second upload would get a renamed copy.
            if let Some(old) = self.lookup(name).await? {
                self.remove(&old).await?;
            }
            *self.cache.lock().await = None;
            let len = data.len() as u64;
            let v = if data.len() >= PART { self.upload_multipart(name, &data, &parent).await? } else { self.upload_small(name, data, &parent).await? };
            let e = (v["status"] == "success").then(|| Entry::parse(&v["fileEntry"])).flatten();
            let e = e.ok_or_else(|| AppError::connection(format!("Drime upload of {name} failed: {}", v["message"].as_str().unwrap_or("invalid response")), "Retry."))?;
            if e.size != len {
                return Err(AppError::connection(format!("Drime stored {} of {len} bytes for {name}", e.size), "Retry the upload."));
            }
            Ok(())
        })
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let e = self.lookup(name).await?.ok_or_else(|| AppError::not_found(format!("Drime: {name} not found")))?;
            let url = format!("{}/file-entries/download/{}", self.api(), urlencoding::encode(&e.hash));
            let r = http::check(self.send(|_| self.http.get(&url), "Drime download").await?, "Drime download").await?;
            let b = r.bytes().await.map_err(|er| http::net("Drime download", er))?.to_vec();
            if b.len() as u64 != e.size {
                return Err(AppError::connection(format!("Drime returned {} of {} bytes for {name}", b.len(), e.size), "Retry."));
            }
            Ok(b)
        })
    }

    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let e = match self.lookup(name).await {
                Err(e) if e.user_message().contains("does not exist") => return Ok(()),
                r => r?,
            };
            if let Some(e) = e {
                self.remove(&e).await?;
            }
            if let Some(c) = self.cache.lock().await.as_mut() {
                c.remove(name);
            }
            Ok(())
        })
    }

    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move { self.target(true).await.map(|_| ()) })
    }

    fn quota(&self) -> BoxFut<'_, Option<Quota>> {
        Box::pin(async move {
            let url = format!("{}/user/space-usage", self.api());
            let v = self.json(|_| self.http.get(&url), "Drime space usage").await?;
            // `available` is the plan's total space (PyDrime: free = available - used).
            let (Some(total), Some(used)) = (v["available"].as_u64(), v["used"].as_u64()) else { return Ok(None) };
            Ok(Some(Quota { total, free: total.saturating_sub(used) }))
        })
    }
}
