//! Azure Blob Storage (Shared Key or SAS), including the Azurite emulator.
//!
//! Large volumes go up as blocks of a configurable size; access tiers,
//! archive rehydration and version-level immutability policies are
//! supported.

use super::http::{self, TlsOptions};
use super::{direct_child, join_key, BoxFut, ObjectStore};
use crate::core::error::AppError;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::BTreeMap;
use ufop_core::backup::backend::LockMode;
use ufop_core::backup::RemoteFile;

const VERSION: &str = "2021-08-06";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AzureConfig {
    pub account: String,
    pub container: String,
    pub prefix: String,
    /// Blob endpoint; empty = `https://<account>.blob.core.windows.net`.
    /// Azurite: `http://127.0.0.1:10000/<account>`.
    pub endpoint: String,
    /// `Hot`, `Cool`, `Cold` or `Archive` (empty = account default).
    pub access_tier: String,
    /// Upload volumes larger than this in blocks of this size (MiB).
    pub block_size_mb: u32,
    /// Rehydration priority for archived blobs: `Standard` or `High`.
    pub rehydrate_priority: String,
    pub tls: TlsOptions,
}

impl Default for AzureConfig {
    fn default() -> Self {
        Self {
            account: String::new(),
            container: String::new(),
            prefix: String::new(),
            endpoint: String::new(),
            access_tier: String::new(),
            block_size_mb: 4,
            rehydrate_priority: "Standard".into(),
            tls: TlsOptions::default(),
        }
    }
}

enum Auth {
    SharedKey(Vec<u8>),
    Sas(String),
}

pub struct AzureStore {
    cfg: AzureConfig,
    auth: Auth,
    http: reqwest::Client,
    /// Endpoint without trailing slash, and its URL path (for signing).
    base: String,
    base_path: String,
}

impl AzureStore {
    /// `secret` = base64 account key, or a SAS token (`sv=…` / `?sv=…`).
    pub fn new(cfg: AzureConfig, secret: String) -> Result<Self, AppError> {
        if cfg.account.trim().is_empty() || cfg.container.trim().is_empty() {
            return Err(AppError::validation("Enter the storage account and container"));
        }
        let base = if cfg.endpoint.trim().is_empty() { format!("https://{}.blob.core.windows.net", cfg.account) } else { cfg.endpoint.trim().trim_end_matches('/').to_string() };
        let u = url::Url::parse(&base).map_err(|e| AppError::validation(format!("Invalid Azure endpoint: {e}")))?;
        let s = secret.trim();
        let auth = if s.starts_with("sv=") || s.starts_with("?sv=") || s.contains("&sig=") {
            Auth::Sas(s.trim_start_matches('?').to_string())
        } else {
            Auth::SharedKey(base64::engine::general_purpose::STANDARD.decode(s).map_err(|_| AppError::validation("The Azure account key is not valid base64"))?)
        };
        Ok(Self { base_path: u.path().trim_end_matches('/').to_string(), http: http::client(&cfg.tls)?, base, cfg, auth })
    }

    fn sign(&self, method: &str, path: &str, query: &[(&str, String)], headers: &BTreeMap<String, String>, content_length: usize) -> Option<String> {
        let Auth::SharedKey(key) = &self.auth else { return None };
        let h = |n: &str| headers.get(n).cloned().unwrap_or_default();
        let len = if content_length == 0 { String::new() } else { content_length.to_string() };
        let canon_headers: String = headers.iter().filter(|(k, _)| k.starts_with("x-ms-")).map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
        let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (k.to_lowercase(), v.clone())).collect();
        q.sort();
        let canon_query: String = q.iter().map(|(k, v)| format!("\n{k}:{v}")).collect();
        let resource = format!("/{}{}{}", self.cfg.account, self.base_path, path);
        let sts = format!(
            "{method}\n{}\n{}\n{len}\n{}\n{}\n\n{}\n{}\n{}\n{}\n{}\n{canon_headers}{resource}{canon_query}",
            h("content-encoding"),
            h("content-language"),
            h("content-md5"),
            h("content-type"),
            h("if-modified-since"),
            h("if-match"),
            h("if-none-match"),
            h("if-unmodified-since"),
            h("range"),
        );
        let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
        m.update(sts.as_bytes());
        Some(format!("SharedKey {}:{}", self.cfg.account, base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes())))
    }

    async fn send(&self, method: reqwest::Method, blob: Option<&str>, query: &[(&str, String)], extra: &[(&str, String)], body: Vec<u8>) -> Result<reqwest::Response, AppError> {
        let path = match blob {
            Some(b) => format!("/{}/{}", self.cfg.container, super::s3::uri_encode(b, true)),
            None => format!("/{}", self.cfg.container),
        };
        let mut headers: BTreeMap<String, String> = BTreeMap::new();
        headers.insert("x-ms-date".into(), chrono::Utc::now().format("%a, %d %b %Y %H:%M:%S GMT").to_string());
        headers.insert("x-ms-version".into(), VERSION.into());
        for (k, v) in extra {
            headers.insert(k.to_lowercase(), v.clone());
        }
        let mut qs: Vec<String> = query.iter().map(|(k, v)| format!("{k}={}", urlencoding::encode(v))).collect();
        if let Auth::Sas(sas) = &self.auth {
            qs.push(sas.clone());
        }
        let url = format!("{}{path}{}", self.base, if qs.is_empty() { String::new() } else { format!("?{}", qs.join("&")) });
        let mut req = self.http.request(method.clone(), &url);
        if let Some(a) = self.sign(method.as_str(), &path, query, &headers, body.len()) {
            req = req.header("authorization", a);
        }
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req.body(body).send().await.map_err(|e| http::net("Azure request", e))
    }

    fn md5(data: &[u8]) -> String {
        use md5::Digest;
        base64::engine::general_purpose::STANDARD.encode(md5::Md5::digest(data))
    }
}

impl ObjectStore for AzureStore {
    fn describe(&self) -> String {
        format!("azure://{}/{}/{}", self.cfg.account, self.cfg.container, self.cfg.prefix.trim_matches('/'))
    }

    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let prefix = self.cfg.prefix.trim_matches('/');
            let pfx = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
            let mut out = Vec::new();
            let mut marker = String::new();
            loop {
                let mut q = vec![("restype", "container".to_string()), ("comp", "list".to_string()), ("prefix", pfx.clone())];
                if !marker.is_empty() {
                    q.push(("marker", marker.clone()));
                }
                let r = http::check(self.send(reqwest::Method::GET, None, &q, &[], Vec::new()).await?, "Azure listing").await?;
                let xml = r.text().await.map_err(|e| http::net("Azure listing", e))?;
                for b in http::xml_all(&xml, "Blob") {
                    let name = http::xml_unescape(&http::xml_one(&b, "Name").unwrap_or_default());
                    let size = http::xml_one(&b, "Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
                    if let Some(n) = direct_child(prefix, &name) {
                        out.push(RemoteFile { name: n.to_string(), size });
                    }
                }
                marker = http::xml_one(&xml, "NextMarker").unwrap_or_default();
                if marker.is_empty() {
                    break;
                }
            }
            Ok(out)
        })
    }

    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let block = (self.cfg.block_size_mb.max(1) as usize) << 20;
            let mut tier = Vec::new();
            if !self.cfg.access_tier.is_empty() {
                tier.push(("x-ms-access-tier", self.cfg.access_tier.clone()));
            }
            if data.len() <= block {
                let mut h = vec![("x-ms-blob-type", "BlockBlob".to_string()), ("content-md5", Self::md5(&data)), ("content-type", "application/octet-stream".to_string())];
                h.extend(tier);
                http::check(self.send(reqwest::Method::PUT, Some(&key), &[], &h, data).await?, "Azure upload").await?;
                return Ok(());
            }
            let mut ids = Vec::new();
            for (i, chunk) in data.chunks(block).enumerate() {
                // Block ids must have equal length within a blob.
                let id = base64::engine::general_purpose::STANDARD.encode(format!("block-{i:08}"));
                http::check(
                    self.send(reqwest::Method::PUT, Some(&key), &[("comp", "block".into()), ("blockid", id.clone())], &[("content-md5", Self::md5(chunk))], chunk.to_vec()).await?,
                    "Azure block upload",
                )
                .await?;
                ids.push(id);
            }
            let body: String = ids.iter().map(|i| format!("<Latest>{i}</Latest>")).collect();
            let body = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>{body}</BlockList>").into_bytes();
            let mut h = vec![("content-type", "application/xml".to_string())];
            h.extend(tier);
            http::check(self.send(reqwest::Method::PUT, Some(&key), &[("comp", "blocklist".into())], &h, body).await?, "Azure block list").await?;
            Ok(())
        })
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let r = self.send(reqwest::Method::GET, Some(&key), &[], &[], Vec::new()).await?;
            if r.status().as_u16() == 409 {
                let body = r.text().await.unwrap_or_default();
                if body.contains("BlobArchived") {
                    // Cold storage: ask Azure to rehydrate the blob to Hot.
                    let h = [("x-ms-access-tier", "Hot".to_string()), ("x-ms-rehydrate-priority", self.cfg.rehydrate_priority.clone())];
                    let _ = self.send(reqwest::Method::PUT, Some(&key), &[("comp", "tier".into())], &h, Vec::new()).await;
                    return Err(AppError::connection(format!("{name} is in the Archive tier; rehydration was requested"), "Retry after rehydration completes (up to 15 hours)."));
                }
                return Err(AppError::connection(format!("Azure download failed: {body}"), "Check the destination."));
            }
            let r = http::check(r, "Azure download").await?;
            Ok(r.bytes().await.map_err(|e| http::net("Azure download", e))?.to_vec())
        })
    }

    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            http::check(self.send(reqwest::Method::DELETE, Some(&key), &[], &[], Vec::new()).await?, "Azure delete").await?;
            Ok(())
        })
    }

    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            let r = self.send(reqwest::Method::PUT, None, &[("restype", "container".into())], &[], Vec::new()).await?;
            if r.status().as_u16() == 409 {
                return Ok(()); // already exists
            }
            http::check(r, "Azure container creation").await?;
            Ok(())
        })
    }

    fn supports_lock(&self) -> bool {
        true
    }

    fn lock<'a>(&'a self, name: &'a str, until: chrono::DateTime<chrono::Utc>, mode: LockMode) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let m = match mode {
                LockMode::Governance => "Unlocked",
                LockMode::Compliance => "Locked",
            };
            let h = [("x-ms-immutability-policy-until-date", until.format("%a, %d %b %Y %H:%M:%S GMT").to_string()), ("x-ms-immutability-policy-mode", m.to_string())];
            http::check(self.send(reqwest::Method::PUT, Some(&key), &[("comp", "immutabilityPolicies".into())], &h, Vec::new()).await?, "Azure immutability policy").await?;
            Ok(())
        })
    }
}
