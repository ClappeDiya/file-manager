//! Server-to-Server Direct Transfer (T-033).
//!
//! Features:
//! - Direct transfer between remote endpoints where possible (SFTP-to-SFTP, S3-to-S3)
//! - Fallback to local relay when direct transfer is impossible
//! - Clear indication of transfer method in queue
//! - Supports mixed protocol pairs with automatic method selection

use crate::core::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Constants ──

/// Local relay buffer size: 8 MB chunks for relay transfers.
#[allow(dead_code)]
const RELAY_BUFFER_SIZE: u64 = 8 * 1024 * 1024;
/// Maximum parallel streams for relay transfers.
#[allow(dead_code)]
const MAX_RELAY_STREAMS: u32 = 4;

// ── Types ──

/// How a server-to-server transfer is executed.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferMethod {
    /// Direct server-to-server transfer (no local data transit).
    Direct,
    /// Local machine relays data between source and destination.
    LocalRelay,
    /// Hybrid: some operations direct, some relayed.
    Hybrid,
}

impl TransferMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::LocalRelay => "local_relay",
            Self::Hybrid => "hybrid",
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::Direct => "Direct (server-to-server)",
            Self::LocalRelay => "Relayed (via local machine)",
            Self::Hybrid => "Hybrid (direct + relay)",
        }
    }
}

/// Protocol type for endpoint identification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EndpointProtocol {
    Sftp,
    Ftp,
    S3,
    WebDav,
    Smb,
    Nfs,
    GoogleDrive,
    Dropbox,
    OneDrive,
    Local,
}

impl EndpointProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sftp => "sftp",
            Self::Ftp => "ftp",
            Self::S3 => "s3",
            Self::WebDav => "webdav",
            Self::Smb => "smb",
            Self::Nfs => "nfs",
            Self::GoogleDrive => "google_drive",
            Self::Dropbox => "dropbox",
            Self::OneDrive => "onedrive",
            Self::Local => "local",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "sftp" => Self::Sftp,
            "ftp" | "ftps" => Self::Ftp,
            "s3" => Self::S3,
            "webdav" => Self::WebDav,
            "smb" => Self::Smb,
            "nfs" => Self::Nfs,
            "google_drive" => Self::GoogleDrive,
            "dropbox" => Self::Dropbox,
            "onedrive" => Self::OneDrive,
            "local" => Self::Local,
            _ => Self::Local,
        }
    }
}

/// A remote endpoint for server-to-server transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferEndpoint {
    /// Connection profile ID from connection manager.
    pub connection_id: Uuid,
    /// Protocol of the endpoint.
    pub protocol: EndpointProtocol,
    /// Host/server address.
    pub host: String,
    /// Port (protocol-specific).
    pub port: Option<u16>,
    /// Path on the remote server.
    pub path: String,
    /// Display name for the endpoint.
    pub display_name: String,
}

/// A server-to-server transfer job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerTransferJob {
    /// Unique job ID.
    pub id: Uuid,
    /// Source endpoint.
    pub source: TransferEndpoint,
    /// Destination endpoint.
    pub destination: TransferEndpoint,
    /// Transfer method being used.
    pub method: TransferMethod,
    /// Why this method was chosen.
    pub method_reason: String,
    /// Files to transfer.
    pub files: Vec<ServerTransferFile>,
    /// Total bytes to transfer.
    pub total_bytes: u64,
    /// Bytes transferred so far.
    pub transferred_bytes: u64,
    /// Current status.
    pub status: ServerTransferStatus,
    /// Transfer speed in bytes/sec.
    pub speed_bps: u64,
    /// Estimated time remaining.
    pub eta_seconds: Option<u64>,
    /// Error message if failed.
    pub error_message: Option<String>,
    /// When the job was created.
    pub created_at: DateTime<Utc>,
    /// When the job was last updated.
    pub updated_at: DateTime<Utc>,
    /// When the job completed (if finished).
    pub completed_at: Option<DateTime<Utc>>,
}

/// A file within a server-to-server transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerTransferFile {
    /// Source path relative to source root.
    pub source_path: String,
    /// Destination path relative to dest root.
    pub dest_path: String,
    /// File size in bytes.
    pub size: u64,
    /// Whether this entry is a directory.
    pub is_dir: bool,
    /// Transfer status for this file.
    pub status: FileTransferStatus,
    /// Error message if this file failed.
    pub error: Option<String>,
}

/// Status of a server-to-server transfer job.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ServerTransferStatus {
    /// Analyzing source and dest to determine method.
    Analyzing,
    /// Queued for execution.
    Queued,
    /// Currently transferring.
    Active,
    /// Paused by user.
    Paused,
    /// Successfully completed.
    Completed,
    /// Failed with error.
    Failed,
    /// Cancelled by user.
    Cancelled,
}

impl ServerTransferStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Analyzing => "analyzing",
            Self::Queued => "queued",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Status of an individual file in a transfer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileTransferStatus {
    Pending,
    Active,
    Completed,
    Failed,
    Skipped,
}

impl FileTransferStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
        }
    }
}

/// Capability matrix entry indicating if direct transfer is possible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DirectTransferCapability {
    pub source_protocol: EndpointProtocol,
    pub dest_protocol: EndpointProtocol,
    pub supports_direct: bool,
    pub method: TransferMethod,
    pub description: String,
}

// ── Protocol Pair Compatibility ──

/// Determine if two protocols support direct server-to-server transfer.
pub fn check_direct_capability(
    source: EndpointProtocol,
    dest: EndpointProtocol,
) -> DirectTransferCapability {
    let (supports_direct, method, description) = match (source, dest) {
        // Same-protocol direct transfers
        (EndpointProtocol::S3, EndpointProtocol::S3) => (
            true,
            TransferMethod::Direct,
            "S3-to-S3 copy using server-side CopyObject API".to_string(),
        ),
        (EndpointProtocol::Sftp, EndpointProtocol::Sftp) => (
            true,
            TransferMethod::Direct,
            "SFTP-to-SFTP via SCP between servers (if servers allow)".to_string(),
        ),

        // Cloud-to-cloud with API support
        (EndpointProtocol::GoogleDrive, EndpointProtocol::GoogleDrive) => (
            true,
            TransferMethod::Direct,
            "Google Drive copy within same or cross-account".to_string(),
        ),
        (EndpointProtocol::Dropbox, EndpointProtocol::Dropbox) => (
            true,
            TransferMethod::Direct,
            "Dropbox copy API for same-service transfers".to_string(),
        ),
        (EndpointProtocol::OneDrive, EndpointProtocol::OneDrive) => (
            true,
            TransferMethod::Direct,
            "OneDrive/SharePoint copy within Microsoft Graph".to_string(),
        ),

        // Cross-protocol pairs that must relay
        (EndpointProtocol::S3, EndpointProtocol::Sftp)
        | (EndpointProtocol::Sftp, EndpointProtocol::S3) => (
            false,
            TransferMethod::LocalRelay,
            "Different protocols require local relay".to_string(),
        ),

        (EndpointProtocol::GoogleDrive, EndpointProtocol::Dropbox)
        | (EndpointProtocol::Dropbox, EndpointProtocol::GoogleDrive) => (
            false,
            TransferMethod::LocalRelay,
            "Cross-cloud providers require local relay".to_string(),
        ),

        // Local endpoint involved
        (EndpointProtocol::Local, _) | (_, EndpointProtocol::Local) => (
            false,
            TransferMethod::LocalRelay,
            "Local endpoint requires direct I/O".to_string(),
        ),

        // SMB/NFS - same protocol may work on same network
        (EndpointProtocol::Smb, EndpointProtocol::Smb) => (
            false,
            TransferMethod::LocalRelay,
            "SMB-to-SMB copy via local relay (robocopy/rsync available as OS tools)".to_string(),
        ),
        (EndpointProtocol::Nfs, EndpointProtocol::Nfs) => (
            false,
            TransferMethod::LocalRelay,
            "NFS-to-NFS copy via local relay (both mounts accessible locally)".to_string(),
        ),

        // Default: relay for any other combination
        _ => (
            false,
            TransferMethod::LocalRelay,
            "Incompatible protocols require local relay".to_string(),
        ),
    };

    DirectTransferCapability {
        source_protocol: source,
        dest_protocol: dest,
        supports_direct,
        method,
        description,
    }
}

/// Get the full capability matrix for all protocol pairs.
pub fn get_capability_matrix() -> Vec<DirectTransferCapability> {
    let protocols = [
        EndpointProtocol::Sftp,
        EndpointProtocol::Ftp,
        EndpointProtocol::S3,
        EndpointProtocol::WebDav,
        EndpointProtocol::Smb,
        EndpointProtocol::Nfs,
        EndpointProtocol::GoogleDrive,
        EndpointProtocol::Dropbox,
        EndpointProtocol::OneDrive,
        EndpointProtocol::Local,
    ];

    let mut matrix = Vec::new();
    for &src in &protocols {
        for &dst in &protocols {
            if src != dst {
                matrix.push(check_direct_capability(src, dst));
            }
        }
    }
    matrix
}

// ── Server Transfer Manager ──

/// Manages server-to-server transfer jobs.
pub struct ServerTransferManager {
    /// Active transfer jobs.
    jobs: Arc<RwLock<HashMap<Uuid, ServerTransferJob>>>,
}

impl Default for ServerTransferManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ServerTransferManager {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Create a new server-to-server transfer job.
    /// Automatically determines the best transfer method.
    pub async fn create_transfer(
        &self,
        source: TransferEndpoint,
        destination: TransferEndpoint,
        files: Vec<ServerTransferFile>,
    ) -> Result<ServerTransferJob, AppError> {
        // Determine transfer method
        let capability = check_direct_capability(source.protocol, destination.protocol);

        let total_bytes: u64 = files.iter().map(|f| f.size).sum();
        let now = Utc::now();

        let job = ServerTransferJob {
            id: Uuid::new_v4(),
            source,
            destination,
            method: capability.method,
            method_reason: capability.description,
            files,
            total_bytes,
            transferred_bytes: 0,
            status: ServerTransferStatus::Queued,
            speed_bps: 0,
            eta_seconds: None,
            error_message: None,
            created_at: now,
            updated_at: now,
            completed_at: None,
        };

        tracing::info!(
            "Created server transfer job {} ({} -> {}, method: {}, {} bytes)",
            job.id,
            job.source.display_name,
            job.destination.display_name,
            job.method.as_str(),
            total_bytes
        );

        let mut jobs = self.jobs.write().await;
        jobs.insert(job.id, job.clone());

        Ok(job)
    }

    /// Start executing a transfer job.
    pub async fn start_transfer(&self, job_id: Uuid) -> Result<(), AppError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found"),
            advice: "The job may have been cancelled.".to_string(),
        })?;

        if job.status != ServerTransferStatus::Queued
            && job.status != ServerTransferStatus::Paused
        {
            return Err(AppError::Transfer {
                message: format!(
                    "Cannot start transfer in {} state",
                    job.status.as_str()
                ),
                advice: "Only queued or paused transfers can be started.".to_string(),
            });
        }

        job.status = ServerTransferStatus::Active;
        job.updated_at = Utc::now();

        tracing::info!(
            "Starting server transfer {} via {}",
            job_id,
            job.method.as_str()
        );

        // In production, this would:
        // - For Direct: initiate server-side copy operations
        //   - S3: CopyObject API
        //   - SFTP: scp between servers
        //   - Cloud: service-specific copy APIs
        // - For LocalRelay: start streaming chunks source -> local -> dest
        // - For Hybrid: mix of both strategies
        //
        // Each method would update progress and handle errors/retries.

        Ok(())
    }

    /// Pause a running transfer.
    pub async fn pause_transfer(&self, job_id: Uuid) -> Result<(), AppError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found"),
            advice: "The job may have been cancelled.".to_string(),
        })?;

        if job.status != ServerTransferStatus::Active {
            return Err(AppError::Transfer {
                message: "Can only pause active transfers".to_string(),
                advice: "The transfer is not currently running.".to_string(),
            });
        }

        job.status = ServerTransferStatus::Paused;
        job.updated_at = Utc::now();

        tracing::info!("Paused server transfer {}", job_id);
        Ok(())
    }

    /// Cancel a transfer job.
    pub async fn cancel_transfer(&self, job_id: Uuid) -> Result<(), AppError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found"),
            advice: "The job may have already completed.".to_string(),
        })?;

        job.status = ServerTransferStatus::Cancelled;
        job.updated_at = Utc::now();

        tracing::info!("Cancelled server transfer {}", job_id);
        Ok(())
    }

    /// Get a specific transfer job.
    pub async fn get_transfer(&self, job_id: Uuid) -> Option<ServerTransferJob> {
        let jobs = self.jobs.read().await;
        jobs.get(&job_id).cloned()
    }

    /// List all transfer jobs.
    pub async fn list_transfers(&self) -> Vec<ServerTransferJob> {
        let jobs = self.jobs.read().await;
        let mut list: Vec<_> = jobs.values().cloned().collect();
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        list
    }

    /// List only active transfer jobs.
    pub async fn list_active_transfers(&self) -> Vec<ServerTransferJob> {
        let jobs = self.jobs.read().await;
        jobs.values()
            .filter(|j| {
                j.status == ServerTransferStatus::Active
                    || j.status == ServerTransferStatus::Queued
            })
            .cloned()
            .collect()
    }

    /// Update transfer progress (called by background worker).
    pub async fn update_progress(
        &self,
        job_id: Uuid,
        transferred_bytes: u64,
        speed_bps: u64,
        current_file_index: Option<usize>,
    ) -> Result<(), AppError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found"),
            advice: "The job may have been removed.".to_string(),
        })?;

        job.transferred_bytes = transferred_bytes;
        job.speed_bps = speed_bps;
        job.updated_at = Utc::now();

        if speed_bps > 0 {
            let remaining = job.total_bytes.saturating_sub(transferred_bytes);
            job.eta_seconds = Some(remaining / speed_bps);
        }

        // Mark current file as active
        if let Some(idx) = current_file_index {
            if idx < job.files.len() {
                // Mark previous files as completed
                for file in job.files[..idx].iter_mut() {
                    if file.status == FileTransferStatus::Active {
                        file.status = FileTransferStatus::Completed;
                    }
                }
                job.files[idx].status = FileTransferStatus::Active;
            }
        }

        Ok(())
    }

    /// Mark a transfer as completed.
    pub async fn complete_transfer(&self, job_id: Uuid) -> Result<(), AppError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found"),
            advice: "The job may have been removed.".to_string(),
        })?;

        let now = Utc::now();
        job.status = ServerTransferStatus::Completed;
        job.transferred_bytes = job.total_bytes;
        job.updated_at = now;
        job.completed_at = Some(now);

        // Mark all files as completed
        for file in &mut job.files {
            if file.status != FileTransferStatus::Failed && file.status != FileTransferStatus::Skipped {
                file.status = FileTransferStatus::Completed;
            }
        }

        tracing::info!("Server transfer {} completed", job_id);
        Ok(())
    }

    /// Mark a transfer as failed.
    pub async fn fail_transfer(
        &self,
        job_id: Uuid,
        error: String,
    ) -> Result<(), AppError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found"),
            advice: "The job may have been removed.".to_string(),
        })?;

        job.status = ServerTransferStatus::Failed;
        job.error_message = Some(error);
        job.updated_at = Utc::now();

        tracing::error!("Server transfer {} failed: {:?}", job_id, job.error_message);
        Ok(())
    }

    /// Retry a failed transfer.
    pub async fn retry_transfer(&self, job_id: Uuid) -> Result<(), AppError> {
        let mut jobs = self.jobs.write().await;
        let job = jobs.get_mut(&job_id).ok_or_else(|| AppError::Transfer {
            message: format!("Transfer job {job_id} not found"),
            advice: "The job may have been removed.".to_string(),
        })?;

        if job.status != ServerTransferStatus::Failed {
            return Err(AppError::Transfer {
                message: "Can only retry failed transfers".to_string(),
                advice: "The transfer has not failed.".to_string(),
            });
        }

        // Reset failed files to pending
        for file in &mut job.files {
            if file.status == FileTransferStatus::Failed {
                file.status = FileTransferStatus::Pending;
                file.error = None;
            }
        }

        job.status = ServerTransferStatus::Queued;
        job.error_message = None;
        job.updated_at = Utc::now();

        tracing::info!("Retrying server transfer {}", job_id);
        Ok(())
    }

    /// Remove completed/failed/cancelled jobs from the list.
    pub async fn cleanup_completed(&self) -> u32 {
        let mut jobs = self.jobs.write().await;
        let before = jobs.len();
        jobs.retain(|_, j| {
            j.status != ServerTransferStatus::Completed
                && j.status != ServerTransferStatus::Failed
                && j.status != ServerTransferStatus::Cancelled
        });
        (before - jobs.len()) as u32
    }

    /// Check what transfer method would be used for a given source/dest pair.
    pub fn preview_method(
        &self,
        source_protocol: &str,
        dest_protocol: &str,
    ) -> DirectTransferCapability {
        let src = EndpointProtocol::from_str_lossy(source_protocol);
        let dst = EndpointProtocol::from_str_lossy(dest_protocol);
        check_direct_capability(src, dst)
    }
}

impl Clone for ServerTransferManager {
    fn clone(&self) -> Self {
        Self {
            jobs: self.jobs.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_to_s3_direct() {
        let cap = check_direct_capability(EndpointProtocol::S3, EndpointProtocol::S3);
        assert!(cap.supports_direct);
        assert_eq!(cap.method, TransferMethod::Direct);
    }

    #[test]
    fn test_sftp_to_sftp_direct() {
        let cap = check_direct_capability(EndpointProtocol::Sftp, EndpointProtocol::Sftp);
        assert!(cap.supports_direct);
        assert_eq!(cap.method, TransferMethod::Direct);
    }

    #[test]
    fn test_s3_to_sftp_relay() {
        let cap = check_direct_capability(EndpointProtocol::S3, EndpointProtocol::Sftp);
        assert!(!cap.supports_direct);
        assert_eq!(cap.method, TransferMethod::LocalRelay);
    }

    #[test]
    fn test_local_always_relay() {
        let cap = check_direct_capability(EndpointProtocol::Local, EndpointProtocol::S3);
        assert!(!cap.supports_direct);
        assert_eq!(cap.method, TransferMethod::LocalRelay);

        let cap2 = check_direct_capability(EndpointProtocol::Sftp, EndpointProtocol::Local);
        assert!(!cap2.supports_direct);
        assert_eq!(cap2.method, TransferMethod::LocalRelay);
    }

    #[test]
    fn test_cloud_to_cloud_same() {
        let cap = check_direct_capability(
            EndpointProtocol::GoogleDrive,
            EndpointProtocol::GoogleDrive,
        );
        assert!(cap.supports_direct);
        assert_eq!(cap.method, TransferMethod::Direct);

        let cap2 =
            check_direct_capability(EndpointProtocol::Dropbox, EndpointProtocol::Dropbox);
        assert!(cap2.supports_direct);

        let cap3 =
            check_direct_capability(EndpointProtocol::OneDrive, EndpointProtocol::OneDrive);
        assert!(cap3.supports_direct);
    }

    #[test]
    fn test_cross_cloud_relay() {
        let cap = check_direct_capability(
            EndpointProtocol::GoogleDrive,
            EndpointProtocol::Dropbox,
        );
        assert!(!cap.supports_direct);
        assert_eq!(cap.method, TransferMethod::LocalRelay);
    }

    #[test]
    fn test_capability_matrix() {
        let matrix = get_capability_matrix();
        // 10 protocols, each pair (excluding self) = 10 * 9 = 90 entries
        assert_eq!(matrix.len(), 90);

        // Verify all entries have valid methods
        for entry in &matrix {
            match entry.method {
                TransferMethod::Direct | TransferMethod::LocalRelay | TransferMethod::Hybrid => {}
            }
        }
    }

    #[tokio::test]
    async fn test_create_transfer_job() {
        let mgr = ServerTransferManager::new();

        let source = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::S3,
            host: "s3.amazonaws.com".to_string(),
            port: None,
            path: "/bucket-a/data/".to_string(),
            display_name: "AWS S3 Bucket A".to_string(),
        };

        let dest = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::S3,
            host: "s3.amazonaws.com".to_string(),
            port: None,
            path: "/bucket-b/backup/".to_string(),
            display_name: "AWS S3 Bucket B".to_string(),
        };

        let files = vec![ServerTransferFile {
            source_path: "report.csv".to_string(),
            dest_path: "report.csv".to_string(),
            size: 1_000_000,
            is_dir: false,
            status: FileTransferStatus::Pending,
            error: None,
        }];

        let job = mgr.create_transfer(source, dest, files).await.unwrap();
        assert_eq!(job.method, TransferMethod::Direct);
        assert_eq!(job.total_bytes, 1_000_000);
        assert_eq!(job.status, ServerTransferStatus::Queued);
    }

    #[tokio::test]
    async fn test_cross_protocol_creates_relay() {
        let mgr = ServerTransferManager::new();

        let source = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::Sftp,
            host: "server1.example.com".to_string(),
            port: Some(22),
            path: "/home/user/data/".to_string(),
            display_name: "Server 1 SFTP".to_string(),
        };

        let dest = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::S3,
            host: "s3.amazonaws.com".to_string(),
            port: None,
            path: "/backup-bucket/".to_string(),
            display_name: "AWS S3 Backup".to_string(),
        };

        let files = vec![ServerTransferFile {
            source_path: "data.tar.gz".to_string(),
            dest_path: "data.tar.gz".to_string(),
            size: 500_000_000,
            is_dir: false,
            status: FileTransferStatus::Pending,
            error: None,
        }];

        let job = mgr.create_transfer(source, dest, files).await.unwrap();
        assert_eq!(job.method, TransferMethod::LocalRelay);
    }

    #[tokio::test]
    async fn test_transfer_lifecycle() {
        let mgr = ServerTransferManager::new();

        let source = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::S3,
            host: "s3.us-east-1.amazonaws.com".to_string(),
            port: None,
            path: "/src-bucket/".to_string(),
            display_name: "Source Bucket".to_string(),
        };

        let dest = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::S3,
            host: "s3.eu-west-1.amazonaws.com".to_string(),
            port: None,
            path: "/dst-bucket/".to_string(),
            display_name: "Dest Bucket".to_string(),
        };

        let files = vec![ServerTransferFile {
            source_path: "data.csv".to_string(),
            dest_path: "data.csv".to_string(),
            size: 100_000,
            is_dir: false,
            status: FileTransferStatus::Pending,
            error: None,
        }];

        let job = mgr.create_transfer(source, dest, files).await.unwrap();
        let job_id = job.id;

        // Start
        mgr.start_transfer(job_id).await.unwrap();
        let job = mgr.get_transfer(job_id).await.unwrap();
        assert_eq!(job.status, ServerTransferStatus::Active);

        // Update progress
        mgr.update_progress(job_id, 50_000, 10_000, Some(0))
            .await
            .unwrap();

        // Pause
        mgr.pause_transfer(job_id).await.unwrap();
        let job = mgr.get_transfer(job_id).await.unwrap();
        assert_eq!(job.status, ServerTransferStatus::Paused);

        // Resume
        mgr.start_transfer(job_id).await.unwrap();

        // Complete
        mgr.complete_transfer(job_id).await.unwrap();
        let job = mgr.get_transfer(job_id).await.unwrap();
        assert_eq!(job.status, ServerTransferStatus::Completed);
        assert!(job.completed_at.is_some());
    }

    #[tokio::test]
    async fn test_fail_and_retry() {
        let mgr = ServerTransferManager::new();

        let source = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::Sftp,
            host: "server.example.com".to_string(),
            port: Some(22),
            path: "/data/".to_string(),
            display_name: "Server SFTP".to_string(),
        };

        let dest = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::Sftp,
            host: "backup.example.com".to_string(),
            port: Some(22),
            path: "/backup/".to_string(),
            display_name: "Backup SFTP".to_string(),
        };

        let files = vec![ServerTransferFile {
            source_path: "file.dat".to_string(),
            dest_path: "file.dat".to_string(),
            size: 1000,
            is_dir: false,
            status: FileTransferStatus::Pending,
            error: None,
        }];

        let job = mgr.create_transfer(source, dest, files).await.unwrap();
        let job_id = job.id;

        mgr.start_transfer(job_id).await.unwrap();

        // Fail
        mgr.fail_transfer(job_id, "Connection lost".to_string())
            .await
            .unwrap();
        let job = mgr.get_transfer(job_id).await.unwrap();
        assert_eq!(job.status, ServerTransferStatus::Failed);

        // Retry
        mgr.retry_transfer(job_id).await.unwrap();
        let job = mgr.get_transfer(job_id).await.unwrap();
        assert_eq!(job.status, ServerTransferStatus::Queued);
    }

    #[tokio::test]
    async fn test_cancel_transfer() {
        let mgr = ServerTransferManager::new();

        let source = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::S3,
            host: "s3.amazonaws.com".to_string(),
            port: None,
            path: "/bucket/".to_string(),
            display_name: "S3".to_string(),
        };

        let dest = TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::S3,
            host: "s3.amazonaws.com".to_string(),
            port: None,
            path: "/backup/".to_string(),
            display_name: "S3 Backup".to_string(),
        };

        let job = mgr
            .create_transfer(source, dest, vec![])
            .await
            .unwrap();

        mgr.cancel_transfer(job.id).await.unwrap();
        let job = mgr.get_transfer(job.id).await.unwrap();
        assert_eq!(job.status, ServerTransferStatus::Cancelled);
    }

    #[tokio::test]
    async fn test_cleanup_completed() {
        let mgr = ServerTransferManager::new();

        let ep = || TransferEndpoint {
            connection_id: Uuid::new_v4(),
            protocol: EndpointProtocol::S3,
            host: "s3.amazonaws.com".to_string(),
            port: None,
            path: "/bucket/".to_string(),
            display_name: "S3".to_string(),
        };

        // Create 3 jobs
        let j1 = mgr.create_transfer(ep(), ep(), vec![]).await.unwrap();
        let j2 = mgr.create_transfer(ep(), ep(), vec![]).await.unwrap();
        let _j3 = mgr.create_transfer(ep(), ep(), vec![]).await.unwrap();

        // Complete j1, cancel j2, leave j3 queued
        mgr.complete_transfer(j1.id).await.unwrap();
        mgr.cancel_transfer(j2.id).await.unwrap();

        let removed = mgr.cleanup_completed().await;
        assert_eq!(removed, 2);

        let remaining = mgr.list_transfers().await;
        assert_eq!(remaining.len(), 1);
    }

    #[test]
    fn test_preview_method() {
        let mgr = ServerTransferManager::new();

        let cap = mgr.preview_method("s3", "s3");
        assert!(cap.supports_direct);
        assert_eq!(cap.method, TransferMethod::Direct);

        let cap2 = mgr.preview_method("sftp", "s3");
        assert!(!cap2.supports_direct);
        assert_eq!(cap2.method, TransferMethod::LocalRelay);
    }
}
