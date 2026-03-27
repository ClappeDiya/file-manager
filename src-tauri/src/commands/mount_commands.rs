//! Tauri commands for remote drive mounting.
//!
//! Uses native OS mount commands (mount_smbfs, sshfs, mount_nfs, etc.)
//! to mount remote connections as local directories. Includes capability
//! detection to report clearly when mounting is not possible.

use crate::connectors::ConnectionManager;
use crate::core::error::AppError;
use crate::mount_engine::{MountCapability, MountConfig, MountInfo, MountManager, MountResult, MountStatus};
use crate::storage::Repository;
use tauri::State;
use uuid::Uuid;

/// Check whether a protocol can be mounted on the current OS.
#[tauri::command]
pub async fn check_mount_capability(
    protocol: String,
    mount_mgr: State<'_, MountManager>,
) -> Result<MountCapability, AppError> {
    Ok(mount_mgr.check_capability(&protocol))
}

/// Mount a remote connection at a local directory using native OS commands.
///
/// Validates the connection exists, checks mount capability for the protocol,
/// creates the mount point directory if needed, and executes the OS mount.
#[tauri::command]
pub async fn mount_remote(
    connection_id: String,
    mount_point: String,
    mount_mgr: State<'_, MountManager>,
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
) -> Result<MountResult, AppError> {
    let cid = Uuid::parse_str(&connection_id).map_err(|_| AppError::connection(
        format!("Invalid connection ID: {connection_id}"),
        "Provide a valid UUID.",
    ))?;

    // Validate mount point is a reasonable path
    if mount_point.trim().is_empty() {
        return Err(AppError::connection(
            "Mount point cannot be empty".to_string(),
            "Provide a valid directory path for the mount point.",
        ));
    }

    mount_mgr
        .mount_remote(cid, mount_point, &repo, &connection_mgr)
        .await
}

/// Unmount a remote drive.
///
/// Removes the mount from active state and updates the database entry.
#[tauri::command]
pub async fn unmount_remote(
    mount_id: String,
    mount_mgr: State<'_, MountManager>,
    repo: State<'_, Repository>,
) -> Result<(), AppError> {
    let mid = Uuid::parse_str(&mount_id).map_err(|_| AppError::connection(
        format!("Invalid mount ID: {mount_id}"),
        "Provide a valid UUID.",
    ))?;

    mount_mgr.unmount_remote(mid, &repo).await
}

/// List all active and saved mounts.
#[tauri::command]
pub async fn list_mounts(
    mount_mgr: State<'_, MountManager>,
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
) -> Result<Vec<MountInfo>, AppError> {
    mount_mgr.list_mounts(&repo, &connection_mgr).await
}

/// Get the status of a specific mount.
#[tauri::command]
pub async fn get_mount_status(
    mount_id: String,
    mount_mgr: State<'_, MountManager>,
) -> Result<MountStatus, AppError> {
    let mid = Uuid::parse_str(&mount_id).map_err(|_| AppError::connection(
        format!("Invalid mount ID: {mount_id}"),
        "Provide a valid UUID.",
    ))?;

    Ok(mount_mgr.get_mount_status(mid).await)
}

/// Save a mount configuration for persistent/auto-mount.
///
/// This saves the connection + mount point pair so it can be
/// automatically mounted on application startup.
#[tauri::command]
pub async fn save_mount_config(
    connection_id: String,
    mount_point: String,
    auto_mount: bool,
    mount_mgr: State<'_, MountManager>,
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
) -> Result<MountConfig, AppError> {
    let cid = Uuid::parse_str(&connection_id).map_err(|_| AppError::connection(
        format!("Invalid connection ID: {connection_id}"),
        "Provide a valid UUID.",
    ))?;

    if mount_point.trim().is_empty() {
        return Err(AppError::connection(
            "Mount point cannot be empty".to_string(),
            "Provide a valid directory path for the mount point.",
        ));
    }

    mount_mgr
        .save_mount_config(cid, mount_point, auto_mount, &repo, &connection_mgr)
        .await
}

/// Delete a saved mount configuration.
#[tauri::command]
pub async fn delete_mount_config(
    mount_id: String,
    mount_mgr: State<'_, MountManager>,
    repo: State<'_, Repository>,
) -> Result<(), AppError> {
    let mid = Uuid::parse_str(&mount_id).map_err(|_| AppError::connection(
        format!("Invalid mount ID: {mount_id}"),
        "Provide a valid UUID.",
    ))?;

    mount_mgr.delete_mount_config(mid, &repo).await
}
