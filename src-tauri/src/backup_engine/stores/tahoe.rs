//! Tahoe-LAFS through a gateway node's web API (WAPI).

use super::http;
use super::{BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use ufop_core::backup::RemoteFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TahoeConfig {
    /// Gateway node, e.g. `http://127.0.0.1:3456`.
    pub node_url: String,
    /// Writable directory capability (`URI:DIR2:…`).
    pub dir_cap: String,
    /// Sub-folder below the directory.
    pub path: String,
}

impl Default for TahoeConfig {
    fn default() -> Self {
        Self { node_url: "http://127.0.0.1:3456".into(), dir_cap: String::new(), path: String::new() }
    }
}

pub struct TahoeStore {
    cfg: TahoeConfig,
    http: reqwest::Client,
}

impl TahoeStore {
    pub fn new(cfg: TahoeConfig) -> Result<Self, AppError> {
        if !cfg.dir_cap.starts_with("URI:DIR2:") {
            return Err(AppError::validation("Enter a writable Tahoe-LAFS directory capability (URI:DIR2:…)"));
        }
        Ok(Self { http: http::client(&Default::default())?, cfg })
    }

    fn url(&self, name: Option<&str>) -> String {
        let base = format!("{}/uri/{}", self.cfg.node_url.trim_end_matches('/'), urlencoding::encode(&self.cfg.dir_cap));
        let mut parts: Vec<String> = self.cfg.path.split('/').filter(|p| !p.is_empty()).map(|p| urlencoding::encode(p).into_owned()).collect();
        if let Some(n) = name {
            parts.push(urlencoding::encode(n).into_owned());
        }
        if parts.is_empty() { base } else { format!("{base}/{}", parts.join("/")) }
    }
}

impl ObjectStore for TahoeStore {
    fn describe(&self) -> String {
        format!("tahoe://{}/{}", self.cfg.dir_cap.chars().take(24).collect::<String>(), self.cfg.path)
    }
    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let r = self.http.get(format!("{}?t=json", self.url(None))).send().await.map_err(|e| http::net("Tahoe listing", e))?;
            let v: serde_json::Value = http::check(r, "Tahoe listing").await?.json().await.map_err(|e| http::net("Tahoe listing", e))?;
            let mut out = Vec::new();
            if let Some(children) = v[1]["children"].as_object() {
                for (name, c) in children {
                    if c[0] == "filenode" {
                        out.push(RemoteFile { name: name.clone(), size: c[1]["size"].as_u64().unwrap_or(0) });
                    }
                }
            }
            Ok(out)
        })
    }
    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let r = self.http.put(self.url(Some(name))).body(data).send().await.map_err(|e| http::net("Tahoe upload", e))?;
            http::check(r, "Tahoe upload").await?;
            Ok(())
        })
    }
    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let r = self.http.get(self.url(Some(name))).send().await.map_err(|e| http::net("Tahoe download", e))?;
            Ok(http::check(r, "Tahoe download").await?.bytes().await.map_err(|e| http::net("Tahoe download", e))?.to_vec())
        })
    }
    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let r = self.http.delete(self.url(Some(name))).send().await.map_err(|e| http::net("Tahoe delete", e))?;
            http::check(r, "Tahoe delete").await?;
            Ok(())
        })
    }
    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            if self.cfg.path.trim_matches('/').is_empty() {
                return Ok(());
            }
            // Creates every missing directory along the path.
            let r = self.http.put(format!("{}?t=mkdir", self.url(None))).send().await.map_err(|e| http::net("Tahoe mkdir", e))?;
            if r.status().as_u16() == 409 {
                return Ok(());
            }
            http::check(r, "Tahoe mkdir").await?;
            Ok(())
        })
    }
}
