//! Movistar Cloud (Telefónica "MiCloud", Zefiro/Funambol-style `sapi` API),
//! `https://micloud.movistar.es`.
//!
//! There is no public API reference. Endpoints, parameters and response
//! shapes follow Duplicati's Movistar Cloud backend, which drives the same
//! web API the browser client uses:
//! <https://github.com/duplicati/duplicati/tree/master/Duplicati/Library/Backend/MovistarCloud>
//! (`MovistarCloudApiClient.cs`, `MovistarCloudBackend.cs`).
//!
//! - sign-in: `POST /sapi/login?action=login` (form `login`, `password`,
//!   header `X-Deviceid`) → `data.jsessionid`, `data.validationkey`; later
//!   calls send both as cookies and `validationkey` as a query parameter. An
//!   HTML page or 401 means the session expired: the store signs in again once.
//! - list: `POST /sapi/media?action=get&folderid=…&limit=…` (files) — the API
//!   has no paging, so a listing that reaches the limit is refused rather
//!   than used partially.
//! - put: `POST https://upload.micloud.movistar.es/sapi/upload?action=save&acceptasynchronous=true`
//!   (multipart: JSON `data` part + `file` part), then poll
//!   `get-validation-status` until the item is usable (`U`).
//! - get: `POST /sapi/media?action=get&origin=omh,dropbox` → signed `url`.
//! - delete: `POST /sapi/media/file?action=delete&softdelete=true` (to the trash).
//! - quota: `POST /sapi/media?action=get-storage-space&softdeleted=true`.
//!
//! Movistar closes the storage of accounts that leave Movistar; the store
//! reports the service's sign-in error in that case.

use super::http::{self, TlsOptions};
use super::{BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;
use tokio::sync::Mutex;
use ufop_core::backup::backend::Quota;
use ufop_core::backup::RemoteFile;

const BASE: &str = "https://micloud.movistar.es";
const UPLOAD: &str = "https://upload.micloud.movistar.es";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MovistarConfig {
    /// Account e-mail address.
    pub email: String,
    /// The web client's device id (`X-Deviceid` request header in the browser's developer tools).
    pub device_id: String,
    /// Folder path, e.g. `Backups/laptop` (empty = the root folder).
    pub folder: String,
    /// Maximum items per listing call.
    pub list_limit: u32,
    /// Wait until the service has validated each upload.
    pub wait_for_validation: bool,
    pub validation_timeout_secs: u64,
    pub validation_poll_secs: u64,
    /// Service / upload hosts (tests); empty = Movistar's.
    pub base_url: String,
    pub upload_url: String,
    pub tls: TlsOptions,
}

impl Default for MovistarConfig {
    fn default() -> Self {
        Self {
            email: String::new(),
            device_id: String::new(),
            folder: String::new(),
            list_limit: 2000,
            wait_for_validation: true,
            validation_timeout_secs: 600,
            validation_poll_secs: 2,
            base_url: String::new(),
            upload_url: String::new(),
            tls: TlsOptions::default(),
        }
    }
}

#[derive(Clone)]
struct Session {
    jsessionid: String,
    validation_key: String,
}

/// File name → (media id, size) of every copy in the backup folder.
type MediaIndex = HashMap<String, Vec<(u64, u64)>>;

pub struct MovistarStore {
    cfg: MovistarConfig,
    password: String,
    http: reqwest::Client,
    session: Mutex<Option<Session>>,
    folder: Mutex<Option<u64>>,
    cache: Mutex<Option<MediaIndex>>,
}

/// Session expired / not signed in (the service answers with its HTML login page).
struct Expired;

fn id_of(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}

fn valid_name(name: &str) -> Result<(), AppError> {
    // Duplicati restricts names to these characters for this service.
    match name.chars().find(|c| !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))) {
        Some(c) => Err(AppError::validation(format!("Movistar Cloud: unsupported character '{c}' in '{name}'"))),
        None => Ok(()),
    }
}

impl MovistarStore {
    pub fn new(cfg: MovistarConfig, password: String) -> Result<Self, AppError> {
        if cfg.email.trim().is_empty() || cfg.device_id.trim().is_empty() {
            return Err(AppError::validation("Enter the Movistar Cloud e-mail address and device id"));
        }
        Ok(Self { http: http::client(&cfg.tls)?, cfg, password, session: Mutex::new(None), folder: Mutex::new(None), cache: Mutex::new(None) })
    }

    fn base(&self) -> String {
        if self.cfg.base_url.trim().is_empty() { BASE.into() } else { self.cfg.base_url.trim().trim_end_matches('/').to_string() }
    }

    fn upload_base(&self) -> String {
        if self.cfg.upload_url.trim().is_empty() { UPLOAD.into() } else { self.cfg.upload_url.trim().trim_end_matches('/').to_string() }
    }

    async fn sign_in(&self) -> Result<Session, AppError> {
        let base = self.base();
        // The home page hands out the initial session cookies.
        let jar = std::sync::Arc::new(reqwest::cookie::Jar::default());
        let c = http::builder(&self.cfg.tls)?.cookie_provider(jar).build().map_err(|e| AppError::internal(format!("HTTP client: {e}")))?;
        http::check(c.get(format!("{base}/")).send().await.map_err(|e| http::net("Movistar Cloud", e))?, "Movistar Cloud").await?;
        let r = c
            .post(format!("{base}/sapi/login?action=login"))
            .header("X-Deviceid", self.cfg.device_id.trim())
            .form(&[("login", self.cfg.email.trim()), ("password", self.password.as_str())])
            .send()
            .await
            .map_err(|e| http::net("Movistar Cloud sign-in", e))?;
        let v: Value = http::check(r, "Movistar Cloud sign-in").await?.json().await.map_err(|e| http::net("Movistar Cloud sign-in", e))?;
        let (Some(js), Some(vk)) = (v["data"]["jsessionid"].as_str().filter(|s| !s.is_empty()), v["data"]["validationkey"].as_str().filter(|s| !s.is_empty())) else {
            let why = v["error"]["message"].as_str().or(v["message"].as_str()).unwrap_or("no session returned");
            return Err(AppError::connection(format!("Movistar Cloud sign-in failed: {why}"), "Check the e-mail address, password and device id."));
        };
        let s = Session { jsessionid: js.to_string(), validation_key: vk.to_string() };
        *self.session.lock().await = Some(s.clone());
        Ok(s)
    }

    async fn current(&self) -> Result<Session, AppError> {
        // Bind first: a guard in the match scrutinee would live through
        // `sign_in`, which takes the same lock (deadlock on first use).
        let cached = self.session.lock().await.clone();
        match cached {
            Some(s) => Ok(s),
            None => self.sign_in().await,
        }
    }

    fn with_session(&self, rb: reqwest::RequestBuilder, s: &Session) -> reqwest::RequestBuilder {
        rb.query(&[("validationkey", s.validation_key.as_str())]).header("cookie", format!("JSESSIONID={}; validationkey={}", s.jsessionid, s.validation_key))
    }

    async fn try_json(&self, build: &(dyn Fn() -> reqwest::RequestBuilder + Send + Sync), s: &Session, what: &str) -> Result<Result<Value, Expired>, AppError> {
        let r = self.with_session(build(), s).send().await.map_err(|e| http::net(what, e))?;
        let html = r.headers().get("content-type").and_then(|v| v.to_str().ok()).is_some_and(|c| c.contains("text/html"));
        if r.status().as_u16() == 401 || html {
            return Ok(Err(Expired));
        }
        let text = http::check(r, what).await?.text().await.map_err(|e| http::net(what, e))?;
        if text.trim_start().starts_with('<') {
            return Ok(Err(Expired));
        }
        let v: Value = serde_json::from_str(&text).map_err(|e| AppError::connection(format!("{what}: unexpected reply ({e})"), "Retry."))?;
        if let Some(err) = v.get("error").filter(|e| !e.is_null()) {
            return Err(AppError::connection(format!("{what} failed: {}", err["message"].as_str().unwrap_or(&err.to_string())), "Check the destination settings."));
        }
        Ok(Ok(v))
    }

    /// An API call with the session; signs in again once when it expired.
    async fn call(&self, build: impl Fn() -> reqwest::RequestBuilder + Send + Sync, what: &str) -> Result<Value, AppError> {
        let s = self.current().await?;
        if let Ok(v) = self.try_json(&build, &s, what).await? {
            return Ok(v);
        }
        let s = self.sign_in().await?;
        self.try_json(&build, &s, what).await?.map_err(|_| AppError::connection(format!("{what}: the session was refused right after signing in"), "Check the account; Movistar may have closed its cloud storage."))
    }

    async fn root(&self) -> Result<u64, AppError> {
        let url = format!("{}/sapi/media/folder/root", self.base());
        let v = self.call(|| self.http.get(&url).query(&[("action", "get")]), "Movistar Cloud root folder").await?;
        v["data"]["folders"].get(0).and_then(|f| id_of(&f["id"])).ok_or_else(|| AppError::connection("Movistar Cloud returned no root folder", "Retry."))
    }

    async fn subfolder(&self, parent: u64, name: &str) -> Result<Option<u64>, AppError> {
        let url = format!("{}/sapi/media/folder", self.base());
        let (p, l) = (parent.to_string(), self.cfg.list_limit.max(1).to_string());
        let v = self.call(|| self.http.get(&url).query(&[("action", "list"), ("parentid", p.as_str()), ("limit", l.as_str())]), "Movistar Cloud folder listing").await?;
        Ok(v["data"]["folders"].as_array().into_iter().flatten().find(|f| f["name"] == name).and_then(|f| id_of(&f["id"])))
    }

    async fn target(&self, create: bool) -> Result<u64, AppError> {
        if let Some(id) = *self.folder.lock().await {
            return Ok(id);
        }
        let mut cur = self.root().await?;
        for seg in self.cfg.folder.split('/').map(str::trim).filter(|s| !s.is_empty()) {
            cur = match self.subfolder(cur, seg).await? {
                Some(id) => id,
                None if create => {
                    let url = format!("{}/sapi/media/folder", self.base());
                    let body = json!({ "data": { "magic": false, "offline": false, "name": seg, "parentid": cur } });
                    let v = self.call(|| self.http.post(&url).query(&[("action", "save")]).json(&body), "Movistar Cloud folder creation").await?;
                    id_of(&v["data"]["folder"]["id"]).ok_or_else(|| AppError::connection(format!("Movistar Cloud did not create folder '{seg}'"), "Retry."))?
                }
                None => return Err(AppError::not_found(format!("Movistar Cloud folder '{}' does not exist", self.cfg.folder))),
            };
        }
        *self.folder.lock().await = Some(cur);
        Ok(cur)
    }

    /// name → [(id, size)] of the target folder's files.
    async fn files(&self) -> Result<HashMap<String, Vec<(u64, u64)>>, AppError> {
        let folder = self.target(false).await?;
        let url = format!("{}/sapi/media", self.base());
        let limit = self.cfg.list_limit.max(1);
        let (f, l) = (folder.to_string(), limit.to_string());
        let body = json!({ "data": { "fields": ["name", "modificationdate", "size", "etag"] } });
        let v = self.call(|| self.http.post(&url).query(&[("action", "get"), ("folderid", f.as_str()), ("limit", l.as_str())]).json(&body), "Movistar Cloud listing").await?;
        let media = v["data"]["media"].as_array().cloned().unwrap_or_default();
        if media.len() as u64 >= limit as u64 {
            return Err(AppError::connection(
                format!("Movistar Cloud listing returned {} items, the list limit; it may be incomplete", media.len()),
                "Raise the list limit in the destination settings.",
            ));
        }
        let mut out: HashMap<String, Vec<(u64, u64)>> = HashMap::new();
        for m in media {
            if let (Some(id), Some(name)) = (id_of(&m["id"]), m["name"].as_str()) {
                out.entry(name.to_string()).or_default().push((id, m["size"].as_u64().unwrap_or(0)));
            }
        }
        *self.cache.lock().await = Some(out.clone());
        Ok(out)
    }

    async fn lookup(&self, name: &str) -> Result<Vec<(u64, u64)>, AppError> {
        if let Some(c) = self.cache.lock().await.as_ref() {
            if let Some(e) = c.get(name) {
                return Ok(e.clone());
            }
        }
        Ok(self.files().await?.remove(name).unwrap_or_default())
    }

    async fn remove(&self, id: u64) -> Result<(), AppError> {
        let url = format!("{}/sapi/media/file", self.base());
        let body = json!({ "data": { "files": [id] } });
        self.call(|| self.http.post(&url).query(&[("action", "delete"), ("softdelete", "true")]).json(&body), "Movistar Cloud delete").await.map(|_| ())
    }

    async fn wait_validated(&self, id: u64, name: &str) -> Result<(), AppError> {
        let url = format!("{}/sapi/media", self.base());
        let body = json!({ "data": { "ids": [{ "id": id }] } });
        let deadline = tokio::time::Instant::now() + Duration::from_secs(self.cfg.validation_timeout_secs);
        loop {
            let v = self.call(|| self.http.post(&url).query(&[("action", "get-validation-status")]).json(&body), "Movistar Cloud upload validation").await?;
            if v["data"]["ids"][0]["status"].as_str().is_some_and(|s| s.eq_ignore_ascii_case("U")) {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                // Duplicati treats this as best effort as well: the data is stored, validation lags.
                tracing::warn!(target: "backup", "Movistar Cloud did not finish validating {name} (id {id}) within {}s", self.cfg.validation_timeout_secs);
                return Ok(());
            }
            tokio::time::sleep(Duration::from_secs(self.cfg.validation_poll_secs.max(1))).await;
        }
    }
}

impl ObjectStore for MovistarStore {
    fn describe(&self) -> String {
        format!("movistarcloud://{}/{}", self.cfg.email.trim(), self.cfg.folder.trim_matches('/'))
    }

    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move { Ok(self.files().await?.into_iter().filter_map(|(name, v)| v.last().map(|(_, size)| RemoteFile { name, size: *size })).collect()) })
    }

    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            valid_name(name)?;
            let folder = self.target(false).await?;
            let previous = self.lookup(name).await?;
            let url = format!("{}/sapi/upload", self.upload_base());
            let meta = json!({ "data": {
                "name": name,
                "size": data.len(),
                "modificationdate": chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string(),
                "contenttype": "application/octet-stream",
                "folderid": folder,
            } })
            .to_string();
            let v = self
                .call(
                    || {
                        let form = reqwest::multipart::Form::new()
                            .part("data", reqwest::multipart::Part::text(meta.clone()).mime_str("application/json").expect("static mime"))
                            .part("file", reqwest::multipart::Part::bytes(data.clone()).file_name(name.to_string()).mime_str("application/octet-stream").expect("static mime"));
                        self.http.post(&url).query(&[("action", "save"), ("acceptasynchronous", "true")]).multipart(form)
                    },
                    "Movistar Cloud upload",
                )
                .await?;
            let id = id_of(&v["id"]).ok_or_else(|| AppError::connection(format!("Movistar Cloud upload of {name} returned no id"), "Retry."))?;
            *self.cache.lock().await = None;
            if self.cfg.wait_for_validation {
                self.wait_validated(id, name).await?;
            }
            for (old, _) in previous.into_iter().filter(|(o, _)| *o != id) {
                self.remove(old).await?;
            }
            Ok(())
        })
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let (id, size) = self.lookup(name).await?.pop().ok_or_else(|| AppError::not_found(format!("Movistar Cloud: {name} not found")))?;
            let url = format!("{}/sapi/media", self.base());
            let body = json!({ "data": { "ids": [id], "fields": ["url", "name", "size", "etag"] } });
            let v = self.call(|| self.http.post(&url).query(&[("action", "get"), ("origin", "omh,dropbox")]).json(&body), "Movistar Cloud download link").await?;
            let link = v["data"]["media"][0]["url"].as_str().ok_or_else(|| AppError::connection(format!("Movistar Cloud returned no download link for {name}"), "Retry."))?.to_string();
            // The link is signed; the session cookies go only to Movistar's own hosts.
            let base_host = url::Url::parse(&self.base()).ok().and_then(|u| u.host_str().map(String::from)).unwrap_or_default();
            let mut rb = self.http.get(&link);
            if url::Url::parse(&link).ok().and_then(|u| u.host_str().map(String::from)).is_some_and(|h| h == base_host || h.ends_with(&format!(".{base_host}"))) {
                let s = self.current().await?;
                rb = rb.header("cookie", format!("JSESSIONID={}; validationkey={}", s.jsessionid, s.validation_key));
            }
            let r = http::check(rb.send().await.map_err(|e| http::net("Movistar Cloud download", e))?, "Movistar Cloud download").await?;
            let b = r.bytes().await.map_err(|e| http::net("Movistar Cloud download", e))?.to_vec();
            if size > 0 && b.len() as u64 != size {
                return Err(AppError::connection(format!("Movistar Cloud returned {} of {size} bytes for {name}", b.len()), "Retry."));
            }
            Ok(b)
        })
    }

    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let ids = match self.lookup(name).await {
                Err(e) if e.user_message().contains("does not exist") => return Ok(()),
                r => r?,
            };
            for (id, _) in ids {
                self.remove(id).await?;
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
            let url = format!("{}/sapi/media", self.base());
            let v = self
                .call(|| self.http.post(&url).query(&[("action", "get-storage-space"), ("softdeleted", "true")]).header("content-type", "application/x-www-form-urlencoded").body(""), "Movistar Cloud storage space")
                .await?;
            let d = &v["data"];
            if d["nolimit"].as_bool() == Some(true) {
                return Ok(None);
            }
            let (Some(used), Some(free)) = (d["used"].as_u64(), d["free"].as_u64()) else { return Ok(None) };
            Ok(Some(Quota { total: used + free, free }))
        })
    }
}
