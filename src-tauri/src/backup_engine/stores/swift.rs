//! OpenStack Swift (Keystone v3 password or application credential, or v1 / TempAuth).

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
    /// Keystone application credential id; the secret is then its secret
    /// (the credential's own project scope applies).
    pub application_credential_id: String,
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
            application_credential_id: String::new(),
            container: String::new(),
            prefix: String::new(),
            tls: TlsOptions::default(),
        }
    }
}

/// A signed-in Swift account: the storage URL and token, renewed on a 401.
/// Shared by the backup store and the file connector.
pub struct SwiftSession {
    cfg: SwiftConfig,
    secret: String,
    http: reqwest::Client,
    /// (storage URL, token)
    session: Mutex<(String, String)>,
}

impl SwiftSession {
    /// Sign in with `cfg` (container and prefix are not used here).
    pub async fn connect(cfg: SwiftConfig, secret: String) -> Result<Self, AppError> {
        if cfg.auth_url.trim().is_empty() {
            return Err(AppError::validation("Enter the Swift auth URL"));
        }
        let http = http::client(&cfg.tls)?;
        let s = Self::auth(&http, &cfg, &secret).await?;
        Ok(Self { cfg, secret, http, session: Mutex::new(s) })
    }

    /// Keystone v3 URL: the given one when it already names a version, else `<url>/v3`.
    fn keystone_url(url: &str) -> String {
        let url = url.trim().trim_end_matches('/');
        if url.rsplit('/').next().is_some_and(|last| last.starts_with('v') && last[1..].starts_with(|c: char| c.is_ascii_digit())) {
            url.to_string()
        } else {
            format!("{url}/v3")
        }
    }

    async fn auth(http: &reqwest::Client, cfg: &SwiftConfig, secret: &str) -> Result<(String, String), AppError> {
        if cfg.auth_version == "1" {
            let url = cfg.auth_url.trim().trim_end_matches('/');
            let r = http.get(url).header("X-Auth-User", &cfg.username).header("X-Auth-Key", secret).send().await.map_err(|e| http::net("Swift sign-in", e))?;
            let r = http::check(r, "Swift sign-in").await?;
            let h = |n: &str| r.headers().get(n).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
            let (url, token) = (h("x-storage-url"), h("x-auth-token"));
            if url.is_empty() || token.is_empty() {
                return Err(AppError::connection("The Swift sign-in returned no storage URL or token", "Check the auth URL (for TempAuth usually …/auth/v1.0)."));
            }
            return Ok((url, token));
        }
        let identity = if cfg.application_credential_id.trim().is_empty() {
            serde_json::json!({ "methods": ["password"], "password": { "user": { "name": cfg.username, "domain": { "name": cfg.user_domain }, "password": secret } } })
        } else {
            serde_json::json!({ "methods": ["application_credential"], "application_credential": { "id": cfg.application_credential_id.trim(), "secret": secret } })
        };
        // Application credentials carry their own project scope.
        let body = if cfg.application_credential_id.trim().is_empty() {
            serde_json::json!({ "auth": { "identity": identity, "scope": { "project": { "name": cfg.project, "domain": { "name": cfg.project_domain } } } } })
        } else {
            serde_json::json!({ "auth": { "identity": identity } })
        };
        let r = http.post(format!("{}/auth/tokens", Self::keystone_url(&cfg.auth_url))).json(&body).send().await.map_err(|e| http::net("Keystone sign-in", e))?;
        let r = http::check(r, "Keystone sign-in").await?;
        let token = r.headers().get("x-subject-token").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
        let v: serde_json::Value = r.json().await.map_err(|e| http::net("Keystone sign-in", e))?;
        let endpoint = v["token"]["catalog"].as_array().into_iter().flatten().filter(|s| s["type"] == "object-store").flat_map(|s| s["endpoints"].as_array().cloned().unwrap_or_default()).find(|e| {
            e["interface"] == "public" && (cfg.region.is_empty() || e["region"] == cfg.region.as_str() || e["region_id"] == cfg.region.as_str())
        });
        let url = endpoint.and_then(|e| e["url"].as_str().map(String::from)).ok_or_else(|| AppError::connection("No object-store endpoint in the Keystone catalog", "Check the region and project."))?;
        Ok((url.trim_end_matches('/').to_string(), token))
    }

    /// The account's storage URL (`https://swift/v1/AUTH_project`).
    pub async fn storage_url(&self) -> String {
        self.session.lock().await.0.clone()
    }

    /// Send a request to `path` (already encoded, below the storage URL:
    /// `/container/object`, or empty for the account), signing in again once on a 401.
    pub async fn send(&self, method: reqwest::Method, path: &str, query: &[(&str, String)], headers: &[(&str, String)], body: bytes::Bytes) -> Result<reqwest::Response, AppError> {
        for attempt in 0..2 {
            let (url, token) = self.session.lock().await.clone();
            let mut r = self.http.request(method.clone(), format!("{url}{path}")).header("X-Auth-Token", &token).query(query);
            for (k, v) in headers {
                r = r.header(*k, v);
            }
            let resp = r.body(body.clone()).send().await.map_err(|e| http::net("Swift request", e))?;
            if resp.status().as_u16() == 401 && attempt == 0 {
                *self.session.lock().await = Self::auth(&self.http, &self.cfg, &self.secret).await?;
                continue;
            }
            return Ok(resp);
        }
        unreachable!()
    }

    /// `send` + fail on a non-success status.
    pub async fn call(&self, what: &str, method: reqwest::Method, path: &str, query: &[(&str, String)], headers: &[(&str, String)], body: bytes::Bytes) -> Result<reqwest::Response, AppError> {
        http::check(self.send(method, path, query, headers, body).await?, what).await
    }

    /// One page of a JSON listing (account or container), from `marker` on.
    /// Servers may cap pages below `limit`, so only an empty page ends a listing.
    pub async fn list_page(&self, what: &str, path: &str, extra: &[(&str, String)], marker: &str, limit: usize) -> Result<Vec<serde_json::Value>, AppError> {
        let mut q = vec![("format", "json".to_string()), ("limit", limit.to_string())];
        q.extend(extra.iter().cloned());
        if !marker.is_empty() {
            q.push(("marker", marker.to_string()));
        }
        let r = self.call(what, reqwest::Method::GET, path, &q, &[], bytes::Bytes::new()).await?;
        if r.status().as_u16() == 204 {
            return Ok(Vec::new());
        }
        r.json().await.map_err(|e| http::net(what, e))
    }
}

/// Swift's default (and maximum) listing page size.
pub const LIST_PAGE: usize = 10_000;

/// `/container` or `/container/object` with each part percent-encoded.
pub fn object_path(container: &str, key: Option<&str>) -> String {
    match key {
        Some(k) => format!("/{}/{}", super::s3::uri_encode(container, false), super::s3::uri_encode(k, true)),
        None => format!("/{}", super::s3::uri_encode(container, false)),
    }
}

pub struct SwiftStore {
    cfg: SwiftConfig,
    session: SwiftSession,
}

impl SwiftStore {
    pub async fn connect(cfg: SwiftConfig, password: String) -> Result<Self, AppError> {
        if cfg.auth_url.is_empty() || cfg.container.is_empty() {
            return Err(AppError::validation("Enter the Swift auth URL and container"));
        }
        let session = SwiftSession::connect(cfg.clone(), password).await?;
        Ok(Self { cfg, session })
    }

    async fn req(&self, method: reqwest::Method, key: Option<&str>, query: &[(&str, String)], body: Option<Vec<u8>>) -> Result<reqwest::Response, AppError> {
        let path = object_path(&self.cfg.container, key);
        let mut headers = Vec::new();
        if let Some(b) = &body {
            use md5::Digest;
            headers.push(("ETag", hex::encode(md5::Md5::digest(b))));
        }
        self.session.send(method, &path, query, &headers, body.map(bytes::Bytes::from).unwrap_or_default()).await
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
                let q = [("prefix", pfx.clone()), ("delimiter", "/".to_string())];
                let v = self.session.list_page("Swift listing", &object_path(&self.cfg.container, None), &q, &marker, LIST_PAGE).await?;
                if v.is_empty() {
                    break;
                }
                for o in &v {
                    if let Some(n) = o["name"].as_str().and_then(|n| direct_child(prefix, n)) {
                        out.push(RemoteFile { name: n.to_string(), size: o["bytes"].as_u64().unwrap_or(0) });
                    }
                }
                marker = v.last().and_then(|o| o["name"].as_str().or(o["subdir"].as_str())).unwrap_or_default().to_string();
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
