//! Tauri commands for versioned backups (jobs, runs, restore, maintenance,
//! configuration portability and the scheduler).

use crate::backup_engine::{
    api, portability, BackupDestination, BackupJob, BackupManager, BackupOperation, BackupRun,
    Fileset, OperationResult, SchedulerStatus,
};
use crate::backup_engine::defaults::{GlobalDefaults, SavedDestination};
use crate::core::error::AppError;
use tauri::State;
use ufop_core::backup::crypto::{self, PassphraseStrength};
use ufop_core::backup::maintenance::{FoundFile, VersionComparison};
use ufop_core::backup::{FileEntry, RestoreOptions, RestoreReport};

#[tauri::command]
pub async fn backup_list_jobs(mgr: State<'_, BackupManager>) -> Result<Vec<BackupJob>, AppError> {
    Ok(mgr.list_jobs().await)
}

#[tauri::command]
pub async fn backup_get_job(id: String, mgr: State<'_, BackupManager>) -> Result<BackupJob, AppError> {
    mgr.get_job(&id).await
}

/// Create (empty `id`) or update a job. `passphrase`: omitted keeps the
/// stored one, empty string clears it.
#[tauri::command]
pub async fn backup_save_job(job: BackupJob, passphrase: Option<String>, mgr: State<'_, BackupManager>) -> Result<BackupJob, AppError> {
    mgr.save_job(job, passphrase).await
}

#[tauri::command]
pub async fn backup_delete_job(id: String, delete_local_db: bool, delete_remote: bool, mgr: State<'_, BackupManager>) -> Result<(), AppError> {
    mgr.delete_job(&id, delete_local_db, delete_remote).await
}

#[tauri::command]
pub async fn backup_test_destination(destination: BackupDestination, create: bool, mgr: State<'_, BackupManager>) -> Result<usize, AppError> {
    mgr.test_destination(&destination, create).await
}

/// Randomized upload/list/download/hash round-trips against a destination.
#[tauri::command]
pub async fn backup_test_destination_compat(destination: BackupDestination, files: Option<usize>, max_size: Option<usize>, mgr: State<'_, BackupManager>) -> Result<Vec<String>, AppError> {
    mgr.test_destination_compat(&destination, files.unwrap_or(3), max_size.unwrap_or(1 << 20)).await
}

/// Queue a backup in the background; returns the task id.
#[tauri::command]
pub async fn backup_run(id: String, dry_run: Option<bool>, mgr: State<'_, BackupManager>) -> Result<String, AppError> {
    mgr.start_backup(&id, dry_run.unwrap_or(false)).await
}

/// Run a maintenance/restore operation and wait for its result.
#[tauri::command]
pub async fn backup_execute(id: String, operation: BackupOperation, mgr: State<'_, BackupManager>) -> Result<OperationResult, AppError> {
    if matches!(operation, BackupOperation::Backup { .. }) {
        return Err(AppError::validation("Use backup_run to start a backup"));
    }
    mgr.execute(&id, operation).await
}

#[tauri::command]
pub async fn backup_cancel(id: String, mgr: State<'_, BackupManager>) -> Result<bool, AppError> {
    Ok(mgr.cancel(&id))
}

#[tauri::command]
pub async fn backup_status(mgr: State<'_, BackupManager>) -> Result<SchedulerStatus, AppError> {
    Ok(mgr.status().await)
}

/// Pause scheduled backups, for `minutes` or indefinitely.
#[tauri::command]
pub async fn backup_pause(minutes: Option<u32>, mgr: State<'_, BackupManager>) -> Result<SchedulerStatus, AppError> {
    mgr.pause(minutes);
    Ok(mgr.status().await)
}

#[tauri::command]
pub async fn backup_resume(mgr: State<'_, BackupManager>) -> Result<SchedulerStatus, AppError> {
    mgr.resume();
    Ok(mgr.status().await)
}

#[tauri::command]
pub async fn backup_list_versions(id: String, mgr: State<'_, BackupManager>) -> Result<Vec<Fileset>, AppError> {
    mgr.list_versions(&id).await
}

#[tauri::command]
pub async fn backup_list_files(id: String, version: usize, prefix: Option<String>, mgr: State<'_, BackupManager>) -> Result<Vec<FileEntry>, AppError> {
    mgr.list_files(&id, version, prefix).await
}

#[tauri::command]
pub async fn backup_search(id: String, pattern: String, version: Option<usize>, mgr: State<'_, BackupManager>) -> Result<Vec<FoundFile>, AppError> {
    mgr.search(&id, pattern, version).await
}

#[tauri::command]
pub async fn backup_compare(id: String, base: usize, compare: usize, mgr: State<'_, BackupManager>) -> Result<VersionComparison, AppError> {
    mgr.compare(&id, base, compare).await
}

#[tauri::command]
pub async fn backup_run_history(job_id: Option<String>, limit: Option<u32>, mgr: State<'_, BackupManager>) -> Result<Vec<BackupRun>, AppError> {
    mgr.run_history(job_id, limit.unwrap_or(50)).await
}

#[tauri::command]
pub async fn backup_prune_history(days: u32, mgr: State<'_, BackupManager>) -> Result<usize, AppError> {
    mgr.prune_history(days.max(1)).await
}

/// Export a job as JSON. `include_passphrase` embeds the backup passphrase;
/// `export_passphrase` encrypts the whole export.
#[tauri::command]
pub async fn backup_export_config(id: String, include_passphrase: bool, export_passphrase: Option<String>, include_stats: Option<bool>, mgr: State<'_, BackupManager>) -> Result<String, AppError> {
    api::export_config(&mgr, &id, include_passphrase, export_passphrase.as_deref(), include_stats.unwrap_or(false)).await
}

/// Parse an export without saving it (for review), or save it immediately.
#[tauri::command]
pub async fn backup_import_config(json: String, export_passphrase: Option<String>, import_stats: Option<bool>, save: bool, mgr: State<'_, BackupManager>) -> Result<api::ImportPreview, AppError> {
    api::import_config(&mgr, &json, export_passphrase.as_deref(), import_stats.unwrap_or(false), save).await
}

#[tauri::command]
pub async fn backup_export_is_encrypted(json: String) -> Result<bool, AppError> {
    Ok(portability::is_encrypted_export(&json))
}

#[tauri::command]
pub async fn backup_export_cli(id: String, mgr: State<'_, BackupManager>) -> Result<String, AppError> {
    Ok(portability::cli_command(&mgr.get_job(&id).await?))
}

/// List versions of a repository without a configured job.
#[tauri::command]
pub async fn backup_browse_remote(destination: BackupDestination, passphrase: Option<String>, prefix: Option<String>, mgr: State<'_, BackupManager>) -> Result<Vec<Fileset>, AppError> {
    mgr.browse_remote(destination, passphrase, prefix.unwrap_or_else(|| "ufop".into())).await
}

/// Restore from a repository using only its location and passphrase.
#[tauri::command]
pub async fn backup_direct_restore(destination: BackupDestination, passphrase: Option<String>, prefix: Option<String>, options: RestoreOptions, mgr: State<'_, BackupManager>) -> Result<RestoreReport, AppError> {
    mgr.direct_restore(destination, passphrase, prefix.unwrap_or_else(|| "ufop".into()), options).await
}

/// Read the job configuration stored inside a remote repository.
#[tauri::command]
pub async fn backup_discover_remote_config(destination: BackupDestination, passphrase: Option<String>, prefix: Option<String>, mgr: State<'_, BackupManager>) -> Result<Option<serde_json::Value>, AppError> {
    mgr.discover_remote_config(destination, passphrase, prefix.unwrap_or_else(|| "ufop".into())).await
}

#[tauri::command]
pub async fn backup_get_global_defaults(mgr: State<'_, BackupManager>) -> Result<GlobalDefaults, AppError> {
    mgr.global_defaults().await
}

#[tauri::command]
pub async fn backup_set_global_defaults(defaults: GlobalDefaults, mgr: State<'_, BackupManager>) -> Result<GlobalDefaults, AppError> {
    mgr.set_global_defaults(defaults).await
}

/// The job as it runs, with global defaults applied.
#[tauri::command]
pub async fn backup_effective_job(id: String, mgr: State<'_, BackupManager>) -> Result<BackupJob, AppError> {
    mgr.effective_job(&id).await
}

#[tauri::command]
pub async fn backup_list_destinations(mgr: State<'_, BackupManager>) -> Result<Vec<SavedDestination>, AppError> {
    mgr.list_destinations().await
}

#[tauri::command]
pub async fn backup_save_destination(destination: SavedDestination, mgr: State<'_, BackupManager>) -> Result<SavedDestination, AppError> {
    mgr.save_destination(destination).await
}

#[tauri::command]
pub async fn backup_delete_destination(id: String, mgr: State<'_, BackupManager>) -> Result<(), AppError> {
    mgr.delete_destination(&id).await
}

/// Continue an existing backup on this computer (history kept).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn backup_adopt_repository(
    destination: BackupDestination,
    passphrase: Option<String>,
    prefix: Option<String>,
    name: Option<String>,
    sources: Option<Vec<String>>,
    source_map: Option<Vec<(String, String)>>,
    mgr: State<'_, BackupManager>,
) -> Result<BackupJob, AppError> {
    mgr.adopt_repository(destination, passphrase, prefix.unwrap_or_else(|| "ufop".into()), name, sources.unwrap_or_default(), source_map.unwrap_or_default()).await
}

#[tauri::command]
pub async fn backup_passphrase_strength(passphrase: String) -> Result<PassphraseStrength, AppError> {
    Ok(crypto::passphrase_strength(&passphrase))
}

#[tauri::command]
pub async fn backup_generate_passphrase(length: Option<usize>) -> Result<String, AppError> {
    Ok(crypto::generate_passphrase(length.unwrap_or(32)))
}

#[tauri::command]
pub async fn backup_capabilities(mgr: State<'_, BackupManager>) -> Result<api::BackupCapabilities, AppError> {
    Ok(api::capabilities(&mgr))
}

/// Count what the job's sources and filters select, without backing up.
#[tauri::command]
pub async fn backup_preview_sources(job: BackupJob) -> Result<api::SourcePreview, AppError> {
    api::preview_sources(job).await
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct DesktopSettings {
    /// Closing the window keeps UFOP running in the tray.
    pub run_in_background: bool,
    /// Start UFOP (hidden in the tray) when the user logs in.
    pub start_at_login: bool,
}

#[tauri::command]
pub async fn backup_get_desktop_settings(app: tauri::AppHandle, mode: State<'_, crate::tray::BackgroundMode>) -> Result<DesktopSettings, AppError> {
    use tauri_plugin_autostart::ManagerExt;
    Ok(DesktopSettings { run_in_background: mode.enabled(), start_at_login: app.autolaunch().is_enabled().unwrap_or(false) })
}

#[tauri::command]
pub async fn backup_set_desktop_settings(
    settings: DesktopSettings,
    app: tauri::AppHandle,
    mode: State<'_, crate::tray::BackgroundMode>,
    repo: State<'_, crate::storage::Repository>,
) -> Result<DesktopSettings, AppError> {
    use tauri_plugin_autostart::ManagerExt;
    crate::tray::save_background_mode(&repo, &mode, settings.run_in_background).await?;
    let al = app.autolaunch();
    let res = if settings.start_at_login { al.enable() } else { al.disable() };
    res.map_err(|e| AppError::state(format!("Could not change start-at-login: {e}"), "Check that the app may register itself to start at login."))?;
    backup_get_desktop_settings(app, mode).await
}
