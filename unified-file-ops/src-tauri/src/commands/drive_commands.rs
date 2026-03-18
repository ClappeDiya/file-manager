//! Tauri commands for SMB/CIFS (T-023), NFS (T-024), and Drive-to-Drive (T-025).

use crate::connectors::local_drive::{
    DriveInfo, LocalDriveConnector, PreflightResult,
};
use crate::connectors::nfs::{NfsConnector, NfsExport};
use crate::connectors::smb::{DiscoveredShare, SmbConnector, SmbDiscoveryResult};
use crate::core::error::AppError;

// ──────────────────────────────────────────────
// T-023: SMB/CIFS Commands
// ──────────────────────────────────────────────

/// Discover SMB shares on the local network.
#[tauri::command]
pub async fn smb_discover_shares(
    timeout_secs: Option<u64>,
) -> Result<SmbDiscoveryResult, AppError> {
    SmbConnector::discover_shares(timeout_secs.unwrap_or(10)).await
}

/// List shares available on a specific SMB host.
#[tauri::command]
pub async fn smb_list_shares(
    host: String,
    username: Option<String>,
    password: Option<String>,
) -> Result<Vec<DiscoveredShare>, AppError> {
    SmbConnector::list_shares(
        &host,
        username.as_deref(),
        password.as_deref(),
    )
    .await
}

// ──────────────────────────────────────────────
// T-024: NFS Commands
// ──────────────────────────────────────────────

/// List NFS exports available on a server.
#[tauri::command]
pub async fn nfs_list_exports(
    host: String,
) -> Result<Vec<NfsExport>, AppError> {
    NfsConnector::list_exports(&host).await
}

// ──────────────────────────────────────────────
// T-025: Drive-to-Drive Transfer Commands
// ──────────────────────────────────────────────

/// Detect all mounted drives/volumes on the system.
#[tauri::command]
pub async fn detect_drives() -> Result<Vec<DriveInfo>, AppError> {
    LocalDriveConnector::detect_drives().await
}

/// Run preflight checks before a drive-to-drive transfer.
#[tauri::command]
pub async fn transfer_preflight(
    source_path: String,
    dest_path: String,
    total_bytes: u64,
) -> Result<PreflightResult, AppError> {
    LocalDriveConnector::preflight_check(&source_path, &dest_path, total_bytes).await
}

/// Eject a removable drive.
#[tauri::command]
pub async fn eject_drive(
    mount_point: String,
) -> Result<(), AppError> {
    LocalDriveConnector::eject_drive(&mount_point).await
}
