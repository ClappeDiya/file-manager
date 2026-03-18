//! Tauri IPC commands for OS notification preferences and sending.
//!
//! Stores notification preferences in the config table (as JSON) and
//! sends native OS notifications via `tauri-plugin-notification`.

use crate::core::error::AppError;
use crate::core::traits::StorageOperations;
use crate::storage::Repository;
use serde::{Deserialize, Serialize};
use tauri::State;
use tauri_plugin_notification::NotificationExt;

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

/// Config key used to persist notification preferences.
const NOTIFICATION_PREFS_KEY: &str = "notification_prefs";

/// Notification preference configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationPrefs {
    pub transfer_complete: bool,
    pub transfer_failed: bool,
    pub sync_complete: bool,
    pub sync_failed: bool,
    pub connection_lost: bool,
    pub only_when_backgrounded: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            transfer_complete: true,
            transfer_failed: true,
            sync_complete: false,
            sync_failed: true,
            connection_lost: false,
            only_when_backgrounded: true,
        }
    }
}

// ──────────────────────────────────────────────
// Commands
// ──────────────────────────────────────────────

/// Retrieve current notification preferences from the config table.
/// Returns defaults if nothing has been saved yet.
#[tauri::command]
pub async fn get_notification_prefs(
    repo: State<'_, Repository>,
) -> Result<NotificationPrefs, AppError> {
    let value = StorageOperations::get_config(repo.inner(), NOTIFICATION_PREFS_KEY).await?;
    match value {
        Some(json_str) => {
            let prefs: NotificationPrefs = serde_json::from_str(&json_str).map_err(|e| {
                tracing::warn!("Invalid notification prefs in config, using defaults: {e}");
                AppError::state(
                    "Notification preferences were corrupted",
                    "Defaults have been restored. You can reconfigure in Settings.",
                )
            })?;
            Ok(prefs)
        }
        None => Ok(NotificationPrefs::default()),
    }
}

/// Save notification preferences to the config table as JSON.
#[tauri::command]
pub async fn set_notification_prefs(
    prefs: NotificationPrefs,
    repo: State<'_, Repository>,
) -> Result<(), AppError> {
    let json_str = serde_json::to_string(&prefs).map_err(|e| {
        AppError::state(
            format!("Failed to serialize notification preferences: {e}"),
            "Try again.",
        )
    })?;
    StorageOperations::set_config(repo.inner(), NOTIFICATION_PREFS_KEY, &json_str).await?;
    tracing::info!("Notification preferences updated");
    Ok(())
}

/// Send an OS notification with the given title and body.
/// Uses `tauri_plugin_notification::NotificationExt` on the app handle.
#[tauri::command]
pub async fn send_notification(
    title: String,
    body: String,
    app_handle: tauri::AppHandle,
) -> Result<(), AppError> {
    app_handle
        .notification()
        .builder()
        .title(&title)
        .body(&body)
        .show()
        .map_err(|e| {
            tracing::error!("Failed to send OS notification: {e}");
            AppError::state(
                format!("Failed to send notification: {e}"),
                "Check that the application has notification permissions in your system settings.",
            )
        })?;

    tracing::debug!("OS notification sent: {title}");
    Ok(())
}
