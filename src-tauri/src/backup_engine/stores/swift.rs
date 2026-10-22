//! OpenStack Swift (Keystone v3 password auth, or v1 / TempAuth).

use super::http::{self, TlsOptions};
use super::{direct_child, join_key, BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use ufop_core::backup::RemoteFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SwiftConfig {
    /// Keystone v3 (`…/v3`) or v1 auth URL.
    pub auth_url: String,
    /// `3` (Keystone) or `1` (TempAuth / v1).
    pub auth_version: String,
    pub username: String,
    pub user_domain: String,
    pub project: String,
    pub project_domain: String,
    pub region: String,
    pub container: String,
    pub prefix: String,
    pub tls: TlsOptions,
}

impl Default for SwiftConfig {
    fn default() -> Self {
        Self {
            auth_url: String::new(),
            auth_version: "3".into(),
            username: String::new(),
            user_domain: "Default".into(),
            project: String::new(),
            project_domain: "Default".into(),
            region: String::new(),
            container: String::new(),
            prefix: String::new(),
            tls: TlsOptions::default(),
        }
    }
}

pub struct SwiftStore {
    cfg: SwiftConfig,
    password: String,
    http: reqwest::Client,
    /// (storage URL, token)
    session: Mutex<(String, String)>,
}

impl SwiftStore {
    pub async fn connect(cfg: SwiftConfig, password: String) -> Result<Self, AppError> {
        if cfg.auth_url.is_empty() || cfg.container.is_empty() {
            return Err(AppError::validation("Enter the Swift auth URL and container"));
        }
        let http = http::client(&cfg.tls)?;
        let s = Self::auth(&http, &cfg, &password).await?;
        Ok(Self { cfg, password, http, session: Mutex::new(s) })
    }

    async fn auth(http: &reqwest::Client, cfg: &SwiftConfig, password: &str) -> Result<(String, String), AppError> {
        let url = cfg.auth_url.trim_end_matches('/');
        if cfg.auth_version == "1" {
            let r = http.get(url).header("X-Auth-User", &cfg.username).header("X-Auth-Key", password).send().await.map_err(|e| http::net("Swift sign-in", e))?;
            let r = http::check(r, "Swift sign-in").await?;
            let h = |n: &str| r.headers().get(n).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
            return Ok((h("x-storage-url"), h("x-auth-token")));
        }
        let body = serde_json::json!({ "auth": {
            "identity": { "methods": ["password"], "password": { "user": { "name": cfg.username, "domain": { "name": cfg.user_domain }, "password": password } } },
            "scope": { "project": { "name": cfg.project, "domain": { "name": cfg.project_domain } } }
        }});
        let r = http.post(format!("{url}/auth/tokens")).json(&body).send().await.map_err(|e| http::net("Keystone sign-in", e))?;
        let r = http::check(r, "Keystone sign-in").await?;
        let token = r.headers().get("x-subject-token").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let v: serde_json::Value = r.json().await.map_err(|e| http::net("Keystone sign-in", e))?;
        let endpoint = v["token"]["catalog"].as_array().into_iter().flatten().filter(|s| s["type"] == "object-store").flat_map(|s| s["endpoints"].as_array().cloned().unwrap_or_default()).find(|e| {
            e["interface"] == "public" && (cfg.region.is_empty() || e["region"] == cfg.region.as_str() || e["region_id"] == cfg.region.as_str())
        });
        let url = endpoint.and_then(|e| e["url"].as_str().map(String::from)).ok_or_else(|| AppError::connection("No object-store endpoint in the Keystone catalog", "Check the region and project."))?;
        Ok((url, token))
    }

    async fn req(&self, method: reqwest::Method, key: Option<&str>, query: &[(&str, String)], body: Option<Vec<u8>>) -> Result<reqwest::Response, AppError> {
        for attempt in 0..2 {
            let (url, token) = self.session.lock().await.clone();
            let path = match key {
                Some(k) => format!("{url}/{}/{}", self.cfg.container, super::s3::uri_encode(k, true)),
                None => format!("{url}/{}", self.cfg.container),
            };
            let mut r = self.http.request(method.clone(), &path).header("X-Auth-Token", &token).query(query);
            if let Some(b) = body.clone() {
                use md5::Digest;
                r = r.header("ETag", hex::encode(md5::Md5::digest(&b))).body(b);
            }
            let resp = r.send().await.map_err(|e| http::net("Swift request", e))?;
            if resp.status().as_u16() == 401 && attempt == 0 {
                *self.session.lock().await = Self::auth(&self.http, &self.cfg, &self.password).await?;
                continue;
            }
            return Ok(resp);
        }
        unreachable!()
    }
}

impl ObjectStore for SwiftStore {
    fn describe(&self) -> String {
        format!("swift://{}/{}", self.cfg.container, self.cfg.prefix.trim_matches('/'))
    }
    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let prefix = self.cfg.prefix.trim_matches('/');
            let pfx = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
            let mut out = Vec::new();
            let mut marker = String::new();
            loop {
                let q = [("format", "json".to_string()), ("prefix", pfx.clone()), ("delimiter", "/".to_string()), ("marker", marker.clone())];
                let r = http::check(self.req(reqwest::Method::GET, None, &q, None).await?, "Swift listing").await?;
                let v: Vec<serde_json::Value> = r.json().await.map_err(|e| http::net("Swift listing", e))?;
                if v.is_empty() {
                    break;
                }
                for o in &v {
                    if let Some(n) = o["name"].as_str().and_then(|n| direct_child(prefix, n)) {
                        out.push(RemoteFile { name: n.to_string(), size: o["bytes"].as_u64().unwrap_or(0) });
                    }
                }
                marker = v.last().and_then(|o| o["name"].as_str().or(o["subdir"].as_str())).unwrap_or_default().to_string();
                if v.len() < 10_000 {
                    break;
                }
            }
            Ok(out)
        })
    }
    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            http::check(self.req(reqwest::Method::PUT, Some(&join_key(&self.cfg.prefix, name)), &[], Some(data)).await?, "Swift upload").await?;
            Ok(())
        })
    }
    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let r = http::check(self.req(reqwest::Method::GET, Some(&join_key(&self.cfg.prefix, name)), &[], None).await?, "Swift download").await?;
            Ok(r.bytes().await.map_err(|e| http::net("Swift download", e))?.to_vec())
        })
    }
    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            http::check(self.req(reqwest::Method::DELETE, Some(&join_key(&self.cfg.prefix, name)), &[], None).await?, "Swift delete").await?;
            Ok(())
        })
    }
    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            http::check(self.req(reqwest::Method::PUT, None, &[], None).await?, "Swift container creation").await?;
            Ok(())
        })
    }
}
