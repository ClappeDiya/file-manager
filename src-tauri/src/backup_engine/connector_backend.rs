//! Bridges the synchronous backup [`Backend`] trait to the async protocol
//! connectors (SFTP, FTP/FTPS, WebDAV, S3-compatible). Volumes are staged
//! through temp files because connectors transfer file paths.
//!
//! Each backup run gets its own connector instance, so a running job never
//! disturbs (or is disturbed by) the user's interactive session on the same
//! protocol.

use crate::connectors::{Connector, FtpConnector, S3Connector, SftpConnector, WebDavConnector};
use crate::core::error::AppError;
use crate::core::types::ConnectionProfile;
use std::sync::Arc;
use tokio::runtime::Handle;
use ufop_core::backup::{Backend, RemoteFile};

/// Protocols whose connectors implement upload/download/list/remove.
pub const BACKUP_PROTOCOLS: &[&str] = &["sftp", "ftp", "ftps", "webdav", "s3"];

/// Create a fresh connector for a backup run.
pub fn new_connector(protocol: &str) -> Option<Arc<dyn Connector>> {
    Some(match protocol {
        "sftp" => Arc::new(SftpConnector::new()),
        "ftp" | "ftps" => Arc::new(FtpConnector::new()),
        "webdav" => Arc::new(WebDavConnector::new()),
        "s3" => Arc::new(S3Connector::new()),
        _ => return None,
    })
}

pub struct ConnectorBackend {
    connector: Arc<dyn Connector>,
    dir: String,
    label: String,
    rt: Handle,
    timeout: Option<std::time::Duration>,
}

impl ConnectorBackend {
    /// Connect with the profile's stored secret. Must be called from within
    /// a Tokio runtime; the returned backend is used from a blocking thread.
    pub async fn connect(profile: &ConnectionProfile, secret: Option<String>, remote_dir: &str) -> Result<Self, AppError> {
        let protocol = profile.protocol.as_str();
        let connector = new_connector(protocol).ok_or_else(|| {
            AppError::connection(
                format!("Backups to {protocol} connections are not supported yet"),
                format!("Use one of: {}, or a local / mounted folder.", BACKUP_PROTOCOLS.join(", ")),
            )
        })?;
        connector.connect_with_secret(profile, secret).await?;
        let dir = if remote_dir.trim().is_empty() { profile.remote_path.clone() } else { remote_dir.to_string() };
        Ok(Self {
            connector,
            label: format!("{protocol}://{}{}", profile.host, dir),
            dir: dir.trim_end_matches('/').to_string(),
            rt: Handle::current(),
            timeout: None,
        })
    }

    /// Fail an operation that takes longer than `t` (network timeout).
    pub fn set_timeout(&mut self, t: Option<std::time::Duration>) {
        self.timeout = t;
    }

    fn wait<T>(&self, what: &str, f: impl std::future::Future<Output = Result<T, AppError>>) -> Result<T, AppError> {
        match self.timeout {
            None => self.rt.block_on(f),
            Some(t) => self.rt.block_on(async { tokio::time::timeout(t, f).await }).unwrap_or_else(|_| {
                Err(AppError::connection(
                    format!("{what} on {} timed out after {}s", self.label, t.as_secs()),
                    "Check the connection or raise the job's network timeout.",
                ))
            }),
        }
    }

    fn remote(&self, name: &str) -> String {
        format!("{}/{name}", self.dir)
    }

    fn staging(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("ufop-vol-{}-{name}", uuid::Uuid::new_v4().simple()))
    }
}

impl Drop for ConnectorBackend {
    fn drop(&mut self) {
        let c = self.connector.clone();
        self.rt.spawn(async move {
            let _ = c.disconnect().await;
        });
    }
}

impl Backend for ConnectorBackend {
    fn describe(&self) -> String {
        self.label.clone()
    }

    fn list(&self) -> Result<Vec<RemoteFile>, AppError> {
        let dir = if self.dir.is_empty() { "/".to_string() } else { self.dir.clone() };
        let entries = self.wait("Listing", self.connector.list_remote(&dir))?;
        Ok(entries.into_iter().filter(|e| !e.is_dir).map(|e| RemoteFile { name: e.name, size: e.size }).collect())
    }

    fn put(&self, name: &str, data: &[u8]) -> Result<(), AppError> {
        let tmp = Self::staging(name);
        std::fs::write(&tmp, data)?;
        let r = self.wait("Upload", self.connector.upload(&tmp.to_string_lossy(), &self.remote(name)));
        let _ = std::fs::remove_file(&tmp);
        r.map(|_| ())
    }

    fn get(&self, name: &str) -> Result<Vec<u8>, AppError> {
        let tmp = Self::staging(name);
        let r = self.wait("Download", self.connector.download(&self.remote(name), &tmp.to_string_lossy()));
        let data = r.and_then(|_| Ok(std::fs::read(&tmp)?));
        let _ = std::fs::remove_file(&tmp);
        data
    }

    fn delete(&self, name: &str) -> Result<(), AppError> {
        self.wait("Delete", self.connector.remove(&self.remote(name), false))
    }

    fn create_folder(&self) -> Result<(), AppError> {
        if self.list().is_ok() {
            return Ok(());
        }
        self.wait("Creating the folder", self.connector.make_directory(&self.dir))
    }
}
