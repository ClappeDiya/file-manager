//! Shared backup API used by both the Tauri commands and the server's
//! JSON RPC endpoint, so the two surfaces can never drift apart.

use super::{portability, BackupDestination, BackupJob, BackupManager, BackupOperation, Fileset};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use ufop_core::backup::crypto;
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
    portability::export_job(&job, pass, export_passphrase, include_stats)
}

pub async fn import_config(mgr: &BackupManager, json: &str, export_passphrase: Option<&str>, import_stats: bool, save: bool) -> Result<ImportPreview, AppError> {
    let (job, pass) = portability::import_job(json, export_passphrase, import_stats)?;
    let has_passphrase = pass.is_some();
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
        "backup_list_versions" => ok(mgr.list_versions(&arg::<String>(a, "id")?).await?),
        "backup_list_files" => ok(mgr.list_files(&arg::<String>(a, "id")?, arg(a, "version")?, opt(a, "prefix")?).await?),
        "backup_search" => ok(mgr.search(&arg::<String>(a, "id")?, arg(a, "pattern")?, opt(a, "version")?).await?),
        "backup_compare" => ok(mgr.compare(&arg::<String>(a, "id")?, arg(a, "base")?, arg(a, "compare")?).await?),
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
        "backup_get_global_defaults" => ok(mgr.global_defaults().await?),
        "backup_set_global_defaults" => ok(mgr.set_global_defaults(arg(a, "defaults")?).await?),
        "backup_effective_job" => ok(mgr.effective_job(&arg::<String>(a, "id")?).await?),
        "backup_list_destinations" => ok(mgr.list_destinations().await?),
        "backup_save_destination" => ok(mgr.save_destination(arg(a, "destination")?).await?),
        "backup_delete_destination" => ok(mgr.delete_destination(&arg::<String>(a, "id")?).await?),
        "backup_adopt_repository" => ok(mgr
            .adopt_repository(
                arg(a, "destination")?,
                opt(a, "passphrase")?,
                prefix()?,
                opt(a, "name")?,
                opt(a, "sources")?.unwrap_or_default(),
                opt(a, "sourceMap")?.unwrap_or_default(),
            )
            .await?),
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
    "backup_delete_destination", "backup_adopt_repository",
];
