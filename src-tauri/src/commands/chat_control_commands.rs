//! Tauri commands for chat-bot remote control (Settings → Advanced).

use crate::chat_control::service::{ChatControlService, ChatControlStatus, SecretUpdate};
use crate::chat_control::ChatControlSettings;
use crate::core::error::AppError;
use serde::Serialize;
use tauri::State;

/// The settings and the live status.
#[derive(Debug, Serialize)]
pub struct ChatControlView {
    pub settings: ChatControlSettings,
    pub status: ChatControlStatus,
}

/// Remote-control settings (secrets are never returned; the status says
/// whether each is stored) and whether the bot is running.
#[tauri::command]
pub async fn chat_control_get(svc: State<'_, ChatControlService>) -> Result<ChatControlView, AppError> {
    Ok(ChatControlView { settings: svc.settings().await, status: svc.status().await })
}

/// Save the settings and any new bot tokens, then start or stop the bot to
/// match. `None` keeps a stored token, an empty string removes it.
#[tauri::command]
pub async fn chat_control_save(
    settings: ChatControlSettings,
    telegram_token: Option<String>,
    slack_bot_token: Option<String>,
    slack_app_token: Option<String>,
    svc: State<'_, ChatControlService>,
) -> Result<ChatControlView, AppError> {
    let status = svc.save(settings, SecretUpdate { telegram_token, slack_bot_token, slack_app_token }).await?;
    Ok(ChatControlView { settings: svc.settings().await, status })
}

/// Live status only (polled while the settings screen is open).
#[tauri::command]
pub async fn chat_control_status(svc: State<'_, ChatControlService>) -> Result<ChatControlStatus, AppError> {
    Ok(svc.status().await)
}
