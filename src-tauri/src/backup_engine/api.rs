//! Shared backup API used by both the Tauri commands and the server's
//! JSON RPC endpoint, so the two surfaces can never drift apart.

use super::{portability, BackupDestination, BackupJob, BackupManager, BackupOperation, Fileset};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ufop_core::backup::{crypto, photos};
use ufop_core::backup::filters::FILTER_GROUPS;
use ufop_core::backup::RestoreOptions;

#[derive(Serialize)]
pub struct ImportPreview {
    pub job: BackupJob,
    pub has_passphrase: bool,
}

#[derive(Serialize)]
pub struct FilterGroupInfo {
    pub name: String,
    pub description: String,
    pub patterns: Vec<String>,
}

#[derive(Serialize)]
pub struct BackupCapabilities {
    pub filter_groups: Vec<FilterGroupInfo>,
    pub connection_protocols: Vec<String>,
    pub default_no_compress_extensions: Vec<String>,
    pub modules: Vec<super::policy::ModuleInfo>,
}

#[derive(Serialize)]
pub struct SourcePreview {
    pub files: u64,
    pub folders: u64,
    pub total_size: u64,
    pub sample: Vec<String>,
    pub warnings: Vec<String>,
}

pub async fn export_config(mgr: &BackupManager, id: &str, include_passphrase: bool, export_passphrase: Option<&str>, include_stats: bool) -> Result<String, AppError> {
    let job = mgr.get_job(id).await?;
    let pass = if include_passphrase { mgr.passphrase(id).await? } else { None };
    let templates = mgr.list_templates().await?.into_iter().filter(|t| job.template_ids.contains(&t.id)).collect();
    portability::export_job_with(&job, pass, export_passphrase, include_stats, templates)
}

pub async fn import_config(mgr: &BackupManager, json: &str, export_passphrase: Option<&str>, import_stats: bool, save: bool) -> Result<ImportPreview, AppError> {
    let (mut job, pass, templates) = portability::import_job_with(json, export_passphrase, import_stats)?;
    let has_passphrase = pass.is_some();
    if save {
        // Templates the job is based on: reuse a local one of the same id or
        // name, otherwise create it; ids the job cannot resolve are dropped.
        let known = mgr.list_templates().await?;
        let mut ids = Vec::new();
        for id in &job.template_ids {
            if known.iter().any(|k| &k.id == id && !k.enforced) {
                ids.push(id.clone());
            } else if let Some(t) = templates.iter().find(|t| &t.id == id) {
                let local = known.iter().find(|k| !k.managed && k.name.eq_ignore_ascii_case(t.name.trim()));
                let saved = match local {
                    Some(l) => l.clone(),
                    None => mgr.save_template(super::templates::PolicyTemplate { id: String::new(), ..t.clone() }).await?,
                };
                ids.push(saved.id);
            }
        }
        job.template_ids = ids;
    }
    let job = if save { mgr.save_job(job, pass).await? } else { job };
    Ok(ImportPreview { job, has_passphrase })
}

pub fn capabilities(mgr: &BackupManager) -> BackupCapabilities {
    let policy = mgr.module_policy();
    BackupCapabilities {
        filter_groups: FILTER_GROUPS
            .iter()
            .map(|(n, d, p)| FilterGroupInfo { name: n.to_string(), description: d.to_string(), patterns: p.iter().map(|s| s.to_string()).collect() })
            .collect(),
        connection_protocols: super::connector_backend::BACKUP_PROTOCOLS
            .iter()
            .filter(|p| policy.check_job(p, true, true).is_ok())
            .map(|s| s.to_string())
            .collect(),
        default_no_compress_extensions: ufop_core::backup::default_no_compress_extensions(),
        modules: policy.modules(),
    }
}

/// Count what the job's sources and filters select, without backing up.
pub async fn preview_sources(job: BackupJob) -> Result<SourcePreview, AppError> {
    tokio::task::spawn_blocking(move || {
        let filter = job.filter.clone().compile()?;
        let sources: Vec<std::path::PathBuf> = job.sources.iter().filter(|s| !s.trim().is_empty()).map(Into::into).collect();
        let (entries, warnings) = ufop_core::backup::engine::scan_sources(&sources, &filter, &job.options)?;
        let mut p = SourcePreview { files: 0, folders: 0, total_size: 0, sample: Vec::new(), warnings };
        for (_, e) in &entries {
            match e.kind {
                ufop_core::backup::EntryKind::Dir => p.folders += 1,
                _ => {
                    p.files += 1;
                    p.total_size += e.size;
                    if p.sample.len() < 50 {
                        p.sample.push(e.path.clone());
                    }
                }
            }
        }
        Ok(p)
    })
    .await
    .map_err(|e| AppError::internal(e.to_string()))?
}

/// Photos libraries in the user's Pictures folder. Only macOS has them;
/// elsewhere the list is empty and the user chooses a library path.
pub fn photos_libraries() -> Vec<photos::LibraryCandidate> {
    if !cfg!(target_os = "macos") {
        return Vec::new();
    }
    dirs::picture_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Pictures")))
        .map(|d| photos::find_libraries(&d))
        .unwrap_or_default()
}

/// Asset counts and album titles of a library (`path` or `photos://` URI).
pub async fn photos_library_info(path: String, temp_dir: Option<String>) -> Result<photos::LibraryInfo, AppError> {
    let lib = photos::parse_source(&path).unwrap_or_else(|| std::path::PathBuf::from(path.trim()));
    if lib.as_os_str().is_empty() {
        return Err(AppError::validation("Choose a Photos library"));
    }
    let tmp = temp_dir.filter(|t| !t.trim().is_empty()).map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    tokio::task::spawn_blocking(move || photos::library_info(&lib, &tmp)).await.map_err(|e| AppError::internal(e.to_string()))?
}

fn arg<T: for<'de> Deserialize<'de>>(args: &Value, key: &str) -> Result<T, AppError> {
    serde_json::from_value(args.get(key).cloned().unwrap_or(Value::Null))
        .map_err(|e| AppError::validation(format!("Invalid argument '{key}': {e}")))
}

fn opt<T: for<'de> Deserialize<'de>>(args: &Value, key: &str) -> Result<Option<T>, AppError> {
    match args.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(_) => arg(args, key).map(Some),
    }
}

fn ok<T: Serialize>(v: T) -> Result<Value, AppError> {
    Ok(serde_json::to_value(v)?)
}

/// Execute a backup command by its Tauri name with camelCase JSON args
/// (exactly what the frontend sends to `invoke`).
pub async fn dispatch(mgr: &BackupManager, command: &str, a: &Value) -> Result<Value, AppError> {
    // Boxed: the match holds a future per command, too large for a caller's stack frame.
    Box::pin(dispatch_inner(mgr, command, a)).await
}

async fn dispatch_inner(mgr: &BackupManager, command: &str, a: &Value) -> Result<Value, AppError> {
    let prefix = || opt::<String>(a, "prefix").map(|p| p.unwrap_or_else(|| "ufop".into()));
    match command {
        "backup_list_jobs" => ok(mgr.list_jobs().await),
        "backup_get_job" => ok(mgr.get_job(&arg::<String>(a, "id")?).await?),
        "backup_save_job" => ok(mgr.save_job(arg(a, "job")?, opt(a, "passphrase")?).await?),
        "backup_delete_job" => ok(mgr.delete_job(&arg::<String>(a, "id")?, arg(a, "deleteLocalDb")?, arg(a, "deleteRemote")?).await?),
        "backup_test_destination" => ok(mgr.test_destination(&arg::<BackupDestination>(a, "destination")?, arg(a, "create")?).await?),
        "backup_test_destination_compat" => ok(mgr
            .test_destination_compat(&arg::<BackupDestination>(a, "destination")?, opt(a, "files")?.unwrap_or(3), opt(a, "maxSize")?.unwrap_or(1 << 20))
            .await?),
        "backup_run" => ok(mgr.start_backup(&arg::<String>(a, "id")?, opt(a, "dryRun")?.unwrap_or(false)).await?),
        "backup_execute" => {
            let op: BackupOperation = arg(a, "operation")?;
            if matches!(op, BackupOperation::Backup { .. }) {
                return Err(AppError::validation("Use backup_run to start a backup"));
            }
            ok(mgr.execute(&arg::<String>(a, "id")?, op).await?)
        }
        "backup_cancel" => ok(mgr.cancel(&arg::<String>(a, "id")?)),
        "backup_status" => ok(mgr.status().await),
        "backup_pause" => {
            mgr.pause(opt(a, "minutes")?);
            ok(mgr.status().await)
        }
        "backup_resume" => {
            mgr.resume();
            ok(mgr.status().await)
        }
        "backup_list_versions" => ok(mgr.list_versions_in(&arg::<String>(a, "id")?, opt(a, "source")?, opt(a, "tags")?.unwrap_or_default()).await?),
        "backup_list_files" => ok(mgr.list_files_in(&arg::<String>(a, "id")?, arg(a, "version")?, opt(a, "prefix")?, opt(a, "source")?).await?),
        "backup_search" => ok(mgr.search_in(&arg::<String>(a, "id")?, arg(a, "pattern")?, opt(a, "version")?, opt(a, "source")?).await?),
        "backup_compare" => ok(mgr.compare_in(&arg::<String>(a, "id")?, arg(a, "base")?, arg(a, "compare")?, opt(a, "source")?).await?),
        "backup_list_sources" => ok(mgr.list_sources(&arg::<String>(a, "id")?).await?),
        "backup_preview_retention" => ok(mgr.preview_retention(&arg::<String>(a, "id")?, opt(a, "policy")?, opt(a, "source")?).await?),
        "backup_set_version_metadata" => ok(mgr
            .execute(&arg::<String>(a, "id")?, BackupOperation::SetVersionMetadata { version: arg(a, "version")?, source: opt(a, "source")?, patch: arg(a, "patch")? })
            .await?),
        "backup_run_all" => ok(mgr.run_all(opt(a, "filter")?.unwrap_or_default()).await?),
        "backup_estimate" => ok(mgr.estimate(&arg::<String>(a, "jobId")?, opt(a, "assumedBytesPerSec")?).await?),
        "backup_run_history" => ok(mgr.run_history(opt(a, "jobId")?, opt(a, "limit")?.unwrap_or(50)).await?),
        "backup_prune_history" => ok(mgr.prune_history(arg::<u32>(a, "days")?.max(1)).await?),
        "backup_export_config" => ok(export_config(
            mgr,
            &arg::<String>(a, "id")?,
            arg(a, "includePassphrase")?,
            opt::<String>(a, "exportPassphrase")?.as_deref(),
            opt(a, "includeStats")?.unwrap_or(false),
        )
        .await?),
        "backup_import_config" => ok(import_config(
            mgr,
            &arg::<String>(a, "json")?,
            opt::<String>(a, "exportPassphrase")?.as_deref(),
            opt(a, "importStats")?.unwrap_or(false),
            arg(a, "save")?,
        )
        .await?),
        "backup_export_is_encrypted" => ok(portability::is_encrypted_export(&arg::<String>(a, "json")?)),
        "backup_export_cli" => ok(portability::cli_command(&mgr.get_job(&arg::<String>(a, "id")?).await?)),
        "backup_browse_remote" => ok::<Vec<Fileset>>(mgr.browse_remote(arg(a, "destination")?, opt(a, "passphrase")?, prefix()?).await?),
        "backup_direct_restore" => ok(mgr.direct_restore(arg(a, "destination")?, opt(a, "passphrase")?, prefix()?, arg::<RestoreOptions>(a, "options")?).await?),
        "backup_discover_remote_config" => ok(mgr.discover_remote_config(arg(a, "destination")?, opt(a, "passphrase")?, prefix()?).await?),
        "backup_passphrase_strength" => ok(crypto::passphrase_strength(&arg::<String>(a, "passphrase")?)),
        "backup_generate_passphrase" => ok(crypto::generate_passphrase(opt(a, "length")?.unwrap_or(32))),
        "backup_capabilities" => ok(capabilities(mgr)),
        "backup_preview_sources" => ok(preview_sources(arg(a, "job")?).await?),
        "backup_photos_libraries" => ok(photos_libraries()),
        "backup_photos_library_info" => ok(photos_library_info(arg(a, "path")?, opt(a, "tempDir")?).await?),
        "backup_get_global_defaults" => ok(mgr.global_defaults().await?),
        "backup_set_global_defaults" => ok(mgr.set_global_defaults(arg(a, "defaults")?).await?),
        "backup_effective_job" => ok(mgr.effective_job(&arg::<String>(a, "id")?).await?),
        "backup_list_destinations" => ok(mgr.list_destinations().await?),
        "backup_save_destination" => ok(mgr.save_destination(arg(a, "destination")?).await?),
        "backup_delete_destination" => ok(mgr.delete_destination(&arg::<String>(a, "id")?).await?),
        "backup_database_inventory" => ok(mgr.database_inventory().await?),
        "backup_cleanup_databases" => ok(mgr.cleanup_databases(arg(a, "deleteOrphans")?, arg(a, "vacuum")?).await?),
        "backup_crypt_file" => ok(mgr.crypt_file(arg(a, "decrypt")?, arg(a, "input")?, arg(a, "output")?, arg(a, "passphrase")?, opt(a, "preset")?).await?),
        "backup_decrypt_volume" => ok(mgr.decrypt_volume(arg(a, "volume")?, arg(a, "header")?, arg(a, "output")?, opt(a, "passphrase")?).await?),
        "backup_get_secret_settings" => ok(mgr.secret_settings().await?),
        "backup_set_secret_settings" => ok(mgr.set_secret_settings(arg(a, "settings")?).await?),
        "backup_test_secret_providers" => ok(mgr.test_secret_providers(opt(a, "providerId")?, opt(a, "names")?.unwrap_or_default()).await?),
        "backup_job_placeholders" => ok(mgr.job_placeholders(&arg::<String>(a, "jobId")?).await?),
        "backup_test_notification" => ok(mgr.test_notification(arg(a, "notification")?).await?),
        "backup_bug_report" => ok(mgr.bug_report(&arg::<String>(a, "jobId")?, arg(a, "outPath")?).await?),
        "backup_diagnose" => ok(mgr.diagnose(&arg::<String>(a, "jobId")?, opt::<bool>(a, "network")?.unwrap_or(true), opt::<u32>(a, "throughputKb")?.unwrap_or(0)).await?),
        "backup_recovery_download" => ok(mgr.recovery_download(arg(a, "destination")?, opt(a, "passphrase")?, prefix()?, arg(a, "folder")?, opt(a, "options")?.unwrap_or_default()).await?),
        "backup_recovery_index" => ok(mgr.recovery_index(arg(a, "folder")?).await?),
        "backup_recovery_list" => ok(mgr.recovery_list(arg(a, "folder")?).await?),
        "backup_recovery_restore" => ok(mgr.recovery_restore(arg(a, "folder")?, arg(a, "version")?, opt(a, "paths")?.unwrap_or_default(), arg(a, "target")?).await?),
        "backup_recovery_convert" => ok(mgr.recovery_convert(arg(a, "folder")?, arg(a, "destination")?, arg(a, "options")?).await?),
        "backup_remote_list" => ok(mgr.remote_list(arg(a, "destination")?).await?),
        "backup_remote_get" => ok(mgr.remote_get(arg(a, "destination")?, arg(a, "name")?, arg(a, "localPath")?).await?),
        "backup_remote_put" => ok(mgr.remote_put(arg(a, "destination")?, arg(a, "localPath")?, arg(a, "name")?).await?),
        "backup_remote_delete" => ok(mgr.remote_delete(arg(a, "destination")?, arg(a, "name")?, opt(a, "force")?.unwrap_or(false)).await?),
        "backup_start_operation" => ok(mgr.start_operation(&arg::<String>(a, "id")?, arg(a, "operation")?).await?),
        "backup_set_destination_secret" => ok(mgr.set_destination_secret(&arg::<String>(a, "credentialId")?, &arg::<String>(a, "secret")?).await?),
        "backup_has_destination_secret" => ok(mgr.has_destination_secret(&arg::<String>(a, "credentialId")?).await?),
        "backup_cloud_providers" => ok(super::stores::PROVIDERS.to_vec()),
        "backup_lock_status" => ok(mgr.lock_status(&arg::<String>(a, "jobId")?).await?),
        "backup_break_lock" => ok(mgr.break_lock(&arg::<String>(a, "jobId")?, opt(a, "confirm")?.unwrap_or(false), opt(a, "ownerId")?).await?),
        "backup_set_database_encryption" => ok(mgr.set_database_encryption(&arg::<String>(a, "jobId")?, arg(a, "enabled")?).await?),
        "backup_adopt_repository" => ok(mgr
            .adopt_repository(
                arg(a, "destination")?,
                opt(a, "passphrase")?,
                prefix()?,
                opt(a, "name")?,
                opt(a, "sources")?.unwrap_or_default(),
                opt(a, "sourceMap")?.unwrap_or_default(),
                opt(a, "readOnly")?.unwrap_or(false),
            )
            .await?),
        "backup_resolve_version" => ok(mgr.resolve_version(&arg::<String>(a, "id")?, arg(a, "selector")?, opt(a, "source")?).await?),
        "backup_read_file" => ok(mgr
            .read_file(&arg::<String>(a, "id")?, super::access::selector_of(opt(a, "version")?, opt(a, "selector")?), arg(a, "path")?, opt(a, "offset")?.unwrap_or(0), opt(a, "length")?, opt(a, "source")?)
            .await?),
        "backup_export_file" => ok(mgr
            .export_file(&arg::<String>(a, "id")?, super::access::selector_of(opt(a, "version")?, opt(a, "selector")?), arg(a, "path")?, arg(a, "dest")?, opt(a, "source")?)
            .await?),
        "backup_cache_stats" => ok(mgr.cache_stats().await?),
        "backup_clear_cache" => ok(mgr.clear_cache().await?),
        "backup_get_cache_settings" => ok(mgr.cache_settings().await?),
        "backup_set_cache_settings" => ok(mgr.set_cache_settings(arg(a, "settings")?).await?),
        "backup_list_templates" => ok(mgr.list_templates().await?),
        "backup_get_template" => ok(mgr.get_template(&arg::<String>(a, "id")?).await?),
        "backup_save_template" => ok(mgr.save_template(arg(a, "template")?).await?),
        "backup_delete_template" => ok(mgr.delete_template(&arg::<String>(a, "id")?).await?),
        "backup_apply_template" => ok(mgr.apply_template(&arg::<String>(a, "templateId")?, &arg::<String>(a, "jobId")?).await?),
        "backup_export_templates" => ok(mgr.export_templates(&opt::<Vec<String>>(a, "ids")?.unwrap_or_default()).await?),
        "backup_import_templates" => ok(mgr.import_templates(&arg::<String>(a, "json")?).await?),
        "backup_effective_settings" => ok(mgr.effective_settings(opt::<String>(a, "id")?.as_deref(), opt(a, "job")?).await?),
        "backup_schedule_preview" => ok(arg::<super::schedule::BackupSchedule>(a, "schedule")?.preview(chrono::Utc::now(), opt(a, "count")?.unwrap_or(5))?),
        other => Err(AppError::validation(format!("Unknown command '{other}'"))),
    }
}

/// Every command `dispatch` serves (kept in sync with lib.rs by a test).
pub const COMMANDS: &[&str] = &[
    "backup_list_jobs", "backup_get_job", "backup_save_job", "backup_delete_job", "backup_test_destination",
    "backup_test_destination_compat", "backup_run", "backup_execute", "backup_cancel", "backup_status", "backup_pause",
    "backup_resume", "backup_list_versions", "backup_list_files", "backup_search", "backup_compare", "backup_run_history",
    "backup_prune_history", "backup_export_config", "backup_import_config", "backup_export_is_encrypted", "backup_export_cli",
    "backup_browse_remote", "backup_direct_restore", "backup_discover_remote_config", "backup_passphrase_strength",
    "backup_generate_passphrase", "backup_capabilities", "backup_preview_sources", "backup_get_global_defaults",
    "backup_set_global_defaults", "backup_effective_job", "backup_list_destinations", "backup_save_destination",
    "backup_delete_destination", "backup_adopt_repository", "backup_database_inventory", "backup_cleanup_databases", "backup_bug_report", "backup_diagnose", "backup_test_notification", "backup_get_secret_settings", "backup_set_secret_settings", "backup_crypt_file", "backup_decrypt_volume", "backup_test_secret_providers", "backup_job_placeholders", "backup_recovery_download", "backup_recovery_index", "backup_recovery_list", "backup_recovery_restore", "backup_recovery_convert", "backup_remote_list", "backup_remote_get", "backup_remote_put", "backup_remote_delete", "backup_start_operation", "backup_set_destination_secret", "backup_has_destination_secret", "backup_cloud_providers",
    "backup_photos_libraries", "backup_photos_library_info", "backup_lock_status", "backup_break_lock", "backup_set_database_encryption",
    "backup_list_sources", "backup_preview_retention", "backup_set_version_metadata", "backup_run_all", "backup_estimate",
    "backup_resolve_version", "backup_read_file", "backup_export_file", "backup_cache_stats", "backup_clear_cache",
    "backup_get_cache_settings", "backup_set_cache_settings", "backup_schedule_preview", "backup_list_templates", "backup_get_template",
    "backup_save_template", "backup_delete_template", "backup_apply_template", "backup_export_templates", "backup_import_templates",
    "backup_effective_settings",
];
