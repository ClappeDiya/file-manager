//! Consumer cloud drives over OAuth: Dropbox, Google Drive, Box, pCloud.
//!
//! The secret is either a bare access token or a JSON object
//! `{access_token, refresh_token, client_id, client_secret, token_url}`;
//! with a refresh token the store renews an expired access token itself.

use super::http;
use super::{BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::sync::Mutex;
use ufop_core::backup::RemoteFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DriveConfig {
    /// Folder path, e.g. `Backups/laptop`.
    pub folder: String,
    /// API base override (self-hosted / tests); empty = the service default.
    pub api_base: String,
    /// Upload/content host override; empty = the service default.
    pub content_base: String,
    /// pCloud data region: `us` or `eu`.
    pub region: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Dropbox,
    GoogleDrive,
    Box,
    Pcloud,
}

/// OAuth credentials with optional refresh.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OAuthSecret {
    pub access_token: String,
    pub refresh_token: String,
    pub client_id: String,
    pub client_secret: String,
    pub token_url: String,
}

impl OAuthSecret {
    pub fn parse(secret: &str, default_token_url: &str) -> Result<Self, AppError> {
        let s = secret.trim();
        let mut o: OAuthSecret = if s.starts_with('{') {
            serde_json::from_str(s).map_err(|e| AppError::validation(format!("Invalid OAuth credentials: {e}")))?
        } else {
            OAuthSecret { access_token: s.to_string(), ..Default::default() }
        };
        if o.token_url.is_empty() {
            o.token_url = default_token_url.to_string();
        }
        if o.access_token.is_empty() && o.refresh_token.is_empty() {
            return Err(AppError::validation("The OAuth credentials need an access or refresh token"));
        }
        Ok(o)
    }
}

/// Access token holder that refreshes on demand.
pub struct Tokens {
    secret: Mutex<OAuthSecret>,
}

impl Tokens {
    pub fn new(s: OAuthSecret) -> Self {
        Self { secret: Mutex::new(s) }
    }

    pub async fn access(&self, http: &reqwest::Client) -> Result<String, AppError> {
        let needs = self.secret.lock().await.access_token.is_empty();
        if needs {
            self.refresh(http).await?;
        }
        Ok(self.secret.lock().await.access_token.clone())
    }

    /// Renew the access token; `false` when no refresh token is available.
    pub async fn refresh(&self, http: &reqwest::Client) -> Result<bool, AppError> {
        let s = self.secret.lock().await.clone();
        if s.refresh_token.is_empty() {
            return Ok(false);
        }
        let mut form = vec![("grant_type", "refresh_token".to_string()), ("refresh_token", s.refresh_token.clone())];
        if !s.client_id.is_empty() {
            form.push(("client_id", s.client_id.clone()));
        }
        if !s.client_secret.is_empty() {
            form.push(("client_secret", s.client_secret.clone()));
        }
        let r = http.post(&s.token_url).form(&form).send().await.map_err(|e| http::net("Token refresh", e))?;
        let v: serde_json::Value = http::check(r, "Token refresh").await?.json().await.map_err(|e| http::net("Token refresh", e))?;
        let mut g = self.secret.lock().await;
        g.access_token = v["access_token"].as_str().unwrap_or_default().to_string();
        if let Some(rt) = v["refresh_token"].as_str() {
            g.refresh_token = rt.to_string();
        }
        Ok(!g.access_token.is_empty())
    }
}

pub struct DriveStore {
    kind: Kind,
    cfg: DriveConfig,
    http: reqwest::Client,
    tokens: Tokens,
    /// Google Drive / Box: folder id and name → file id cache.
    folder_id: Mutex<Option<String>>,
    ids: Mutex<HashMap<String, String>>,
}

fn segs(folder: &str) -> Vec<&str> {
    folder.split('/').filter(|s| !s.is_empty()).collect()
}

impl DriveStore {
    pub fn new(kind: Kind, cfg: DriveConfig, secret: String) -> Result<Self, AppError> {
        let token_url = match kind {
            Kind::Dropbox => "https://api.dropboxapi.com/oauth2/token",
            Kind::GoogleDrive => "https://oauth2.googleapis.com/token",
            Kind::Box => "https://api.box.com/oauth2/token",
            Kind::Pcloud => "https://api.pcloud.com/oauth2_token",
        };
        Ok(Self {
            kind,
            http: http::client(&Default::default())?,
            tokens: Tokens::new(OAuthSecret::parse(&secret, token_url)?),
            cfg,
            folder_id: Mutex::new(None),
            ids: Mutex::new(HashMap::new()),
        })
    }

    fn api(&self) -> String {
        if !self.cfg.api_base.is_empty() {
            return self.cfg.api_base.trim_end_matches('/').to_string();
        }
        match self.kind {
            Kind::Dropbox => "https://api.dropboxapi.com".into(),
            Kind::GoogleDrive => "https://www.googleapis.com".into(),
            Kind::Box => "https://api.box.com".into(),
            Kind::Pcloud if self.cfg.region == "eu" => "https://eapi.pcloud.com".into(),
            Kind::Pcloud => "https://api.pcloud.com".into(),
        }
    }

    fn content(&self) -> String {
        if !self.cfg.content_base.is_empty() {
            return self.cfg.content_base.trim_end_matches('/').to_string();
        }
        match self.kind {
            Kind::Dropbox => "https://content.dropboxapi.com".into(),
            Kind::GoogleDrive => "https://www.googleapis.com".into(),
            Kind::Box => "https://upload.box.com".into(),
            Kind::Pcloud => self.api(),
        }
    }

    /// Send with the bearer token; on 401 refresh once and retry.
    async fn send(&self, build: impl Fn(&str) -> reqwest::RequestBuilder, what: &str) -> Result<reqwest::Response, AppError> {
        let token = self.tokens.access(&self.http).await?;
        let r = build(&token).send().await.map_err(|e| http::net(what, e))?;
        if r.status().as_u16() == 401 && self.tokens.refresh(&self.http).await? {
            let token = self.tokens.access(&self.http).await?;
            return build(&token).send().await.map_err(|e| http::net(what, e));
        }
        Ok(r)
    }

    async fn json(&self, build: impl Fn(&str) -> reqwest::RequestBuilder, what: &str) -> Result<serde_json::Value, AppError> {
        let r = http::check(self.send(build, what).await?, what).await?;
        let v: serde_json::Value = r.json().await.map_err(|e| http::net(what, e))?;
        // pCloud reports errors in a 200 body.
        if self.kind == Kind::Pcloud && v["result"].as_i64().unwrap_or(0) != 0 {
            return Err(AppError::connection(format!("{what}: pCloud error {} {}", v["result"], v["error"].as_str().unwrap_or("")), "Check the folder and token."));
        }
        Ok(v)
    }

    fn dbx_path(&self, name: Option<&str>) -> String {
        let mut p = format!("/{}", segs(&self.cfg.folder).join("/"));
        if let Some(n) = name {
            if !p.ends_with('/') {
                p.push('/');
            }
            p.push_str(n);
        }
        if p == "/" && name.is_none() {
            String::new()
        } else {
            p
        }
    }

    // ── Google Drive / Box: id-based trees ──

    /// Folder id for `cfg.folder`, created when `create`.
    async fn folder(&self, create: bool) -> Result<String, AppError> {
        if let Some(id) = self.folder_id.lock().await.clone() {
            return Ok(id);
        }
        let mut id = if self.kind == Kind::Box { "0".to_string() } else { "root".to_string() };
        for seg in segs(&self.cfg.folder) {
            id = match self.child_id(&id, seg, true).await? {
                Some(c) => c,
                None if create => self.mkdir(&id, seg).await?,
                None => return Err(AppError::not_found(format!("Folder '{}' does not exist", self.cfg.folder))),
            };
        }
        *self.folder_id.lock().await = Some(id.clone());
        Ok(id)
    }

    async fn child_id(&self, parent: &str, name: &str, folder: bool) -> Result<Option<String>, AppError> {
        Ok(self.children(parent).await?.into_iter().find(|(n, _, _, is_dir)| n == name && *is_dir == folder).map(|(_, id, _, _)| id))
    }

    /// (name, id, size, is_folder) of a folder's children.
    async fn children(&self, parent: &str) -> Result<Vec<(String, String, u64, bool)>, AppError> {
        let mut out = Vec::new();
        match self.kind {
            Kind::GoogleDrive => {
                let mut page: Option<String> = None;
                loop {
                    let q = format!("'{parent}' in parents and trashed = false");
                    let api = self.api();
                    let pg = page.clone();
                    let v = self
                        .json(
                            |t| {
                                let mut r = self.http.get(format!("{api}/drive/v3/files")).bearer_auth(t).query(&[("q", q.as_str()), ("fields", "nextPageToken,files(id,name,size,mimeType)"), ("pageSize", "1000")]);
                                if let Some(p) = &pg {
                                    r = r.query(&[("pageToken", p.as_str())]);
                                }
                                r
                            },
                            "Google Drive listing",
                        )
                        .await?;
                    for f in v["files"].as_array().into_iter().flatten() {
                        let dir = f["mimeType"] == "application/vnd.google-apps.folder";
                        let size = f["size"].as_str().and_then(|s| s.parse().ok()).unwrap_or(0);
                        out.push((f["name"].as_str().unwrap_or_default().to_string(), f["id"].as_str().unwrap_or_default().to_string(), size, dir));
                    }
                    page = v["nextPageToken"].as_str().map(String::from);
                    if page.is_none() {
                        break;
                    }
                }
            }
            Kind::Box => {
                let mut offset = 0u64;
                loop {
                    let api = self.api();
                    let off = offset.to_string();
                    let v = self
                        .json(|t| self.http.get(format!("{api}/2.0/folders/{parent}/items")).bearer_auth(t).query(&[("fields", "name,size,type"), ("limit", "1000"), ("offset", off.as_str())]), "Box listing")
                        .await?;
                    let entries = v["entries"].as_array().cloned().unwrap_or_default();
                    for e in &entries {
                        out.push((e["name"].as_str().unwrap_or_default().to_string(), e["id"].as_str().unwrap_or_default().to_string(), e["size"].as_u64().unwrap_or(0), e["type"] == "folder"));
                    }
                    offset += entries.len() as u64;
                    if entries.is_empty() || offset >= v["total_count"].as_u64().unwrap_or(0) {
                        break;
                    }
                }
            }
            _ => {}
        }
        Ok(out)
    }

    async fn mkdir(&self, parent: &str, name: &str) -> Result<String, AppError> {
        let api = self.api();
        let v = match self.kind {
            Kind::GoogleDrive => {
                let body = serde_json::json!({ "name": name, "mimeType": "application/vnd.google-apps.folder", "parents": [parent] });
                self.json(|t| self.http.post(format!("{api}/drive/v3/files")).bearer_auth(t).json(&body), "Google Drive folder creation").await?
            }
            _ => {
                let body = serde_json::json!({ "name": name, "parent": { "id": parent } });
                self.json(|t| self.http.post(format!("{api}/2.0/folders")).bearer_auth(t).json(&body), "Box folder creation").await?
            }
        };
        Ok(v["id"].as_str().unwrap_or_default().to_string())
    }

    async fn file_id(&self, name: &str) -> Result<Option<String>, AppError> {
        if let Some(id) = self.ids.lock().await.get(name).cloned() {
            return Ok(Some(id));
        }
        let folder = self.folder(false).await?;
        let id = self.child_id(&folder, name, false).await?;
        if let Some(i) = &id {
            self.ids.lock().await.insert(name.to_string(), i.clone());
        }
        Ok(id)
    }
}

impl ObjectStore for DriveStore {
    fn describe(&self) -> String {
        let k = match self.kind {
            Kind::Dropbox => "dropbox",
            Kind::GoogleDrive => "gdrive",
            Kind::Box => "box",
            Kind::Pcloud => "pcloud",
        };
        format!("{k}://{}", segs(&self.cfg.folder).join("/"))
    }

    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let mut out = Vec::new();
            match self.kind {
                Kind::Dropbox => {
                    let api = self.api();
                    let body = serde_json::json!({ "path": self.dbx_path(None), "recursive": false, "limit": 2000 });
                    let mut v = self.json(|t| self.http.post(format!("{api}/2/files/list_folder")).bearer_auth(t).json(&body), "Dropbox listing").await?;
                    loop {
                        for e in v["entries"].as_array().into_iter().flatten() {
                            if e[".tag"] == "file" {
                                out.push(RemoteFile { name: e["name"].as_str().unwrap_or_default().to_string(), size: e["size"].as_u64().unwrap_or(0) });
                            }
                        }
                        if v["has_more"] != true {
                            break;
                        }
                        let cursor = serde_json::json!({ "cursor": v["cursor"] });
                        v = self.json(|t| self.http.post(format!("{api}/2/files/list_folder/continue")).bearer_auth(t).json(&cursor), "Dropbox listing").await?;
                    }
                }
                Kind::GoogleDrive | Kind::Box => {
                    let folder = self.folder(false).await?;
                    let mut ids = self.ids.lock().await;
                    for (name, id, size, dir) in self.children(&folder).await? {
                        if !dir {
                            ids.insert(name.clone(), id);
                            out.push(RemoteFile { name, size });
                        }
                    }
                }
                Kind::Pcloud => {
                    let api = self.api();
                    let path = format!("/{}", segs(&self.cfg.folder).join("/"));
                    let v = self.json(|t| self.http.get(format!("{api}/listfolder")).bearer_auth(t).query(&[("path", path.as_str())]), "pCloud listing").await?;
                    for e in v["metadata"]["contents"].as_array().into_iter().flatten() {
                        if e["isfolder"] != true {
                            out.push(RemoteFile { name: e["name"].as_str().unwrap_or_default().to_string(), size: e["size"].as_u64().unwrap_or(0) });
                        }
                    }
                }
            }
            Ok(out)
        })
    }

    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let content = self.content();
            match self.kind {
                Kind::Dropbox => {
                    let arg = serde_json::json!({ "path": self.dbx_path(Some(name)), "mode": "overwrite", "mute": true }).to_string();
                    let r = self
                        .send(|t| self.http.post(format!("{content}/2/files/upload")).bearer_auth(t).header("Dropbox-API-Arg", arg.as_str()).header("Content-Type", "application/octet-stream").body(data.clone()), "Dropbox upload")
                        .await?;
                    http::check(r, "Dropbox upload").await?;
                }
                Kind::GoogleDrive => {
                    if let Some(id) = self.file_id(name).await? {
                        let r = self.send(|t| self.http.patch(format!("{content}/upload/drive/v3/files/{id}?uploadType=media")).bearer_auth(t).body(data.clone()), "Google Drive upload").await?;
                        http::check(r, "Google Drive upload").await?;
                    } else {
                        let folder = self.folder(true).await?;
                        let meta = serde_json::json!({ "name": name, "parents": [folder] }).to_string();
                        let boundary = format!("ufop{}", uuid::Uuid::new_v4().simple());
                        let mut body = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
                        body.extend_from_slice(&data);
                        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
                        let ct = format!("multipart/related; boundary={boundary}");
                        let v = self
                            .json(|t| self.http.post(format!("{content}/upload/drive/v3/files?uploadType=multipart&fields=id")).bearer_auth(t).header("Content-Type", ct.as_str()).body(body.clone()), "Google Drive upload")
                            .await?;
                        if let Some(id) = v["id"].as_str() {
                            self.ids.lock().await.insert(name.to_string(), id.to_string());
                        }
                    }
                }
                Kind::Box => {
                    let existing = self.file_id(name).await?;
                    let folder = self.folder(true).await?;
                    let attrs = serde_json::json!({ "name": name, "parent": { "id": folder } }).to_string();
                    let url = match &existing {
                        Some(id) => format!("{content}/api/2.0/files/{id}/content"),
                        None => format!("{content}/api/2.0/files/content"),
                    };
                    let v = self
                        .json(
                            |t| {
                                let form = reqwest::multipart::Form::new()
                                    .text("attributes", attrs.clone())
                                    .part("file", reqwest::multipart::Part::bytes(data.clone()).file_name(name.to_string()));
                                self.http.post(&url).bearer_auth(t).multipart(form)
                            },
                            "Box upload",
                        )
                        .await?;
                    if let Some(id) = v["entries"][0]["id"].as_str() {
                        self.ids.lock().await.insert(name.to_string(), id.to_string());
                    }
                }
                Kind::Pcloud => {
                    let path = format!("/{}", segs(&self.cfg.folder).join("/"));
                    self.json(
                        |t| self.http.put(format!("{content}/uploadfile")).bearer_auth(t).query(&[("path", path.as_str()), ("filename", name), ("nopartial", "1")]).body(data.clone()),
                        "pCloud upload",
                    )
                    .await?;
                }
            }
            Ok(())
        })
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let r = match self.kind {
                Kind::Dropbox => {
                    let content = self.content();
                    let arg = serde_json::json!({ "path": self.dbx_path(Some(name)) }).to_string();
                    self.send(|t| self.http.post(format!("{content}/2/files/download")).bearer_auth(t).header("Dropbox-API-Arg", arg.as_str()), "Dropbox download").await?
                }
                Kind::GoogleDrive => {
                    let id = self.file_id(name).await?.ok_or_else(|| AppError::not_found(format!("{name} is not in the Google Drive folder")))?;
                    let api = self.api();
                    self.send(|t| self.http.get(format!("{api}/drive/v3/files/{id}?alt=media")).bearer_auth(t), "Google Drive download").await?
                }
                Kind::Box => {
                    let id = self.file_id(name).await?.ok_or_else(|| AppError::not_found(format!("{name} is not in the Box folder")))?;
                    let api = self.api();
                    self.send(|t| self.http.get(format!("{api}/2.0/files/{id}/content")).bearer_auth(t), "Box download").await?
                }
                Kind::Pcloud => {
                    let api = self.api();
                    let path = format!("/{}/{name}", segs(&self.cfg.folder).join("/")).replace("//", "/");
                    let v = self.json(|t| self.http.get(format!("{api}/getfilelink")).bearer_auth(t).query(&[("path", path.as_str())]), "pCloud link").await?;
                    let host = v["hosts"][0].as_str().unwrap_or_default();
                    let p = v["path"].as_str().unwrap_or_default();
                    let url = if host.starts_with("http") { format!("{host}{p}") } else { format!("https://{host}{p}") };
                    self.http.get(url).send().await.map_err(|e| http::net("pCloud download", e))?
                }
            };
            Ok(http::check(r, "Download").await?.bytes().await.map_err(|e| http::net("Download", e))?.to_vec())
        })
    }

    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let api = self.api();
            match self.kind {
                Kind::Dropbox => {
                    let body = serde_json::json!({ "path": self.dbx_path(Some(name)) });
                    self.json(|t| self.http.post(format!("{api}/2/files/delete_v2")).bearer_auth(t).json(&body), "Dropbox delete").await?;
                }
                Kind::GoogleDrive | Kind::Box => {
                    let Some(id) = self.file_id(name).await? else { return Ok(()) };
                    let url = if self.kind == Kind::GoogleDrive { format!("{api}/drive/v3/files/{id}") } else { format!("{api}/2.0/files/{id}") };
                    let r = self.send(|t| self.http.delete(&url).bearer_auth(t), "Delete").await?;
                    http::check(r, "Delete").await?;
                    self.ids.lock().await.remove(name);
                }
                Kind::Pcloud => {
                    let path = format!("/{}/{name}", segs(&self.cfg.folder).join("/")).replace("//", "/");
                    self.json(|t| self.http.get(format!("{api}/deletefile")).bearer_auth(t).query(&[("path", path.as_str())]), "pCloud delete").await?;
                }
            }
            Ok(())
        })
    }

    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            let api = self.api();
            match self.kind {
                Kind::Dropbox => {
                    let path = self.dbx_path(None);
                    if path.is_empty() {
                        return Ok(());
                    }
                    let body = serde_json::json!({ "path": path, "autorename": false });
                    let r = self.send(|t| self.http.post(format!("{api}/2/files/create_folder_v2")).bearer_auth(t).json(&body), "Dropbox folder creation").await?;
                    if r.status().as_u16() == 409 {
                        return Ok(()); // path/conflict: exists
                    }
                    http::check(r, "Dropbox folder creation").await?;
                }
                Kind::GoogleDrive | Kind::Box => {
                    self.folder(true).await?;
                }
                Kind::Pcloud => {
                    let mut acc = String::new();
                    for s in segs(&self.cfg.folder) {
                        acc = format!("{acc}/{s}");
                        self.json(|t| self.http.get(format!("{api}/createfolderifnotexists")).bearer_auth(t).query(&[("path", acc.as_str())]), "pCloud folder creation").await?;
                    }
                }
            }
            Ok(())
        })
    }
}
