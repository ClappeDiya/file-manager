//! Azure Data Lake Storage Gen2 connector (the `dfs` endpoint of an account
//! with a hierarchical namespace): real folders, atomic renames and
//! recursive-free deletes.
//!
//! - `/` lists file systems; `/fs/dir` lists one level (paged).
//! - Uploads create the file under a hidden temporary name, append 4 MiB
//!   chunks, flush, and rename it into place, so readers never see a
//!   partial file.
//! - Folders are removed only when empty; file systems are never deleted
//!   from here.
//! - Share links are read-only SAS URLs through the blob endpoint.

use super::azure_rest::{self as az, Service, Slot};
use crate::backup_engine::stores::http;
use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use reqwest::Method;
use serde::Deserialize;
use std::future::Future;
use std::pin::Pin;

const WHAT: &str = "Azure Data Lake Storage";

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

#[derive(Deserialize)]
struct FileSystems {
    #[serde(default)]
    filesystems: Vec<FsItem>,
}

#[derive(Deserialize)]
struct FsItem {
    name: String,
    #[serde(rename = "lastModified", default)]
    last_modified: String,
}

#[derive(Deserialize)]
struct Paths {
    #[serde(default)]
    paths: Vec<PathItem>,
}

/// DFS returns booleans and numbers as strings.
#[derive(Deserialize)]
struct PathItem {
    name: String,
    #[serde(rename = "isDirectory", default)]
    is_directory: Option<serde_json::Value>,
    #[serde(rename = "contentLength", default)]
    content_length: Option<serde_json::Value>,
    #[serde(rename = "lastModified", default)]
    last_modified: String,
}

fn truthy(v: &Option<serde_json::Value>) -> bool {
    matches!(v, Some(serde_json::Value::Bool(true))) || matches!(v, Some(serde_json::Value::String(s)) if s == "true")
}

fn number(v: &Option<serde_json::Value>) -> u64 {
    match v {
        Some(serde_json::Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(serde_json::Value::String(s)) => s.parse().unwrap_or(0),
        _ => 0,
    }
}

fn path_of(fs: &str, p: &str) -> String {
    if p.is_empty() {
        format!("/{}", az::encode_path(fs))
    } else {
        format!("/{}/{}", az::encode_path(fs), az::encode_path(p))
    }
}

#[derive(Default)]
pub struct AdlsGen2Connector {
    slot: Slot,
}

impl AdlsGen2Connector {
    pub fn new() -> Self {
        Self::default()
    }

    async fn list_filesystems(&self) -> Result<Vec<FileEntry>, AppError> {
        let c = self.slot.get(WHAT).await?;
        let mut out = Vec::new();
        let mut cont = String::new();
        loop {
            let mut q = vec![("resource", "account".to_string())];
            if !cont.is_empty() {
                q.push(("continuation", cont.clone()));
            }
            let r = c.call("Data Lake listing", Method::GET, "/", &q, &[], Vec::new()).await?;
            cont = r.headers().get("x-ms-continuation").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let body: FileSystems = r.json().await.map_err(|e| http::net("Data Lake listing", e))?;
            out.extend(body.filesystems.into_iter().map(|f| az::entry("", &f.name, true, 0, az::parse_time(&f.last_modified))));
            if cont.is_empty() {
                return Ok(out);
            }
        }
    }

    async fn list_dir(&self, fs: &str, dir: &str, limit: Option<u32>) -> Result<Vec<FileEntry>, AppError> {
        let c = self.slot.get(WHAT).await?;
        let parent = format!("/{fs}{}", if dir.is_empty() { String::new() } else { format!("/{dir}") });
        let mut out = Vec::new();
        let mut cont = String::new();
        loop {
            let mut q = vec![("resource", "filesystem".to_string()), ("recursive", "false".to_string())];
            if !dir.is_empty() {
                q.push(("directory", dir.to_string()));
            }
            if let Some(n) = limit {
                q.push(("maxResults", n.to_string()));
            }
            if !cont.is_empty() {
                q.push(("continuation", cont.clone()));
            }
            let r = c.call("Data Lake listing", Method::GET, &path_of(fs, ""), &q, &[], Vec::new()).await?;
            cont = r.headers().get("x-ms-continuation").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let body: Paths = r.json().await.map_err(|e| http::net("Data Lake listing", e))?;
            for p in body.paths {
                let name = p.name.rsplit('/').next().unwrap_or(&p.name).to_string();
                out.push(az::entry(&parent, &name, truthy(&p.is_directory), number(&p.content_length), az::parse_time(&p.last_modified)));
            }
            if cont.is_empty() || limit.is_some() {
                return Ok(out);
            }
        }
    }

    async fn rename(&self, fs: &str, from: &str, to: &str) -> Result<(), AppError> {
        let c = self.slot.get(WHAT).await?;
        let src = format!("/{}/{}", az::encode_path(fs), az::encode_path(from));
        c.call("Data Lake rename", Method::PUT, &path_of(fs, to), &[("mode", "legacy".into())], &[("x-ms-rename-source", src)], Vec::new()).await?;
        Ok(())
    }

    async fn delete(&self, fs: &str, p: &str) -> Result<(), AppError> {
        let c = self.slot.get(WHAT).await?;
        c.call("Data Lake delete", Method::DELETE, &path_of(fs, p), &[("recursive", "false".into())], &[], Vec::new()).await?;
        Ok(())
    }

    async fn write_file(&self, fs: &str, p: &str, local: &str) -> Result<u64, AppError> {
        let c = self.slot.get(WHAT).await?;
        let path = path_of(fs, p);
        c.call("Data Lake file creation", Method::PUT, &path, &[("resource", "file".into())], &[], Vec::new()).await?;
        let n = az::for_each_chunk(local, |off, chunk| {
            let (c, path) = (c.clone(), path.clone());
            async move { c.call("Data Lake append", Method::PATCH, &path, &[("action", "append".into()), ("position", off.to_string())], &[], chunk).await.map(|_| ()) }
        })
        .await?;
        c.call("Data Lake flush", Method::PATCH, &path, &[("action", "flush".into()), ("position", n.to_string())], &[], Vec::new()).await?;
        Ok(n)
    }
}

impl Connector for AdlsGen2Connector {
    fn connect(&self, profile: &ConnectionProfile) -> Fut<'_, ()> {
        self.connect_with_secret(profile, None)
    }

    fn connect_with_secret(&self, profile: &ConnectionProfile, secret: Option<String>) -> Fut<'_, ()> {
        let profile = profile.clone();
        Box::pin(async move {
            self.slot.set(Some(az::client_for_profile(&profile, secret, Service::Dfs)?)).await;
            let (fs, _) = az::split_root(&profile.remote_path);
            let check = if fs.is_empty() { self.list_filesystems().await.map(|_| ()) } else { self.list_dir(&fs, "", Some(1)).await.map(|_| ()) };
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
        let (fs, dir) = az::split_root(path);
        Box::pin(async move {
            if fs.is_empty() {
                self.list_filesystems().await
            } else {
                self.list_dir(&fs, &dir, None).await
            }
        })
    }

    fn is_connected(&self) -> bool {
        self.slot.is_set()
    }

    fn upload(&self, local_path: &str, remote_path: &str) -> Fut<'_, u64> {
        let (l, r) = (local_path.to_string(), remote_path.to_string());
        Box::pin(async move {
            let (fs, p) = az::split_root(&r);
            if fs.is_empty() || p.is_empty() {
                return Err(AppError::validation("Upload into a file system: /filesystem/path/file"));
            }
            super::staged_upload(
                &p,
                |temp| {
                    let (fs, l) = (fs.clone(), l.clone());
                    async move { self.write_file(&fs, &temp, &l).await }
                },
                |from, to| {
                    let fs = fs.clone();
                    async move { self.rename(&fs, &from, &to).await }
                },
                |x| {
                    let fs = fs.clone();
                    async move { self.delete(&fs, &x).await }
                },
            )
            .await
        })
    }

    fn download(&self, remote_path: &str, local_path: &str) -> Fut<'_, u64> {
        let (r, l) = (remote_path.to_string(), local_path.to_string());
        Box::pin(async move {
            let (fs, p) = az::split_root(&r);
            let c = self.slot.get(WHAT).await?;
            az::save_body(c.call("Data Lake download", Method::GET, &path_of(&fs, &p), &[], &[], Vec::new()).await?, &l).await
        })
    }

    fn make_directory(&self, remote_path: &str) -> Fut<'_, ()> {
        let (fs, dir) = az::split_root(remote_path);
        Box::pin(async move {
            let c = self.slot.get(WHAT).await?;
            if fs.is_empty() {
                return Err(AppError::validation("Give the folder or file system name"));
            }
            let resource = if dir.is_empty() { "filesystem" } else { "directory" };
            c.call("Data Lake folder creation", Method::PUT, &path_of(&fs, &dir), &[("resource", resource.into())], &[], Vec::new()).await?;
            Ok(())
        })
    }

    fn remove(&self, remote_path: &str, _is_dir: bool) -> Fut<'_, ()> {
        let (fs, p) = az::split_root(remote_path);
        Box::pin(async move {
            if p.is_empty() {
                return Err(az::refuse_root_delete("file system"));
            }
            self.delete(&fs, &p).await
        })
    }

    fn create_share_link(&self, remote_path: &str, options: &super::ShareOptions) -> Fut<'_, super::ShareLink> {
        let (r, o) = (remote_path.to_string(), options.clone());
        Box::pin(async move { az::share_link(&self.slot, WHAT, "adls_gen2", &r, &o).await })
    }
}
