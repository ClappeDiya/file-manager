//! OpenStack Swift connector (Keystone v3 password or application
//! credential, or TempAuth / v1).
//!
//! - `/` lists containers; `/container/dir/` lists objects one level deep
//!   (pseudo-folders from the `/` delimiter), page by page.
//! - Files up to the segment size go up in one PUT (with an ETag Swift
//!   verifies); larger ones as a Static Large Object: segments in
//!   `<container>_segments`, then a manifest. Replacing a large object
//!   removes its old segments.
//! - Folders are zero-byte `dir/` objects (`application/directory`);
//!   `/container` creates the container. Folders and containers are removed
//!   only when empty; deleting a large object deletes its segments too.
//! - Share links are TempURLs signed with the container's or account's
//!   `Temp-URL-Key`.
//!
//! Sign-in and requests go through [`SwiftSession`], shared with the Swift
//! backup destination.

use super::azure_rest as az;
use crate::backup_engine::stores::http;
use crate::backup_engine::stores::swift::{object_path, SwiftConfig, SwiftSession, LIST_PAGE};
use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry, SwiftAuthKind};
use bytes::Bytes;
use reqwest::Method;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

const WHAT: &str = "OpenStack Swift";

/// Objects above this size are uploaded as Static Large Objects.
const DEFAULT_SEGMENT_SIZE: u64 = 32 * 1024 * 1024;

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

pub struct SwiftConnector {
    session: RwLock<Option<Arc<SwiftSession>>>,
    segment_size: u64,
}

impl Default for SwiftConnector {
    fn default() -> Self {
        Self::new()
    }
}

/// The session settings for a connection profile.
pub fn config_for_profile(profile: &ConnectionProfile) -> Result<SwiftConfig, AppError> {
    let s = profile.extras.swift.clone().unwrap_or_default();
    let text = |o: &Option<String>| o.as_deref().map(str::trim).unwrap_or_default().to_string();
    let auth_url = match text(&s.auth_url) {
        u if !u.is_empty() => u,
        _ if profile.host.trim().is_empty() => return Err(AppError::validation("Enter the Swift auth URL (or the Keystone host)")),
        _ => {
            let host = profile.host.trim().trim_end_matches('/');
            let base = if host.contains("://") { host.to_string() } else { format!("https://{host}") };
            let base = match profile.port {
                Some(p) if !base.rsplit("//").next().unwrap_or_default().contains(':') => format!("{base}:{p}"),
                _ => base,
            };
            if s.auth_kind == SwiftAuthKind::TempAuth { format!("{base}/auth/v1.0") } else { format!("{base}/v3") }
        }
    };
    let domain = match text(&s.domain_name) {
        d if d.is_empty() => "Default".to_string(),
        d => d,
    };
    let cfg = SwiftConfig {
        auth_url,
        auth_version: if s.auth_kind == SwiftAuthKind::TempAuth { "1".into() } else { "3".into() },
        username: text(&profile.username),
        user_domain: domain.clone(),
        project: text(&s.project_name),
        project_domain: domain,
        region: text(&s.region),
        application_credential_id: if s.auth_kind == SwiftAuthKind::ApplicationCredential { text(&s.application_credential_id) } else { String::new() },
        ..Default::default()
    };
    match s.auth_kind {
        SwiftAuthKind::Password if cfg.username.is_empty() || cfg.project.is_empty() => Err(AppError::validation("Enter the user name and project for Keystone sign-in")),
        SwiftAuthKind::ApplicationCredential if cfg.application_credential_id.is_empty() => Err(AppError::validation("Enter the application credential ID")),
        SwiftAuthKind::TempAuth if cfg.username.is_empty() => Err(AppError::validation("Enter the TempAuth user (account:user)")),
        _ => Ok(cfg),
    }
}

/// Swift's `last_modified` (`2024-01-02T03:04:05.123456`, UTC without a zone).
fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f").ok().map(|t| t.and_utc())
}

fn md5_hex(b: &[u8]) -> String {
    use md5::Digest;
    hex::encode(md5::Md5::digest(b))
}

fn header(r: &reqwest::Response, name: &str) -> Option<String> {
    r.headers().get(name).and_then(|v| v.to_str().ok()).map(String::from)
}

/// `/container/object` (unencoded, as a manifest lists it) → encoded request path.
fn encoded_segment_path(p: &str) -> String {
    let (c, k) = p.trim_start_matches('/').split_once('/').unwrap_or((p, ""));
    object_path(&percent_decode(c), Some(&percent_decode(k)))
}

fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

impl SwiftConnector {
    pub fn new() -> Self {
        Self { session: RwLock::new(None), segment_size: DEFAULT_SEGMENT_SIZE }
    }

    /// A connector that uploads files above `bytes` as large objects.
    pub fn with_segment_size(bytes: u64) -> Self {
        Self { session: RwLock::new(None), segment_size: bytes.max(1) }
    }

    async fn s(&self) -> Result<Arc<SwiftSession>, AppError> {
        self.session.read().await.clone().ok_or_else(|| az::not_connected(WHAT))
    }

    async fn list_containers(&self) -> Result<Vec<FileEntry>, AppError> {
        let s = self.s().await?;
        let mut out = Vec::new();
        let mut marker = String::new();
        loop {
            let page = s.list_page("Swift container listing", "", &[], &marker, LIST_PAGE).await?;
            let Some(last) = page.last().and_then(|o| o["name"].as_str()) else { return Ok(out) };
            marker = last.to_string();
            for c in &page {
                let name = c["name"].as_str().unwrap_or_default();
                out.push(az::entry("", name, true, c["bytes"].as_u64().unwrap_or(0), c["last_modified"].as_str().and_then(parse_time)));
            }
        }
    }

    /// Objects and pseudo-folders directly under `prefix` (no trailing `/`).
    async fn list_level(&self, container: &str, prefix: &str) -> Result<Vec<FileEntry>, AppError> {
        let s = self.s().await?;
        let pfx = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
        let parent = format!("/{container}{}", if prefix.is_empty() { String::new() } else { format!("/{prefix}") });
        let path = object_path(container, None);
        let q = [("prefix", pfx.clone()), ("delimiter", "/".to_string())];
        let mut out: Vec<FileEntry> = Vec::new();
        let mut seen_dirs = std::collections::HashSet::new();
        let mut marker = String::new();
        loop {
            let page = s.list_page("Swift listing", &path, &q, &marker, LIST_PAGE).await?;
            let Some(last) = page.last().and_then(|o| o["name"].as_str().or(o["subdir"].as_str())) else { return Ok(out) };
            marker = last.to_string();
            for o in &page {
                if let Some(sub) = o["subdir"].as_str() {
                    let n = sub.strip_prefix(&pfx).unwrap_or(sub).trim_end_matches('/');
                    if !n.is_empty() && seen_dirs.insert(n.to_string()) {
                        out.push(az::entry(&parent, n, true, 0, None));
                    }
                    continue;
                }
                let name = o["name"].as_str().unwrap_or_default();
                let n = name.strip_prefix(&pfx).unwrap_or(name);
                if n.is_empty() {
                    continue; // this folder's own marker
                }
                if let Some(d) = n.strip_suffix('/') {
                    // A folder marker listed as an object (servers without delimiter support).
                    if !d.contains('/') && seen_dirs.insert(d.to_string()) {
                        out.push(az::entry(&parent, d, true, 0, None));
                    }
                    continue;
                }
                out.push(az::entry(&parent, n, false, o["bytes"].as_u64().unwrap_or(0), o["last_modified"].as_str().and_then(parse_time)));
            }
        }
    }

    /// `true` for a Static Large Object manifest (`false` when missing).
    async fn is_large(&self, path: &str) -> Result<bool, AppError> {
        let r = self.s().await?.send(Method::HEAD, path, &[], &[], Bytes::new()).await?;
        Ok(r.status().is_success() && header(&r, "x-static-large-object").is_some_and(|v| v.eq_ignore_ascii_case("true")))
    }

    /// Segment paths (`/container/object`) of a large object, or none for a plain one.
    async fn segments_of(&self, path: &str) -> Result<Vec<String>, AppError> {
        if !self.is_large(path).await? {
            return Ok(Vec::new());
        }
        let r = self.s().await?.call("Swift manifest read", Method::GET, path, &[("multipart-manifest", "get".into())], &[], Bytes::new()).await?;
        let v: Vec<serde_json::Value> = r.json().await.map_err(|e| http::net("Swift manifest read", e))?;
        Ok(v.iter().filter_map(|seg| seg["name"].as_str().map(String::from)).collect())
    }

    async fn delete_segments(&self, segments: &[String]) {
        if let Ok(s) = self.s().await {
            for seg in segments {
                let _ = s.send(Method::DELETE, &encoded_segment_path(seg), &[], &[], Bytes::new()).await;
            }
        }
    }

    async fn upload_object(&self, local: &str, remote: &str) -> Result<u64, AppError> {
        let (container, key) = az::split_root(remote);
        if container.is_empty() || key.is_empty() {
            return Err(AppError::validation("Upload into a container: /container/path/file"));
        }
        let s = self.s().await?;
        let path = object_path(&container, Some(&key));
        let len = tokio::fs::metadata(local).await?.len();
        let detect = ("X-Detect-Content-Type", "true".to_string());
        let old_segments = self.segments_of(&path).await?;
        if len <= self.segment_size {
            let data = Bytes::from(tokio::fs::read(local).await?);
            s.call("Swift upload", Method::PUT, &path, &[], &[("ETag", md5_hex(&data)), detect], data).await?;
            self.delete_segments(&old_segments).await;
            return Ok(len);
        }

        // Static Large Object: segments, then the manifest that joins them.
        let seg_container = format!("{container}_segments");
        s.call("Swift segment container creation", Method::PUT, &object_path(&seg_container, None), &[], &[], Bytes::new()).await?;
        let base = format!("{key}/slo/{}/{len}/{}", chrono::Utc::now().format("%Y%m%dT%H%M%S%.6fZ"), self.segment_size);
        let mut manifest = Vec::new();
        let mut uploaded = Vec::new();
        let result: Result<(), AppError> = async {
            use tokio::io::AsyncReadExt;
            let mut file = tokio::fs::File::open(local).await?;
            for i in 0u64.. {
                let mut buf = Vec::with_capacity(self.segment_size as usize);
                (&mut file).take(self.segment_size).read_to_end(&mut buf).await?;
                if buf.is_empty() {
                    return Ok(());
                }
                let name = format!("{base}/{i:08}");
                let etag = md5_hex(&buf);
                let size = buf.len();
                uploaded.push(format!("/{seg_container}/{name}"));
                s.call("Swift segment upload", Method::PUT, &object_path(&seg_container, Some(&name)), &[], &[("ETag", etag.clone())], Bytes::from(buf)).await?;
                manifest.push(serde_json::json!({ "path": format!("/{seg_container}/{name}"), "etag": etag, "size_bytes": size }));
            }
            Ok(())
        }
        .await;
        let result = match result {
            Ok(()) => {
                let body = Bytes::from(serde_json::to_vec(&manifest).unwrap_or_default());
                s.call("Swift large object manifest", Method::PUT, &path, &[("multipart-manifest", "put".into())], &[detect], body).await.map(|_| ())
            }
            Err(e) => Err(e),
        };
        if let Err(e) = result {
            self.delete_segments(&uploaded).await;
            return Err(e);
        }
        self.delete_segments(&old_segments).await;
        Ok(len)
    }

    /// `true` when `container/prefix/` holds anything besides its own marker.
    async fn has_children(&self, container: &str, prefix: &str) -> Result<bool, AppError> {
        let s = self.s().await?;
        let pfx = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
        let page = s.list_page("Swift listing", &object_path(container, None), &[("prefix", pfx.clone())], "", 2).await?;
        Ok(page.iter().any(|o| o["name"].as_str().is_some_and(|n| n != pfx)))
    }

    /// The TempURL key of the container, else of the account.
    async fn temp_url_key(&self, container: &str) -> Result<String, AppError> {
        let s = self.s().await?;
        for (path, prefix) in [(object_path(container, None), "x-container-meta-temp-url-key"), (String::new(), "x-account-meta-temp-url-key")] {
            let r = s.call("Swift metadata read", Method::HEAD, &path, &[], &[], Bytes::new()).await?;
            if let Some(k) = [prefix.to_string(), format!("{prefix}-2")].iter().find_map(|h| header(&r, h)).filter(|k| !k.is_empty()) {
                return Ok(k);
            }
        }
        Err(AppError::connection(
            "This Swift account has no TempURL key, so share links cannot be signed",
            "Set one on the account or container (for example `swift post -m Temp-URL-Key:<secret>`), then try again.",
        ))
    }

    async fn share_link(&self, remote: &str, o: &super::ShareOptions) -> Result<super::ShareLink, AppError> {
        let hours = az::sas_hours(o)?;
        let (container, key) = az::split_root(remote);
        if container.is_empty() || key.is_empty() {
            return Err(AppError::validation("Choose a file to share"));
        }
        let secret = self.temp_url_key(&container).await?;
        let storage = self.s().await?.storage_url().await;
        let url = reqwest::Url::parse(&storage).map_err(|e| AppError::internal(format!("Swift storage URL: {e}")))?;
        let expires = chrono::Utc::now() + chrono::Duration::hours(hours as i64);
        // The signature covers the decoded request path.
        let signed_path = format!("{}/{container}/{key}", percent_decode(url.path()).trim_end_matches('/'));
        Ok(super::ShareLink {
            url: format!("{storage}{}?temp_url_sig={}&temp_url_expires={}", object_path(&container, Some(&key)), temp_url_signature(&secret, expires.timestamp(), &signed_path), expires.timestamp()),
            expires_at: Some(expires),
            scope: super::ShareScope::Anyone,
            provider: "swift".into(),
            note: Some("Read-only TempURL: anyone holding it can download until it expires, or until the TempURL key changes.".into()),
        })
    }
}

/// HMAC-SHA256 TempURL signature for a GET of `path` until `expires`.
pub fn temp_url_signature(key: &str, expires: i64, path: &str) -> String {
    use hmac::{Hmac, Mac};
    let mut m = Hmac::<sha2::Sha256>::new_from_slice(key.as_bytes()).expect("HMAC takes any key length");
    m.update(format!("GET\n{expires}\n{path}").as_bytes());
    hex::encode(m.finalize().into_bytes())
}

impl Connector for SwiftConnector {
    fn connect(&self, profile: &ConnectionProfile) -> Fut<'_, ()> {
        self.connect_with_secret(profile, None)
    }

    fn connect_with_secret(&self, profile: &ConnectionProfile, secret: Option<String>) -> Fut<'_, ()> {
        let profile = profile.clone();
        Box::pin(async move {
            let cfg = config_for_profile(&profile)?;
            let secret = secret.filter(|s| !s.is_empty()).ok_or_else(|| AppError::validation("Enter the password (or application credential secret)"))?;
            let session = Arc::new(SwiftSession::connect(cfg, secret).await?);
            // The sign-in only proves the credentials; reading the account proves the storage URL works.
            session.call("Swift account read", Method::HEAD, "", &[], &[], Bytes::new()).await?;
            *self.session.write().await = Some(session);
            Ok(())
        })
    }

    fn disconnect(&self) -> Fut<'_, ()> {
        Box::pin(async move {
            *self.session.write().await = None;
            Ok(())
        })
    }

    fn list_remote(&self, path: &str) -> Fut<'_, Vec<FileEntry>> {
        let (container, prefix) = az::split_root(path);
        Box::pin(async move {
            if container.is_empty() {
                self.list_containers().await
            } else {
                self.list_level(&container, &prefix).await
            }
        })
    }

    fn is_connected(&self) -> bool {
        self.session.try_read().map(|s| s.is_some()).unwrap_or(false)
    }

    fn upload(&self, local_path: &str, remote_path: &str) -> Fut<'_, u64> {
        let (l, r) = (local_path.to_string(), remote_path.to_string());
        Box::pin(async move { self.upload_object(&l, &r).await })
    }

    fn download(&self, remote_path: &str, local_path: &str) -> Fut<'_, u64> {
        let (container, key) = az::split_root(remote_path);
        let l = local_path.to_string();
        Box::pin(async move {
            if key.is_empty() {
                return Err(AppError::validation("Choose a file to download"));
            }
            let r = self.s().await?.call("Swift download", Method::GET, &object_path(&container, Some(&key)), &[], &[], Bytes::new()).await?;
            az::save_body_as(r, &l, "Swift download").await
        })
    }

    fn make_directory(&self, remote_path: &str) -> Fut<'_, ()> {
        let (container, dir) = az::split_root(remote_path);
        Box::pin(async move {
            let s = self.s().await?;
            if container.is_empty() {
                return Err(AppError::validation("Give the folder or container name"));
            }
            if dir.is_empty() {
                s.call("Swift container creation", Method::PUT, &object_path(&container, None), &[], &[], Bytes::new()).await?;
                return Ok(());
            }
            let ct = ("Content-Type", "application/directory".to_string());
            s.call("Swift folder creation", Method::PUT, &object_path(&container, Some(&format!("{dir}/"))), &[], &[ct], Bytes::new()).await?;
            Ok(())
        })
    }

    fn remove(&self, remote_path: &str, is_dir: bool) -> Fut<'_, ()> {
        let (container, key) = az::split_root(remote_path);
        let remote_path = remote_path.to_string();
        Box::pin(async move {
            let s = self.s().await?;
            if container.is_empty() {
                return Err(AppError::validation("Choose what to delete"));
            }
            if is_dir || key.is_empty() {
                if self.has_children(&container, &key).await? {
                    return Err(AppError::validation(format!("{remote_path} is not empty")));
                }
                if key.is_empty() {
                    s.call("Swift container delete", Method::DELETE, &object_path(&container, None), &[], &[], Bytes::new()).await?;
                    return Ok(());
                }
                // Pseudo-folders may have no marker; deleting a missing one is fine.
                let r = s.send(Method::DELETE, &object_path(&container, Some(&format!("{key}/"))), &[], &[], Bytes::new()).await?;
                if !(r.status().is_success() || r.status().as_u16() == 404) {
                    http::check(r, "Swift folder delete").await?;
                }
                return Ok(());
            }
            let path = object_path(&container, Some(&key));
            // A large object's manifest delete removes its segments too.
            let q: &[(&str, String)] = if self.is_large(&path).await? { &[("multipart-manifest", "delete".to_string())] } else { &[] };
            s.call("Swift delete", Method::DELETE, &path, q, &[], Bytes::new()).await?;
            Ok(())
        })
    }

    fn create_share_link(&self, remote_path: &str, options: &super::ShareOptions) -> Fut<'_, super::ShareLink> {
        let (r, o) = (remote_path.to_string(), options.clone());
        Box::pin(async move { self.share_link(&r, &o).await })
    }
}

#[cfg(test)]
#[path = "swift_tests.rs"]
mod tests;
