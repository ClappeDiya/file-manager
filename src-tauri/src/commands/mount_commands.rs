//! Tauri commands for remote drive mounting.
//!
//! Uses native OS mount commands (mount_smbfs, sshfs, mount_nfs, etc.)
//! to mount remote connections as local directories. Includes capability
//! detection to report clearly when mounting is not possible.

use crate::connectors::ConnectionManager;
use crate::core::error::AppError;
use crate::mount_engine::{MountCapability, MountConfig, MountInfo, MountManager, MountOptions, MountResult, MountStatus, PinReport, UnmountAllReport};
use crate::storage::Repository;
use tauri::State;
use uuid::Uuid;

/// Emitted to the window whenever a mount changes state or a saved mount is added, edited or removed.
pub const MOUNTS_CHANGED_EVENT: &str = "ufop://mounts-changed";

/// Tell the window about every mount change (it refreshes its Mounts list).
pub fn forward_mount_changes(app: tauri::AppHandle, mount_mgr: &MountManager) {
    use tauri::Emitter;
    let mut changes = mount_mgr.subscribe();
    tauri::async_runtime::spawn(async move {
        while changes.changed().await.is_ok() {
            let _ = app.emit(MOUNTS_CHANGED_EVENT, ());
        }
    });
}

/// Open a mounted drive's folder in the OS file manager (what "Open after mounting" does).
/// A drive that mounted is never reported as failed because the folder would not open:
/// the reason is added to the message instead.
async fn open_after_mount(mount_mgr: &MountManager, mut result: MountResult) -> MountResult {
    match open_mounted(mount_mgr, result.mount_id).await {
        Ok(()) => {}
        Err(e) => result.message = format!("{} (it could not be opened: {})", result.message, e.user_message()),
    }
    result
}

async fn open_mounted(mount_mgr: &MountManager, mount_id: Uuid) -> Result<(), AppError> {
    let path = mount_mgr.mounted_path(mount_id).await?;
    super::file_ops_commands::open_file_with_default(path).await
}

/// Mount a saved configuration; opens it afterwards when its options say so.
/// Shared by the Mounts list and the tray.
pub async fn mount_saved_and_open(mount_mgr: &MountManager, mount_id: Uuid, repo: &Repository, connection_mgr: &ConnectionManager) -> Result<MountResult, AppError> {
    let wants_open = mount_mgr.saved_options(mount_id, repo, connection_mgr).await.map(|o| o.open_after_mount).unwrap_or(false);
    let result = mount_mgr.mount_saved(mount_id, repo, connection_mgr).await?;
    Ok(if wants_open { open_after_mount(mount_mgr, result).await } else { result })
}

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
    options: Option<MountOptions>,
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

    let options = options.unwrap_or_default();
    let wants_open = options.open_after_mount;
    let result = mount_mgr.mount_remote_with(cid, mount_point, options, None, &repo, &connection_mgr).await?;
    Ok(if wants_open { open_after_mount(&mount_mgr, result).await } else { result })
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
    options: Option<MountOptions>,
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
        .save_mount_config(cid, mount_point, auto_mount, options, &repo, &connection_mgr)
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

fn mount_uuid(mount_id: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(mount_id).map_err(|_| AppError::validation(format!("Invalid mount ID: {mount_id}")))
}

/// The organization's mount profile in force (`null` = none).
#[tauri::command]
pub async fn mount_policy(mount_mgr: State<'_, MountManager>) -> Result<Option<ufop_core::agent::ManagedMounts>, AppError> {
    Ok(mount_mgr.managed().await)
}

/// Change a saved mount (the drive must be unmounted). `connection_id` is optional: omitted keeps it.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_mount_config(
    mount_id: String,
    connection_id: Option<String>,
    mount_point: String,
    auto_mount: bool,
    options: Option<MountOptions>,
    mount_mgr: State<'_, MountManager>,
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
) -> Result<MountConfig, AppError> {
    let cid = connection_id
        .map(|c| Uuid::parse_str(&c).map_err(|_| AppError::validation(format!("Invalid connection ID: {c}"))))
        .transpose()?;
    mount_mgr
        .update_mount_config(mount_uuid(&mount_id)?, cid, mount_point, auto_mount, options.unwrap_or_default(), &repo, &connection_mgr)
        .await
}

/// The options a saved mount was saved with (what the user chose, before the organization's
/// limits). The Edit dialog starts from these, so saving does not turn forced values into choices.
#[tauri::command]
pub async fn saved_mount_options(
    mount_id: String,
    mount_mgr: State<'_, MountManager>,
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
) -> Result<MountOptions, AppError> {
    mount_mgr.saved_options(mount_uuid(&mount_id)?, &repo, &connection_mgr).await
}

/// Mount a saved mount again with its own settings.
#[tauri::command]
pub async fn mount_saved_config(
    mount_id: String,
    mount_mgr: State<'_, MountManager>,
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
) -> Result<MountResult, AppError> {
    mount_saved_and_open(&mount_mgr, mount_uuid(&mount_id)?, &repo, &connection_mgr).await
}

/// Unmount every mounted drive; the report lists the ones that refused.
#[tauri::command]
pub async fn unmount_all_mounts(mount_mgr: State<'_, MountManager>, repo: State<'_, Repository>) -> Result<UnmountAllReport, AppError> {
    Ok(mount_mgr.unmount_all(&repo).await)
}

/// Drive letters (D–Z) nothing uses now. Empty except on Windows.
#[tauri::command]
pub async fn free_drive_letters() -> Result<Vec<String>, AppError> {
    Ok(crate::mount_engine::policy::free_drive_letters())
}

/// Open a mounted drive in the OS file manager.
#[tauri::command]
pub async fn open_mount_point(mount_id: String, mount_mgr: State<'_, MountManager>) -> Result<(), AppError> {
    open_mounted(&mount_mgr, mount_uuid(&mount_id)?).await
}

/// Make a folder of a mounted cloud drive available offline (full cache mode).
#[tauri::command]
pub async fn mount_pin_offline(mount_id: String, path: Option<String>, mount_mgr: State<'_, MountManager>) -> Result<PinReport, AppError> {
    mount_mgr.pin_offline(mount_uuid(&mount_id)?, path.as_deref().unwrap_or("")).await
}

/// Bytes in a mount's local cache.
#[tauri::command]
pub async fn mount_cache_usage(mount_id: String, mount_mgr: State<'_, MountManager>, repo: State<'_, Repository>) -> Result<u64, AppError> {
    Ok(mount_mgr.cache_usage_of(mount_uuid(&mount_id)?, &repo).await)
}

/// Delete a mount's local cache (unmounted drives only); returns the bytes freed.
#[tauri::command]
pub async fn mount_clear_cache(mount_id: String, mount_mgr: State<'_, MountManager>, repo: State<'_, Repository>) -> Result<u64, AppError> {
    mount_mgr.clear_cache_of(mount_uuid(&mount_id)?, &repo).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn a_drive_that_will_not_open_is_still_reported_as_mounted() {
        let mgr = MountManager::new();
        let id = Uuid::new_v4();
        let r = MountResult { mount_id: id, mount_point: "/mnt/x".into(), status: MountStatus::Mounted, message: "Successfully mounted 'x'".into() };
        let out = open_after_mount(&mgr, r).await;
        assert_eq!(out.status, MountStatus::Mounted);
        assert!(out.message.starts_with("Successfully mounted 'x' (it could not be opened: "), "{}", out.message);
        assert!(open_mounted(&mgr, id).await.is_err(), "only mounted drives open");
    }

    #[test]
    fn mount_ids_are_validated() {
        assert!(mount_uuid("nope").is_err());
        assert!(mount_uuid(&Uuid::new_v4().to_string()).is_ok());
    }
}
