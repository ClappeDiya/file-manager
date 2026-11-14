//! Microsoft Graph drives: OneDrive (personal / business), a SharePoint
//! site's document library, or a Microsoft 365 group's files.

use super::http;
use super::oauth_drive::{OAuthSecret, Tokens};
use super::{BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use ufop_core::backup::RemoteFile;

/// Graph upload sessions take chunks in multiples of 320 KiB.
const CHUNK: usize = 320 * 1024 * 16; // 5 MiB

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphConfig {
    /// `me` (OneDrive), `site:<site-id>` (SharePoint), `group:<group-id>`
    /// (Microsoft 365 group) or `drive:<drive-id>`.
    pub drive: String,
    pub folder: String,
    /// Graph base (national clouds / tests); empty = `https://graph.microsoft.com/v1.0`.
    pub api_base: String,
}

impl Default for GraphConfig {
    fn default() -> Self {
        Self { drive: "me".into(), folder: String::new(), api_base: String::new() }
    }
}

pub struct GraphStore {
    cfg: GraphConfig,
    http: reqwest::Client,
    tokens: Tokens,
}

impl GraphStore {
    pub fn new(cfg: GraphConfig, secret: String) -> Result<Self, AppError> {
        Ok(Self { http: http::client(&Default::default())?, tokens: Tokens::new(OAuthSecret::parse(&secret, "https://login.microsoftonline.com/common/oauth2/v2.0/token")?), cfg })
    }

    fn drive_base(&self) -> Result<String, AppError> {
        let api = if self.cfg.api_base.is_empty() { "https://graph.microsoft.com/v1.0".to_string() } else { self.cfg.api_base.trim_end_matches('/').to_string() };
        let d = self.cfg.drive.trim();
        Ok(match d.split_once(':') {
            None if d.is_empty() || d == "me" => format!("{api}/me/drive"),
            Some(("site", id)) => format!("{api}/sites/{id}/drive"),
            Some(("group", id)) => format!("{api}/groups/{id}/drive"),
            Some(("drive", id)) => format!("{api}/drives/{id}"),
            _ => return Err(AppError::validation(format!("Unknown drive '{d}': use me, site:<id>, group:<id> or drive:<id>"))),
        })
    }

    fn item(&self, name: Option<&str>) -> Result<String, AppError> {
        let mut parts: Vec<String> = self.cfg.folder.split('/').filter(|s| !s.is_empty()).map(|s| urlencoding::encode(s).into_owned()).collect();
        if let Some(n) = name {
            parts.push(urlencoding::encode(n).into_owned());
        }
        let base = self.drive_base()?;
        Ok(if parts.is_empty() { format!("{base}/root") } else { format!("{base}/root:/{}:", parts.join("/")) })
    }

    async fn send(&self, build: impl Fn(&str) -> reqwest::RequestBuilder, what: &str) -> Result<reqwest::Response, AppError> {
        let token = self.tokens.access(&self.http).await?;
        let r = build(&token).send().await.map_err(|e| http::net(what, e))?;
        if r.status().as_u16() == 401 && self.tokens.refresh(&self.http).await? {
            let token = self.tokens.access(&self.http).await?;
            return build(&token).send().await.map_err(|e| http::net(what, e));
        }
        Ok(r)
    }
}

impl ObjectStore for GraphStore {
    fn describe(&self) -> String {
        format!("onedrive://{}/{}", self.cfg.drive, self.cfg.folder.trim_matches('/'))
    }

    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let mut url = format!("{}/children?$select=name,size,file,folder&$top=1000", self.item(None)?);
            let mut out = Vec::new();
            loop {
                let u = url.clone();
                let r = http::check(self.send(|t| self.http.get(&u).bearer_auth(t), "OneDrive listing").await?, "OneDrive listing").await?;
                let v: serde_json::Value = r.json().await.map_err(|e| http::net("OneDrive listing", e))?;
                for i in v["value"].as_array().into_iter().flatten() {
                    if i.get("file").is_some() {
                        out.push(RemoteFile { name: i["name"].as_str().unwrap_or_default().to_string(), size: i["size"].as_u64().unwrap_or(0) });
                    }
                }
                match v["@odata.nextLink"].as_str() {
                    Some(n) => url = n.to_string(),
                    None => break,
                }
            }
            Ok(out)
        })
    }

    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let item = self.item(Some(name))?;
            if data.len() <= 4 << 20 {
                let u = format!("{item}/content");
                http::check(self.send(|t| self.http.put(&u).bearer_auth(t).body(data.clone()), "OneDrive upload").await?, "OneDrive upload").await?;
                return Ok(());
            }
            let u = format!("{item}/createUploadSession");
            let body = serde_json::json!({ "item": { "@microsoft.graph.conflictBehavior": "replace" } });
            let r = http::check(self.send(|t| self.http.post(&u).bearer_auth(t).json(&body), "OneDrive upload session").await?, "OneDrive upload session").await?;
            let v: serde_json::Value = r.json().await.map_err(|e| http::net("OneDrive upload session", e))?;
            let upload = v["uploadUrl"].as_str().ok_or_else(|| AppError::connection("No upload URL returned", "Retry."))?.to_string();
            let total = data.len();
            for (i, chunk) in data.chunks(CHUNK).enumerate() {
                let start = i * CHUNK;
                let end = start + chunk.len() - 1;
                // The upload URL is pre-authorised: no bearer token.
                let r = self
                    .http
                    .put(&upload)
                    .header("Content-Range", format!("bytes {start}-{end}/{total}"))
                    .body(chunk.to_vec())
                    .send()
                    .await
                    .map_err(|e| http::net("OneDrive chunk", e))?;
                if let Err(e) = http::check(r, "OneDrive chunk").await {
                    let _ = self.http.delete(&upload).send().await;
                    return Err(e);
                }
            }
            Ok(())
        })
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let u = format!("{}/content", self.item(Some(name))?);
            let r = http::check(self.send(|t| self.http.get(&u).bearer_auth(t), "OneDrive download").await?, "OneDrive download").await?;
            Ok(r.bytes().await.map_err(|e| http::net("OneDrive download", e))?.to_vec())
        })
    }

    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let u = self.item(Some(name))?;
            let r = self.send(|t| self.http.delete(&u).bearer_auth(t), "OneDrive delete").await?;
            if r.status().as_u16() == 404 {
                return Ok(());
            }
            http::check(r, "OneDrive delete").await?;
            Ok(())
        })
    }

    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            let base = self.drive_base()?;
            let mut parent = String::new();
            for seg in self.cfg.folder.split('/').filter(|s| !s.is_empty()) {
                let u = if parent.is_empty() { format!("{base}/root/children") } else { format!("{base}/root:/{parent}:/children") };
                let body = serde_json::json!({ "name": seg, "folder": {}, "@microsoft.graph.conflictBehavior": "fail" });
                let r = self.send(|t| self.http.post(&u).bearer_auth(t).json(&body), "OneDrive folder creation").await?;
                if r.status().as_u16() != 409 {
                    http::check(r, "OneDrive folder creation").await?;
                }
                parent = if parent.is_empty() { urlencoding::encode(seg).into_owned() } else { format!("{parent}/{}", urlencoding::encode(seg)) };
            }
            Ok(())
        })
    }
}
