//! Tauri commands for Smart Spaces.

use crate::connectors::ConnectionManager;
use crate::core::error::AppError;
use crate::spaces_engine::{SmartSpace, SpaceStatus, SpaceStatusResult, SpacesManager};
use crate::storage::Repository;
use tauri::State;

// ── CRUD ──

#[tauri::command]
pub async fn create_space(
    space: SmartSpace,
    repo: State<'_, Repository>,
    mgr: State<'_, SpacesManager>,
) -> Result<SmartSpace, AppError> {
    mgr.save_space(&repo, &space).await?;
    tracing::info!("Created smart space '{}' ({})", space.name, space.id);
    Ok(space)
}

#[tauri::command]
pub async fn list_spaces(
    mgr: State<'_, SpacesManager>,
) -> Result<Vec<SmartSpace>, AppError> {
    Ok(mgr.list_spaces().await)
}

#[tauri::command]
pub async fn get_space(
    space_id: String,
    mgr: State<'_, SpacesManager>,
) -> Result<SmartSpace, AppError> {
    mgr.get_space(&space_id).await.ok_or_else(|| AppError::State {
        message: format!("Smart space '{}' not found", space_id),
        advice: "Check the space ID.".to_string(),
    })
}

#[tauri::command]
pub async fn update_space(
    space: SmartSpace,
    repo: State<'_, Repository>,
    mgr: State<'_, SpacesManager>,
) -> Result<SmartSpace, AppError> {
    mgr.save_space(&repo, &space).await?;
    tracing::info!("Updated smart space '{}' ({})", space.name, space.id);
    Ok(space)
}

#[tauri::command]
pub async fn delete_space(
    space_id: String,
    repo: State<'_, Repository>,
    mgr: State<'_, SpacesManager>,
) -> Result<(), AppError> {
    mgr.delete_space(&repo, &space_id).await?;
    tracing::info!("Deleted smart space {}", space_id);
    Ok(())
}

// ── Status ──

#[tauri::command]
pub async fn get_space_status(
    space_id: String,
    mgr: State<'_, SpacesManager>,
    conn_mgr: State<'_, ConnectionManager>,
) -> Result<SpaceStatusResult, AppError> {
    let space = mgr.get_space(&space_id).await.ok_or_else(|| AppError::State {
        message: format!("Smart space '{}' not found", space_id),
        advice: "Check the space ID.".to_string(),
    })?;

    // No remote connection → local only
    let Some(ref connection_id) = space.connection_id else {
        return Ok(SpaceStatusResult {
            space_id,
            status: SpaceStatus::LocalOnly,
            connection_alive: false,
            last_sync_at: None,
            conflict_count: 0,
        });
    };

    // Check if connection profile exists (best-effort status check)
    let connection_alive = conn_mgr
        .list_connections()
        .await
        .unwrap_or_default()
        .iter()
        .any(|c| c.id.to_string() == *connection_id);

    // Determine aggregate status
    let status = if !connection_alive {
        SpaceStatus::Offline
    } else if space.sync_pair_id.is_some() {
        // Has sync configured and connection exists → show as connected
        SpaceStatus::Connected
    } else {
        SpaceStatus::Connected
    };

    Ok(SpaceStatusResult {
        space_id,
        status,
        connection_alive,
        last_sync_at: None,
        conflict_count: 0,
    })
}

// ── Activation ──

#[tauri::command]
pub async fn activate_space(
    space_id: String,
    mgr: State<'_, SpacesManager>,
) -> Result<SmartSpace, AppError> {
    mgr.get_space(&space_id).await.ok_or_else(|| AppError::State {
        message: format!("Smart space '{}' not found", space_id),
        advice: "Check the space ID.".to_string(),
    })
}

// ── Automation Links ──

#[tauri::command]
pub async fn attach_space_automation(
    space_id: String,
    rule_id: String,
    repo: State<'_, Repository>,
    mgr: State<'_, SpacesManager>,
) -> Result<(), AppError> {
    mgr.attach_automation(&repo, &space_id, &rule_id).await?;
    tracing::info!("Attached automation {} to space {}", rule_id, space_id);
    Ok(())
}

#[tauri::command]
pub async fn detach_space_automation(
    space_id: String,
    rule_id: String,
    repo: State<'_, Repository>,
    mgr: State<'_, SpacesManager>,
) -> Result<(), AppError> {
    mgr.detach_automation(&repo, &space_id, &rule_id).await?;
    tracing::info!("Detached automation {} from space {}", rule_id, space_id);
    Ok(())
}
