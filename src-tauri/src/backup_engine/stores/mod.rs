//! Native cloud destinations for backups.
//!
//! Each provider implements the async [`ObjectStore`] (flat list / put /
//! get / delete of named volumes in one folder). [`StoreBackend`] adapts a
//! store to the engine's synchronous [`Backend`], blocking on the Tokio
//! runtime with an optional per-operation timeout.
//!
//! Provider settings live in [`CloudConfig`]; the secret (access key,
//! account key, refresh token, password, …) is kept in the credential
//! store and handed to [`open`] separately.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;
use tokio::runtime::Handle;
use ufop_core::backup::backend::{LockMode, Quota};
use ufop_core::backup::{Backend, RemoteFile};

pub mod azure;
pub mod b2;
pub mod gcs;
pub mod graph;
pub mod http;
pub mod oauth_drive;
pub mod rclone;
pub mod s3;
pub mod smb;
pub mod swift;
pub mod tahoe;
#[cfg(test)]
mod mocks;
#[cfg(test)]
mod tests;

pub type BoxFut<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

/// A flat folder of named objects.
pub trait ObjectStore: Send + Sync {
    fn describe(&self) -> String;
    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>>;
    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()>;
    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>>;
    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()>;
    /// Create the folder / bucket path when missing.
    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn supports_lock(&self) -> bool {
        false
    }
    fn lock<'a>(&'a self, name: &'a str, until: chrono::DateTime<chrono::Utc>, mode: LockMode) -> BoxFut<'a, ()> {
        let _ = (name, until, mode);
        Box::pin(async move { Err(AppError::validation(format!("{} does not support object locks", self.describe()))) })
    }
    fn quota(&self) -> BoxFut<'_, Option<Quota>> {
        Box::pin(async { Ok(None) })
    }
}

/// Synchronous engine backend over an [`ObjectStore`].
pub struct StoreBackend {
    store: Box<dyn ObjectStore>,
    rt: Handle,
    timeout: Option<Duration>,
}

impl StoreBackend {
    /// Must be created inside a Tokio runtime; used from blocking threads.
    pub fn new(store: Box<dyn ObjectStore>, timeout: Option<Duration>) -> Self {
        Self { store, rt: Handle::current(), timeout }
    }

    fn wait<T>(&self, what: &str, f: BoxFut<'_, T>) -> Result<T, AppError> {
        match self.timeout {
            None => self.rt.block_on(f),
            Some(t) => self.rt.block_on(async { tokio::time::timeout(t, f).await }).unwrap_or_else(|_| {
                Err(AppError::connection(
                    format!("{what} on {} timed out after {}s", self.store.describe(), t.as_secs()),
                    "Check the connection or raise the job's network timeout.",
                ))
            }),
        }
    }
}

impl Backend for StoreBackend {
    fn describe(&self) -> String {
        self.store.describe()
    }
    fn list(&self) -> Result<Vec<RemoteFile>, AppError> {
        let mut v = self.wait("Listing", self.store.list())?;
        v.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(v)
    }
    fn put(&self, name: &str, data: &[u8]) -> Result<(), AppError> {
        self.wait("Upload", self.store.put(name, data.to_vec()))
    }
    fn get(&self, name: &str) -> Result<Vec<u8>, AppError> {
        self.wait("Download", self.store.get(name))
    }
    fn delete(&self, name: &str) -> Result<(), AppError> {
        self.wait("Delete", self.store.delete(name))
    }
    fn create_folder(&self) -> Result<(), AppError> {
        self.wait("Creating the folder", self.store.create_folder())
    }
    fn supports_lock(&self) -> bool {
        self.store.supports_lock()
    }
    fn lock(&self, name: &str, until: chrono::DateTime<chrono::Utc>, mode: LockMode) -> Result<(), AppError> {
        self.wait("Locking", self.store.lock(name, until, mode))
    }
    fn quota(&self) -> Option<Quota> {
        self.wait("Quota", self.store.quota()).ok().flatten()
    }
}

/// Provider settings (secrets excluded). `type` names the provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum CloudConfig {
    /// AWS S3 and S3-compatible services (IDrive e2, Storj gateway,
    /// Aliyun OSS, Tencent COS, Wasabi, MinIO, …).
    S3(s3::S3Config),
    Azure(azure::AzureConfig),
    Gcs(gcs::GcsConfig),
    B2(b2::B2Config),
    Swift(swift::SwiftConfig),
    Dropbox(oauth_drive::DriveConfig),
    GoogleDrive(oauth_drive::DriveConfig),
    Box(oauth_drive::DriveConfig),
    Pcloud(oauth_drive::DriveConfig),
    /// OneDrive (personal / business), a SharePoint site library or a
    /// Microsoft 365 group's files, through Microsoft Graph.
    OneDrive(graph::GraphConfig),
    TahoeLafs(tahoe::TahoeConfig),
    Rclone(rclone::RcloneConfig),
    Smb(smb::SmbConfig),
}

impl CloudConfig {
    pub fn provider(&self) -> &'static str {
        match self {
            Self::S3(_) => "s3",
            Self::Azure(_) => "azure",
            Self::Gcs(_) => "gcs",
            Self::B2(_) => "b2",
            Self::Swift(_) => "swift",
            Self::Dropbox(_) => "dropbox",
            Self::GoogleDrive(_) => "google_drive",
            Self::Box(_) => "box",
            Self::Pcloud(_) => "pcloud",
            Self::OneDrive(_) => "onedrive",
            Self::TahoeLafs(_) => "tahoe_lafs",
            Self::Rclone(_) => "rclone",
            Self::Smb(_) => "smb",
        }
    }

    /// The service's network endpoint `(host, port, tls)` for diagnostics;
    /// `None` for rclone (the remote is opaque).
    pub fn network_endpoint(&self) -> Option<(String, u16, bool)> {
        let or = |v: &str, d: String| if v.trim().is_empty() { d } else { v.trim().to_string() };
        let url = match self {
            Self::S3(c) => or(&c.endpoint, s3::preset_endpoint(&c.preset, &c.region).or_else(|| s3::preset_endpoint("aws", &c.region)).unwrap_or_default()),
            Self::Azure(c) => or(&c.endpoint, format!("https://{}.blob.core.windows.net", c.account)),
            Self::Gcs(c) => or(&c.endpoint, "https://storage.googleapis.com".into()),
            Self::B2(c) => or(&c.api_url, "https://api.backblazeb2.com".into()),
            Self::Swift(c) => c.auth_url.clone(),
            Self::Dropbox(c) => or(&c.api_base, "https://api.dropboxapi.com".into()),
            Self::GoogleDrive(c) => or(&c.api_base, "https://www.googleapis.com".into()),
            Self::Box(c) => or(&c.api_base, "https://api.box.com".into()),
            Self::Pcloud(c) => or(&c.api_base, if c.region == "eu" { "https://eapi.pcloud.com".into() } else { "https://api.pcloud.com".into() }),
            Self::OneDrive(c) => or(&c.api_base, "https://graph.microsoft.com".into()),
            Self::TahoeLafs(c) => c.node_url.clone(),
            Self::Rclone(_) => return None,
            Self::Smb(c) => return Some((c.server.clone(), c.port.unwrap_or(445), false)),
        };
        let u = url::Url::parse(&url).ok()?;
        Some((u.host_str()?.to_string(), u.port_or_known_default()?, u.scheme() == "https"))
    }

    pub fn describe(&self) -> String {
        match self {
            Self::S3(c) => format!("s3://{}/{}", c.bucket, c.prefix),
            Self::Azure(c) => format!("azure://{}/{}/{}", c.account, c.container, c.prefix),
            Self::Gcs(c) => format!("gs://{}/{}", c.bucket, c.prefix),
            Self::B2(c) => format!("b2://{}/{}", c.bucket, c.prefix),
            Self::Swift(c) => format!("swift://{}/{}", c.container, c.prefix),
            Self::Dropbox(c) | Self::GoogleDrive(c) | Self::Box(c) | Self::Pcloud(c) => format!("{}://{}", self.provider(), c.folder),
            Self::OneDrive(c) => format!("onedrive://{}", c.folder),
            Self::TahoeLafs(c) => format!("tahoe://{}", c.dir_cap.chars().take(24).collect::<String>()),
            Self::Rclone(c) => format!("rclone://{}:{}", c.remote, c.path),
            Self::Smb(c) => format!("smb://{}/{}/{}", c.server, c.share, c.path),
        }
    }
}

/// A provider as shown in the destination picker.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// What the secret field holds.
    pub secret: &'static str,
    /// How this build verified the provider: `emulator`, `real-server`,
    /// `api-mock` or `cli` — `api-mock` providers are not yet verified
    /// against the live service and are shown as such.
    pub verified: &'static str,
    /// Hidden unless "show untested providers" is on.
    pub hidden: bool,
}

pub const PROVIDERS: &[ProviderInfo] = &[
    ProviderInfo { id: "s3", name: "Amazon S3 / S3-compatible (IDrive e2, Storj, Aliyun OSS, Tencent COS, Wasabi, MinIO)", secret: "Secret access key (optionally + newline + session token)", verified: "emulator", hidden: false },
    ProviderInfo { id: "azure", name: "Azure Blob Storage", secret: "Account key (base64) or SAS token", verified: "emulator", hidden: false },
    ProviderInfo { id: "gcs", name: "Google Cloud Storage", secret: "Service-account key JSON or OAuth access token", verified: "emulator", hidden: false },
    ProviderInfo { id: "smb", name: "SMB / Windows share (native, no mount)", secret: "Password", verified: "real-server", hidden: false },
    ProviderInfo { id: "tahoe_lafs", name: "Tahoe-LAFS", secret: "(none — the directory capability is the credential)", verified: "real-server", hidden: false },
    ProviderInfo { id: "rclone", name: "Rclone (MEGA, Jottacloud, Koofr, … any rclone remote)", secret: "rclone config password (optional)", verified: "cli", hidden: false },
    ProviderInfo { id: "b2", name: "Backblaze B2", secret: "Application key", verified: "api-mock", hidden: false },
    ProviderInfo { id: "swift", name: "OpenStack Swift", secret: "Password", verified: "api-mock", hidden: false },
    ProviderInfo { id: "dropbox", name: "Dropbox", secret: "OAuth token JSON (access/refresh token, client id)", verified: "api-mock", hidden: false },
    ProviderInfo { id: "google_drive", name: "Google Drive", secret: "OAuth token JSON", verified: "api-mock", hidden: false },
    ProviderInfo { id: "onedrive", name: "OneDrive / SharePoint / Microsoft 365 group", secret: "OAuth token JSON", verified: "api-mock", hidden: false },
    ProviderInfo { id: "box", name: "Box", secret: "OAuth token JSON", verified: "api-mock", hidden: true },
    ProviderInfo { id: "pcloud", name: "pCloud", secret: "OAuth access token", verified: "api-mock", hidden: true },
];

/// Open the store for a configuration with its secret.
pub async fn open(config: &CloudConfig, secret: Option<String>) -> Result<Box<dyn ObjectStore>, AppError> {
    let need = |s: Option<String>| s.filter(|s| !s.is_empty()).ok_or_else(|| AppError::validation("This destination's credentials are missing; edit the destination to enter them"));
    Ok(match config {
        CloudConfig::S3(c) => Box::new(s3::S3Store::new(c.clone(), need(secret)?)?),
        CloudConfig::Azure(c) => Box::new(azure::AzureStore::new(c.clone(), need(secret)?)?),
        CloudConfig::Gcs(c) => Box::new(gcs::GcsStore::new(c.clone(), need(secret)?)?),
        CloudConfig::B2(c) => Box::new(b2::B2Store::connect(c.clone(), need(secret)?).await?),
        CloudConfig::Swift(c) => Box::new(swift::SwiftStore::connect(c.clone(), need(secret)?).await?),
        CloudConfig::Dropbox(c) => Box::new(oauth_drive::DriveStore::new(oauth_drive::Kind::Dropbox, c.clone(), need(secret)?)?),
        CloudConfig::GoogleDrive(c) => Box::new(oauth_drive::DriveStore::new(oauth_drive::Kind::GoogleDrive, c.clone(), need(secret)?)?),
        CloudConfig::Box(c) => Box::new(oauth_drive::DriveStore::new(oauth_drive::Kind::Box, c.clone(), need(secret)?)?),
        CloudConfig::Pcloud(c) => Box::new(oauth_drive::DriveStore::new(oauth_drive::Kind::Pcloud, c.clone(), need(secret)?)?),
        CloudConfig::OneDrive(c) => Box::new(graph::GraphStore::new(c.clone(), need(secret)?)?),
        CloudConfig::TahoeLafs(c) => Box::new(tahoe::TahoeStore::new(c.clone())?),
        CloudConfig::Rclone(c) => Box::new(rclone::RcloneStore::new(c.clone(), secret)),
        CloudConfig::Smb(c) => Box::new(smb::SmbStore::new(c.clone(), secret)),
    })
}

/// Join a folder prefix and an object name (`prefix/name`, no leading `/`).
pub fn join_key(prefix: &str, name: &str) -> String {
    let p = prefix.trim_matches('/');
    if p.is_empty() {
        name.to_string()
    } else {
        format!("{p}/{name}")
    }
}

/// Last path segment of a key (objects in sub-folders are ignored by stores).
pub fn direct_child<'a>(prefix: &str, key: &'a str) -> Option<&'a str> {
    let p = prefix.trim_matches('/');
    let rest = if p.is_empty() { key } else { key.strip_prefix(p)?.strip_prefix('/')? };
    (!rest.is_empty() && !rest.contains('/')).then_some(rest)
}
