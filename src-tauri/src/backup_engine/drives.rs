//! "Open as drive": a backup version (or every version) served as a
//! read-only WebDAV drive on 127.0.0.1 with a random token, and mounted with
//! the OS's own WebDAV client when one is available. The server is
//! `ufop_core::backup::davserve`; mounting is `mount_engine::webdav_mount`.
//!
//! Drives close when asked, when their job is deleted, when the app exits,
//! and after their idle timeout (no request for that many minutes).

use super::access::selector_of;
use super::{blocking, lock, BackupManager};
use crate::core::error::AppError;
use crate::mount_engine::webdav_mount::{self, OsMount};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use ufop_core::backup::access::BlockSource;
use ufop_core::backup::davserve::{self, DavContent, DavServer, DavVersion};
use ufop_core::backup::select::VersionSelector;

/// What to open.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DriveRequest {
    /// Version number (0 = newest) when `selector` is not given.
    pub version: Option<usize>,
    pub selector: Option<VersionSelector>,
    /// Every version as a folder (`<date time>/…`) instead of one version.
    pub all_versions: bool,
    /// Shared repositories: another source, or `*`.
    pub source: Option<String>,
    /// Mount it with the OS's WebDAV client (default); off = URL only.
    pub mount: Option<bool>,
    /// Local port (0 or none = a free one).
    pub port: Option<u16>,
    /// Close after this many minutes without requests (0 or none = never).
    pub idle_timeout_minutes: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveInfo {
    pub id: String,
    pub job_id: String,
    pub job_name: String,
    /// `http://127.0.0.1:<port>/<token>/`.
    pub url: String,
    /// For clients that ask for credentials: any user name, the token as password.
    pub username: String,
    pub password: String,
    /// The version (single-version drives).
    pub version: Option<usize>,
    pub timestamp: Option<String>,
    pub all_versions: bool,
    /// Folder or drive letter where the OS mounted it.
    pub mount_point: Option<String>,
    pub mounted_with: Option<String>,
    /// Why mounting did not work (the URL still does).
    pub mount_error: Option<String>,
    pub started_at: String,
    pub idle_timeout_minutes: u32,
}

pub(super) struct Drive {
    info: DriveInfo,
    server: DavServer,
    os: Option<OsMount>,
}

impl BackupManager {
    /// Serve a version as a read-only drive and mount it.
    pub async fn open_drive(&self, job_id: &str, req: DriveRequest) -> Result<DriveInfo, AppError> {
        let setup = self.read_setup(job_id).await?;
        let job_name = setup.job.name.clone();
        let selector = selector_of(req.version, req.selector.clone());
        let (all, source) = (req.all_versions, req.source.clone());
        let port = req.port.unwrap_or(0);
        let (server, version, timestamp) = blocking(move || {
            let (content, version, timestamp) = setup.run(source.as_deref(), "mount", |repo, db| {
                let sets = db.filesets()?;
                let (versions, version, timestamp) = if all {
                    (sets.iter().map(|f| DavVersion { fileset_id: f.id, timestamp: f.timestamp.clone() }).collect::<Vec<_>>(), None, None)
                } else {
                    let i = selector.resolve(&sets)?;
                    (vec![DavVersion { fileset_id: sets[i].id, timestamp: sets[i].timestamp.clone() }], Some(i), Some(sets[i].timestamp.clone()))
                };
                let src = BlockSource::shared(repo, setup.backend.clone())?;
                let db2 = Mutex::new(setup.open_db()?);
                let loader = Box::new(move |id: i64| lock(&db2).files(id));
                Ok((DavContent::new(src, versions, !all, loader)?, version, timestamp))
            })?;
            let addr = SocketAddr::from(([127, 0, 0, 1], port));
            Ok((davserve::serve(Arc::new(content), addr, davserve::new_token())?, version, timestamp))
        })
        .await?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut info = DriveInfo {
            id: id.clone(),
            job_id: job_id.to_string(),
            job_name,
            url: server.url(),
            username: "ufop".into(),
            password: server.token().to_string(),
            version,
            timestamp,
            all_versions: all,
            mount_point: None,
            mounted_with: None,
            mount_error: None,
            started_at: Utc::now().to_rfc3339(),
            idle_timeout_minutes: req.idle_timeout_minutes.unwrap_or(0),
        };
        let mut os = None;
        if req.mount.unwrap_or(true) {
            let folder = self.inner.data_dir.join("drives").join(&id);
            match webdav_mount::mount(&info.url, &folder).await {
                Ok(m) => {
                    info.mount_point = Some(m.mount_point.clone());
                    info.mounted_with = Some(m.tool.clone());
                    os = Some(m);
                }
                Err(e) => info.mount_error = Some(e),
            }
        }
        lock(&self.inner.drives).push(Drive { info: info.clone(), server, os });
        Ok(info)
    }

    pub fn list_drives(&self) -> Vec<DriveInfo> {
        lock(&self.inner.drives).iter().map(|d| d.info.clone()).collect()
    }

    /// Unmount and stop one drive; false when there is no such drive.
    pub async fn close_drive(&self, id: &str) -> Result<bool, AppError> {
        let d = {
            let mut g = lock(&self.inner.drives);
            g.iter().position(|d| d.info.id == id).map(|i| g.remove(i))
        };
        let Some(d) = d else { return Ok(false) };
        close(d).await;
        Ok(true)
    }

    /// Close every drive of a job (job deleted).
    pub async fn close_drives_of(&self, job_id: &str) {
        let gone: Vec<Drive> = {
            let mut g = lock(&self.inner.drives);
            let (gone, keep) = std::mem::take(&mut *g).into_iter().partition(|d| d.info.job_id == job_id);
            *g = keep;
            gone
        };
        for d in gone {
            close(d).await;
        }
    }

    /// Close drives idle for longer than their timeout; returns how many.
    pub async fn close_idle_drives(&self) -> usize {
        let now = Utc::now().timestamp();
        let gone: Vec<Drive> = {
            let mut g = lock(&self.inner.drives);
            let (gone, keep) = std::mem::take(&mut *g)
                .into_iter()
                .partition(|d| d.info.idle_timeout_minutes > 0 && now - d.server.last_access() > d.info.idle_timeout_minutes as i64 * 60);
            *g = keep;
            gone
        };
        let n = gone.len();
        for d in gone {
            tracing::info!("Closing idle backup drive {} ({})", d.info.url, d.info.job_name);
            close(d).await;
        }
        n
    }

    /// Unmount and stop every drive without a runtime (app exit).
    pub fn close_all_drives_blocking(&self) {
        let all = std::mem::take(&mut *lock(&self.inner.drives));
        for d in all {
            if let Some(m) = &d.os {
                webdav_mount::unmount_blocking(m);
            }
            d.server.stop();
        }
    }
}

async fn close(d: Drive) {
    if let Some(m) = &d.os {
        webdav_mount::unmount(m).await;
    }
    let _ = tokio::task::spawn_blocking(move || d.server.stop()).await;
}
