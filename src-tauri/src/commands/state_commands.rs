use crate::commands::confirmation_commands::validate_confirmation;
use crate::core::error::AppError;
use crate::core::traits::StorageOperations;
use crate::core::types::WorkspaceState;
use crate::security::audit::SecurityAudit;
use crate::security::confirmation::ConfirmationManager;
use crate::storage::Repository;
use tauri::State;

/// Save workspace state (called on app close or periodically).
#[tauri::command]
pub async fn save_workspace_state(
    state: WorkspaceState,
    repo: State<'_, Repository>,
) -> Result<(), AppError> {
    tracing::info!("Saving workspace state");
    StorageOperations::save_workspace_state(repo.inner(), &state).await?;
    Ok(())
}

/// Load workspace state (called on app open).
#[tauri::command]
pub async fn load_workspace_state(
    repo: State<'_, Repository>,
) -> Result<WorkspaceState, AppError> {
    tracing::info!("Loading workspace state");
    let state = StorageOperations::load_workspace_state(repo.inner()).await?;
    Ok(state)
}

/// Get a configuration value by key.
#[tauri::command]
pub async fn get_config(
    key: String,
    repo: State<'_, Repository>,
) -> Result<Option<String>, AppError> {
    StorageOperations::get_config(repo.inner(), &key).await
}

/// Set a configuration value.
#[tauri::command]
pub async fn set_config(
    key: String,
    value: String,
    repo: State<'_, Repository>,
) -> Result<(), AppError> {
    StorageOperations::set_config(repo.inner(), &key, &value).await
}

/// Reset the database (for troubleshooting).
/// Security: requires a confirmation token to prevent accidental or XSS-driven data loss.
#[tauri::command]
pub async fn reset_database(
    confirmation_token: String,
    repo: State<'_, Repository>,
    confirmation_mgr: State<'_, ConfirmationManager>,
) -> Result<(), AppError> {
    validate_confirmation(&confirmation_token, "reset_database", confirmation_mgr.inner()).await?;
    SecurityAudit::log_destructive_op(repo.inner(), "reset_database", "Full database reset confirmed by user").await;
    tracing::warn!("Database reset confirmed by user — proceeding");
    repo.reset().await
}
