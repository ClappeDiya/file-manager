//! Tauri commands for versioned backups (jobs, runs, restore, maintenance,
//! configuration portability and the scheduler).

use crate::backup_engine::{
    api, portability, BackupDestination, BackupJob, BackupManager, BackupOperation, BackupRun,
    Fileset, OperationResult, SchedulerStatus,
};
use crate::backup_engine::defaults::{GlobalDefaults, SavedDestination};
use crate::backup_engine::{DatabaseCleanupReport, DatabaseEntry};
use ufop_core::backup::recovery;
use crate::core::error::AppError;
use crate::governance::ediscovery::{self, EDiscoveryReport, EDiscoveryRequest};
use crate::ledger::OperationLedger;
use crate::security::audit::{AuditTrail, SecurityAudit};
use crate::storage::Repository;
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

/// Change an encrypted job's passphrase in place (repository format 2) and
/// store the new one. `current` defaults to the stored passphrase;
/// `kdf_preset` (interactive | default | sensitive) sets a new Argon2id cost.
#[tauri::command]
pub async fn backup_change_passphrase(
    id: String,
    current: Option<String>,
    new_passphrase: String,
    kdf_preset: Option<String>,
    mgr: State<'_, BackupManager>,
) -> Result<ufop_core::backup::format::FormatReport, AppError> {
    let kdf = match kdf_preset.as_deref().filter(|p| !p.is_empty()) {
        Some(p) => Some(ufop_core::backup::crypto::KdfParams::preset(p).ok_or_else(|| AppError::validation(format!("Unknown key-derivation preset '{p}'")))?),
        None => None,
    };
    mgr.change_passphrase(&id, current, new_passphrase, kdf).await
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
pub async fn backup_list_versions(id: String, source: Option<String>, tags: Option<Vec<String>>, mgr: State<'_, BackupManager>) -> Result<Vec<Fileset>, AppError> {
    mgr.list_versions_in(&id, source, tags.unwrap_or_default()).await
}

#[tauri::command]
pub async fn backup_list_files(id: String, version: usize, prefix: Option<String>, source: Option<String>, mgr: State<'_, BackupManager>) -> Result<Vec<FileEntry>, AppError> {
    mgr.list_files_in(&id, version, prefix, source).await
}

#[tauri::command]
pub async fn backup_search(id: String, pattern: String, version: Option<usize>, source: Option<String>, mgr: State<'_, BackupManager>) -> Result<Vec<FoundFile>, AppError> {
    mgr.search_in(&id, pattern, version, source).await
}

#[tauri::command]
pub async fn backup_compare(id: String, base: usize, compare: usize, source: Option<String>, mgr: State<'_, BackupManager>) -> Result<VersionComparison, AppError> {
    mgr.compare_in(&id, base, compare, source).await
}

/// Sources (jobs / computers) with versions in the job's repository.
#[tauri::command]
pub async fn backup_list_sources(id: String, mgr: State<'_, BackupManager>) -> Result<Vec<ufop_core::backup::shared::SourceInfo>, AppError> {
    mgr.list_sources(&id).await
}

/// Keep/delete decision per version for the job's (or an unsaved) policy.
#[tauri::command]
pub async fn backup_preview_retention(
    id: String,
    policy: Option<ufop_core::backup::RetentionPolicy>,
    source: Option<String>,
    mgr: State<'_, BackupManager>,
) -> Result<ufop_core::backup::maintenance::RetentionPreview, AppError> {
    mgr.preview_retention(&id, policy, source).await
}

/// Change a version's note, tags or pin; returns the operation result
/// (its `report` is the updated version).
#[tauri::command]
pub async fn backup_set_version_metadata(
    id: String,
    version: usize,
    source: Option<String>,
    patch: ufop_core::backup::versions::MetadataPatch,
    mgr: State<'_, BackupManager>,
) -> Result<OperationResult, AppError> {
    mgr.execute(&id, BackupOperation::SetVersionMetadata { version, source, patch }).await
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

/// The machine-wide caps in force (this machine's, with the console's on top).
#[tauri::command]
pub async fn backup_effective_caps(mgr: State<'_, BackupManager>) -> Result<crate::backup_engine::defaults::TransferCaps, AppError> {
    Ok(mgr.effective_caps().await)
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
    read_only: Option<bool>,
    mgr: State<'_, BackupManager>,
) -> Result<BackupJob, AppError> {
    mgr.adopt_repository(destination, passphrase, prefix.unwrap_or_else(|| "ufop".into()), name, sources.unwrap_or_default(), source_map.unwrap_or_default(), read_only.unwrap_or(false))
        .await
}

/// Who holds the job's repository lock(s) right now.
#[tauri::command]
pub async fn backup_lock_status(job_id: String, mgr: State<'_, BackupManager>) -> Result<ufop_core::backup::lock::LockStatus, AppError> {
    mgr.lock_status(&job_id).await
}

/// Force-unlock the job's repository (needs `confirm`; `owner_id` limits it
/// to the lock that was shown).
#[tauri::command]
pub async fn backup_break_lock(job_id: String, confirm: bool, owner_id: Option<String>, mgr: State<'_, BackupManager>) -> Result<ufop_core::backup::lock::BreakLockReport, AppError> {
    mgr.break_lock(&job_id, confirm, owner_id).await
}

/// Turn local-database encryption on or off for a job and migrate now.
#[tauri::command]
pub async fn backup_set_database_encryption(job_id: String, enabled: bool, mgr: State<'_, BackupManager>) -> Result<ufop_core::backup::db::RekeyReport, AppError> {
    mgr.set_database_encryption(&job_id, enabled).await
}

#[tauri::command]
pub async fn backup_database_inventory(mgr: State<'_, BackupManager>) -> Result<Vec<DatabaseEntry>, AppError> {
    mgr.database_inventory().await
}

#[tauri::command]
pub async fn backup_cleanup_databases(delete_orphans: bool, vacuum: bool, mgr: State<'_, BackupManager>) -> Result<DatabaseCleanupReport, AppError> {
    mgr.cleanup_databases(delete_orphans, vacuum).await
}

#[tauri::command]
pub async fn backup_crypt_file(decrypt: bool, input: String, output: String, passphrase: String, preset: Option<String>, mgr: State<'_, BackupManager>) -> Result<ufop_core::backup::filecrypt::CryptReport, AppError> {
    mgr.crypt_file(decrypt, input, output, passphrase, preset).await
}

#[tauri::command]
pub async fn backup_decrypt_volume(volume: String, header: String, output: String, passphrase: Option<String>, mgr: State<'_, BackupManager>) -> Result<ufop_core::backup::filecrypt::CryptReport, AppError> {
    mgr.decrypt_volume(volume, header, output, passphrase).await
}

#[tauri::command]
pub async fn backup_get_secret_settings(mgr: State<'_, BackupManager>) -> Result<crate::backup_engine::secrets::SecretSettings, AppError> {
    mgr.secret_settings().await
}

#[tauri::command]
pub async fn backup_set_secret_settings(settings: crate::backup_engine::secrets::SecretSettings, mgr: State<'_, BackupManager>) -> Result<crate::backup_engine::secrets::SecretSettings, AppError> {
    mgr.set_secret_settings(settings).await
}

#[tauri::command]
pub async fn backup_test_secret_providers(provider_id: Option<String>, names: Option<Vec<String>>, mgr: State<'_, BackupManager>) -> Result<Vec<crate::backup_engine::secrets::ProviderTest>, AppError> {
    mgr.test_secret_providers(provider_id, names.unwrap_or_default()).await
}

#[tauri::command]
pub async fn backup_job_placeholders(job_id: String, mgr: State<'_, BackupManager>) -> Result<crate::backup_engine::secrets::PlaceholderReport, AppError> {
    mgr.job_placeholders(&job_id).await
}

#[tauri::command]
pub async fn backup_test_notification(notification: crate::backup_engine::notify::Notification, mgr: State<'_, BackupManager>) -> Result<(), AppError> {
    mgr.test_notification(notification).await
}

#[tauri::command]
pub async fn backup_bug_report(job_id: String, out_path: String, mgr: State<'_, BackupManager>) -> Result<crate::backup_engine::bugreport::BugReport, AppError> {
    mgr.bug_report(&job_id, out_path).await
}

#[tauri::command]
pub async fn backup_diagnose(job_id: String, network: Option<bool>, throughput_kb: Option<u32>, mgr: State<'_, BackupManager>) -> Result<crate::backup_engine::diagnostics::Diagnostics, AppError> {
    mgr.diagnose(&job_id, network.unwrap_or(true), throughput_kb.unwrap_or(0)).await
}

#[tauri::command]
pub async fn backup_recovery_download(destination: BackupDestination, passphrase: Option<String>, prefix: Option<String>, folder: String, options: Option<recovery::DownloadOptions>, mgr: State<'_, BackupManager>) -> Result<recovery::DownloadReport, AppError> {
    mgr.recovery_download(destination, passphrase, prefix.unwrap_or_else(|| "ufop".into()), folder, options.unwrap_or_default()).await
}

#[tauri::command]
pub async fn backup_recovery_index(folder: String, mgr: State<'_, BackupManager>) -> Result<recovery::IndexReport, AppError> {
    mgr.recovery_index(folder).await
}

#[tauri::command]
pub async fn backup_recovery_list(folder: String, mgr: State<'_, BackupManager>) -> Result<Vec<recovery::RecoveryVersion>, AppError> {
    mgr.recovery_list(folder).await
}

#[tauri::command]
pub async fn backup_recovery_restore(folder: String, version: usize, paths: Option<Vec<String>>, target: String, mgr: State<'_, BackupManager>) -> Result<recovery::RecoveryRestoreReport, AppError> {
    mgr.recovery_restore(folder, version, paths.unwrap_or_default(), target).await
}

#[tauri::command]
pub async fn backup_recovery_convert(folder: String, destination: BackupDestination, options: recovery::ConvertOptions, mgr: State<'_, BackupManager>) -> Result<recovery::ConvertReport, AppError> {
    mgr.recovery_convert(folder, destination, options).await
}

#[tauri::command]
pub async fn backup_remote_list(destination: BackupDestination, mgr: State<'_, BackupManager>) -> Result<Vec<ufop_core::backup::RemoteFile>, AppError> {
    mgr.remote_list(destination).await
}

#[tauri::command]
pub async fn backup_remote_get(destination: BackupDestination, name: String, local_path: String, mgr: State<'_, BackupManager>) -> Result<u64, AppError> {
    mgr.remote_get(destination, name, local_path).await
}

#[tauri::command]
pub async fn backup_remote_put(destination: BackupDestination, local_path: String, name: String, mgr: State<'_, BackupManager>) -> Result<u64, AppError> {
    mgr.remote_put(destination, local_path, name).await
}

#[tauri::command]
pub async fn backup_remote_delete(destination: BackupDestination, name: String, force: Option<bool>, mgr: State<'_, BackupManager>) -> Result<(), AppError> {
    mgr.remote_delete(destination, name, force.unwrap_or(false)).await
}

#[tauri::command]
pub async fn backup_start_operation(id: String, operation: BackupOperation, mgr: State<'_, BackupManager>) -> Result<String, AppError> {
    mgr.start_operation(&id, operation).await
}

#[tauri::command]
pub async fn backup_set_destination_secret(credential_id: String, secret: String, mgr: State<'_, BackupManager>) -> Result<(), AppError> {
    mgr.set_destination_secret(&credential_id, &secret).await
}

#[tauri::command]
pub async fn backup_has_destination_secret(credential_id: String, mgr: State<'_, BackupManager>) -> Result<bool, AppError> {
    mgr.has_destination_secret(&credential_id).await
}

#[tauri::command]
pub async fn backup_cloud_providers() -> Result<Vec<crate::backup_engine::stores::ProviderInfo>, AppError> {
    Ok(crate::backup_engine::stores::PROVIDERS.to_vec())
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

/// Photos libraries found in `~/Pictures` (macOS; empty elsewhere).
#[tauri::command]
pub async fn backup_photos_libraries() -> Result<Vec<ufop_core::backup::photos::LibraryCandidate>, AppError> {
    Ok(api::photos_libraries())
}

/// Asset counts and album titles of a Photos library (path or `photos://` URI).
#[tauri::command]
pub async fn backup_photos_library_info(path: String, temp_dir: Option<String>) -> Result<ufop_core::backup::photos::LibraryInfo, AppError> {
    api::photos_library_info(path, temp_dir).await
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

// ── Legal hold and eDiscovery ───────────────────────────────────────────

/// Place a legal hold on a job: retention is suspended and nothing in the
/// backup set can be removed until it is released. Recorded in the audit log.
#[tauri::command]
pub async fn backup_place_legal_hold(id: String, matter: String, reason: String, mgr: State<'_, BackupManager>, repo: State<'_, Repository>) -> Result<BackupJob, AppError> {
    let job = mgr.place_legal_hold(&id, &matter, &reason, "user").await?;
    SecurityAudit::log(repo.inner(), "legal_hold.place", "user", &format!("backup:{}", job.id), &format!("\"{}\" held for matter {}: {}", job.name, matter.trim(), reason.trim())).await;
    Ok(job)
}

#[tauri::command]
pub async fn backup_release_legal_hold(id: String, mgr: State<'_, BackupManager>, repo: State<'_, Repository>) -> Result<BackupJob, AppError> {
    let (job, hold) = mgr.release_legal_hold(&id).await?;
    SecurityAudit::log(repo.inner(), "legal_hold.release", "user", &format!("backup:{}", job.id), &format!("\"{}\" released from matter {} (placed {} by {})", job.name, hold.matter, hold.placed_at, hold.placed_by)).await;
    Ok(job)
}

/// Write an eDiscovery bundle (audit log, ledger, held backups, optionally
/// restored files) with a SHA-256 manifest. Recorded in the audit log.
#[tauri::command]
pub async fn ediscovery_export(
    request: EDiscoveryRequest,
    mgr: State<'_, BackupManager>,
    ledger: State<'_, OperationLedger>,
    audit: State<'_, AuditTrail>,
    repo: State<'_, Repository>,
) -> Result<EDiscoveryReport, AppError> {
    let report = ediscovery::export(&request, mgr.inner(), ledger.inner(), audit.inner(), "user").await?;
    SecurityAudit::log(repo.inner(), "ediscovery.export", "user", &report.output, &format!("matter {}: {} entries, sha256 {}", request.matter.trim(), report.entries, report.sha256)).await;
    Ok(report)
}
