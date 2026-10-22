//! Management console ("Managed by console") commands for the settings UI.
//! The protocol and loop live in `ufop_core::agent`; see `fleet_agent`.

use crate::core::error::AppError;
use crate::fleet_agent::FleetAgent;
use tauri::State;
use ufop_core::agent::{parse_tags, AgentStatus, ClaimPollResult, ClaimStatus};

#[tauri::command]
pub async fn fleet_agent_status(agent: State<'_, FleetAgent>) -> Result<AgentStatus, AppError> {
    Ok(agent.status())
}

/// Register with a registration link token (`ufop_reg_…`). `tags` is comma separated.
#[tauri::command]
pub async fn fleet_agent_register(agent: State<'_, FleetAgent>, console_url: String, registration_token: String, tags: Option<String>) -> Result<AgentStatus, AppError> {
    agent.register(&console_url, &registration_token, tags.as_deref().map(parse_tags).unwrap_or_default()).await
}

/// Start claiming: returns the code the administrator enters in the console.
#[tauri::command]
pub async fn fleet_agent_start_claim(agent: State<'_, FleetAgent>, console_url: String) -> Result<ClaimStatus, AppError> {
    agent.start_claim(&console_url).await
}

/// `pending` until the administrator entered the code, then `registered`.
#[tauri::command]
pub async fn fleet_agent_poll_claim(agent: State<'_, FleetAgent>) -> Result<ClaimPollResult, AppError> {
    agent.poll_claim().await
}

#[tauri::command]
pub async fn fleet_agent_cancel_claim(agent: State<'_, FleetAgent>) -> Result<AgentStatus, AppError> {
    agent.cancel_claim();
    Ok(agent.status())
}

/// Leave the console; `force` also when it cannot be reached.
#[tauri::command]
pub async fn fleet_agent_unregister(agent: State<'_, FleetAgent>, force: Option<bool>) -> Result<AgentStatus, AppError> {
    agent.unregister(force.unwrap_or(false)).await
}

#[tauri::command]
pub async fn fleet_agent_set_enabled(agent: State<'_, FleetAgent>, enabled: bool) -> Result<AgentStatus, AppError> {
    Ok(agent.set_enabled(enabled).await)
}

/// Heartbeat, commands and policy sync now.
#[tauri::command]
pub async fn fleet_agent_sync_now(agent: State<'_, FleetAgent>) -> Result<AgentStatus, AppError> {
    agent.sync_now().await
}
