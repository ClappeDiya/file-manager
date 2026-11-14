//! Azure Blob Storage connector (Shared Key, SAS token or connection
//! string; public, government and China clouds, Azurite).
//!
//! - `/` lists containers; `/container/dir/` lists blobs one level deep
//!   (virtual folders from the `/` delimiter).
//! - Uploads stream in 4 MiB blocks (Put Block + Put Block List; small files
//!   in one Put Blob), so a blob only appears once complete.
//! - Folders are zero-byte `dir/` marker blobs; `/container` creates the
//!   container. Folders and containers are removed only when empty.
//! - Share links are read-only service SAS URLs.

use super::azure_rest::{self as az, Service, Slot};
use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use base64::Engine;
use reqwest::Method;
use std::future::Future;
use std::pin::Pin;

const WHAT: &str = "Azure Blob Storage";

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

#[derive(Default)]
pub struct AzureBlobConnector {
    slot: Slot,
}

impl AzureBlobConnector {
    pub fn new() -> Self {
        Self::default()
    }

    async fn list_containers(&self) -> Result<Vec<FileEntry>, AppError> {
        let c = self.slot.get(WHAT).await?;
        let mut out = Vec::new();
        let mut marker = String::new();
        loop {
            let mut q = vec![("comp", "list".to_string())];
            if !marker.is_empty() {
                q.push(("marker", marker.clone()));
            }
            let xml = c.call("Azure container listing", Method::GET, "/", &q, &[], Vec::new()).await?.text().await.unwrap_or_default();
            for x in http::xml_all(&xml, "Container") {
                let name = http::xml_unescape(&http::xml_one(&x, "Name").unwrap_or_default());
                let modified = http::xml_one(&x, "Last-Modified").and_then(|t| az::parse_time(&t));
                out.push(az::entry("", &name, true, 0, modified));
            }
            marker = http::xml_one(&xml, "NextMarker").unwrap_or_default();
            if marker.is_empty() {
                return Ok(out);
            }
        }
    }

    /// Blobs and virtual folders directly under `prefix` (no trailing `/`).
    async fn list_level(&self, container: &str, prefix: &str, limit: Option<u32>) -> Result<Vec<FileEntry>, AppError> {
        let c = self.slot.get(WHAT).await?;
        let pfx = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
        let parent = format!("/{container}{}", if prefix.is_empty() { String::new() } else { format!("/{prefix}") });
        let mut out = Vec::new();
        let mut marker = String::new();
        loop {
            let mut q = vec![("restype", "container".to_string()), ("comp", "list".to_string()), ("delimiter", "/".to_string()), ("prefix", pfx.clone())];
            if let Some(n) = limit {
                q.push(("maxresults", n.to_string()));
            }
            if !marker.is_empty() {
                q.push(("marker", marker.clone()));
            }
            let xml = c.call("Azure listing", Method::GET, &format!("/{}", az::encode_path(container)), &q, &[], Vec::new()).await?.text().await.unwrap_or_default();
            for x in http::xml_all(&xml, "BlobPrefix") {
                let name = http::xml_unescape(&http::xml_one(&x, "Name").unwrap_or_default());
                let n = name.strip_prefix(&pfx).unwrap_or(&name).trim_end_matches('/');
                if !n.is_empty() {
                    out.push(az::entry(&parent, n, true, 0, None));
                }
            }
            for x in http::xml_all(&xml, "Blob") {
                let name = http::xml_unescape(&http::xml_one(&x, "Name").unwrap_or_default());
                let n = name.strip_prefix(&pfx).unwrap_or(&name);
                if n.is_empty() || n.ends_with('/') {
                    continue; // this folder's own marker
                }
                let size = http::xml_one(&x, "Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0);
                out.push(az::entry(&parent, n, false, size, http::xml_one(&x, "Last-Modified").and_then(|t| az::parse_time(&t))));
            }
            marker = http::xml_one(&xml, "NextMarker").unwrap_or_default();
            if marker.is_empty() || limit.is_some() {
                return Ok(out);
            }
        }
    }

    async fn upload_blob(&self, local: &str, remote: &str) -> Result<u64, AppError> {
        let (container, blob) = az::split_root(remote);
        if container.is_empty() || blob.is_empty() {
            return Err(AppError::validation("Upload into a container: /container/path/file"));
        }
        let c = self.slot.get(WHAT).await?;
        let path = format!("/{}/{}", az::encode_path(&container), az::encode_path(&blob));
        let len = tokio::fs::metadata(local).await?.len();
        let ct = [("content-type", "application/octet-stream".to_string())];
        if len <= az::CHUNK as u64 {
            let data = tokio::fs::read(local).await?;
            let h = [("x-ms-blob-type", "BlockBlob".to_string()), ct[0].clone()];
            c.call("Azure upload", Method::PUT, &path, &[], &h, data).await?;
            return Ok(len);
        }
        let mut ids = Vec::new();
        let n = az::for_each_chunk(local, |off, chunk| {
            // Block ids must have equal length within a blob.
            let id = base64::engine::general_purpose::STANDARD.encode(format!("{:012}", off / az::CHUNK as u64));
            ids.push(id.clone());
            let (c, path) = (c.clone(), path.clone());
            async move { c.call("Azure block upload", Method::PUT, &path, &[("comp", "block".into()), ("blockid", id)], &[], chunk).await.map(|_| ()) }
        })
        .await?;
        let body: String = ids.iter().map(|i| format!("<Latest>{i}</Latest>")).collect();
        let body = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>{body}</BlockList>").into_bytes();
        let h = [("content-type", "application/xml".to_string()), ("x-ms-blob-content-type", "application/octet-stream".to_string())];
        c.call("Azure block list", Method::PUT, &path, &[("comp", "blocklist".into())], &h, body).await?;
        Ok(n)
    }
}

use crate::backup_engine::stores::http;

impl Connector for AzureBlobConnector {
    fn connect(&self, profile: &ConnectionProfile) -> Fut<'_, ()> {
        self.connect_with_secret(profile, None)
    }

    fn connect_with_secret(&self, profile: &ConnectionProfile, secret: Option<String>) -> Fut<'_, ()> {
        let profile = profile.clone();
        Box::pin(async move {
            let client = az::client_for_profile(&profile, secret, Service::Blob)?;
            self.slot.set(Some(client)).await;
            // Verify the credentials: the account's containers, or (for a
            // container-scoped SAS) the container in the remote path.
            let (container, _) = az::split_root(&profile.remote_path);
            let check = if container.is_empty() { self.list_containers().await.map(|_| ()) } else { self.list_level(&container, "", Some(1)).await.map(|_| ()) };
            if let Err(e) = check {
                self.slot.set(None).await;
                return Err(e);
            }
            Ok(())
        })
    }

    fn disconnect(&self) -> Fut<'_, ()> {
        Box::pin(async move {
            self.slot.set(None).await;
            Ok(())
        })
    }

    fn list_remote(&self, path: &str) -> Fut<'_, Vec<FileEntry>> {
        let (container, prefix) = az::split_root(path);
        Box::pin(async move {
            if container.is_empty() {
                self.list_containers().await
            } else {
                self.list_level(&container, &prefix, None).await
            }
        })
    }

    fn is_connected(&self) -> bool {
        self.slot.is_set()
    }

    fn upload(&self, local_path: &str, remote_path: &str) -> Fut<'_, u64> {
        let (l, r) = (local_path.to_string(), remote_path.to_string());
        Box::pin(async move { self.upload_blob(&l, &r).await })
    }

    fn download(&self, remote_path: &str, local_path: &str) -> Fut<'_, u64> {
        let (r, l) = (remote_path.to_string(), local_path.to_string());
        Box::pin(async move {
            let c = self.slot.get(WHAT).await?;
            let resp = c.call("Azure download", Method::GET, &az::encode_path(&format!("/{}", r.trim_start_matches('/'))), &[], &[], Vec::new()).await?;
            az::save_body(resp, &l).await
        })
    }

    fn make_directory(&self, remote_path: &str) -> Fut<'_, ()> {
        let (container, dir) = az::split_root(remote_path);
        Box::pin(async move {
            let c = self.slot.get(WHAT).await?;
            if container.is_empty() {
                return Err(AppError::validation("Give the folder or container name"));
            }
            if dir.is_empty() {
                c.call("Azure container creation", Method::PUT, &format!("/{}", az::encode_path(&container)), &[("restype", "container".into())], &[], Vec::new()).await?;
                return Ok(());
            }
            let path = format!("/{}/{}/", az::encode_path(&container), az::encode_path(&dir));
            c.call("Azure folder creation", Method::PUT, &path, &[], &[("x-ms-blob-type", "BlockBlob".to_string())], Vec::new()).await?;
            Ok(())
        })
    }

    fn remove(&self, remote_path: &str, is_dir: bool) -> Fut<'_, ()> {
        let (container, key) = az::split_root(remote_path);
        let remote_path = remote_path.to_string();
        Box::pin(async move {
            let c = self.slot.get(WHAT).await?;
            if key.is_empty() {
                return Err(az::refuse_root_delete("container"));
            }
            if is_dir {
                if !self.list_level(&container, &key, Some(1)).await?.is_empty() {
                    return Err(AppError::validation(format!("{remote_path} is not empty")));
                }
                // Virtual folders may have no marker; deleting a missing one is fine.
                let r = c.send(Method::DELETE, &format!("/{}/{}/", az::encode_path(&container), az::encode_path(&key)), &[], &[], Vec::new()).await?;
                if r.status().is_success() || r.status().as_u16() == 404 {
                    return Ok(());
                }
                http::check(r, "Azure folder delete").await?;
                return Ok(());
            }
            c.call("Azure delete", Method::DELETE, &format!("/{}/{}", az::encode_path(&container), az::encode_path(&key)), &[], &[], Vec::new()).await?;
            Ok(())
        })
    }

    fn create_share_link(&self, remote_path: &str, options: &super::ShareOptions) -> Fut<'_, super::ShareLink> {
        let (r, o) = (remote_path.to_string(), options.clone());
        Box::pin(async move { az::share_link(&self.slot, WHAT, "azure_blob", &r, &o).await })
    }
}
