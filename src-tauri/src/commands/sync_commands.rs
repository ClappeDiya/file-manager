//! Tauri commands for the sync engine (T-039..T-042).
//!
//! Provides IPC handlers for sync pair management, dry-run preview,
//! conflict resolution, reporting, and rollback.

use crate::core::error::AppError;
use crate::core::types::*;
use crate::sync_engine::SyncManager;
use tauri::State;
use uuid::Uuid;

// ── Helper ──

fn parse_uuid(s: &str) -> Result<Uuid, AppError> {
    Uuid::parse_str(s).map_err(|e| AppError::Sync {
        message: format!("Invalid UUID '{}': {}", s, e),
        advice: "Provide a valid UUID.".to_string(),
    })
}

// ── T-039: Sync Pair CRUD ──

/// Create a new sync pair.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn create_sync_pair(
    name: String,
    source_path: String,
    dest_path: String,
    mode: String,
    trigger: String,
    cron_expr: Option<String>,
    filter_json: Option<String>,
    conflict_policy: Option<String>,
    verify_mode: Option<String>,
    checksum_enabled: Option<bool>,
    time_offset_secs: Option<i64>,
    manager: State<'_, SyncManager>,
) -> Result<SyncPair, AppError> {
    let sync_mode = SyncMode::from_str_lossy(&mode);

    let trigger_mode = match trigger.as_str() {
        "scheduled" => SyncTrigger::Scheduled {
            cron_expr: cron_expr.unwrap_or_else(|| "0 0 */6 * * * *".to_string()),
        },
        "watch" => SyncTrigger::Watch,
        "api" => SyncTrigger::Api,
        _ => SyncTrigger::Manual,
    };

    let filter: SyncFilter = filter_json
        .and_then(|j| serde_json::from_str(&j).ok())
        .unwrap_or_default();

    let policy = conflict_policy
        .map(|s| SyncConflictPolicy::from_str_lossy(&s))
        .unwrap_or_default();

    let verify = verify_mode
        .map(|s| SyncVerifyMode::from_str_lossy(&s))
        .unwrap_or_default();

    let pair = SyncPair {
        id: Uuid::new_v4(),
        name,
        source_path,
        dest_path,
        mode: sync_mode,
        enabled: true,
        last_run: None,
        trigger: trigger_mode,
        filter,
        conflict_policy: policy,
        verify_mode: verify,
        checksum_enabled: checksum_enabled.unwrap_or(false),
        created_at: chrono::Utc::now(),
        time_offset_secs,
    };

    use crate::core::traits::SyncOperations;
    manager.create_pair(pair).await
}

/// Detect server time offset by comparing local clock to remote filesystem timestamp.
#[tauri::command]
pub async fn detect_time_offset(
    _source_path: String,
    dest_path: String,
) -> Result<i64, AppError> {
    use std::path::Path;
    use std::time::SystemTime;

    let dest_root = Path::new(&dest_path);
    if !dest_root.exists() {
        return Err(AppError::Sync {
            message: "Destination path does not exist".to_string(),
            advice: "Check the destination path.".to_string(),
        });
    }

    // Write a temp file to the destination, read its mtime, compare to local clock
    let temp_name = format!(".ufop_time_probe_{}", chrono::Utc::now().timestamp_millis());
    let temp_path = dest_root.join(&temp_name);

    std::fs::write(&temp_path, "time_probe").map_err(|e| AppError::Sync {
        message: format!("Failed to write time probe file: {e}"),
        advice: "Check write permissions on the destination.".to_string(),
    })?;

    let metadata = std::fs::metadata(&temp_path).map_err(|e| AppError::Sync {
        message: format!("Failed to read time probe metadata: {e}"),
        advice: "Check file permissions.".to_string(),
    })?;

    let remote_mtime = metadata.modified().map_err(|e| AppError::Sync {
        message: format!("Failed to get modification time: {e}"),
        advice: "Filesystem may not support modification times.".to_string(),
    })?;

    // Clean up
    let _ = std::fs::remove_file(&temp_path);

    let now = SystemTime::now();
    let offset = match now.duration_since(remote_mtime) {
        Ok(d) => d.as_secs() as i64,
        Err(e) => -(e.duration().as_secs() as i64),
    };

    Ok(offset)
}

/// Delete a sync pair.
#[tauri::command]
pub async fn delete_sync_pair(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&pair_id)?;
    use crate::core::traits::SyncOperations;
    manager.delete_pair(id).await
}

/// List all sync pairs.
#[tauri::command]
pub async fn list_sync_pairs(
    manager: State<'_, SyncManager>,
) -> Result<Vec<SyncPair>, AppError> {
    use crate::core::traits::SyncOperations;
    manager.list_pairs().await
}

/// Update a sync pair.
#[tauri::command]
pub async fn update_sync_pair(
    pair_json: String,
    manager: State<'_, SyncManager>,
) -> Result<SyncPair, AppError> {
    let pair: SyncPair = serde_json::from_str(&pair_json).map_err(|e| AppError::Sync {
        message: format!("Invalid sync pair data: {}", e),
        advice: "Check the sync pair configuration.".to_string(),
    })?;
    manager.update_pair(pair).await
}

/// Run a sync for a pair.
#[tauri::command]
pub async fn run_sync(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<SyncReport, AppError> {
    let id = parse_uuid(&pair_id)?;
    manager.execute_sync(id).await
}

/// Get the health indicator for a sync pair.
#[tauri::command]
pub async fn get_sync_health(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<SyncHealth, AppError> {
    let id = parse_uuid(&pair_id)?;
    Ok(manager.get_health(id).await)
}

// ── T-040: Dry-Run Preview ──

/// Perform a dry-run preview for a sync pair.
#[tauri::command]
pub async fn sync_dry_run(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<SyncPreview, AppError> {
    let id = parse_uuid(&pair_id)?;
    manager.dry_run(id).await
}

/// Export a dry-run preview as CSV.
#[tauri::command]
pub async fn export_sync_preview_csv(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<String, AppError> {
    let id = parse_uuid(&pair_id)?;
    manager.export_preview_csv(id).await
}

// ── T-041: Conflict Resolution ──

/// Get pending conflicts for resolution.
#[tauri::command]
pub async fn get_sync_conflicts(
    manager: State<'_, SyncManager>,
) -> Result<Vec<SyncConflictItem>, AppError> {
    Ok(manager.get_pending_conflicts().await)
}

/// Resolve a specific conflict.
#[tauri::command]
pub async fn resolve_sync_conflict(
    conflict_id: String,
    resolution: String,
    manager: State<'_, SyncManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&conflict_id)?;
    let policy = SyncConflictPolicy::from_str_lossy(&resolution);
    manager.resolve_conflict_item(id, policy).await
}

/// Get quarantine entries for a pair.
#[tauri::command]
pub async fn get_quarantine_entries(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<Vec<QuarantineEntry>, AppError> {
    let id = parse_uuid(&pair_id)?;
    Ok(manager.get_quarantine(id).await)
}

// ── T-042: Verification, Rollback, Reporting ──

/// Get sync reports for a pair.
#[tauri::command]
pub async fn get_sync_reports(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<Vec<SyncReport>, AppError> {
    let id = parse_uuid(&pair_id)?;
    Ok(manager.get_reports(id).await)
}

/// Rollback the last sync run for a pair.
#[tauri::command]
pub async fn rollback_sync(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<u64, AppError> {
    let id = parse_uuid(&pair_id)?;
    manager.rollback_last_run(id).await
}

/// Export a sync report as CSV.
#[tauri::command]
pub async fn export_sync_report_csv(
    pair_id: String,
    report_id: String,
    manager: State<'_, SyncManager>,
) -> Result<String, AppError> {
    let pid = parse_uuid(&pair_id)?;
    let rid = parse_uuid(&report_id)?;
    manager.export_report_csv(pid, rid).await
}

/// Export a sync report as JSON.
#[tauri::command]
pub async fn export_sync_report_json(
    pair_id: String,
    report_id: String,
    manager: State<'_, SyncManager>,
) -> Result<String, AppError> {
    let pid = parse_uuid(&pair_id)?;
    let rid = parse_uuid(&report_id)?;
    manager.export_report_json(pid, rid).await
}

/// Start filesystem watcher for a sync pair.
#[tauri::command]
pub async fn start_sync_watcher(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&pair_id)?;
    manager.start_watcher(id).await
}

/// Stop filesystem watcher for a sync pair.
#[tauri::command]
pub async fn stop_sync_watcher(
    pair_id: String,
    manager: State<'_, SyncManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&pair_id)?;
    manager.stop_watcher(id).await
}

/// Validate a cron expression for scheduling.
#[tauri::command]
pub async fn validate_sync_cron(
    cron_expr: String,
) -> Result<bool, AppError> {
    match crate::sync_engine::scheduler::SyncScheduler::validate_cron(&cron_expr) {
        Ok(_) => Ok(true),
        Err(_) => Ok(false),
    }
}

// ── Sync Filter Presets ──

/// Get available sync filter presets.
#[tauri::command]
pub async fn get_sync_filter_presets() -> Result<Vec<crate::sync_engine::FilterPreset>, AppError> {
    Ok(crate::sync_engine::get_filter_presets())
}
