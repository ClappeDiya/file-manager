//! Protocol and cloud connectors - SFTP, FTP, WebDAV, SMB, NFS, S3, GDrive, etc.
//!
//! Each connector implements the `Connector` trait for pluggable remote access.
//! Connection manager provides save, test, import/export, and credential management.
//!
//! Implemented connectors:
//! - SFTP (T-020): SSH-based file transfer with key auth, jump hosts, host key verification
//! - FTP/FTPS (T-021): Classic FTP with active/passive mode, TLS support
//! - WebDAV (T-022): HTTP-based DAV with locking, chunked uploads, Nextcloud/ownCloud compat
//! - SMB/CIFS (T-023): Windows shares, NAS devices, LAN discovery
//! - NFS (T-024): NFSv3/v4 mounts with Unix permission display
//! - Local Drive (T-025): Drive-to-drive transfers with preflight checks
//! - S3 (T-026): Amazon S3 and S3-compatible (MinIO, Wasabi, B2 S3 mode)
//! - Google Drive (T-027): OAuth 2.0, My Drive/Shared Drives, Docs export, versioning
//! - Dropbox (T-028): OAuth 2.0, team/shared folders, session uploads
//! - OneDrive (T-029): Personal/Business OAuth, SharePoint, naming restrictions
//! - Backblaze B2 (T-030): Native API, large file uploads, S3 compatibility
//! - Peer (T-031): mDNS/Bonjour peer discovery, TLS encrypted peer-to-peer transfer
//! - Server-to-Server (T-033): Direct transfer between remote endpoints
//! - Azure Blob Storage, Data Lake Storage Gen2 and Files: Shared Key, SAS or
//!   connection string (`azure_rest`), file ops, read-only SAS links
//! - OpenStack Swift: Keystone v3/TempAuth, container/object ops, SLO/DLO, TempURL

pub mod folder_size;
pub mod walk;
pub mod presets;
pub mod rclone;
pub mod connection_manager;
pub mod known_hosts;
pub mod ftp;
pub mod import_profiles;
pub mod local_drive;
pub mod nfs;
pub mod peer;
pub mod peer_wire;
pub mod server_to_server;
pub mod sftp;
pub mod smb;
pub mod webdav;
// T-026..T-030: Cloud connectors
pub mod b2;
pub mod dropbox;
pub mod google_drive;
pub mod google_errors;
pub mod oauth;
pub mod oauth_flow;
pub mod onedrive;
pub mod s3;
// AWS credential chain, SSO, and STS federation
pub mod aws_chain;
pub mod aws_credentials;
// Azure Blob Storage and OpenStack Swift connectors
pub mod azure_blob;
pub mod openstack_swift;
pub mod azure_rest;
pub mod azure_ad;
pub mod azure_adls;
pub mod azure_files;
#[cfg(test)]
pub(crate) mod azure_tests;
#[cfg(test)]
pub(crate) mod mock_http;
#[cfg(test)]
mod oauth_tests;

use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, ConnectionProtocol, FileEntry};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

pub use connection_manager::ConnectionManager;
pub use ftp::FtpConnector;
pub use local_drive::{DriveInfo, DriveType, FilesystemType, LocalDriveConnector, PreflightResult};
pub use nfs::NfsConnector;
pub use sftp::SftpConnector;
pub use smb::SmbConnector;
pub use webdav::WebDavConnector;
// Cloud connectors (T-026..T-030)
pub use b2::B2Connector;
pub use dropbox::DropboxConnector;
pub use google_drive::GoogleDriveConnector;
pub use onedrive::OneDriveConnector;
pub use s3::S3Connector;
// Azure Blob Storage and OpenStack Swift
pub use azure_blob::AzureBlobConnector;
pub use azure_adls::AdlsGen2Connector;
pub use azure_files::AzureFilesConnector;
pub use openstack_swift::SwiftConnector;
// T-031, T-033: Peer and server-to-server
pub use peer::PeerManager;
pub use server_to_server::ServerTransferManager;

/// Who a share link opens to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareScope {
    /// Anyone with the link.
    #[default]
    Anyone,
    /// Only people in the account's organization (OneDrive / Dropbox teams).
    Organization,
}

/// What a share link should allow.
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct ShareOptions {
    /// Link lifetime; `None` = the provider's default (or no expiry).
    #[serde(default)]
    pub expires_in_secs: Option<u64>,
    /// Password the recipient must enter (where the provider supports it).
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub scope: ShareScope,
    /// Recipients may edit, not only view.
    #[serde(default)]
    pub allow_edit: bool,
}

/// A created share link.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct ShareLink {
    pub url: String,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub scope: ShareScope,
    pub provider: String,
    /// Caveats worth showing (e.g. "an existing link was reused").
    pub note: Option<String>,
}

/// Everything secret a saved connection keeps in the credential store.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct ConnectionSecrets {
    /// Password, access secret, private-key path, bearer token or OAuth
    /// grant, depending on the protocol.
    pub secret: Option<String>,
    /// SFTP: passphrase of the private key.
    pub key_passphrase: Option<String>,
}

impl ConnectionSecrets {
    pub fn new(secret: Option<String>) -> Self {
        Self { secret, key_passphrase: None }
    }
}

impl std::fmt::Debug for ConnectionSecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionSecrets")
            .field("secret", &self.secret.as_ref().map(|_| "<redacted>"))
            .field("key_passphrase", &self.key_passphrase.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// A byte count that providers send as a number or a decimal string.
pub(crate) fn json_u64(v: &serde_json::Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.parse().ok()))
}

pub(crate) fn share_unsupported(what: &str) -> AppError {
    AppError::validation(format!("This provider can't {what} on share links"))
}

/// Storage space of a remote account. Every figure is optional: providers
/// report different subsets (an unlimited plan has no total).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RemoteQuota {
    pub total: Option<u64>,
    pub used: Option<u64>,
    pub free: Option<u64>,
    /// In the provider's trash / recycle bin.
    pub trashed: Option<u64>,
}

/// Size of a remote folder and everything below it.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct FolderSize {
    pub bytes: u64,
    pub files: u64,
    /// `None` when the server-side figure does not count folders (rclone).
    pub folders: Option<u64>,
    /// `false` when the walk was cancelled (the figures are partial).
    pub complete: bool,
}

/// Trait for remote filesystem connectors.
pub trait Connector: Send + Sync {
    fn connect(&self, profile: &ConnectionProfile) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>>;
    fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>>;
    fn list_remote(
        &self,
        path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<FileEntry>, AppError>> + Send + '_>>;
    fn is_connected(&self) -> bool;

    /// The saved connection this instance is signed in as, for connectors
    /// whose one shared instance can serve several accounts (Google Drive,
    /// Dropbox, OneDrive, B2). `None`: not tracked (the default) or not
    /// connected.
    fn connected_profile_id(&self) -> Option<uuid::Uuid> {
        None
    }

    /// A new, disconnected instance configured like this one (endpoints,
    /// credential store), for connectors that track
    /// [`Connector::connected_profile_id`]. An action for an account other
    /// than the one this shared instance is signed in as runs on it, so the
    /// instance the user is browsing is never re-signed behind their back.
    /// `None` (the default): not tracked.
    fn detached(&self) -> Option<Arc<dyn Connector>> {
        None
    }

    /// Connect using a secret held in the credential store (password, access
    /// secret, or private-key path, depending on the protocol). Connectors
    /// that need no secret fall back to [`Connector::connect`].
    fn connect_with_secret(
        &self,
        profile: &ConnectionProfile,
        _secret: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        self.connect(profile)
    }

    /// Connect with every stored secret of the connection. Only connectors
    /// with more than one secret (SFTP key + passphrase) override this.
    fn connect_with_secrets(
        &self,
        profile: &ConnectionProfile,
        secrets: ConnectionSecrets,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        self.connect_with_secret(profile, secrets.secret)
    }

    /// Upload a local file to `remote_path`; returns bytes written.
    fn upload(
        &self,
        _local_path: &str,
        _remote_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<u64, AppError>> + Send + '_>> {
        Box::pin(async { Err(unsupported_op("upload")) })
    }

    /// Download `remote_path` to a local file; returns bytes written.
    fn download(
        &self,
        _remote_path: &str,
        _local_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<u64, AppError>> + Send + '_>> {
        Box::pin(async { Err(unsupported_op("download")) })
    }

    /// Create a remote directory.
    fn make_directory(
        &self,
        _remote_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async { Err(unsupported_op("create folder")) })
    }

    /// Create a remote directory and return its path as this connector
    /// lists it, which is where its contents go. Path-based connectors
    /// return `remote_path` (the default); Google Drive addresses folders
    /// by id, so it returns the parent's path plus the new folder's id.
    fn make_directory_path(&self, remote_path: &str) -> Pin<Box<dyn Future<Output = Result<String, AppError>> + Send + '_>> {
        let path = remote_path.to_string();
        Box::pin(async move {
            self.make_directory(&path).await?;
            Ok(path)
        })
    }

    /// Delete a remote file or (empty) directory.
    fn remove(
        &self,
        _remote_path: &str,
        _is_dir: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async { Err(unsupported_op("delete")) })
    }

    /// Rename or move `from` to `to` on the server (both remote paths of
    /// this connection). Connectors without a server-side rename keep the
    /// default, which refuses; callers then copy and delete instead.
    fn rename_entry(
        &self,
        _from: &str,
        _to: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async {
            Err(AppError::Connection {
                message: "Renaming is not supported for this connection type.".to_string(),
                advice: "Copy the item under the new name, then delete the original.".to_string(),
            })
        })
    }

    /// Where the item at `from` is listed after `rename_entry(from, to)`:
    /// `to` for path-based connectors (the default). Google Drive keeps the
    /// item's id, so it is the new parent's path plus that id.
    fn renamed_path(&self, _from: &str, to: &str) -> String {
        to.to_string()
    }

    /// Copy the file `from` to `to` on the server without the bytes passing
    /// through this machine. `Ok(false)` (the default): not supported here,
    /// relay it (download + upload) instead.
    fn server_copy(
        &self,
        _from: &str,
        _to: &str,
    ) -> Pin<Box<dyn Future<Output = Result<bool, AppError>> + Send + '_>> {
        Box::pin(async { Ok(false) })
    }

    /// The local file name `entry` is saved as when downloaded (Google Drive
    /// exports native documents and adds the format's extension).
    fn download_name(&self, entry: &FileEntry) -> String {
        entry.name.clone()
    }

    /// Whether downloading `entry` converts it (Google Drive exports native
    /// documents) rather than copying its bytes. A pane move of such a file
    /// out of its connection keeps the original.
    fn converts_on_download(&self, _entry: &FileEntry) -> bool {
        false
    }

    /// Storage quota of the connected account; `None` when the protocol
    /// has no notion of it (the default).
    fn quota(&self) -> Pin<Box<dyn Future<Output = Result<Option<RemoteQuota>, AppError>> + Send + '_>> {
        Box::pin(async { Ok(None) })
    }

    /// A server-side size of `remote_path` (rclone `size`), when the
    /// protocol has one; `None` (the default) = walk the listing instead.
    fn folder_size_fast(&self, _remote_path: &str) -> Pin<Box<dyn Future<Output = Result<Option<FolderSize>, AppError>> + Send + '_>> {
        Box::pin(async { Ok(None) })
    }

    /// Create a link others can open `remote_path` with.
    fn create_share_link(
        &self,
        _remote_path: &str,
        _options: &ShareOptions,
    ) -> Pin<Box<dyn Future<Output = Result<ShareLink, AppError>> + Send + '_>> {
        Box::pin(async {
            Err(AppError::Connection {
                message: "Share links are not available for this connection type.".to_string(),
                advice: "Share links work with S3, Azure Blob, OneDrive, Dropbox and Google Drive.".to_string(),
            })
        })
    }
}

/// Whether `connector` can serve saved connection `id` as it is: connected
/// and, when it tracks who it is signed in as, signed in as that one (so an
/// action for one account never runs against another of the same
/// provider).
pub fn connected_as<C: Connector + ?Sized>(connector: &C, id: Option<uuid::Uuid>) -> bool {
    connector.is_connected()
        && match (id, connector.connected_profile_id()) {
            (Some(want), Some(have)) => want == have,
            _ => true,
        }
}

/// Suffix of a file still being uploaded (`.<name>.ufop-part`).
pub const STAGING_SUFFIX: &str = ".ufop-part";

/// Where an upload to `remote` is written before it is renamed into place:
/// a hidden sibling, so watchers and readers never see a half-written file.
pub fn staging_path(remote: &str) -> String {
    match remote.rsplit_once('/') {
        Some((dir, name)) => format!("{dir}/.{name}{STAGING_SUFFIX}"),
        None => format!(".{remote}{STAGING_SUFFIX}"),
    }
}

/// Upload via a temporary name, then rename into place. If the rename
/// fails (some servers won't rename over an existing file), the old file
/// is removed and the rename retried. A failed upload leaves no partial
/// file behind.
pub(crate) async fn staged_upload<U, UF, R, RF, D, DF>(remote: &str, upload: U, rename: R, delete: D) -> Result<u64, AppError>
where
    U: FnOnce(String) -> UF,
    UF: Future<Output = Result<u64, AppError>>,
    R: Fn(String, String) -> RF,
    RF: Future<Output = Result<(), AppError>>,
    D: Fn(String) -> DF,
    DF: Future<Output = Result<(), AppError>>,
{
    let temp = staging_path(remote);
    let bytes = match upload(temp.clone()).await {
        Ok(b) => b,
        Err(e) => {
            let _ = delete(temp).await;
            return Err(e);
        }
    };
    if rename(temp.clone(), remote.to_string()).await.is_ok() {
        return Ok(bytes);
    }
    let retried = match delete(remote.to_string()).await {
        Ok(()) => rename(temp.clone(), remote.to_string()).await,
        Err(e) => Err(e),
    };
    if let Err(e) = retried {
        let _ = delete(temp).await;
        return Err(AppError::Connection {
            message: format!("Uploaded, but could not move it into place as {remote}: {e}"),
            advice: "Check you may rename and replace files in that folder.".into(),
        });
    }
    Ok(bytes)
}

fn unsupported_op(what: &str) -> AppError {
    AppError::Connection {
        message: format!("{what} is not supported for this connection type yet."),
        advice: "Use SFTP, FTP/FTPS, WebDAV or S3 for remote file operations.".to_string(),
    }
}

/// Split an S3-style path (`/bucket/key/with/slashes`) into bucket and key.
pub(crate) fn split_bucket_key(path: &str) -> Result<(String, String), AppError> {
    let trimmed = path.trim_start_matches('/');
    match trimmed.split_once('/') {
        Some((bucket, key)) if !bucket.is_empty() && !key.is_empty() => Ok((bucket.to_string(), key.to_string())),
        _ => Err(AppError::Connection {
            message: format!("'{path}' is not a bucket/key path."),
            advice: "Use a path like /my-bucket/folder/file.txt.".to_string(),
        }),
    }
}

/// Registry that maps protocols to connector implementations. Clones share
/// the same connector instances.
#[derive(Clone)]
pub struct ConnectorRegistry {
    connectors: HashMap<String, Arc<dyn Connector>>,
}

impl ConnectorRegistry {
    pub fn new() -> Self {
        let mut registry = Self {
            connectors: HashMap::new(),
        };

        // Register built-in connectors
        registry.register("sftp", Arc::new(SftpConnector::new()));
        registry.register("ftp", Arc::new(FtpConnector::new()));
        registry.register("ftps", Arc::new(FtpConnector::new()));
        registry.register("webdav", Arc::new(WebDavConnector::new()));
        registry.register("smb", Arc::new(SmbConnector::new()));
        registry.register("nfs", Arc::new(NfsConnector::new()));
        registry.register("local", Arc::new(LocalDriveConnector::new()));
        // Cloud connectors (T-026..T-030)
        registry.register("s3", Arc::new(S3Connector::new()));
        registry.register("google_drive", Arc::new(GoogleDriveConnector::new()));
        registry.register("dropbox", Arc::new(DropboxConnector::new()));
        registry.register("onedrive", Arc::new(OneDriveConnector::new()));
        registry.register("b2", Arc::new(B2Connector::new()));
        // Azure Blob Storage and OpenStack Swift
        registry.register("azure_blob", Arc::new(AzureBlobConnector::new()));
        registry.register("adls_gen2", Arc::new(AdlsGen2Connector::new()));
        registry.register("azure_files", Arc::new(AzureFilesConnector::new()));
        registry.register("swift", Arc::new(SwiftConnector::new()));
        registry.register("rclone", Arc::new(rclone::RcloneConnector::new()));

        registry
    }

    /// The built-in connectors, with these OAuth connector instances (built
    /// with the app's credential store, so rotated tokens are saved, and
    /// shared with the commands that need their typed APIs).
    pub fn with_cloud(gdrive: Arc<GoogleDriveConnector>, dropbox: Arc<DropboxConnector>, onedrive: Arc<OneDriveConnector>) -> Self {
        let mut registry = Self::new();
        registry.register("google_drive", gdrive);
        registry.register("dropbox", dropbox);
        registry.register("onedrive", onedrive);
        registry
    }

    /// Register a connector for a protocol.
    pub fn register(&mut self, protocol: &str, connector: Arc<dyn Connector>) {
        self.connectors.insert(protocol.to_string(), connector);
    }

    /// Get the connector for a protocol.
    pub fn get(&self, protocol: &str) -> Option<Arc<dyn Connector>> {
        self.connectors.get(protocol).cloned()
    }

    /// Get a connector by ConnectionProtocol enum.
    pub fn get_by_protocol(&self, protocol: ConnectionProtocol) -> Option<Arc<dyn Connector>> {
        self.get(protocol.as_str())
    }

    /// List all registered protocols.
    pub fn protocols(&self) -> Vec<String> {
        self.connectors.keys().cloned().collect()
    }
}

impl Default for ConnectorRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn staging_names() {
        assert_eq!(staging_path("/in/report.pdf"), "/in/.report.pdf.ufop-part");
        assert_eq!(staging_path("report.pdf"), ".report.pdf.ufop-part");
    }

    #[tokio::test]
    async fn staged_upload_renames_replaces_and_cleans_up() {
        use std::sync::Mutex;
        let fs: Arc<Mutex<std::collections::HashMap<String, &'static str>>> = Default::default();
        let log: Arc<Mutex<Vec<String>>> = Default::default();
        // A server whose rename refuses to overwrite (like SFTP v3).
        let server = |fail_upload: bool| {
            let (fs, log) = (fs.clone(), log.clone());
            let (fs2, log2, fs3, log3) = (fs.clone(), log.clone(), fs.clone(), log.clone());
            (
                move |p: String| {
                    let (fs, log) = (fs.clone(), log.clone());
                    async move {
                        log.lock().unwrap().push(format!("up {p}"));
                        fs.lock().unwrap().insert(p, "new");
                        if fail_upload { Err(AppError::validation("network")) } else { Ok(3) }
                    }
                },
                move |from: String, to: String| {
                    let (fs, log) = (fs2.clone(), log2.clone());
                    async move {
                        log.lock().unwrap().push(format!("mv {from} {to}"));
                        let mut m = fs.lock().unwrap();
                        if m.contains_key(&to) {
                            return Err(AppError::validation("exists"));
                        }
                        let v = m.remove(&from).unwrap();
                        m.insert(to, v);
                        Ok(())
                    }
                },
                move |p: String| {
                    let (fs, log) = (fs3.clone(), log3.clone());
                    async move {
                        log.lock().unwrap().push(format!("rm {p}"));
                        fs.lock().unwrap().remove(&p);
                        Ok(())
                    }
                },
            )
        };
        let (u, r, d) = server(false);
        assert_eq!(staged_upload("/d/a.txt", u, &r, &d).await.unwrap(), 3);
        assert_eq!(fs.lock().unwrap().get("/d/a.txt"), Some(&"new"));
        // Replacing an existing file: delete then rename.
        fs.lock().unwrap().insert("/d/a.txt".into(), "old");
        let (u, r, d) = server(false);
        staged_upload("/d/a.txt", u, &r, &d).await.unwrap();
        assert_eq!(fs.lock().unwrap().get("/d/a.txt"), Some(&"new"));
        assert_eq!(fs.lock().unwrap().len(), 1, "no temp file left");
        // Failed upload: the partial temp file is removed, the target untouched.
        fs.lock().unwrap().insert("/d/a.txt".into(), "old");
        let (u, r, d) = server(true);
        assert!(staged_upload("/d/a.txt", u, &r, &d).await.is_err());
        assert_eq!(fs.lock().unwrap().get("/d/a.txt"), Some(&"old"));
        assert_eq!(fs.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_registry_creation() {
        let reg = ConnectorRegistry::new();
        // Should have SFTP, FTP, FTPS, WebDAV, SMB, NFS, Local, and cloud connectors registered
        assert!(reg.get("sftp").is_some());
        assert!(reg.get("ftp").is_some());
        assert!(reg.get("ftps").is_some());
        assert!(reg.get("webdav").is_some());
        assert!(reg.get("smb").is_some());
        assert!(reg.get("nfs").is_some());
        assert!(reg.get("local").is_some());
        assert!(reg.get("s3").is_some());
        assert!(reg.get("google_drive").is_some());
        assert!(reg.get("dropbox").is_some());
        assert!(reg.get("onedrive").is_some());
        assert!(reg.get("b2").is_some());
        assert!(reg.get("azure_blob").is_some());
        assert!(reg.get("swift").is_some());
        assert!(reg.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_protocols() {
        let reg = ConnectorRegistry::new();
        let protocols = reg.protocols();
        assert!(protocols.contains(&"sftp".to_string()));
        assert!(protocols.contains(&"ftp".to_string()));
        assert!(protocols.contains(&"ftps".to_string()));
        assert!(protocols.contains(&"webdav".to_string()));
        assert!(protocols.contains(&"smb".to_string()));
        assert!(protocols.contains(&"nfs".to_string()));
        assert!(protocols.contains(&"local".to_string()));
        assert!(protocols.contains(&"s3".to_string()));
        assert!(protocols.contains(&"google_drive".to_string()));
        assert!(protocols.contains(&"dropbox".to_string()));
        assert!(protocols.contains(&"onedrive".to_string()));
        assert!(protocols.contains(&"b2".to_string()));
        assert!(protocols.contains(&"azure_blob".to_string()));
        assert!(protocols.contains(&"swift".to_string()));
    }

    #[test]
    fn test_registry_get_by_protocol() {
        let reg = ConnectorRegistry::new();
        assert!(reg.get_by_protocol(ConnectionProtocol::Sftp).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::Ftp).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::Ftps).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::WebDav).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::Smb).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::Nfs).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::Local).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::S3).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::GoogleDrive).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::Dropbox).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::OneDrive).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::AzureBlob).is_some());
        assert!(reg.get_by_protocol(ConnectionProtocol::Swift).is_some());
    }
}

#[cfg(test)]
pub(crate) mod live_tests {
    //! Live round-trip tests against real servers. They are skipped unless
    //! `UFOP_LIVE=1`, so `cargo test` stays hermetic. Servers (see docs): SFTP on
    //! :2222, FTP on :2121, WebDAV on :8080, S3 (adobe/s3mock) on :9090.
    use super::*;
    use crate::core::types::ConnectionProtocol;

    fn live() -> bool {
        std::env::var("UFOP_LIVE").map(|v| v == "1").unwrap_or(false)
    }

    pub(crate) fn profile(protocol: ConnectionProtocol, host: &str, port: Option<u16>, user: &str, remote_path: &str) -> ConnectionProfile {
        ConnectionProfile {
            id: uuid::Uuid::new_v4(),
            name: "live".into(),
            protocol,
            host: host.into(),
            port,
            username: Some(user.into()),
            credential_ref: None,
            remote_path: remote_path.into(),
            created_at: chrono::Utc::now(),
            last_used: None,
            group_id: None,
            bandwidth_limit_bps: 0,
            conflict_policy: None,
            retry_policy: None,
            verify_checksums: false,
            checksum_algorithm: None,
            proxy_type: None,
            proxy_host: None,
            proxy_port: None,
            proxy_username: None,
            proxy_password: None,
            default_local_dir: None,
            default_remote_dir: None,
            charset: None,
            default_file_mode: None,
            default_dir_mode: None,
            ftp_use_mlsd: None,
            ftp_force_passive_ip: None,
            ftp_post_login_commands: None,
            symlink_policy: None,
            extras: Default::default(),
        }
    }

    /// connect → mkdir → upload (multi-MB, patterned) → list → download → byte-compare → delete.
    pub(crate) async fn round_trip(protocol: &str, p: ConnectionProfile, secret: Option<&str>, dir: &str) {
        let registry = ConnectorRegistry::new();
        let c = registry.get(protocol).unwrap();
        c.connect_with_secret(&p, secret.map(String::from)).await.unwrap_or_else(|e| panic!("{protocol} connect: {e}"));
        assert!(c.is_connected(), "{protocol} should report connected");

        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("payload.bin");
        let data: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2654435761) >> 24) as u8).collect();
        std::fs::write(&src, &data).unwrap();

        let sub = format!("{dir}/ufop-{}", uuid::Uuid::new_v4().simple());
        c.make_directory(&sub).await.unwrap_or_else(|e| panic!("{protocol} mkdir: {e}"));
        let remote = format!("{sub}/payload.bin");
        let sent = c.upload(src.to_str().unwrap(), &remote).await.unwrap_or_else(|e| panic!("{protocol} upload: {e}"));
        assert_eq!(sent, data.len() as u64, "{protocol} upload byte count");

        let listing = c.list_remote(&sub).await.unwrap_or_else(|e| panic!("{protocol} list: {e}"));
        assert!(listing.iter().any(|f| f.name == "payload.bin"), "{protocol} listing missing file: {listing:?}");

        let dst = tmp.path().join("back.bin");
        let got = c.download(&remote, dst.to_str().unwrap()).await.unwrap_or_else(|e| panic!("{protocol} download: {e}"));
        assert_eq!(got, data.len() as u64, "{protocol} download byte count");
        assert_eq!(std::fs::read(&dst).unwrap(), data, "{protocol} content must round-trip exactly");

        c.remove(&remote, false).await.unwrap_or_else(|e| panic!("{protocol} remove file: {e}"));
        let after = c.list_remote(&sub).await.unwrap();
        assert!(!after.iter().any(|f| f.name == "payload.bin"), "{protocol} file should be gone");
        c.disconnect().await.unwrap();
        assert!(!c.is_connected());
    }

    #[tokio::test]
    async fn live_sftp_round_trip() {
        if !live() { return; }
        // "localhost" also exercises DNS resolution, not just numeric addresses.
        round_trip("sftp", profile(ConnectionProtocol::Sftp, "localhost", Some(2222), "ufop", "/upload"), Some("ufop123"), "/upload").await;
    }

    #[tokio::test]
    async fn live_ftp_round_trip() {
        if !live() { return; }
        round_trip("ftp", profile(ConnectionProtocol::Ftp, "127.0.0.1", Some(2121), "ufop", "/"), Some("ufop123"), "").await;
    }

    #[tokio::test]
    async fn live_webdav_round_trip() {
        if !live() { return; }
        round_trip("webdav", profile(ConnectionProtocol::WebDav, "http://localhost:8080", None, "ufop", "/"), Some("ufop123"), "").await;
    }

    #[tokio::test]
    async fn live_s3_round_trip() {
        if !live() { return; }
        round_trip("s3", profile(ConnectionProtocol::S3, "http://localhost:9090", None, "ufopadmin", "/"), Some("ufopadmin123"), "/ufop-bucket").await;
    }

    #[tokio::test]
    async fn sftp_wrong_password_is_rejected() {
        if !live() { return; }
        let registry = ConnectorRegistry::new();
        let c = registry.get("sftp").unwrap();
        let r = c.connect_with_secret(&profile(ConnectionProtocol::Sftp, "localhost", Some(2222), "ufop", "/upload"), Some("nope".into())).await;
        assert!(r.is_err());
        assert!(!c.is_connected());
    }
}
