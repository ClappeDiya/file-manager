//! Azure Files connector (SMB file shares over the REST API, so no port 445
//! is needed).
//!
//! - `/` lists shares; `/share/dir` lists one level (paged).
//! - Uploads create the file under a hidden temporary name, write 4 MiB
//!   ranges, and rename it into place (replacing an existing file).
//! - Folders are removed only when empty; shares are never deleted from here.
//! - Share links are read-only file SAS URLs.

use super::azure_rest::{self as az, Service, Slot};
use crate::backup_engine::stores::http;
use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use reqwest::Method;
use std::future::Future;
use std::pin::Pin;

const WHAT: &str = "Azure Files";

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

fn path_of(share: &str, p: &str) -> String {
    if p.is_empty() {
        format!("/{}", az::encode_path(share))
    } else {
        format!("/{}/{}", az::encode_path(share), az::encode_path(p))
    }
}

/// SMB properties every created file or folder needs.
fn smb_headers(attributes: &str) -> Vec<(&'static str, String)> {
    vec![
        ("x-ms-file-permission", "inherit".into()),
        ("x-ms-file-attributes", attributes.into()),
        ("x-ms-file-creation-time", "now".into()),
        ("x-ms-file-last-write-time", "now".into()),
    ]
}

#[derive(Default)]
pub struct AzureFilesConnector {
    slot: Slot,
}

impl AzureFilesConnector {
    pub fn new() -> Self {
        Self::default()
    }

    async fn list_shares(&self) -> Result<Vec<FileEntry>, AppError> {
        let c = self.slot.get(WHAT).await?;
        let mut out = Vec::new();
        let mut marker = String::new();
        loop {
            let mut q = vec![("comp", "list".to_string())];
            if !marker.is_empty() {
                q.push(("marker", marker.clone()));
            }
            let xml = c.call("Azure Files share listing", Method::GET, "/", &q, &[], Vec::new()).await?.text().await.unwrap_or_default();
            for x in http::xml_all(&xml, "Share") {
                let name = http::xml_unescape(&http::xml_one(&x, "Name").unwrap_or_default());
                out.push(az::entry("", &name, true, 0, http::xml_one(&x, "Last-Modified").and_then(|t| az::parse_time(&t))));
            }
            marker = http::xml_one(&xml, "NextMarker").unwrap_or_default();
            if marker.is_empty() {
                return Ok(out);
            }
        }
    }

    async fn list_dir(&self, share: &str, dir: &str, limit: Option<u32>) -> Result<Vec<FileEntry>, AppError> {
        let c = self.slot.get(WHAT).await?;
        let parent = format!("/{share}{}", if dir.is_empty() { String::new() } else { format!("/{dir}") });
        let mut out = Vec::new();
        let mut marker = String::new();
        loop {
            let mut q = vec![("restype", "directory".to_string()), ("comp", "list".to_string()), ("include", "Timestamps".to_string())];
            if let Some(n) = limit {
                q.push(("maxresults", n.to_string()));
            }
            if !marker.is_empty() {
                q.push(("marker", marker.clone()));
            }
            let xml = c.call("Azure Files listing", Method::GET, &path_of(share, dir), &q, &[], Vec::new()).await?.text().await.unwrap_or_default();
            for (tag, is_dir) in [("Directory", true), ("File", false)] {
                for x in http::xml_all(&xml, tag) {
                    let name = http::xml_unescape(&http::xml_one(&x, "Name").unwrap_or_default());
                    let size = if is_dir { 0 } else { http::xml_one(&x, "Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0) };
                    let modified = http::xml_one(&x, "LastWriteTime").or_else(|| http::xml_one(&x, "Last-Modified")).and_then(|t| az::parse_time(&t));
                    out.push(az::entry(&parent, &name, is_dir, size, modified));
                }
            }
            marker = http::xml_one(&xml, "NextMarker").unwrap_or_default();
            if marker.is_empty() || limit.is_some() {
                out.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.cmp(&b.name)));
                return Ok(out);
            }
        }
    }

    async fn write_file(&self, share: &str, p: &str, local: &str) -> Result<u64, AppError> {
        let c = self.slot.get(WHAT).await?;
        let path = path_of(share, p);
        let len = tokio::fs::metadata(local).await?.len();
        let mut h = smb_headers("None");
        h.push(("x-ms-type", "file".into()));
        h.push(("x-ms-content-length", len.to_string()));
        c.call("Azure Files file creation", Method::PUT, &path, &[], &h, Vec::new()).await?;
        az::for_each_chunk(local, |off, chunk| {
            let (c, path) = (c.clone(), path.clone());
            let range = format!("bytes={off}-{}", off + chunk.len() as u64 - 1);
            async move { c.call("Azure Files range upload", Method::PUT, &path, &[("comp", "range".into())], &[("x-ms-range", range), ("x-ms-write", "update".into())], chunk).await.map(|_| ()) }
        })
        .await
    }

    async fn rename(&self, share: &str, from: &str, to: &str) -> Result<(), AppError> {
        let c = self.slot.get(WHAT).await?;
        let src = format!("{}{}", c.base(), path_of(share, from));
        let h = [("x-ms-file-rename-source", src), ("x-ms-file-rename-replace-if-exists", "true".to_string())];
        c.call("Azure Files rename", Method::PUT, &path_of(share, to), &[("comp", "rename".into())], &h, Vec::new()).await?;
        Ok(())
    }

    async fn delete(&self, share: &str, p: &str, is_dir: bool) -> Result<(), AppError> {
        let c = self.slot.get(WHAT).await?;
        let q: Vec<(&str, String)> = if is_dir { vec![("restype", "directory".into())] } else { vec![] };
        c.call("Azure Files delete", Method::DELETE, &path_of(share, p), &q, &[], Vec::new()).await?;
        Ok(())
    }
}

impl Connector for AzureFilesConnector {
    fn rename_entry(&self, from: &str, to: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::core::error::AppError>> + Send + '_>> {
        let (from, to) = (from.to_string(), to.to_string());
        Box::pin(async move { let ((share, a), (share2, b)) = (az::split_root(&from), az::split_root(&to));
            if share.is_empty() || a.is_empty() || share != share2 || b.is_empty() {
                return Err(AppError::validation("Rename within one share: /share/path"));
            }
            self.rename(&share, &a, &b).await })
    }


    fn connect(&self, profile: &ConnectionProfile) -> Fut<'_, ()> {
        self.connect_with_secret(profile, None)
    }

    fn connect_with_secret(&self, profile: &ConnectionProfile, secret: Option<String>) -> Fut<'_, ()> {
        let profile = profile.clone();
        Box::pin(async move {
            self.slot.set(Some(az::client_for_profile(&profile, secret, Service::File)?)).await;
            let (share, _) = az::split_root(&profile.remote_path);
            let check = if share.is_empty() { self.list_shares().await.map(|_| ()) } else { self.list_dir(&share, "", Some(1)).await.map(|_| ()) };
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
        let (share, dir) = az::split_root(path);
        Box::pin(async move {
            if share.is_empty() {
                self.list_shares().await
            } else {
                self.list_dir(&share, &dir, None).await
            }
        })
    }

    fn is_connected(&self) -> bool {
        self.slot.is_set()
    }

    fn upload(&self, local_path: &str, remote_path: &str) -> Fut<'_, u64> {
        let (l, r) = (local_path.to_string(), remote_path.to_string());
        Box::pin(async move {
            let (share, p) = az::split_root(&r);
            if share.is_empty() || p.is_empty() {
                return Err(AppError::validation("Upload into a share: /share/path/file"));
            }
            super::staged_upload(
                &p,
                |temp| {
                    let (share, l) = (share.clone(), l.clone());
                    async move { self.write_file(&share, &temp, &l).await }
                },
                |from, to| {
                    let share = share.clone();
                    async move { self.rename(&share, &from, &to).await }
                },
                |x| {
                    let share = share.clone();
                    async move { self.delete(&share, &x, false).await }
                },
            )
            .await
        })
    }

    fn download(&self, remote_path: &str, local_path: &str) -> Fut<'_, u64> {
        let (r, l) = (remote_path.to_string(), local_path.to_string());
        Box::pin(async move {
            let (share, p) = az::split_root(&r);
            let c = self.slot.get(WHAT).await?;
            az::save_body(c.call("Azure Files download", Method::GET, &path_of(&share, &p), &[], &[], Vec::new()).await?, &l).await
        })
    }

    fn make_directory(&self, remote_path: &str) -> Fut<'_, ()> {
        let (share, dir) = az::split_root(remote_path);
        Box::pin(async move {
            let c = self.slot.get(WHAT).await?;
            if share.is_empty() {
                return Err(AppError::validation("Give the folder or share name"));
            }
            if dir.is_empty() {
                c.call("Azure Files share creation", Method::PUT, &path_of(&share, ""), &[("restype", "share".into())], &[], Vec::new()).await?;
            } else {
                c.call("Azure Files folder creation", Method::PUT, &path_of(&share, &dir), &[("restype", "directory".into())], &smb_headers("Directory"), Vec::new()).await?;
            }
            Ok(())
        })
    }

    fn remove(&self, remote_path: &str, is_dir: bool) -> Fut<'_, ()> {
        let (share, p) = az::split_root(remote_path);
        Box::pin(async move {
            if p.is_empty() {
                return Err(az::refuse_root_delete("share"));
            }
            self.delete(&share, &p, is_dir).await
        })
    }

    fn create_share_link(&self, remote_path: &str, options: &super::ShareOptions) -> Fut<'_, super::ShareLink> {
        let (r, o) = (remote_path.to_string(), options.clone());
        Box::pin(async move { az::share_link(&self.slot, WHAT, "azure_files", &r, &o).await })
    }
}
