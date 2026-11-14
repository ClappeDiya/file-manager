//! Tauri IPC commands for OS notification preferences and sending.
//!
//! Stores notification preferences in the config table (as JSON) and
//! sends native OS notifications via `tauri-plugin-notification`.

use crate::core::error::AppError;
use crate::core::traits::StorageOperations;
use crate::storage::Repository;
use tauri::State;
use tauri_plugin_notification::NotificationExt;

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

/// The preferences are shared with the code that acts on them
/// (`notify_hub::desktop`), so what Settings saves is what raises toasts.
pub use crate::notify_hub::desktop::NotificationPrefs;
use crate::notify_hub::desktop::{parse_prefs, NOTIFICATION_PREFS_KEY};

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
    parse_prefs(value.as_deref()).map_err(|e| {
        tracing::warn!("Invalid notification prefs in config, using defaults: {e}");
        AppError::state(
            "Notification preferences were corrupted",
            "Defaults have been restored. You can reconfigure in Settings.",
        )
    })
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
