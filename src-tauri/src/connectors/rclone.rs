//! Connector for anything rclone can reach (Google Cloud Storage, Box,
//! pCloud, Mega, Backblaze, Jottacloud… 70+ backends) through the rclone
//! binary and rclone.conf chosen by `rclone_engine::resolver` (Settings →
//! rclone, then `UFOP_RCLONE`, then `PATH`) and its configured remotes.
//!
//! A connection names a remote (`gcs:` from `rclone config`, or an
//! on-the-fly `:backend,opt=value:`). Listing uses `rclone lsjson`; files
//! move with `rclone copyto`; links come from `rclone link`.

use super::{Connector, ShareLink, ShareOptions, ShareScope};
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use crate::rclone_engine::Rclone;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::sync::RwLock;

const LIST_TIMEOUT: Duration = Duration::from_secs(120);
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(6 * 3600);

/// A configured rclone remote (`rclone listremotes --long`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RcloneRemote {
    pub name: String,
    pub backend: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct LsJsonEntry {
    path: String,
    name: String,
    #[serde(default)]
    size: i64,
    #[serde(default)]
    mod_time: Option<String>,
    #[serde(default)]
    is_dir: bool,
}

/// `gdrive` → `gdrive:`; `:s3,provider=AWS:` and `gdrive:bucket` stay.
pub fn normalize_remote(remote: &str) -> Result<String, AppError> {
    let r = remote.trim();
    if r.is_empty() {
        return Err(AppError::validation("Choose an rclone remote (see `rclone config`)"));
    }
    if r.starts_with('-') {
        return Err(AppError::validation("Invalid rclone remote name"));
    }
    Ok(if r.contains(':') { r.to_string() } else { format!("{r}:") })
}

/// `remote:` + `/a/b` → `remote:a/b` (rclone paths are relative to the remote root).
pub fn target(remote: &str, path: &str) -> String {
    let p = path.trim_start_matches('/');
    if remote.ends_with(':') {
        format!("{remote}{p}")
    } else if p.is_empty() {
        remote.to_string()
    } else {
        format!("{}/{p}", remote.trim_end_matches('/'))
    }
}

/// Run rclone with `args` (the resolved binary and config file); stdout on
/// success, the last stderr line otherwise.
async fn run(args: &[String], timeout: Duration) -> Result<String, AppError> {
    Rclone::current().run(args, timeout).await
}

/// Configured remotes, for the connection form.
pub async fn list_remotes() -> Result<Vec<RcloneRemote>, AppError> {
    crate::rclone_engine::config::list_remotes(&Rclone::current()).await
}

pub fn parse_listremotes(out: &str) -> Vec<RcloneRemote> {
    out.lines()
        .filter_map(|l| {
            let (name, backend) = l.split_once(':')?;
            Some(RcloneRemote { name: format!("{}:", name.trim()), backend: backend.trim().to_string() })
        })
        .filter(|r| r.name.len() > 1)
        .collect()
}

pub fn parse_lsjson(out: &str, dir: &str) -> Result<Vec<FileEntry>, AppError> {
    let items: Vec<LsJsonEntry> = serde_json::from_str(out).map_err(|e| AppError::connection(format!("Unexpected rclone listing: {e}"), "Update rclone."))?;
    let base = dir.trim_end_matches('/');
    Ok(items
        .into_iter()
        .map(|i| {
            let ext = (!i.is_dir).then(|| i.name.rsplit_once('.').map(|(_, e)| e.to_lowercase())).flatten();
            FileEntry {
                path: format!("{base}/{}", i.path),
                is_hidden: i.name.starts_with('.'),
                name: i.name,
                is_dir: i.is_dir,
                is_symlink: false,
                size: i.size.max(0) as u64,
                modified: i.mod_time.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&chrono::Utc)),
                created: None,
                extension: ext,
                permissions: None,
            }
        })
        .collect())
}

pub struct RcloneConnector {
    remote: RwLock<Option<String>>,
    /// rclone binary for `about` / `size` (tests); `None` = the resolved one.
    program: Option<String>,
}

/// `rclone about --json` → quota.
pub fn parse_about(out: &str) -> Result<super::RemoteQuota, AppError> {
    let v: serde_json::Value = serde_json::from_str(out).map_err(|e| AppError::connection(format!("Unexpected rclone about output: {e}"), "Update rclone."))?;
    Ok(super::RemoteQuota { total: super::json_u64(&v["total"]), used: super::json_u64(&v["used"]), free: super::json_u64(&v["free"]), trashed: super::json_u64(&v["trashed"]) })
}

/// `rclone size --json` → size (rclone does not count folders).
pub fn parse_size(out: &str) -> Result<super::FolderSize, AppError> {
    let v: serde_json::Value = serde_json::from_str(out).map_err(|e| AppError::connection(format!("Unexpected rclone size output: {e}"), "Update rclone."))?;
    Ok(super::FolderSize { bytes: super::json_u64(&v["bytes"]).unwrap_or(0), files: super::json_u64(&v["count"]).unwrap_or(0), folders: None, complete: true })
}

impl Default for RcloneConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl RcloneConnector {
    pub fn new() -> Self {
        Self { remote: RwLock::new(None), program: None }
    }

    /// Use `program` as the rclone binary for quota and size.
    pub fn with_program(program: &str) -> Self {
        Self { remote: RwLock::new(None), program: Some(program.to_string()) }
    }

    async fn run_mine(&self, args: &[String], timeout: Duration) -> Result<String, AppError> {
        let rc = Rclone::current();
        match &self.program {
            Some(p) => rc.with_program(p).run(args, timeout).await,
            None => rc.run(args, timeout).await,
        }
    }

    async fn remote(&self) -> Result<String, AppError> {
        self.remote.read().await.clone().ok_or_else(|| AppError::connection("Not connected", "Connect first."))
    }

    /// Connect straight to a remote (tests, quick connect).
    pub async fn connect_remote(&self, remote: &str) -> Result<(), AppError> {
        let r = normalize_remote(remote)?;
        // Cheap reachability check: the remote root, one level.
        run(&["lsjson".into(), "--max-depth".into(), "1".into(), "--no-mimetype".into(), r.clone()], LIST_TIMEOUT).await?;
        *self.remote.write().await = Some(r);
        Ok(())
    }
}

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

impl Connector for RcloneConnector {
    fn rename_entry(&self, from: &str, to: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::core::error::AppError>> + Send + '_>> {
        let (from, to) = (from.to_string(), to.to_string());
        Box::pin(async move { let r = self.remote().await?;
            run(&["moveto".into(), target(&r, &from), target(&r, &to)], TRANSFER_TIMEOUT).await.map(|_| ()) })
    }

    fn server_copy(&self, from: &str, to: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, crate::core::error::AppError>> + Send + '_>> {
        let (from, to) = (from.to_string(), to.to_string());
        Box::pin(async move { let r = self.remote().await?;
            run(&["copyto".into(), target(&r, &from), target(&r, &to)], TRANSFER_TIMEOUT).await.map(|_| true) })
    }


    fn connect(&self, profile: &ConnectionProfile) -> Fut<'_, ()> {
        let remote = profile.extras.rclone_remote.clone().filter(|r| !r.trim().is_empty()).unwrap_or_else(|| profile.host.clone());
        Box::pin(async move { self.connect_remote(&remote).await })
    }

    fn disconnect(&self) -> Fut<'_, ()> {
        Box::pin(async move {
            *self.remote.write().await = None;
            Ok(())
        })
    }

    fn list_remote(&self, path: &str) -> Fut<'_, Vec<FileEntry>> {
        let path = path.to_string();
        Box::pin(async move {
            let r = self.remote().await?;
            let out = run(&["lsjson".into(), "--no-mimetype".into(), target(&r, &path)], LIST_TIMEOUT).await?;
            parse_lsjson(&out, &path)
        })
    }

    fn is_connected(&self) -> bool {
        self.remote.try_read().map(|r| r.is_some()).unwrap_or(false)
    }

    fn upload(&self, local_path: &str, remote_path: &str) -> Fut<'_, u64> {
        let (l, rp) = (local_path.to_string(), remote_path.to_string());
        Box::pin(async move {
            let r = self.remote().await?;
            let size = std::fs::metadata(&l).map(|m| m.len()).map_err(|e| AppError::file_op(format!("Cannot read {l}: {e}"), "Check the file"))?;
            run(&["copyto".into(), l, target(&r, &rp)], TRANSFER_TIMEOUT).await?;
            Ok(size)
        })
    }

    fn download(&self, remote_path: &str, local_path: &str) -> Fut<'_, u64> {
        let (rp, l) = (remote_path.to_string(), local_path.to_string());
        Box::pin(async move {
            let r = self.remote().await?;
            run(&["copyto".into(), target(&r, &rp), l.clone()], TRANSFER_TIMEOUT).await?;
            Ok(std::fs::metadata(&l).map(|m| m.len()).unwrap_or(0))
        })
    }

    fn make_directory(&self, remote_path: &str) -> Fut<'_, ()> {
        let rp = remote_path.to_string();
        Box::pin(async move {
            let r = self.remote().await?;
            run(&["mkdir".into(), target(&r, &rp)], LIST_TIMEOUT).await.map(|_| ())
        })
    }

    fn remove(&self, remote_path: &str, is_dir: bool) -> Fut<'_, ()> {
        let rp = remote_path.to_string();
        Box::pin(async move {
            let r = self.remote().await?;
            let verb = if is_dir { "rmdir" } else { "deletefile" };
            run(&[verb.into(), target(&r, &rp)], LIST_TIMEOUT).await.map(|_| ())
        })
    }

    /// `rclone about --json` (backends without quotas fail: none reported).
    fn quota(&self) -> Fut<'_, Option<super::RemoteQuota>> {
        Box::pin(async move {
            let r = self.remote().await?;
            match self.run_mine(&["about".into(), "--json".into(), r], LIST_TIMEOUT).await {
                Ok(out) => parse_about(&out).map(Some),
                Err(e) if e.to_string().contains("doesn't support about") || e.to_string().contains("not supported") => Ok(None),
                Err(e) => Err(e),
            }
        })
    }

    /// `rclone size --json`: the backend counts, no listing walk.
    fn folder_size_fast(&self, remote_path: &str) -> Fut<'_, Option<super::FolderSize>> {
        let rp = remote_path.to_string();
        Box::pin(async move {
            let r = self.remote().await?;
            let out = self.run_mine(&["size".into(), "--json".into(), target(&r, &rp)], TRANSFER_TIMEOUT).await?;
            parse_size(&out).map(Some)
        })
    }

    fn create_share_link(&self, remote_path: &str, options: &ShareOptions) -> Fut<'_, ShareLink> {
        let (rp, o) = (remote_path.to_string(), options.clone());
        Box::pin(async move {
            if o.password.as_deref().is_some_and(|p| !p.is_empty()) || o.allow_edit || o.scope != ShareScope::Anyone {
                return Err(AppError::validation("rclone links are view-only links anyone can open"));
            }
            let r = self.remote().await?;
            let mut args = vec!["link".to_string(), target(&r, &rp)];
            if let Some(s) = o.expires_in_secs {
                args.push("--expire".into());
                args.push(format!("{s}s"));
            }
            let url = run(&args, LIST_TIMEOUT).await?.trim().to_string();
            Ok(ShareLink {
                url,
                expires_at: o.expires_in_secs.map(|s| chrono::Utc::now() + chrono::Duration::seconds(s as i64)),
                scope: ShareScope::Anyone,
                provider: "rclone".into(),
                note: Some("Some rclone backends ignore the expiry; check the provider if it matters.".into()),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_names_and_targets() {
        assert_eq!(normalize_remote("gdrive").unwrap(), "gdrive:");
        assert_eq!(normalize_remote(":s3,provider=AWS:").unwrap(), ":s3,provider=AWS:");
        assert!(normalize_remote(" ").is_err() && normalize_remote("--config=x").is_err());
        assert_eq!(target("gdrive:", "/a/b.txt"), "gdrive:a/b.txt");
        assert_eq!(target("gdrive:", "/"), "gdrive:");
        assert_eq!(target("gdrive:Team", "/a"), "gdrive:Team/a");
        assert_eq!(parse_listremotes("gcs:    google cloud storage\nbox:      box\n"), vec![
            RcloneRemote { name: "gcs:".into(), backend: "google cloud storage".into() },
            RcloneRemote { name: "box:".into(), backend: "box".into() },
        ]);
    }

    #[test]
    fn parses_lsjson() {
        let out = r#"[{"Path":"docs","Name":"docs","Size":-1,"ModTime":"2026-01-02T03:04:05Z","IsDir":true},{"Path":"a.TXT","Name":"a.TXT","Size":12,"ModTime":"2026-01-02T03:04:05.123456789Z","IsDir":false}]"#;
        let e = parse_lsjson(out, "/in/").unwrap();
        assert_eq!((e[0].path.as_str(), e[0].is_dir, e[0].size), ("/in/docs", true, 0));
        assert_eq!((e[1].size, e[1].extension.as_deref()), (12, Some("txt")));
        assert!(e[1].modified.is_some());
    }

    /// About and size through a stand-in rclone that prints canned JSON.
    #[cfg(unix)]
    #[tokio::test]
    async fn quota_and_size_use_rclone_about_and_size() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake = dir.path().join("rclone");
        std::fs::write(
            &fake,
            "#!/bin/sh\ncase \"$1\" in\n  lsjson) echo '[]' ;;\n  about) echo '{\"total\":1000,\"used\":250,\"free\":750,\"trashed\":5}' ;;\n  size) echo \"{\\\"count\\\":3,\\\"bytes\\\":42,\\\"sizeless\\\":0,\\\"target\\\":\\\"$3\\\"}\" ;;\nesac\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let c = RcloneConnector::with_program(fake.to_str().unwrap());
        *c.remote.write().await = Some("gdrive:".into());
        assert_eq!(c.quota().await.unwrap(), Some(crate::connectors::RemoteQuota { total: Some(1000), used: Some(250), free: Some(750), trashed: Some(5) }));
        let s = c.folder_size_fast("/Photos").await.unwrap().unwrap();
        assert_eq!((s.bytes, s.files, s.folders, s.complete), (42, 3, None, true));
        assert_eq!(parse_size(r#"{"count":1,"bytes":"9"}"#).unwrap().bytes, 9);
        assert!(parse_about("not json").is_err());
    }

    fn rclone_available() -> bool {
        std::process::Command::new(Rclone::current().program).arg("version").output().is_ok_and(|o| o.status.success())
    }

    /// Real rclone against its local backend (`:local:`).
    #[tokio::test]
    async fn round_trip_with_real_rclone() {
        if !rclone_available() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let remote = format!(":local:{}", root.path().display());
        let c = RcloneConnector::new();
        assert!(!c.is_connected());
        c.connect_remote(&remote).await.unwrap();
        assert!(c.is_connected());
        c.make_directory("/docs").await.unwrap();
        let src = tempfile::NamedTempFile::new().unwrap();
        std::fs::write(src.path(), "hello rclone").unwrap();
        assert_eq!(c.upload(src.path().to_str().unwrap(), "/docs/hello.txt").await.unwrap(), 12);
        let list = c.list_remote("/docs").await.unwrap();
        assert_eq!(list.iter().map(|e| (e.name.as_str(), e.size)).collect::<Vec<_>>(), vec![("hello.txt", 12)]);
        assert_eq!(list[0].path, "/docs/hello.txt");
        let out = root.path().join("copy.txt");
        c.download("/docs/hello.txt", out.to_str().unwrap()).await.unwrap();
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "hello rclone");
        c.remove("/docs/hello.txt", false).await.unwrap();
        c.remove("/docs", true).await.unwrap();
        assert!(!root.path().join("docs").exists());
        let err = c.list_remote("/missing").await.unwrap_err().to_string();
        assert!(err.contains("rclone"), "{err}");
        assert!(RcloneConnector::new().connect_remote("nosuchremote").await.is_err());
        c.disconnect().await.unwrap();
        assert!(!c.is_connected());
    }
}
