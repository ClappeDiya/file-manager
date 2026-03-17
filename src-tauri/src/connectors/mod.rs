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
//! - Azure Blob Storage: SAS/AD auth, container/blob ops, access tiers, SAS URLs
//! - OpenStack Swift: Keystone v3/TempAuth, container/object ops, SLO/DLO, TempURL

pub mod connection_manager;
pub mod ftp;
pub mod import_profiles;
pub mod local_drive;
pub mod nfs;
pub mod peer;
pub mod server_to_server;
pub mod sftp;
pub mod smb;
pub mod webdav;
// T-026..T-030: Cloud connectors
pub mod b2;
pub mod dropbox;
pub mod google_drive;
pub mod oauth;
pub mod onedrive;
pub mod s3;
// AWS credential chain, SSO, and STS federation
pub mod aws_credentials;
// Azure Blob Storage and OpenStack Swift connectors
pub mod azure_blob;
pub mod openstack_swift;

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
pub use openstack_swift::SwiftConnector;
// T-031, T-033: Peer and server-to-server
pub use peer::PeerManager;
pub use server_to_server::ServerTransferManager;

/// Trait for remote filesystem connectors.
pub trait Connector: Send + Sync {
    fn connect(&self, profile: &ConnectionProfile) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>>;
    fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>>;
    fn list_remote(
        &self,
        path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<FileEntry>, AppError>> + Send + '_>>;
    fn is_connected(&self) -> bool;
}

/// Registry that maps protocols to connector implementations.
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
        registry.register("swift", Arc::new(SwiftConnector::new()));

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
