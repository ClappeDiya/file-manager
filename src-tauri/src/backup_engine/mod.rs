//! Backup engine: named backup jobs on top of the deduplicating, encrypted
//! repository engine in `ufop_core::backup`.
//!
//! This module adds what the desktop app needs around the core engine:
//! persisted job definitions (`backup_jobs`), passphrases in the OS
//! credential store, local-folder and connector destinations, a serialized
//! task queue with live progress and cancellation, a scheduler with
//! missed-run catch-up and pause/resume, pre/post scripts, HTTP report
//! callbacks, run history (`backup_runs`) and the operation ledger.

pub mod legal_hold;
pub mod paging;
pub mod access;
pub mod api;
pub mod bugreport;
pub mod connector_backend;
pub mod defaults;
pub mod diagnostics;
pub mod drives;
pub mod hardening;
pub mod hooks;
pub mod logs;
/// The channel dispatcher is shared with sync and automation (`crate::notify_hub`).
pub use crate::notify_hub as notify;
mod profile_store;
pub mod policy;
pub mod portability;
pub mod runs;
pub mod power;
pub mod schedule;
pub mod metrics;
pub mod metrics_exporter;
pub mod secrets;
pub mod shutdown;
pub mod templates;
pub mod stores;
pub mod versions;

use crate::connectors::ConnectionManager;
use crate::core::error::AppError;
use crate::ledger::{LedgerEngine, LedgerStatus, OperationLedger, RecordEvent};
use crate::security::settings_store::SettingsStore;
use crate::security::CredentialStore;
use crate::storage::pool::DbPool;
use chrono::{DateTime, Utc};
use hooks::{LogLine, ReportConfig, ScriptConfig};
use rusqlite::params;
use schedule::BackupSchedule;
use serde::{Deserialize, Serialize};
use tracing::Instrument;
use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tokio::sync::{RwLock, Semaphore};
use ufop_core::backup::backend::{RateLimitedBackend, RequestLimits, RetryPolicy, Throttle};
use ufop_core::backup::crypto::KdfParams;
use ufop_core::backup::db::{BackupDb, DbKey, KeyCheck};
use ufop_core::backup::lock::{self as repo_lock, LockKind, LockOptions};
use ufop_core::backup::engine::Repo;
use ufop_core::backup::recovery;
use ufop_core::backup::{
    maintenance, Backend, BackupOptions, LocalBackend, OpContext, Progress, ReadOnlyBackend, RestoreOptions, RetentionPolicy, RetryingBackend,
    SoftDeleteBackend, SourceFilter,
};

/// Credential-store key prefix of a job's local-database key.
const DB_KEY_PREFIX: &str = "backup-dbkey:";
/// Program name recorded in repository locks taken by the app and server.
const LOCK_APP: &str = "ufop-app";

/// Lease and clock-check settings for a job.
fn lock_options(o: &BackupOptions) -> LockOptions {
    LockOptions::from_backup_options(o, LOCK_APP)
}

/// Append `extra` to a result's `warnings` array (created when missing).
fn add_warnings(v: &mut serde_json::Value, extra: Vec<String>) {
    if extra.is_empty() {
        return;
    }
    if let Some(o) = v.as_object_mut() {
        let w = o.entry("warnings").or_insert_with(|| serde_json::Value::Array(Vec::new()));
        if let Some(a) = w.as_array_mut() {
            a.extend(extra.into_iter().map(serde_json::Value::String));
        }
    }
}

pub use ufop_core::backup::Fileset;

/// How [`BackupManager::adopt_repository`] sets up the adopted job.
#[derive(Debug, Clone, Default)]
pub struct AdoptOptions {
    /// Job name (default: the stored configuration's, else the destination's).
    pub name: Option<String>,
    /// Source folders, when they differ from the stored configuration's.
    pub sources: Vec<String>,
    /// Old source folder → new one (a computer replaced or folders moved).
    pub source_map: Vec<(String, String)>,
    /// Restore and browse only.
    pub read_only: bool,
}

/// Where a job's volumes are stored.
// Configuration values (cloned, serialized), not hot data: the size of the
// cloud variant does not matter, and boxing it would change every place
// that builds or matches one.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupDestination {
    /// Local disk, removable drive or mounted share (UNC paths included).
    Local {
        path: String,
        /// Fallback paths tried in order; `*` matches a drive letter/mount.
        #[serde(default)]
        alternate_paths: Vec<String>,
        /// Only accept a path containing this marker file.
        #[serde(default)]
        marker_file: Option<String>,
    },
    /// A saved connection (SFTP, FTP/FTPS, WebDAV, S3).
    Connection {
        connection_id: String,
        #[serde(default)]
        remote_path: String,
    },
    /// A native cloud / network store (S3, Azure, GCS, B2, Swift, Dropbox,
    /// Google Drive, Box, pCloud, OneDrive/SharePoint, Tahoe-LAFS, rclone,
    /// SMB). The secret lives in the credential store under `credential_id`.
    Cloud {
        config: stores::CloudConfig,
        #[serde(default)]
        credential_id: String,
    },
    /// A named entry from the destination library; `remote_path` is an
    /// optional sub-folder below it (so jobs can share one server).
    Library {
        destination_id: String,
        #[serde(default)]
        remote_path: String,
    },
}

impl BackupDestination {
    pub fn describe(&self) -> String {
        match self {
            Self::Local { path, .. } => path.clone(),
            Self::Connection { connection_id, remote_path } => format!("connection {connection_id}:{remote_path}"),
            Self::Library { destination_id, remote_path } => format!("library {destination_id}:{remote_path}"),
            Self::Cloud { config, .. } => config.describe(),
        }
    }

    /// The time an S3 destination is read at (a read-only view), if set.
    pub fn point_in_time(&self) -> Option<chrono::DateTime<chrono::Utc>> {
        match self {
            Self::Cloud { config: stores::CloudConfig::S3(c), .. } => c.point_in_time,
            _ => None,
        }
    }
}

/// How a local / mounted-share destination writes files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalTransfer {
    /// Write to a temporary name, then rename (atomic). Off writes directly.
    pub use_move: bool,
    /// Re-read the size after writing.
    pub verify_size: bool,
    /// Flush every file to disk.
    pub sync_writes: bool,
}

impl Default for LocalTransfer {
    fn default() -> Self {
        Self { use_move: true, verify_size: true, sync_writes: false }
    }
}

/// When a replica is brought up to date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ReplicaSchedule {
    /// After every successful backup.
    #[default]
    AfterBackup,
    /// Every `minutes`, independent of backups.
    Interval { minutes: u32 },
    /// Only when started by hand.
    Manual,
}

/// An extra destination holding a full copy of the backup.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Replica {
    pub id: String,
    pub name: String,
    pub destination: BackupDestination,
    pub enabled: bool,
    pub schedule: ReplicaSchedule,
    /// Remove files the primary no longer has (deleted versions).
    pub delete_extraneous: bool,
    /// Read every copied file back and compare.
    pub verify: bool,
    pub throttle: Throttle,
    pub last_run: Option<String>,
    pub last_result: Option<String>,
}

impl Default for Replica {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            destination: BackupDestination::Local { path: String::new(), alternate_paths: Vec::new(), marker_file: None },
            enabled: true,
            schedule: ReplicaSchedule::AfterBackup,
            delete_extraneous: true,
            verify: false,
            throttle: Throttle::default(),
            last_run: None,
            last_result: None,
        }
    }
}

/// Last-run statistics shown on the dashboard.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct JobStats {
    pub last_run_at: Option<String>,
    pub last_result: Option<String>,
    pub last_duration_ms: Option<u64>,
    pub last_error: Option<String>,
    pub last_scheduled_at: Option<String>,
    pub version_count: u64,
    pub source_size: u64,
    pub backend_size: u64,
    /// Last scheduled start of each maintenance task (`compact`, `verify`, `apply_retention`).
    pub last_maintenance: BTreeMap<String, String>,
    /// Result of the last ransomware-hardening destination probe ("Run checks").
    pub last_hardening_probe: Option<hardening::ProbeResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupJob {
    pub id: String,
    pub name: String,
    pub description: String,
    pub sources: Vec<String>,
    pub filter: SourceFilter,
    pub destination: BackupDestination,
    /// Encrypt volumes with the passphrase kept in the credential store.
    pub encrypted: bool,
    pub options: BackupOptions,
    pub retention: RetentionPolicy,
    pub schedule: BackupSchedule,
    pub scripts: ScriptConfig,
    pub reports: ReportConfig,
    pub throttle: Throttle,
    pub retry: RetryPolicy,
    /// Rename volumes with this prefix instead of deleting them.
    pub soft_delete_prefix: Option<String>,
    /// Embed this job's configuration in remote file lists.
    pub store_config_remotely: bool,
    /// Keep report URLs and script commands in the embedded configuration
    /// (only for encrypted backups, where the file list is encrypted too).
    #[serde(default)]
    pub remote_config_include_secrets: bool,
    /// Per-operation timeout for network destinations (0 = none).
    #[serde(default)]
    pub network_timeout_secs: u64,
    /// Write behaviour for local / mounted-share destinations.
    #[serde(default)]
    pub local_transfer: LocalTransfer,
    /// Request-rate limits and listing parallelism for the destination.
    #[serde(default)]
    pub request_limits: RequestLimits,
    /// Extra destinations kept in sync with the primary.
    #[serde(default)]
    pub replicas: Vec<Replica>,
    /// Email / Telegram / XMPP notifications.
    #[serde(default)]
    pub notifications: Vec<notify::Notification>,
    /// Reusable notification profiles this job also sends to (ids).
    #[serde(default)]
    pub notification_profile_ids: Vec<String>,
    /// Log file and system log outputs.
    #[serde(default)]
    pub log_outputs: logs::LogOutputs,
    /// Secret providers used by this job besides the global ones (ids).
    #[serde(default)]
    pub secret_providers: Vec<String>,
    /// Resolved secret values, masked in run logs (never serialized).
    #[serde(skip)]
    pub(crate) masked_values: Vec<String>,
    /// Keep the computer awake while this job's operations run.
    pub prevent_sleep: bool,
    /// Results that raise a desktop notification: `success`, `warning`, `error`.
    pub notify_on: Vec<String>,
    pub created_at: String,
    pub updated_at: String,
    pub stats: JobStats,
    /// While set, nothing in the backup set can be removed (see `legal_hold`).
    #[serde(default)]
    pub legal_hold: Option<legal_hold::LegalHold>,
    /// Restore and browse only: backups, maintenance and anything else
    /// that writes to the destination is refused, and the connection
    /// itself refuses writes.
    #[serde(default)]
    pub read_only: bool,
    /// Encrypt the local database with a key kept in the credential store.
    /// New jobs: on for encrypted backups. `None` on jobs created before
    /// this option existed: they stay plain until it is turned on.
    #[serde(default)]
    pub encrypt_local_database: Option<bool>,
    /// A command whose output each backup stores as a virtual file.
    #[serde(default)]
    pub stdin_command: Option<hooks::StdinCommand>,
    /// Policy templates this job inherits from, in order (later wins).
    #[serde(default)]
    pub template_ids: Vec<String>,
    /// Settings the job sets itself (field paths such as `retention` or
    /// `options.compression_level`); every other setting is inherited.
    /// `None` (jobs from before templates): every setting that differs from
    /// the built-in default counts as set by the job.
    #[serde(default)]
    pub overrides: Option<Vec<String>>,
    /// Compaction, verification and retention on their own schedules.
    #[serde(default)]
    pub maintenance: schedule::MaintenanceSchedule,
}

impl Default for BackupJob {
    fn default() -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: String::new(),
            name: String::new(),
            description: String::new(),
            sources: Vec::new(),
            filter: SourceFilter::default(),
            destination: BackupDestination::Local { path: String::new(), alternate_paths: Vec::new(), marker_file: None },
            encrypted: true,
            options: BackupOptions::default(),
            retention: RetentionPolicy::default(),
            schedule: BackupSchedule::default(),
            scripts: ScriptConfig::default(),
            reports: ReportConfig::default(),
            throttle: Throttle::default(),
            retry: RetryPolicy { retries: 3, delay_ms: 2000, exponential: true, max_delay_ms: 60_000 },
            soft_delete_prefix: None,
            store_config_remotely: false,
            remote_config_include_secrets: false,
            network_timeout_secs: 0,
            local_transfer: LocalTransfer::default(),
            request_limits: RequestLimits::default(),
            replicas: Vec::new(),
            notifications: Vec::new(),
            notification_profile_ids: Vec::new(),
            log_outputs: logs::LogOutputs::default(),
            secret_providers: Vec::new(),
            masked_values: Vec::new(),
            prevent_sleep: true,
            notify_on: vec!["warning".into(), "error".into()],
            created_at: now.clone(),
            updated_at: now,
            stats: JobStats::default(),
            legal_hold: None,
            read_only: false,
            encrypt_local_database: None,
            stdin_command: None,
            template_ids: Vec::new(),
            overrides: None,
            maintenance: schedule::MaintenanceSchedule::default(),
        }
    }
}

impl BackupJob {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.name.trim().is_empty() {
            return Err(AppError::validation("Give the backup job a name"));
        }
        let has_command = self.stdin_command.as_ref().is_some_and(|c| c.is_set());
        if self.sources.iter().all(|s| s.trim().is_empty()) && !has_command {
            return Err(AppError::validation("Add at least one source folder"));
        }
        if let Some(c) = &self.stdin_command {
            c.validate()?;
        }
        if self.options.rehash_percent > 100 {
            return Err(AppError::validation("The rehash percentage must be between 0 and 100"));
        }
        self.request_limits.validate()?;
        if self.destination.point_in_time().is_some() && !self.read_only {
            return Err(AppError::validation("A destination opened at a point in time is read-only: turn on 'Read-only' for this job"));
        }
        if let Some(k) = &self.options.kdf {
            k.validate()?;
        }
        if self.options.encryption_module == ufop_core::backup::EncryptionModuleKind::Gpg && !self.encrypted {
            return Err(AppError::validation("GPG encryption needs encryption turned on"));
        }
        match &self.destination {
            BackupDestination::Local { path, .. } if path.trim().is_empty() => {
                return Err(AppError::validation("Choose a destination folder"))
            }
            BackupDestination::Connection { connection_id, .. } if uuid::Uuid::parse_str(connection_id).is_err() => {
                return Err(AppError::validation("Choose a saved connection as destination"))
            }
            BackupDestination::Library { destination_id, .. } if destination_id.trim().is_empty() => {
                return Err(AppError::validation("Choose a destination from the library"))
            }
            _ => {}
        }
        for r in &self.replicas {
            if r.destination == self.destination {
                return Err(AppError::validation(format!("Replica '{}' points at the primary destination", r.name)));
            }
            match &r.destination {
                BackupDestination::Local { path, .. } if path.trim().is_empty() => return Err(AppError::validation(format!("Choose a folder for replica '{}'", r.name))),
                BackupDestination::Connection { connection_id, .. } if uuid::Uuid::parse_str(connection_id).is_err() => {
                    return Err(AppError::validation(format!("Choose a connection for replica '{}'", r.name)))
                }
                _ => {}
            }
            if let ReplicaSchedule::Interval { minutes } = r.schedule {
                if minutes < 5 {
                    return Err(AppError::validation("Replica intervals must be at least 5 minutes"));
                }
            }
        }
        self.log_outputs.file_filter.validate()?;
        self.log_outputs.system_filter.validate()?;
        for n in &self.notifications {
            n.validate()?;
        }
        notify::webhook::validate_headers(&self.reports.headers)?;
        if self.remote_config_include_secrets && !self.encrypted {
            return Err(AppError::validation("Secrets can only be stored in the backup when it is encrypted"));
        }
        for s in &self.sources {
            let library = ufop_core::backup::photos::parse_source(s);
            if ufop_core::backup::photos::is_photos_source(s) && library.is_none() {
                return Err(AppError::validation("A Photos source needs the path of a .photoslibrary bundle (photos:///Users/me/Pictures/Photos Library.photoslibrary)"));
            }
            if let BackupDestination::Local { path, .. } = &self.destination {
                let s = library.unwrap_or_else(|| PathBuf::from(s));
                let (s, p) = (s.as_path(), Path::new(path));
                if p.starts_with(s) {
                    return Err(AppError::validation(format!(
                        "The destination {} is inside the source {}; backups would include themselves",
                        p.display(),
                        s.display()
                    )));
                }
            }
        }
        self.retention.validate()?;
        self.schedule.validate()?;
        self.maintenance.validate()?;
        self.filter.clone().compile()?;
        if self.options.block_size < 1024 || self.options.volume_size < self.options.block_size {
            return Err(AppError::validation("Block size must be ≥ 1 KiB and remote volume size ≥ block size"));
        }
        Ok(())
    }
}

/// How a destination connection is opened.
#[derive(Debug, Clone, Copy, Default)]
struct BackendOpts {
    timeout_secs: u64,
    local: LocalTransfer,
    /// The job only reads (narrower scopes where the provider has them).
    read_only: bool,
    limits: RequestLimits,
}

impl BackendOpts {
    fn of(job: &BackupJob) -> Self {
        Self { timeout_secs: job.network_timeout_secs, local: job.local_transfer, read_only: job.read_only, limits: job.request_limits }
    }
}

/// One-off settings for a single restore (the job keeps its own).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RestoreOverrides {
    /// Download limit in bytes/second (0 = unlimited).
    pub download_bps: Option<u64>,
    pub retries: Option<u32>,
    pub network_timeout_secs: Option<u64>,
    /// Mirror restores (`delete_extraneous`) delete files and run only with
    /// this set. The app sets it after the user confirmed the dry-run list
    /// (with a confirmation token); API callers set it explicitly.
    pub confirm_delete: bool,
}

impl RestoreOverrides {
    fn apply(&self, job: &mut BackupJob) {
        if let Some(b) = self.download_bps {
            job.throttle.download_bps = b;
        }
        if let Some(r) = self.retries {
            job.retry.retries = r;
        }
        if let Some(t) = self.network_timeout_secs {
            job.network_timeout_secs = t;
        }
    }
}

/// Database file of a job folder entry (inventory).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseEntry {
    pub job_id: String,
    /// `None` when no job uses this database (orphan).
    pub job_name: Option<String>,
    pub info: Option<ufop_core::backup::db::DbInfo>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseCleanupReport {
    pub orphans_deleted: Vec<String>,
    pub vacuumed: usize,
    pub migrated: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
    pub errors: Vec<String>,
}

/// Every operation the queue can run.
// One value per queued operation; its size does not matter (see BackupDestination).
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum BackupOperation {
    Backup { dry_run: bool },
    Restore {
        options: RestoreOptions,
        /// Settings that apply to this restore only.
        #[serde(default)]
        overrides: RestoreOverrides,
    },
    Verify {
        #[serde(default)]
        full: bool,
        /// Scope, sampling, parallelism and error limit (`None`: `full`
        /// and the job's sample count, as before).
        #[serde(default)]
        options: Option<ufop_core::backup::verify::VerifyOptions>,
    },
    Compact,
    Repair,
    RecreateDatabase,
    ListBroken,
    PurgeBroken { dry_run: bool },
    Purge { paths: Vec<String>, versions: Option<Vec<usize>>, dry_run: bool },
    DeleteVersions {
        versions: Vec<usize>,
        allow_full_removal: bool,
        /// Also delete pinned ("Keep forever") versions.
        #[serde(default)]
        force: bool,
    },
    Consistency,
    /// Rebuild missing block volumes from the source files.
    RebuildMissing,
    /// Write a version's control files into `target`.
    ExtractControlFiles { version: usize, target: String },
    /// Benchmark restore download concurrency.
    AutoTune { sample_volumes: usize },
    /// Create and remove a test snapshot of the sources.
    SnapshotTest,
    /// Lock volumes: only unlocked ones, or extend every lock (`all`).
    ApplyLocks { days: u32, mode: ufop_core::backup::backend::LockMode, #[serde(default)] all: bool },
    /// Volumes with an active lock.
    ListLocks,
    /// Bring one replica (or all enabled ones) up to date.
    Replicate {
        #[serde(default)]
        replica_id: Option<String>,
    },
    /// Upgrade the repository to format 2 (header only), or with
    /// `rollback` restore the saved format 1 header.
    UpgradeFormat {
        #[serde(default)]
        rollback: bool,
    },
    /// Move volumes between the flat and sharded layouts (format 2).
    Reshard {
        layout: ufop_core::backup::layout::RepoLayout,
        #[serde(default)]
        dry_run: bool,
    },
    /// The repository's format settings (version, chunker, codec, cipher, layout).
    FormatInfo,
    /// The decoded repository header with its feature flags (read-only).
    InspectHeader,
    /// Counts and bytes per volume kind, blocks, deduplication and
    /// per-version use (read-only, `ufop backup debug stats`).
    InspectStats,
    /// Change a version's note, tags or pin (rewrites its file list).
    SetVersionMetadata {
        version: usize,
        /// Shared repositories: a version of another source (`None` = own).
        #[serde(default)]
        source: Option<String>,
        #[serde(default)]
        patch: ufop_core::backup::versions::MetadataPatch,
    },
    /// Apply retention now (the job's policy, or `policy`), to the job's
    /// own source or explicitly to `source`.
    ApplyRetention {
        #[serde(default)]
        policy: Option<RetentionPolicy>,
        #[serde(default)]
        source: Option<String>,
    },
    /// Copy versions to another job's repository or a destination.
    CopyVersions {
        versions: Vec<usize>,
        #[serde(default)]
        source: Option<String>,
        target: versions::CopyTarget,
        #[serde(default)]
        dry_run: bool,
    },
    /// Make the repository shared (format 2): other jobs and computers can
    /// write to it and deduplicate against it.
    EnableSharing {
        /// Assign every existing version to this job (not only versions
        /// without an identity).
        #[serde(default = "default_true")]
        claim_all: bool,
    },
    /// Bring the local database up to date with the destination (other
    /// sources' versions in a shared repository) and list the sources.
    SyncSources,
}

fn default_true() -> bool {
    true
}

impl BackupOperation {
    /// The repository lock this operation takes (`None`: no repository access
    /// beyond the header, or handled by the manager).
    pub fn lock_kind(&self) -> Option<LockKind> {
        use LockKind::{Exclusive, Shared};
        Some(match self {
            Self::Backup { dry_run } | Self::PurgeBroken { dry_run } | Self::Purge { dry_run, .. } | Self::Reshard { dry_run, .. } => {
                if *dry_run { Shared } else { Exclusive }
            }
            Self::Compact
            | Self::Repair
            | Self::DeleteVersions { .. }
            | Self::RebuildMissing
            | Self::ApplyLocks { .. }
            | Self::UpgradeFormat { .. }
            | Self::SetVersionMetadata { .. }
            | Self::ApplyRetention { .. }
            | Self::EnableSharing { .. } => Exclusive,
            Self::Restore { .. }
            | Self::Verify { .. }
            | Self::RecreateDatabase
            | Self::ListBroken
            | Self::Consistency
            | Self::ExtractControlFiles { .. }
            | Self::AutoTune { .. }
            | Self::ListLocks
            | Self::InspectStats
            | Self::SyncSources => Shared,
            // Copying locks both repositories itself (shared source, exclusive target).
            Self::SnapshotTest | Self::FormatInfo | Self::InspectHeader | Self::Replicate { .. } | Self::CopyVersions { .. } => return None,
        })
    }

    /// Whether the operation writes to the job's destination (refused for
    /// read-only jobs). Replication writes only to the replicas.
    pub fn writes_destination(&self) -> bool {
        self.lock_kind() == Some(LockKind::Exclusive)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Backup { .. } => "backup",
            Self::Restore { .. } => "restore",
            Self::Verify { .. } => "verify",
            Self::Compact => "compact",
            Self::Repair => "repair",
            Self::RecreateDatabase => "recreate_database",
            Self::ListBroken => "list_broken_files",
            Self::PurgeBroken { .. } => "purge_broken_files",
            Self::Purge { .. } => "purge",
            Self::DeleteVersions { .. } => "delete_versions",
            Self::Consistency => "consistency_check",
            Self::RebuildMissing => "rebuild_missing",
            Self::ExtractControlFiles { .. } => "extract_control_files",
            Self::AutoTune { .. } => "autotune",
            Self::SnapshotTest => "snapshot_test",
            Self::Replicate { .. } => "replicate",
            Self::ApplyLocks { .. } => "apply_locks",
            Self::ListLocks => "list_locks",
            Self::UpgradeFormat { rollback: false } => "upgrade_format",
            Self::UpgradeFormat { rollback: true } => "rollback_format",
            Self::Reshard { .. } => "reshard",
            Self::FormatInfo => "format_info",
            Self::InspectHeader => "inspect_header",
            Self::InspectStats => "inspect_stats",
            Self::SetVersionMetadata { .. } => "set_version_metadata",
            Self::ApplyRetention { .. } => "apply_retention",
            Self::CopyVersions { .. } => "copy_versions",
            Self::EnableSharing { .. } => "enable_sharing",
            Self::SyncSources => "sync_sources",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskInfo {
    pub task_id: String,
    pub job_id: String,
    pub job_name: String,
    pub operation: String,
    /// `queued` or `running`.
    pub state: String,
    pub queued_at: String,
    pub started_at: Option<String>,
    pub progress: Option<Progress>,
    /// Seconds since the task started running.
    #[serde(default)]
    pub elapsed_secs: u64,
    /// Average throughput while running (bytes/second).
    #[serde(default)]
    pub bytes_per_sec: u64,
    /// Estimated seconds left, from the share of items already done.
    #[serde(default)]
    pub eta_secs: Option<u64>,
}

/// (bytes/second, ETA) from progress and elapsed time.
pub fn progress_rate(p: &Progress, elapsed_secs: f64) -> (u64, Option<u64>) {
    if elapsed_secs < 0.5 {
        return (0, None);
    }
    let rate = (p.bytes_processed as f64 / elapsed_secs) as u64;
    let eta = (p.files_total > 0 && p.files_processed > 0 && p.files_processed < p.files_total)
        .then(|| (elapsed_secs * (p.files_total - p.files_processed) as f64 / p.files_processed as f64) as u64);
    (rate, eta)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerStatus {
    pub running: bool,
    pub paused: bool,
    pub paused_until: Option<String>,
    pub tasks: Vec<TaskInfo>,
    pub next_runs: Vec<NextRun>,
    /// Upcoming maintenance on its own schedule (compact, verify, apply_retention).
    #[serde(default)]
    pub next_maintenance: Vec<NextRun>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NextRun {
    pub job_id: String,
    pub job_name: String,
    pub next_run_at: String,
    /// `backup`, or the maintenance task.
    #[serde(default = "backup_op")]
    pub operation: String,
}

fn backup_op() -> String {
    "backup".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupRun {
    pub id: String,
    pub job_id: String,
    pub operation: String,
    /// `success`, `warning`, `error` or `cancelled`.
    pub status: String,
    pub started_at: String,
    pub finished_at: Option<String>,
    pub summary: String,
    pub report: serde_json::Value,
    pub log: Vec<LogLine>,
}

/// A `backup_runs` row (columns as selected by the run history queries).
fn run_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<BackupRun> {
    Ok(BackupRun {
        id: r.get(0)?,
        job_id: r.get(1)?,
        operation: r.get(2)?,
        status: r.get(3)?,
        started_at: r.get(4)?,
        finished_at: r.get(5)?,
        summary: r.get(6)?,
        report: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
        log: serde_json::from_str(&r.get::<_, String>(8)?).unwrap_or_default(),
    })
}

/// Result returned to callers of [`BackupManager::execute`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationResult {
    pub run_id: String,
    pub status: String,
    pub summary: String,
    pub report: serde_json::Value,
    pub log: Vec<LogLine>,
}

struct TaskSlot {
    info: TaskInfo,
    cancel: Arc<AtomicBool>,
    progress: Arc<Mutex<Option<Progress>>>,
}

struct Inner {
    pool: DbPool,
    creds: Arc<CredentialStore>,
    conns: Option<ConnectionManager>,
    ledger: Option<OperationLedger>,
    data_dir: PathBuf,
    jobs: RwLock<HashMap<String, BackupJob>>,
    tasks: Mutex<Vec<TaskSlot>>,
    /// Operations running at once (resized by the transfer caps).
    gate: Semaphore,
    gate_size: Mutex<usize>,
    /// One operation per job at a time, whatever the gate allows.
    job_locks: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
    /// Caps set by the console's managed settings.
    managed_caps: Mutex<Option<ufop_core::agent::ManagedTransfers>>,
    /// Policy templates delivered by the console (read-only).
    managed_templates: Mutex<Vec<templates::PolicyTemplate>>,
    managed_profiles: Mutex<Vec<notify::profiles::NotificationProfile>>,
    /// `Some(None)` = paused indefinitely.
    paused: Mutex<Option<Option<DateTime<Utc>>>>,
    scheduler_running: AtomicBool,
    /// Shutting down: no new operations (see `shutdown`).
    draining: AtomicBool,
    kdf: KdfParams,
    policy: std::sync::RwLock<policy::ModulePolicy>,
    secrets: std::sync::OnceLock<SettingsStore>,
    notifier: std::sync::OnceLock<Notifier>,
    run_listener: std::sync::OnceLock<RunListener>,
    secret_cache: secrets::SecretCache,
    /// Console-delivered (`central:`) secrets when no secure store exists:
    /// kept in memory only and re-delivered by the agent after a restart.
    volatile_secrets: Mutex<HashMap<String, String>>,
    /// Local cache of downloaded volumes (built from its settings on first use).
    volume_cache: Mutex<Option<Arc<ufop_core::backup::cache::VolumeCache>>>,
    /// Open "backup as drive" servers.
    drives: Mutex<Vec<drives::Drive>>,
}

/// Desktop notification sink: (title, body, status).
pub type Notifier = Arc<dyn Fn(&str, &str, &str) + Send + Sync>;

/// A finished operation, as handed to the run listener (the fleet agent
/// forwards these to the management console).
#[derive(Debug, Clone, Serialize)]
pub struct RunEvent {
    pub job_id: String,
    pub job_name: String,
    pub operation: String,
    /// `success`, `warning`, `error` or `cancelled`.
    pub status: String,
    pub started_at: String,
    pub finished_at: String,
    pub duration_ms: u64,
    pub summary: String,
    /// The engine report (redacted like HTTP reports, without log lines).
    pub report: serde_json::Value,
    /// Job statistics after the run.
    pub stats: JobStats,
    /// Repeat interval when the job is scheduled.
    pub schedule_interval_minutes: Option<u32>,
}

/// Receives every finished operation; must not block.
pub type RunListener = Arc<dyn Fn(RunEvent) + Send + Sync>;

#[derive(Clone)]
pub struct BackupManager {
    inner: Arc<Inner>,
}

const PASS_KEY_PREFIX: &str = "backup-passphrase:";
const DEST_KEY_PREFIX: &str = "backup-destination-secret:";

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Default directory for per-job databases.
pub fn default_data_dir() -> PathBuf {
    crate::paths::data_dir().join("backups")
}

impl BackupManager {
    pub fn new(pool: DbPool, creds: Arc<CredentialStore>, conns: Option<ConnectionManager>, ledger: Option<OperationLedger>, data_dir: PathBuf) -> Self {
        Self::with_kdf(pool, creds, conns, ledger, data_dir, KdfParams::default())
    }

    pub fn with_kdf(
        pool: DbPool,
        creds: Arc<CredentialStore>,
        conns: Option<ConnectionManager>,
        ledger: Option<OperationLedger>,
        data_dir: PathBuf,
        kdf: KdfParams,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                pool,
                creds,
                conns,
                ledger,
                data_dir,
                jobs: RwLock::new(HashMap::new()),
                tasks: Mutex::new(Vec::new()),
                gate: Semaphore::new(1),
                gate_size: Mutex::new(1),
                job_locks: Mutex::new(HashMap::new()),
                managed_caps: Mutex::new(None),
                managed_templates: Mutex::new(Vec::new()),
                managed_profiles: Mutex::new(Vec::new()),
                paused: Mutex::new(None),
                scheduler_running: AtomicBool::new(false),
                draining: AtomicBool::new(false),
                kdf,
                policy: std::sync::RwLock::new(policy::ModulePolicy::from_env()),
                secrets: std::sync::OnceLock::new(),
                notifier: std::sync::OnceLock::new(),
                run_listener: std::sync::OnceLock::new(),
                volatile_secrets: Mutex::new(HashMap::new()),
                secret_cache: secrets::SecretCache::default(),
                volume_cache: Mutex::new(None),
                drives: Mutex::new(Vec::new()),
            }),
        }
    }

    /// The database the manager uses (tests that share it).
    #[cfg(test)]
    pub(crate) fn pool_for_tests(&self) -> DbPool {
        self.inner.pool.clone()
    }

    /// Replace the process-wide module allowlist.
    pub fn set_module_policy(&self, p: policy::ModulePolicy) {
        *self.inner.policy.write().unwrap_or_else(|e| e.into_inner()) = p;
    }

    pub fn module_policy(&self) -> policy::ModulePolicy {
        self.inner.policy.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    async fn check_policy(&self, job: &BackupJob) -> Result<(), AppError> {
        let protocol = match self.resolve_destination(&job.destination).await? {
            BackupDestination::Local { .. } => "local".to_string(),
            BackupDestination::Connection { connection_id, .. } => match (&self.inner.conns, uuid::Uuid::parse_str(&connection_id)) {
                (Some(c), Ok(id)) => c.get_connection(id).await?.map(|p| p.protocol.as_str().to_string()).unwrap_or_default(),
                _ => String::new(),
            },
            BackupDestination::Library { .. } => String::new(),
            BackupDestination::Cloud { config, .. } => config.provider().to_string(),
        };
        self.module_policy().check_job(&protocol, job.encrypted, job.options.compression_level > 0)
    }

    fn job_db_path(&self, id: &str) -> PathBuf {
        self.inner.data_dir.join(format!("{id}.sqlite"))
    }

    // ── Job CRUD ──

    pub async fn load_from_db(&self) -> Result<usize, AppError> {
        let rows: Vec<String> = self
            .inner
            .pool
            .execute(|c| {
                let mut st = c.prepare("SELECT config_json FROM backup_jobs")?;
                let v = st.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok(v)
            })
            .await?;
        let mut jobs = self.inner.jobs.write().await;
        for r in rows {
            match serde_json::from_str::<BackupJob>(&r) {
                Ok(j) => {
                    jobs.insert(j.id.clone(), j);
                }
                Err(e) => tracing::warn!("Skipping unreadable backup job: {e}"),
            }
        }
        Ok(jobs.len())
    }

    async fn persist(&self, job: &BackupJob) -> Result<(), AppError> {
        let (id, name, json) = (job.id.clone(), job.name.clone(), serde_json::to_string(job)?);
        self.inner
            .pool
            .execute(move |c| {
                c.execute(
                    "INSERT INTO backup_jobs(id, name, config_json) VALUES (?1, ?2, ?3)
                     ON CONFLICT(id) DO UPDATE SET name = excluded.name, config_json = excluded.config_json, updated_at = datetime('now')",
                    params![id, name, json],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn list_jobs(&self) -> Vec<BackupJob> {
        let mut v: Vec<BackupJob> = self.inner.jobs.read().await.values().cloned().collect();
        v.sort_by_key(|j| j.name.to_lowercase());
        v
    }

    pub async fn get_job(&self, id: &str) -> Result<BackupJob, AppError> {
        self.inner.jobs.read().await.get(id).cloned().ok_or_else(|| AppError::not_found(format!("Backup job {id} not found")))
    }

    /// Create (empty id) or update a job. `passphrase`: `Some("")` clears it,
    /// `None` keeps the stored one.
    pub async fn save_job(&self, mut job: BackupJob, passphrase: Option<String>) -> Result<BackupJob, AppError> {
        let existing = if job.id.is_empty() { None } else { self.inner.jobs.read().await.get(&job.id).cloned() };
        job.sources.retain(|s| !s.trim().is_empty());
        for r in job.replicas.iter_mut().filter(|r| r.id.is_empty()) {
            r.id = uuid::Uuid::new_v4().to_string();
        }
        job.validate()?;
        templates::validate_job_refs(&job, &self.list_templates().await?)?;
        notify::profiles::validate_refs(&job.notification_profile_ids, &self.list_notification_profiles().await?)?;
        for n in job.notifications.iter_mut() {
            self.stash_notification_secrets(n).await?;
        }
        self.stash_report_secrets(&mut job.reports).await?;
        if !job.template_ids.is_empty() && job.overrides.is_none() {
            // First template on an older job: what it set so far stays its own.
            job.overrides = Some(templates::explicit_paths(&job)?.into_iter().collect());
        }
        if let Some(o) = job.overrides.as_mut() {
            o.sort();
            o.dedup();
        }
        self.check_policy(&job).await?;
        let now = Utc::now().to_rfc3339();
        match &existing {
            Some(old) => {
                job.created_at = old.created_at.clone();
                job.stats = old.stats.clone();
                // Only the hold commands place or release a hold.
                job.legal_hold = old.legal_hold.clone();
                if job.encrypt_local_database.is_none() {
                    job.encrypt_local_database = old.encrypt_local_database;
                }
                if old.options.block_size != job.options.block_size && old.stats.version_count > 0 {
                    return Err(AppError::validation(
                        "Block size cannot change after the first backup; create a new job for a new block size",
                    ));
                }
            }
            None => {
                if job.id.is_empty() {
                    job.id = uuid::Uuid::new_v4().to_string();
                }
                job.created_at = now.clone();
                job.legal_hold = None;
                // New encrypted backups get an encrypted local database.
                if job.encrypt_local_database.is_none() {
                    job.encrypt_local_database = Some(job.encrypted);
                }
            }
        }
        job.updated_at = now;
        if let Some(p) = passphrase {
            if p.is_empty() {
                self.delete_secret(&job.id).await;
            } else {
                self.store_secret(&job.id, &p).await?;
            }
        }
        // The stored value, unresolved: saving must not depend on secret providers.
        let gpg_keys = job.options.encryption_module == ufop_core::backup::EncryptionModuleKind::Gpg && !job.options.gpg.recipients.is_empty();
        if job.encrypted && !gpg_keys && self.store_get(&format!("{PASS_KEY_PREFIX}{}", job.id)).await?.is_none() {
            return Err(AppError::validation("An encrypted backup needs a passphrase (or turn encryption off)"));
        }
        self.persist(&job).await?;
        self.inner.jobs.write().await.insert(job.id.clone(), job.clone());
        Ok(job)
    }

    /// Passphrases go to the OS keychain when one is available, otherwise to
    /// the settings store in the database (encrypted with the settings key
    /// when there is one) so they survive restarts on headless machines.
    fn secret_db(&self) -> Option<&SettingsStore> {
        if self.inner.creds.is_keychain_available() {
            None
        } else {
            self.inner.secrets.get()
        }
    }

    async fn store_secret(&self, job_id: &str, value: &str) -> Result<(), AppError> {
        let key = format!("{PASS_KEY_PREFIX}{job_id}");
        match self.secret_db() {
            Some(db) => db.set_secret(&key, value).await,
            None => self.inner.creds.store(&key, value).await,
        }
    }

    async fn delete_secret(&self, job_id: &str) {
        let key = format!("{PASS_KEY_PREFIX}{job_id}");
        if let Some(db) = self.secret_db() {
            let _ = db.delete(&key).await;
        }
        let _ = self.inner.creds.delete(&key).await;
    }

    pub async fn passphrase(&self, job_id: &str) -> Result<Option<String>, AppError> {
        let v = self.store_get(&format!("{PASS_KEY_PREFIX}{job_id}")).await?;
        self.resolve_stored(v).await
    }

    /// The job's stored local-database key, if any.
    async fn stored_db_key(&self, job_id: &str) -> Result<Option<DbKey>, AppError> {
        match self.store_get(&format!("{DB_KEY_PREFIX}{job_id}")).await? {
            Some(h) => Ok(Some(DbKey::from_hex(&h)?)),
            None => Ok(None),
        }
    }

    /// Key and wanted state for opening a job's database; a key is created
    /// (and stored) when encryption is wanted and none exists yet.
    async fn job_db_key(&self, job: &BackupJob) -> Result<(Option<DbKey>, bool), AppError> {
        let want = job.encrypt_local_database == Some(true);
        let mut key = self.stored_db_key(&job.id).await?;
        if want && key.is_none() {
            let k = DbKey::generate();
            self.store_set(&format!("{DB_KEY_PREFIX}{}", job.id), &k.to_hex()).await?;
            key = Some(k);
        }
        Ok((key, want))
    }

    /// Open a job's database without touching the destination. When the
    /// database cannot be decrypted (key lost), it is rebuilt from the
    /// destination first (see [`maintenance::open_job_db`]).
    async fn open_job_db(&self, job: &BackupJob) -> Result<(PathBuf, Option<DbKey>, bool), AppError> {
        let (key, want) = self.job_db_key(job).await?;
        let path = self.job_db_path(&job.id);
        let (p2, k2) = (path.clone(), key.clone());
        let check = blocking(move || BackupDb::key_check(&p2, k2.as_ref())).await?;
        if matches!(check, KeyCheck::KeyMissing | KeyCheck::WrongKey) {
            let resolved = self.resolve_job_secrets(job).await?;
            let backend = self.build_backend(&resolved).await?;
            let pass = if job.encrypted { self.passphrase(&job.id).await? } else { None };
            let (p2, k2, prefix) = (path.clone(), key.clone(), job.options.prefix.clone());
            let notes = blocking(move || {
                let o = maintenance::JobDbOptions { key: k2.as_ref(), encrypt: want };
                repo_lock::with_lock(backend.as_ref(), &prefix, LockKind::Shared, "recreate_database", &lock_options(&resolved.options), &OpContext::new(), || {
                    maintenance::open_job_db(&p2, &o, Some(backend.as_ref()), pass.as_deref(), &prefix, &OpContext::new()).map(|(_, n)| n)
                })
            })
            .await?;
            for n in notes.0.iter().chain(notes.1.iter()) {
                tracing::warn!("Backup job '{}': {n}", job.name);
            }
        }
        Ok((path, key, want))
    }

    /// Turn local-database encryption on or off for a job and apply it now
    /// (one-click migration). Returns what changed.
    pub async fn set_database_encryption(&self, job_id: &str, enabled: bool) -> Result<ufop_core::backup::db::RekeyReport, AppError> {
        let mut job = self.get_job(job_id).await?;
        if self.is_job_busy(job_id) {
            return Err(AppError::state("This job has a running or queued operation", "Wait for it to finish, then try again."));
        }
        job.encrypt_local_database = Some(enabled);
        let job = self.save_job(job, None).await?;
        let (path, key, want) = self.open_job_db(&job).await?;
        let prefix = job.options.prefix.clone();
        blocking(move || {
            let o = maintenance::JobDbOptions { key: key.as_ref(), encrypt: want };
            let (db, _) = maintenance::open_job_db(&path, &o, None, None, &prefix, &OpContext::new())?;
            let info = BackupDb::inspect(&path)?;
            Ok(ufop_core::backup::db::RekeyReport { encrypted: db.is_encrypted(), files: info.files, settings: 0 })
        })
        .await
    }

    /// Who holds the repository lock(s) of a job's destination right now.
    pub async fn lock_status(&self, job_id: &str) -> Result<repo_lock::LockStatus, AppError> {
        let job = self.resolve_job_secrets(&self.effective_job(job_id).await?).await?;
        let backend = self.build_backend(&job).await?;
        let prefix = job.options.prefix.clone();
        blocking(move || repo_lock::status(backend.as_ref(), &prefix)).await
    }

    /// Force-unlock a job's repository. Only when the program named in the
    /// lock is no longer running: `confirm` must be true; with `owner_id`
    /// only that lock is removed (and nothing when it changed meanwhile).
    pub async fn break_lock(&self, job_id: &str, confirm: bool, owner_id: Option<String>) -> Result<repo_lock::BreakLockReport, AppError> {
        let job = self.resolve_job_secrets(&self.effective_job(job_id).await?).await?;
        if job.read_only {
            return Err(Self::read_only_refusal(&job, "break the repository lock"));
        }
        let backend = self.build_backend(&job).await?;
        let prefix = job.options.prefix.clone();
        let r = blocking(move || repo_lock::break_lock(backend.as_ref(), &prefix, confirm, owner_id.as_deref())).await?;
        tracing::warn!("Repository lock of '{}' broken by the user: {:?}", job.name, r.holders);
        Ok(r)
    }

    fn read_only_refusal(job: &BackupJob, what: &str) -> AppError {
        AppError::PermissionDenied {
            message: format!("'{}' is connected read-only: cannot {what}", job.name),
            advice: "Read-only jobs only restore, browse and verify. Turn off 'Read-only' under Destination in the job editor to make changes.".into(),
        }
    }

    /// Change an encrypted job's passphrase in place (format 2: only the
    /// repository header is rewritten), then store the new passphrase for
    /// the job. `current` defaults to the stored passphrase.
    pub async fn change_passphrase(&self, job_id: &str, current: Option<String>, new: String, kdf: Option<KdfParams>) -> Result<ufop_core::backup::format::FormatReport, AppError> {
        let job = self.get_job(job_id).await?;
        if !job.encrypted || job.options.encryption_module == ufop_core::backup::EncryptionModuleKind::Gpg {
            return Err(AppError::state("This job does not use passphrase encryption", "Only jobs encrypted with the built-in cipher have a passphrase to change."));
        }
        if new.trim().is_empty() {
            return Err(AppError::validation("Enter the new passphrase"));
        }
        if job.read_only {
            return Err(Self::read_only_refusal(&job, "change the passphrase"));
        }
        let current = match current.filter(|c| !c.is_empty()) {
            Some(c) => c,
            None => self.passphrase(job_id).await?.ok_or_else(|| AppError::validation("Enter the current passphrase"))?,
        };
        // No backup or maintenance may write while the header changes.
        let job_lock = {
            let mut locks = self.inner.job_locks.lock().unwrap_or_else(|e| e.into_inner());
            locks.entry(job.id.clone()).or_default().clone()
        };
        let _guard = job_lock.lock().await;
        let backend = self.build_backend(&job).await?;
        let (prefix, new2) = (job.options.prefix.clone(), new.clone());
        let kdf = kdf.or(job.options.kdf);
        let lo = lock_options(&job.options);
        let (report, _) = blocking(move || {
            repo_lock::with_lock(backend.as_ref(), &prefix, LockKind::Exclusive, "change_password", &lo, &OpContext::new(), || {
                ufop_core::backup::format::change_password(backend.as_ref(), &prefix, &current, &new2, kdf)
            })
        })
        .await?;
        self.store_secret(job_id, &new).await?;
        Ok(report)
    }

    /// Store (or, with an empty value, remove) a cloud destination's secret.
    pub async fn set_destination_secret(&self, credential_id: &str, secret: &str) -> Result<(), AppError> {
        if credential_id.trim().is_empty() || credential_id.len() > 128 {
            return Err(AppError::validation("Invalid credential id"));
        }
        let key = format!("{DEST_KEY_PREFIX}{credential_id}");
        if secret.is_empty() || credential_id.starts_with(ufop_core::agent::policy::CENTRAL_PREFIX) {
            lock(&self.inner.volatile_secrets).remove(&key);
            if let Some(db) = self.secret_db() {
                let _ = db.delete(&key).await;
            }
            let _ = self.inner.creds.delete(&key).await;
            if secret.is_empty() {
                return Ok(());
            }
            // Never write a console credential to disk unencrypted.
            if !self.has_secure_store() {
                lock(&self.inner.volatile_secrets).insert(key, secret.to_string());
                return Ok(());
            }
        }
        match self.secret_db() {
            Some(db) => db.set_secret(&key, secret).await,
            None => self.inner.creds.store(&key, secret).await,
        }
    }

    async fn destination_secret(&self, credential_id: &str) -> Result<Option<String>, AppError> {
        Box::pin(self.destination_secret_raw(credential_id)).await
    }

    /// A stored secret; a value written as `$name` is read from the secret providers.
    async fn destination_secret_raw(&self, credential_id: &str) -> Result<Option<String>, AppError> {
        if credential_id.is_empty() {
            return Ok(None);
        }
        let v = self.store_get(&format!("{DEST_KEY_PREFIX}{credential_id}")).await?;
        // Provider credentials themselves are never resolved (no recursion).
        if v.as_deref().is_some_and(|s| !secrets::placeholders(s).is_empty()) {
            let settings = self.secret_settings().await?;
            if settings.providers.iter().any(|p| p.credential_id == credential_id) {
                return Ok(v);
            }
            return Box::pin(self.resolve_stored(v)).await;
        }
        Ok(v)
    }

    /// Whether a secret is stored for a destination (never returns it).
    pub async fn has_destination_secret(&self, credential_id: &str) -> Result<bool, AppError> {
        Ok(self.destination_secret(credential_id).await?.is_some_and(|s| !s.is_empty()))
    }

    // ── Secret providers and placeholders ──

    pub async fn secret_settings(&self) -> Result<secrets::SecretSettings, AppError> {
        Ok(self.config_get(secrets::SETTINGS_KEY).await?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    pub async fn set_secret_settings(&self, mut s: secrets::SecretSettings) -> Result<secrets::SecretSettings, AppError> {
        for p in &mut s.providers {
            if p.id.trim().is_empty() {
                p.id = uuid::Uuid::new_v4().to_string();
            }
            if p.kind.needs_credential() && p.credential_id.trim().is_empty() {
                p.credential_id = uuid::Uuid::new_v4().to_string();
            }
        }
        s.validate()?;
        self.config_set(secrets::SETTINGS_KEY, serde_json::to_string(&s)?).await?;
        self.inner.secret_cache.clear();
        Ok(s)
    }

    /// An OS keychain, or a settings store encrypted with a key.
    fn has_secure_store(&self) -> bool {
        self.inner.creds.is_keychain_available() || self.inner.secrets.get().is_some_and(|s| s.is_encrypted())
    }

    async fn store_get(&self, key: &str) -> Result<Option<String>, AppError> {
        if let Some(v) = lock(&self.inner.volatile_secrets).get(key) {
            return Ok(Some(v.clone()));
        }
        match self.secret_db() {
            Some(db) => db.get_secret(key).await,
            None => self.inner.creds.retrieve(key).await,
        }
    }

    async fn store_set(&self, key: &str, value: &str) -> Result<(), AppError> {
        match self.secret_db() {
            Some(db) => db.set_secret(key, value).await,
            None => self.inner.creds.store(key, value).await,
        }
    }

    /// Resolve one secret through `providers` (in order). Returns the value,
    /// the provider name and whether it came from the cache.
    async fn resolve_secret(&self, settings: &secrets::SecretSettings, providers: &[&secrets::SecretProvider], name: &str) -> Result<(String, String, bool), AppError> {
        // `$central:<id>`: a credential delivered by the management console,
        // kept in the secure store under that id (never in a provider or file).
        if let Some(id) = ufop_core::agent::policy::central_credential_id(name).filter(|_| name.starts_with(ufop_core::agent::policy::CENTRAL_PREFIX)) {
            return match self.store_get(&format!("{DEST_KEY_PREFIX}{id}")).await? {
                Some(v) if !v.is_empty() => Ok((v, "management console".into(), false)),
                _ => Err(AppError::Configuration {
                    message: format!("The central credential '{id}' has not been delivered to this machine"),
                    advice: "Check that the machine is registered with the management console and the policy using it applies here.".into(),
                }),
            };
        }
        let ttl = std::time::Duration::from_secs(settings.cache_ttl_secs.max(1));
        let mut errors = Vec::new();
        for p in providers {
            if settings.cache != secrets::CacheMode::None {
                if let Some(v) = self.inner.secret_cache.get(&p.id, name, ttl) {
                    return Ok((v, p.name.clone(), true));
                }
            }
            let cred = if p.kind.needs_credential() { self.destination_secret(&p.credential_id).await? } else { None };
            match secrets::lookup(p, cred.as_deref(), name).await {
                Ok(Some(v)) => {
                    if settings.cache != secrets::CacheMode::None {
                        self.inner.secret_cache.put(&p.id, name, &v);
                    }
                    if settings.cache == secrets::CacheMode::Persistent {
                        let entry = serde_json::json!({ "value": v, "at": Utc::now().to_rfc3339() }).to_string();
                        if let Err(e) = self.store_set(&format!("backup-secret-cache:{}:{name}", p.id), &entry).await {
                            tracing::warn!("Could not persist cached secret: {e}");
                        }
                    }
                    return Ok((v, p.name.clone(), false));
                }
                Ok(None) => {}
                Err(e) => {
                    // An unreachable provider falls back to the persistent cache.
                    if settings.cache == secrets::CacheMode::Persistent {
                        if let Some(entry) = self.store_get(&format!("backup-secret-cache:{}:{name}", p.id)).await.ok().flatten() {
                            if let Some(v) = serde_json::from_str::<serde_json::Value>(&entry).ok().and_then(|e| e["value"].as_str().map(String::from)) {
                                return Ok((v, p.name.clone(), true));
                            }
                        }
                    }
                    errors.push(format!("{}: {}", p.name, e.user_message()));
                }
            }
        }
        Err(AppError::validation(if providers.is_empty() {
            format!("Secret '${name}' is used but no secret provider is configured")
        } else if errors.is_empty() {
            format!("Secret '${name}' was not found in: {}", providers.iter().map(|p| p.name.as_str()).collect::<Vec<_>>().join(", "))
        } else {
            format!("Secret '${name}' could not be read ({})", errors.join("; "))
        }))
    }

    /// Replace `$name` / `${name}` placeholders in a job with their secrets.
    pub async fn resolve_job_secrets(&self, job: &BackupJob) -> Result<BackupJob, AppError> {
        let mut v = serde_json::to_value(job)?;
        let names = secrets::collect(&v);
        if names.is_empty() {
            return Ok(job.clone());
        }
        let settings = self.secret_settings().await?;
        let providers = settings.for_job(&job.secret_providers);
        let mut values = HashMap::new();
        for n in names {
            let (val, _, _) = self.resolve_secret(&settings, &providers, &n).await?;
            values.insert(n, val);
        }
        secrets::apply(&mut v, &values);
        let mut out: BackupJob = serde_json::from_value(v)?;
        out.masked_values = values.into_values().filter(|v| v.len() >= 3).collect();
        out.stats = job.stats.clone();
        Ok(out)
    }

    /// Resolve a stored credential written as a placeholder (`$name`).
    async fn resolve_stored(&self, value: Option<String>) -> Result<Option<String>, AppError> {
        let Some(v) = value else { return Ok(None) };
        let names = secrets::placeholders(&v);
        if names.is_empty() {
            return Ok(Some(v));
        }
        let settings = self.secret_settings().await?;
        let providers = settings.for_job(&[]);
        let mut values = HashMap::new();
        for n in names {
            values.insert(n.clone(), self.resolve_secret(&settings, &providers, &n).await?.0);
        }
        Ok(Some(secrets::substitute(&v, &values)))
    }

    /// Test providers: credentials present, latency, which `probe` names exist.
    pub async fn test_secret_providers(&self, provider_id: Option<String>, probe: Vec<String>) -> Result<Vec<secrets::ProviderTest>, AppError> {
        let settings = self.secret_settings().await?;
        let mut out = Vec::new();
        for p in settings.providers.iter().filter(|p| provider_id.as_deref().map_or(true, |id| id == p.id)) {
            let mut t = secrets::ProviderTest { provider_id: p.id.clone(), provider: p.name.clone(), kind: p.kind.label().into(), ..Default::default() };
            let cred = if p.kind.needs_credential() { self.destination_secret(&p.credential_id).await? } else { None };
            t.has_credential = !p.kind.needs_credential() || cred.as_deref().is_some_and(|c| !c.is_empty());
            if !t.has_credential {
                t.errors.push("The provider's credential is not stored".into());
            }
            let started = std::time::Instant::now();
            let names = if probe.is_empty() { vec!["__ufop_connectivity_probe__".to_string()] } else { probe.clone() };
            for n in &names {
                match secrets::lookup(p, cred.as_deref(), n).await {
                    Ok(Some(_)) => t.found.push(n.clone()),
                    Ok(None) => t.missing.push(n.clone()),
                    Err(e) => t.errors.push(format!("{n}: {}", e.user_message())),
                }
            }
            t.latency_ms = (started.elapsed().as_secs_f64() * 1000.0 / names.len() as f64 * 10.0).round() / 10.0;
            if probe.is_empty() {
                t.missing.clear();
            }
            t.ok = t.errors.is_empty() && t.has_credential && (probe.is_empty() || t.missing.is_empty());
            out.push(t);
        }
        Ok(out)
    }

    /// Placeholders a job uses and where each resolves (values never returned).
    pub async fn job_placeholders(&self, job_id: &str) -> Result<secrets::PlaceholderReport, AppError> {
        let job = self.effective_job(job_id).await?;
        let names = secrets::collect(&serde_json::to_value(&job)?);
        let settings = self.secret_settings().await?;
        let providers = settings.for_job(&job.secret_providers);
        let mut r = secrets::PlaceholderReport { providers: providers.iter().map(|p| p.name.clone()).collect(), ..Default::default() };
        for n in names {
            let st = match self.resolve_secret(&settings, &providers, &n).await {
                Ok((_, p, cached)) => secrets::PlaceholderStatus { name: n, provider: Some(p), from_cache: cached, error: None },
                Err(e) => secrets::PlaceholderStatus { name: n, provider: None, from_cache: false, error: Some(e.user_message()) },
            };
            r.placeholders.push(st);
        }
        Ok(r)
    }

    /// Route run results to OS notifications (set by the desktop app).
    pub fn set_notifier(&self, n: Notifier) {
        let _ = self.inner.notifier.set(n);
    }

    /// Receive every finished operation (set once, by the fleet agent).
    pub fn set_run_listener(&self, l: RunListener) {
        let _ = self.inner.run_listener.set(l);
    }

    /// Persist passphrases in the database when no OS keychain exists.
    pub fn set_settings_store(&self, store: SettingsStore) {
        let _ = self.inner.secrets.set(store);
    }

    /// Delete a job. Optionally remove its local database and/or all remote volumes.
    pub async fn delete_job(&self, id: &str, delete_local_db: bool, delete_remote: bool) -> Result<(), AppError> {
        if lock(&self.inner.tasks).iter().any(|t| t.info.job_id == id) {
            return Err(AppError::state("This job has a running or queued operation", "Cancel it first."));
        }
        let job = self.get_job(id).await?;
        legal_hold::guard_delete(&job)?;
        self.close_drives_of(id).await;
        if delete_remote {
            if job.read_only {
                return Err(Self::read_only_refusal(&job, "delete the backup files at the destination"));
            }
            let backend = self.build_backend(&job).await?;
            let prefix = job.options.prefix.clone();
            let lo = lock_options(&job.options);
            tokio::task::spawn_blocking(move || -> Result<(), AppError> {
                repo_lock::with_lock(backend.as_ref(), &prefix, LockKind::Exclusive, "delete_backup", &lo, &OpContext::new(), || {
                    // Includes volumes in shard folders of sharded repositories;
                    // our own lock goes when the guard is released.
                    for f in backend.list_recursive()? {
                        let name = ufop_core::backup::layout::logical(&f.name);
                        if name.starts_with(&format!("{prefix}-")) && !repo_lock::is_lock_file(&prefix, name) {
                            backend.delete(&f.name)?;
                        }
                    }
                    Ok(())
                })
                .map(|_| ())
            })
            .await
            .map_err(|e| AppError::internal(e.to_string()))??;
        }
        if delete_local_db {
            for ext in ["", "-wal", "-shm"] {
                let _ = std::fs::remove_file(format!("{}{ext}", self.job_db_path(id).display()));
            }
        }
        let key = id.to_string();
        self.inner.pool.execute(move |c| {
            c.execute("DELETE FROM backup_jobs WHERE id = ?1", [&key])?;
            Ok(())
        }).await?;
        self.delete_secret(id).await;
        let dbkey = format!("{DB_KEY_PREFIX}{id}");
        if let Some(db) = self.secret_db() {
            let _ = db.delete(&dbkey).await;
        }
        let _ = self.inner.creds.delete(&dbkey).await;
        self.inner.jobs.write().await.remove(id);
        Ok(())
    }

    // ── Destinations ──

    pub async fn backend_for(&self, dest: &BackupDestination, throttle: Throttle, retry: RetryPolicy, soft_delete: Option<String>) -> Result<Box<dyn Backend>, AppError> {
        Box::pin(self.backend_with_timeout(dest, throttle, retry, soft_delete, BackendOpts::default())).await
    }

    /// Resolve a library entry to its concrete destination (with sub-folder).
    pub async fn resolve_destination(&self, dest: &BackupDestination) -> Result<BackupDestination, AppError> {
        let BackupDestination::Library { destination_id, remote_path } = dest else { return Ok(dest.clone()) };
        let entry = self.list_destinations().await?.into_iter().find(|d| &d.id == destination_id)
            .ok_or_else(|| AppError::not_found("The library destination no longer exists"))?;
        let sub = remote_path.trim().trim_matches(['/', '\\']);
        Ok(match entry.destination {
            BackupDestination::Local { path, alternate_paths, marker_file } if !sub.is_empty() => BackupDestination::Local {
                path: Path::new(&path).join(sub).to_string_lossy().into_owned(),
                alternate_paths: alternate_paths.iter().map(|p| Path::new(p).join(sub).to_string_lossy().into_owned()).collect(),
                marker_file,
            },
            BackupDestination::Connection { connection_id, remote_path: base } if !sub.is_empty() => {
                BackupDestination::Connection { connection_id, remote_path: format!("{}/{sub}", base.trim_end_matches('/')) }
            }
            other => other,
        })
    }

    async fn backend_with_timeout(&self, dest: &BackupDestination, throttle: Throttle, retry: RetryPolicy, soft_delete: Option<String>, opts: BackendOpts) -> Result<Box<dyn Backend>, AppError> {
        let BackendOpts { timeout_secs, local, read_only, limits } = opts;
        let dest = &self.resolve_destination(dest).await?;
        let parallel = limits.list_parallelism.max(1) as usize;
        let raw: Box<dyn Backend> = match dest {
            BackupDestination::Local { path, alternate_paths, marker_file } => {
                let mut b = if alternate_paths.is_empty() && marker_file.is_none() {
                    LocalBackend::new(path)
                } else {
                    let mut c = vec![path.clone()];
                    c.extend(alternate_paths.iter().cloned());
                    LocalBackend::discover(&c, marker_file.as_deref())?
                };
                b.use_move = local.use_move;
                b.verify_upload_size = local.verify_size;
                b.sync_writes = local.sync_writes;
                b.list_parallelism = parallel;
                Box::new(b)
            }
            BackupDestination::Connection { connection_id, remote_path } => {
                let conns = self.inner.conns.as_ref().ok_or_else(|| AppError::state("Connections are unavailable", "Restart the app."))?;
                let id = uuid::Uuid::parse_str(connection_id).map_err(|_| AppError::validation("Invalid connection id"))?;
                let profile = conns.get_connection(id).await?.ok_or_else(|| AppError::not_found("The destination connection no longer exists"))?;
                let secret = conns.get_password(id).await.unwrap_or(None);
                let mut b = connector_backend::ConnectorBackend::connect(&profile, secret, remote_path).await?;
                b.set_timeout((timeout_secs > 0).then(|| std::time::Duration::from_secs(timeout_secs)));
                b.set_list_parallelism(parallel);
                Box::new(b)
            }
            BackupDestination::Library { .. } => return Err(AppError::validation("A library destination cannot point at another library entry")),
            BackupDestination::Cloud { config, credential_id } => {
                let secret = self.destination_secret(credential_id).await?;
                let store = Box::pin(stores::open_with(config, secret, stores::OpenOptions { read_only: read_only || dest.point_in_time().is_some() })).await?;
                Box::new(stores::StoreBackend::new(store, (timeout_secs > 0).then(|| std::time::Duration::from_secs(timeout_secs))).with_list_parallelism(parallel))
            }
        };
        // A point-in-time view never writes, whatever the job says.
        let view_only = raw.is_read_only();
        // Each attempt of a retried request counts against the rate limits.
        let raw: Box<dyn Backend> = if limits.is_limited() { Box::new(RateLimitedBackend::new(raw, limits)) } else { raw };
        let b: Box<dyn Backend> = Box::new(RetryingBackend::new(raw, retry, throttle));
        let b: Box<dyn Backend> = match soft_delete.filter(|p| !p.is_empty()) {
            Some(p) => Box::new(SoftDeleteBackend::new(b, p)),
            None => b,
        };
        Ok(if view_only { Box::new(ReadOnlyBackend::new(b)) } else { b })
    }

    async fn build_backend(&self, job: &BackupJob) -> Result<Box<dyn Backend>, AppError> {
        // Boxed: connecting has large futures (every provider), and callers'
        // state machines stay small (debug builds run them on 2 MiB stacks).
        let b = Box::pin(self.backend_with_timeout(&job.destination, job.throttle, job.retry, job.soft_delete_prefix.clone(), BackendOpts::of(job))).await?;
        Ok(if job.read_only && !b.is_read_only() { Box::new(ReadOnlyBackend::new(b)) } else { b })
    }

    // ── Direct destination file operations ──

    /// Files at a destination (any files, not only backup volumes).
    pub async fn remote_list(&self, dest: BackupDestination) -> Result<Vec<ufop_core::backup::RemoteFile>, AppError> {
        let b = self.backend_for(&dest, Throttle::default(), RetryPolicy::default(), None).await?;
        blocking(move || b.list()).await
    }

    /// Download one file from a destination to a local path.
    pub async fn remote_get(&self, dest: BackupDestination, name: String, local_path: String) -> Result<u64, AppError> {
        let b = self.backend_for(&dest, Throttle::default(), RetryPolicy::default(), None).await?;
        blocking(move || {
            let d = b.get(&name)?;
            std::fs::write(&local_path, &d)?;
            Ok(d.len() as u64)
        })
        .await
    }

    /// Upload a local file to a destination under `name`.
    pub async fn remote_put(&self, dest: BackupDestination, local_path: String, name: String) -> Result<u64, AppError> {
        let b = self.backend_for(&dest, Throttle::default(), RetryPolicy::default(), None).await?;
        blocking(move || {
            let d = std::fs::read(&local_path)?;
            b.put(&name, &d)?;
            Ok(d.len() as u64)
        })
        .await
    }

    /// Delete one file at a destination. Refuses repository files of a
    /// configured job unless `force` (they would break that backup).
    pub async fn remote_delete(&self, dest: BackupDestination, name: String, force: bool) -> Result<(), AppError> {
        if !force {
            let jobs = self.inner.jobs.read().await;
            if let Some(j) = jobs.values().find(|j| j.destination == dest && name.starts_with(&format!("{}-", j.options.prefix))) {
                return Err(AppError::validation(format!("{name} belongs to the backup '{}'; deleting it would damage that backup", j.name)));
            }
        }
        let b = self.backend_for(&dest, Throttle::default(), RetryPolicy::default(), None).await?;
        blocking(move || b.delete(&name)).await
    }

    // ── Replicas ──

    /// Mirror the primary to its replicas (`only` = one replica id).
    async fn replicate(&self, job: &BackupJob, only: Option<&str>, ctx: &OpContext) -> Result<serde_json::Value, AppError> {
        let picked: Vec<Replica> = job.replicas.iter().filter(|r| match only {
            Some(id) => r.id == id,
            None => r.enabled,
        }).cloned().collect();
        if picked.is_empty() {
            return Err(AppError::validation(match only {
                Some(_) => "That replica does not exist",
                None => "This job has no enabled replicas",
            }));
        }
        let primary = self.build_backend(job).await?;
        let mut targets = Vec::new();
        let mut errors = Vec::new();
        for r in picked {
            match Box::pin(self.backend_with_timeout(&r.destination, r.throttle, job.retry, None, BackendOpts { read_only: false, ..BackendOpts::of(job) })).await {
                Ok(b) => targets.push((r, b)),
                Err(e) => errors.push(format!("{}: {}", r.name, e.user_message())),
            }
        }
        let prefix = job.options.prefix.clone();
        let ctx2 = ctx.clone();
        let lo = lock_options(&job.options);
        let results = blocking(move || {
            Ok(targets
                .into_iter()
                .map(|(r, b)| {
                    let o = ufop_core::backup::mirror::MirrorOptions { delete_extraneous: r.delete_extraneous, verify: r.verify, ..Default::default() };
                    let res = ufop_core::backup::mirror::mirror_locked(primary.as_ref(), b.as_ref(), &prefix, &o, &lo, &ctx2);
                    (r, res)
                })
                .collect::<Vec<_>>())
        })
        .await?;
        let now = Utc::now().to_rfc3339();
        let mut out = Vec::new();
        let mut outcome: HashMap<String, String> = HashMap::new();
        for (r, res) in results {
            match res {
                Ok(rep) => {
                    let status = if rep.errors.is_empty() { "success" } else { "error" };
                    errors.extend(rep.errors.iter().map(|e| format!("{}: {e}", r.name)));
                    outcome.insert(r.id.clone(), status.into());
                    out.push(serde_json::json!({ "id": r.id, "name": r.name, "report": rep }));
                }
                Err(e) => {
                    errors.push(format!("{}: {}", r.name, e.user_message()));
                    outcome.insert(r.id.clone(), "error".into());
                }
            }
        }
        let snapshot = {
            let mut jobs = self.inner.jobs.write().await;
            jobs.get_mut(&job.id).map(|j| {
                for r in j.replicas.iter_mut() {
                    if let Some(st) = outcome.get(&r.id) {
                        r.last_run = Some(now.clone());
                        r.last_result = Some(st.clone());
                    }
                }
                j.clone()
            })
        };
        if let Some(j) = snapshot {
            let _ = self.persist(&j).await;
        }
        Ok(serde_json::json!({ "replicas": out, "errors": errors }))
    }

    /// Replicas whose interval has elapsed: (job id, replica id).
    async fn due_replicas(&self, now: DateTime<Utc>) -> Vec<(String, String)> {
        let jobs = self.inner.jobs.read().await;
        let mut out = Vec::new();
        for j in jobs.values() {
            for r in j.replicas.iter().filter(|r| r.enabled) {
                if let ReplicaSchedule::Interval { minutes } = r.schedule {
                    let last = r.last_run.as_deref().and_then(parse_ts);
                    if last.map_or(true, |l| now - l >= chrono::Duration::minutes(minutes as i64)) {
                        out.push((j.id.clone(), r.id.clone()));
                    }
                }
            }
        }
        out
    }

    /// Queue any operation in the background; returns the task id.
    pub async fn start_operation(&self, job_id: &str, op: BackupOperation) -> Result<String, AppError> {
        self.ensure_accepting()?;
        let job = self.get_job(job_id).await?;
        if self.is_job_busy(job_id) {
            return Err(AppError::state("This job is already queued or running", "Wait for it to finish or cancel it."));
        }
        let (task_id, cancel, progress) = self.register_task(&job, op.name());
        let span = op_span(&job, op.name(), &task_id);
        let me = self.clone();
        let tid = task_id.clone();
        tokio::spawn(
            async move {
                if let Err(e) = me.run_registered(job, op, tid.clone(), cancel, progress).await {
                    tracing::warn!("task {tid} failed: {e}");
                }
            }
            .instrument(span),
        );
        Ok(task_id)
    }

    // ── Shared settings: global defaults and destination library ──

    pub(crate) async fn config_get(&self, key: &'static str) -> Result<Option<String>, AppError> {
        self.inner.pool.execute(move |c| Ok(c.query_row("SELECT value FROM config WHERE key = ?1", [key], |r| r.get(0)).ok())).await
    }

    async fn config_set(&self, key: &'static str, value: String) -> Result<(), AppError> {
        self.inner
            .pool
            .execute(move |c| {
                c.execute(
                    "INSERT INTO config (key, value, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
                    rusqlite::params![key, value, Utc::now().to_rfc3339()],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn global_defaults(&self) -> Result<defaults::GlobalDefaults, AppError> {
        Ok(self.config_get(defaults::DEFAULTS_KEY).await?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    pub async fn set_global_defaults(&self, d: defaults::GlobalDefaults) -> Result<defaults::GlobalDefaults, AppError> {
        d.validate()?;
        self.config_set(defaults::DEFAULTS_KEY, serde_json::to_string(&d)?).await?;
        self.refresh_caps().await;
        Ok(d)
    }

    /// The caps in force: the machine's own, with the console's on top.
    pub async fn effective_caps(&self) -> defaults::TransferCaps {
        let own = self.global_defaults().await.map(|d| d.caps).unwrap_or_default();
        let managed = self.inner.managed_caps.lock().unwrap_or_else(|e| e.into_inner()).clone();
        own.with_managed(managed.as_ref())
    }

    /// Set (or clear) the console's caps and apply.
    pub async fn set_managed_caps(&self, m: Option<ufop_core::agent::ManagedTransfers>) {
        *self.inner.managed_caps.lock().unwrap_or_else(|e| e.into_inner()) = m;
        self.refresh_caps().await;
    }

    /// Apply the effective caps: shared upload/download rates and the run gate.
    pub async fn refresh_caps(&self) {
        let caps = self.effective_caps().await;
        ufop_core::backup::backend::GLOBAL_UPLOAD.set(caps.upload_bps);
        ufop_core::backup::backend::GLOBAL_DOWNLOAD.set(caps.download_bps);
        self.resize_gate(caps.concurrency());
    }

    fn resize_gate(&self, want: usize) {
        let mut size = self.inner.gate_size.lock().unwrap_or_else(|e| e.into_inner());
        if want > *size {
            self.inner.gate.add_permits(want - *size);
        } else if want < *size {
            // Take the extra permits back as running operations release them.
            let (inner, n) = (self.inner.clone(), (*size - want) as u32);
            tokio::spawn(async move {
                if let Ok(p) = inner.gate.acquire_many(n).await {
                    p.forget();
                }
            });
        }
        *size = want;
    }

    /// A job with its inherited settings resolved (what actually runs).
    pub async fn effective_job(&self, job_id: &str) -> Result<BackupJob, AppError> {
        let j = self.get_job(job_id).await?;
        Ok(self.resolve_job(&j).await?.job)
    }

    pub async fn list_destinations(&self) -> Result<Vec<defaults::SavedDestination>, AppError> {
        Ok(self.config_get(defaults::LIBRARY_KEY).await?.and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default())
    }

    pub async fn save_destination(&self, mut d: defaults::SavedDestination) -> Result<defaults::SavedDestination, AppError> {
        d.validate()?;
        let mut all = self.list_destinations().await?;
        if d.id.is_empty() {
            d.id = uuid::Uuid::new_v4().to_string();
            d.created_at = Utc::now().to_rfc3339();
        }
        match all.iter_mut().find(|x| x.id == d.id) {
            Some(x) => *x = d.clone(),
            None => all.push(d.clone()),
        }
        self.config_set(defaults::LIBRARY_KEY, serde_json::to_string(&all)?).await?;
        Ok(d)
    }

    pub async fn delete_destination(&self, id: &str) -> Result<(), AppError> {
        let users: Vec<String> = self.inner.jobs.read().await.values()
            .filter(|j| matches!(&j.destination, BackupDestination::Library { destination_id, .. } if destination_id == id))
            .map(|j| j.name.clone())
            .collect();
        if !users.is_empty() {
            return Err(AppError::validation(format!("Still used by: {}", users.join(", "))));
        }
        let mut all = self.list_destinations().await?;
        all.retain(|x| x.id != id);
        self.config_set(defaults::LIBRARY_KEY, serde_json::to_string(&all)?).await
    }

    /// Continue an existing backup on this computer: create a job for the
    /// repository (from its stored configuration when present), map old
    /// source folders to new ones, and rebuild the local database from the
    /// destination so the version history and deduplication carry on.
    pub async fn adopt_repository(&self, dest: BackupDestination, passphrase: Option<String>, prefix: String, opts: AdoptOptions) -> Result<BackupJob, AppError> {
        let AdoptOptions { name, sources, source_map, read_only } = opts;
        let stored = self.discover_remote_config(dest.clone(), passphrase.clone(), prefix.clone()).await?;
        let mut job: BackupJob = match stored {
            Some(v) => serde_json::from_value(v).map_err(|e| AppError::validation(format!("The stored configuration is unreadable: {e}")))?,
            None => BackupJob::default(),
        };
        job.id = String::new();
        job.stats = JobStats::default();
        job.destination = dest;
        job.read_only = read_only;
        // Decided again for this computer (new job: on for encrypted backups).
        job.encrypt_local_database = None;
        job.options.prefix = prefix;
        job.encrypted = passphrase.as_deref().is_some_and(|p| !p.is_empty());
        if !job.encrypted {
            job.remote_config_include_secrets = false;
        }
        if let Some(n) = name.filter(|n| !n.trim().is_empty()) {
            job.name = n;
        } else if job.name.trim().is_empty() {
            job.name = "Continued backup".into();
        }
        if !sources.is_empty() {
            job.sources = sources;
        }
        for s in job.sources.iter_mut() {
            if let Some((from, to)) = source_map.iter().find(|(f, _)| !f.is_empty() && s.starts_with(f.as_str())) {
                *s = format!("{to}{}", &s[from.len()..]);
            }
        }
        if job.sources.is_empty() {
            return Err(AppError::validation("No stored configuration found: give the source folders to back up"));
        }
        // A shared repository: this computer becomes a new source instead
        // of taking over the history of the job whose configuration was stored.
        let probe = self.backend_for(&job.destination, Throttle::default(), RetryPolicy { retries: 1, ..Default::default() }, None).await?;
        let pfx = job.options.prefix.clone();
        if blocking(move || ufop_core::backup::shared::repository_is_shared(probe.as_ref(), &pfx)).await.unwrap_or(false) {
            job.options.identity = Default::default();
            job.options.shared_repository = true;
        }
        let job = self.save_job(job, passphrase.clone()).await?;
        let result = async {
            let backend = self.build_backend(&job).await?;
            let db_path = self.job_db_path(&job.id);
            let pfx = job.options.prefix.clone();
            let (key, want) = self.job_db_key(&job).await?;
            let lo = lock_options(&job.options);
            blocking(move || {
                repo_lock::with_lock(backend.as_ref(), &pfx, LockKind::Shared, "recreate_database", &lo, &OpContext::new(), || {
                    let o = maintenance::JobDbOptions { key: key.as_ref(), encrypt: want };
                    let (db, _) = maintenance::open_job_db(&db_path, &o, None, None, &pfx, &OpContext::new())?;
                    maintenance::recreate_database(backend.as_ref(), &db, passphrase.as_deref(), &pfx, &OpContext::new()).map(|_| ())
                })
                .map(|_| ())
            })
            .await
        }
        .await;
        if let Err(e) = result {
            let _ = self.delete_job(&job.id, true, false).await;
            return Err(e);
        }
        self.refresh_stats(&job.id, None).await;
        self.get_job(&job.id).await
    }

    /// "Open backup as it was at …": a new read-only job for this job's
    /// versioned S3 bucket read as of `at` (destination-level recovery,
    /// e.g. after ransomware rewrote or deleted volumes). Its index is
    /// rebuilt from the old object versions; restore and browse as usual.
    pub async fn open_point_in_time(&self, job_id: &str, at: chrono::DateTime<chrono::Utc>, name: Option<String>) -> Result<BackupJob, AppError> {
        let job = self.get_job(job_id).await?;
        if at > chrono::Utc::now() {
            return Err(AppError::validation("Pick a time in the past"));
        }
        let dest = match self.resolve_destination(&job.destination).await? {
            BackupDestination::Cloud { config: stores::CloudConfig::S3(mut c), credential_id } => {
                c.point_in_time = Some(at);
                BackupDestination::Cloud { config: stores::CloudConfig::S3(c), credential_id }
            }
            _ => {
                return Err(AppError::validation("Point-in-time views need a native S3 destination whose bucket has versioning enabled"));
            }
        };
        let passphrase = if job.encrypted { self.passphrase(job_id).await? } else { None };
        let name = name.filter(|n| !n.trim().is_empty()).unwrap_or_else(|| format!("{} (as of {})", job.name, at.format("%Y-%m-%d %H:%M UTC")));
        self.adopt_repository(dest, passphrase, job.options.prefix.clone(), AdoptOptions { name: Some(name), sources: job.sources.clone(), read_only: true, ..Default::default() }).await
    }

    /// Test a destination; optionally create the folder. Returns the number of files present.
    pub async fn test_destination(&self, dest: &BackupDestination, create: bool) -> Result<usize, AppError> {
        let b = self.backend_for(dest, Throttle::default(), RetryPolicy { retries: 0, ..Default::default() }, None).await?;
        blocking(move || b.test(create)).await
    }

    pub async fn test_destination_compat(&self, dest: &BackupDestination, files: usize, max_size: usize) -> Result<Vec<String>, AppError> {
        let b = self.backend_for(dest, Throttle::default(), RetryPolicy { retries: 0, ..Default::default() }, None).await?;
        blocking(move || ufop_core::backup::backend::test_backend_compatibility(b.as_ref(), files.clamp(1, 20), 1024, max_size.clamp(1024, 16 << 20))).await
    }

    // ── Read-only queries on the local database ──

    async fn with_db<T: Send + 'static>(&self, job_id: &str, f: impl FnOnce(&BackupDb) -> Result<T, AppError> + Send + 'static) -> Result<T, AppError> {
        self.with_db_in(job_id, None, f).await
    }

    /// Run `f` on the job's database, scoped to the job's own source in a
    /// shared repository, or to `source` (`*` = every source).
    async fn with_db_in<T: Send + 'static>(&self, job_id: &str, source: Option<String>, f: impl FnOnce(&BackupDb) -> Result<T, AppError> + Send + 'static) -> Result<T, AppError> {
        let job = self.get_job(job_id).await?;
        let (path, key, want) = self.open_job_db(&job).await?;
        let prefix = job.options.prefix.clone();
        let identity = versions::with_identity(&job).options.identity;
        blocking(move || {
            let o = maintenance::JobDbOptions { key: key.as_ref(), encrypt: want };
            let db = maintenance::open_job_db(&path, &o, None, None, &prefix, &OpContext::new())?.0;
            ufop_core::backup::shared::scope_db(&db, &identity)?;
            if let Some(s) = ufop_core::backup::shared::selector_scope(source.as_deref()) {
                db.set_scope(s);
            }
            f(&db)
        })
        .await
    }

    pub async fn list_versions(&self, job_id: &str) -> Result<Vec<Fileset>, AppError> {
        self.with_db(job_id, |db| db.filesets()).await
    }

    pub async fn list_files(&self, job_id: &str, version: usize, prefix: Option<String>) -> Result<Vec<ufop_core::backup::FileEntry>, AppError> {
        self.with_db(job_id, move |db| {
            let fs = db.fileset_by_version(version)?;
            let mut files = db.files(fs.id)?;
            if let Some(p) = prefix.filter(|p| !p.is_empty()) {
                files.retain(|f| f.path.starts_with(&p));
            }
            files.truncate(20_000);
            Ok(files)
        })
        .await
    }

    pub async fn search(&self, job_id: &str, pattern: String, version: Option<usize>) -> Result<Vec<maintenance::FoundFile>, AppError> {
        self.with_db(job_id, move |db| maintenance::find(db, &pattern, version, 2000)).await
    }

    pub async fn compare(&self, job_id: &str, a: usize, b: usize) -> Result<maintenance::VersionComparison, AppError> {
        self.with_db(job_id, move |db| maintenance::compare_versions(db, a, b)).await
    }

    // ── Operations ──

    fn register_task(&self, job: &BackupJob, op: &str) -> (String, Arc<AtomicBool>, Arc<Mutex<Option<Progress>>>) {
        let id = uuid::Uuid::new_v4().to_string();
        let cancel = Arc::new(AtomicBool::new(false));
        let progress = Arc::new(Mutex::new(None));
        lock(&self.inner.tasks).push(TaskSlot {
            info: TaskInfo {
                task_id: id.clone(),
                job_id: job.id.clone(),
                job_name: job.name.clone(),
                operation: op.to_string(),
                state: "queued".into(),
                queued_at: Utc::now().to_rfc3339(),
                started_at: None,
                progress: None,
                elapsed_secs: 0,
                bytes_per_sec: 0,
                eta_secs: None,
            },
            cancel: cancel.clone(),
            progress: progress.clone(),
        });
        (id, cancel, progress)
    }

    fn set_running(&self, task_id: &str) {
        if let Some(t) = lock(&self.inner.tasks).iter_mut().find(|t| t.info.task_id == task_id) {
            t.info.state = "running".into();
            t.info.started_at = Some(Utc::now().to_rfc3339());
        }
    }

    fn finish_task(&self, task_id: &str) {
        lock(&self.inner.tasks).retain(|t| t.info.task_id != task_id);
    }

    /// Request cancellation of a task (or every task of a job).
    pub fn cancel(&self, id: &str) -> bool {
        let tasks = lock(&self.inner.tasks);
        let mut hit = false;
        for t in tasks.iter().filter(|t| t.info.task_id == id || t.info.job_id == id) {
            t.cancel.store(true, Ordering::SeqCst);
            hit = true;
        }
        hit
    }

    pub fn is_job_busy(&self, job_id: &str) -> bool {
        lock(&self.inner.tasks).iter().any(|t| t.info.job_id == job_id)
    }

    /// Queue a backup in the background; returns the task id.
    pub async fn start_backup(&self, job_id: &str, dry_run: bool) -> Result<String, AppError> {
        self.ensure_accepting()?;
        let job = self.get_job(job_id).await?;
        if self.is_job_busy(job_id) {
            return Err(AppError::state("This job is already queued or running", "Wait for it to finish or cancel it."));
        }
        let (task_id, cancel, progress) = self.register_task(&job, "backup");
        let span = op_span(&job, "backup", &task_id);
        let me = self.clone();
        let tid = task_id.clone();
        tokio::spawn(
            async move {
                if let Err(e) = me.run_registered(job, BackupOperation::Backup { dry_run }, tid.clone(), cancel, progress).await {
                    tracing::warn!("backup task {tid} failed: {e}");
                }
            }
            .instrument(span),
        );
        Ok(task_id)
    }

    /// Run an operation and wait for its result.
    pub async fn execute(&self, job_id: &str, op: BackupOperation) -> Result<OperationResult, AppError> {
        self.ensure_accepting()?;
        let job = self.get_job(job_id).await?;
        let (task_id, cancel, progress) = self.register_task(&job, op.name());
        let span = op_span(&job, op.name(), &task_id);
        // Boxed so callers that await many operations keep small frames.
        Box::pin(self.run_registered(job, op, task_id, cancel, progress).instrument(span)).await
    }

    async fn run_registered(
        &self,
        job: BackupJob,
        op: BackupOperation,
        task_id: String,
        cancel: Arc<AtomicBool>,
        progress: Arc<Mutex<Option<Progress>>>,
    ) -> Result<OperationResult, AppError> {
        let job_lock = {
            let mut locks = self.inner.job_locks.lock().unwrap_or_else(|e| e.into_inner());
            locks.entry(job.id.clone()).or_default().clone()
        };
        let _job_guard = job_lock.lock().await;
        let _permit = match self.inner.gate.acquire().await {
            Ok(p) => p,
            Err(_) => {
                self.finish_task(&task_id);
                return Err(AppError::state("The backup queue is shut down", "Restart the app."));
            }
        };
        self.set_running(&task_id);
        let r = self.run_operation(job, op, cancel, progress).await;
        self.finish_task(&task_id);
        // The operation span (trace export) gets the outcome.
        let span = tracing::Span::current();
        match &r {
            Ok(o) => {
                span.record("backup.result", o.status.as_str());
                if o.status == "error" {
                    span.record("backup.error", o.summary.as_str());
                }
            }
            Err(e) => {
                span.record("backup.result", "error");
                span.record("backup.error", e.user_message().as_str());
            }
        }
        r
    }

    fn script_env(job: &BackupJob, op: &str, event: &str) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        env.insert("UFOP_EVENTNAME".into(), event.into());
        env.insert("UFOP_OPERATIONNAME".into(), op.into());
        env.insert("UFOP_BACKUP_ID".into(), job.id.clone());
        env.insert("UFOP_BACKUP_NAME".into(), job.name.clone());
        env.insert("UFOP_REMOTEURL".into(), job.destination.describe());
        env.insert("UFOP_LOCALPATH".into(), job.sources.join(std::path::MAIN_SEPARATOR_STR));
        env
    }

    async fn run_operation(
        &self,
        job: BackupJob,
        mut op: BackupOperation,
        cancel: Arc<AtomicBool>,
        progress: Arc<Mutex<Option<Progress>>>,
    ) -> Result<OperationResult, AppError> {
        let started = Utc::now();
        let op_name = op.name();
        let mut log: Vec<LogLine> = Vec::new();
        let mut job = job;
        match self.resolve_job(&job).await {
            Ok(r) => {
                log.extend(r.warnings.into_iter().map(LogLine::warn));
                job = r.job;
            }
            Err(e) => log.push(LogLine::warn(format!("Inherited settings not applied: {}", e.user_message()))),
        }
        if let BackupOperation::Restore { overrides, options } = &mut op {
            overrides.apply(&mut job);
            if options.delete_extraneous && !options.dry_run && !overrides.confirm_delete {
                let err = AppError::state(
                    "A mirror restore deletes files and needs confirmation",
                    "Run it as a dry run first to see what would be deleted, then confirm (confirm_delete: true in the API).",
                );
                return self.record(&job, op_name, started, Err(err), log).await;
            }
            if !options.use_cache && self.cache_settings().await.is_ok_and(|c| c.use_for_restore) {
                options.use_cache = true;
            }
        }
        match self.resolve_job_secrets(&job).await {
            Ok(j) => job = j,
            Err(e) => return self.record(&job, op_name, started, Err(e), log).await,
        }
        if job.read_only && op.writes_destination() {
            let err = Self::read_only_refusal(&job, &format!("run '{op_name}'"));
            return self.record(&job, op_name, started, Err(err), log).await;
        }
        let _awake = job.prevent_sleep.then(|| power::SleepInhibitor::acquire(&format!("UFOP {op_name}: {}", job.name)));
        let is_backup = matches!(op, BackupOperation::Backup { .. });

        // Asynchronous hooks report here while the operation runs.
        let async_log = hooks::AsyncHookLog::open();
        if is_backup {
            if let Some(cmd) = job.scripts.run_before.as_deref().filter(|c| !c.trim().is_empty()) {
                let env = Self::script_env(&job, op_name, "BEFORE");
                match job.scripts.before_mode() {
                    hooks::HookMode::Async => {
                        hooks::spawn_script(cmd, &env, job.scripts.timeout_secs, "Pre-backup script", async_log.clone());
                        log.push(LogLine::info("Pre-backup script started (async, not waited for)").tagged("script"));
                    }
                    mode => {
                        let out = hooks::run_script(cmd, &env, job.scripts.timeout_secs).await?;
                        log.extend(out.log.clone());
                        if mode == hooks::HookMode::Required && !out.succeeded() {
                            let err = AppError::state("The required pre-backup script failed", "Fix the script or make it optional.");
                            return self.record(&job, op_name, started, Err(err), log).await;
                        }
                    }
                }
            }
        }

        let release_log: Arc<Mutex<Vec<LogLine>>> = Arc::default();
        let result: Result<serde_json::Value, AppError> = async {
            self.check_policy(&job).await?;
            let backend = self.build_backend(&job).await?;
            let pass = if job.encrypted { self.passphrase(&job.id).await? } else { None };
            if job.encrypted && pass.is_none() {
                return Err(AppError::validation("This job's passphrase is missing from the credential store; edit the job to set it"));
            }
            let db_path = self.job_db_path(&job.id);
            let (db_key, db_encrypt) = self.job_db_key(&job).await?;
            let kdf = self.inner.kdf;
            let job2 = job.clone();
            let prog = progress.clone();
            let mut ctx = OpContext::new().with_cancel_flag(cancel.clone()).with_progress(move |p| *lock(&prog) = Some(p.clone()));
            if let Some(cmd) = job.scripts.run_on_source_release.clone().filter(|c| is_backup && !c.trim().is_empty()) {
                // Runs on the engine thread; it waits so the log is complete.
                let (env, timeout, handle, sink) = (Self::script_env(&job, op_name, "SOURCE_RELEASED"), job.scripts.timeout_secs, tokio::runtime::Handle::current(), release_log.clone());
                ctx = ctx.with_source_released(move || {
                    let lines = match handle.block_on(hooks::run_script(&cmd, &env, timeout)) {
                        Ok(out) => out.log,
                        Err(e) => vec![LogLine::warn(format!("Source-release script: {}", e.user_message()))],
                    };
                    lock(&sink).extend(lines);
                });
            }
            if let BackupOperation::Replicate { replica_id } = &op {
                return self.replicate(&job2, replica_id.as_deref(), &ctx).await;
            }
            if let BackupOperation::CopyVersions { versions, source, target, dry_run } = &op {
                let o = ufop_core::backup::copy::CopyOptions { versions: versions.clone(), source: source.clone(), dry_run: *dry_run };
                return self.copy_versions(&job2, o, target.clone(), &ctx).await;
            }
            let after: Vec<String> = if is_backup {
                job2.replicas.iter().filter(|r| r.enabled && r.schedule == ReplicaSchedule::AfterBackup).map(|r| r.id.clone()).collect()
            } else {
                Vec::new()
            };
            let ctx2 = ctx.clone();
            let cache = self.volume_cache().await.unwrap_or_else(|e| {
                tracing::warn!("Volume cache unavailable: {}", e.user_message());
                None
            });
            let mut v = blocking(move || {
                let dbo = maintenance::JobDbOptions { key: db_key.as_ref(), encrypt: db_encrypt };
                run_core(&job2, op, backend.as_ref(), &db_path, &dbo, pass.as_deref(), kdf, cache, &ctx)
            })
            .await?;
            let clean = v.get("errors").and_then(|e| e.as_array()).map_or(true, |a| a.is_empty()) && v.get("dry_run") != Some(&serde_json::Value::Bool(true));
            if clean {
                let mut reps = Vec::new();
                for id in after {
                    match self.replicate(&job, Some(&id), &ctx2).await {
                        Ok(r) => {
                            if let (Some(errs), Some(o)) = (r.get("errors").and_then(|e| e.as_array()).cloned(), v.as_object_mut()) {
                                // Replica problems surface as warnings of the backup.
                                if let Some(w) = o.get_mut("warnings").and_then(|w| w.as_array_mut()) {
                                    w.extend(errs.into_iter().map(|e| serde_json::Value::String(format!("Replica {}", e.as_str().unwrap_or_default()))));
                                }
                            }
                            reps.push(r);
                        }
                        Err(e) => {
                            if let Some(w) = v.get_mut("warnings").and_then(|w| w.as_array_mut()) {
                                w.push(serde_json::Value::String(format!("Replica: {}", e.user_message())));
                            }
                        }
                    }
                }
                if !reps.is_empty() {
                    if let Some(o) = v.as_object_mut() {
                        o.insert("replication".into(), serde_json::Value::Array(reps));
                    }
                }
            }
            Ok(v)
        }
        .await;

        log.append(&mut lock(&release_log));
        log.extend(async_log.close());
        if cancel.load(Ordering::SeqCst) {
            log.push(LogLine::warn("Operation was cancelled"));
        }
        self.record(&job, op_name, started, result, log).await
    }

    async fn record(
        &self,
        job: &BackupJob,
        op: &str,
        started: DateTime<Utc>,
        result: Result<serde_json::Value, AppError>,
        mut log: Vec<LogLine>,
    ) -> Result<OperationResult, AppError> {
        let finished = Utc::now();
        let (status, summary, report, err) = match &result {
            Ok(v) => {
                let errors = v.get("errors").and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0);
                let warnings = v.get("warnings").and_then(|e| e.as_array()).map(|a| a.len()).unwrap_or(0);
                for i in v.get("info").and_then(|e| e.as_array()).into_iter().flatten() {
                    log.push(LogLine::info(i.as_str().unwrap_or_default()).tagged("engine"));
                }
                for w in v.get("warnings").and_then(|e| e.as_array()).into_iter().flatten() {
                    log.push(LogLine::warn(w.as_str().unwrap_or_default()));
                }
                for e in v.get("errors").and_then(|e| e.as_array()).into_iter().flatten() {
                    log.push(LogLine::error(e.as_str().unwrap_or_default()));
                }
                let status = if errors > 0 { "error" } else if warnings > 0 { "warning" } else { "success" };
                (status.to_string(), summarize(op, v), v.clone(), None)
            }
            Err(e) => {
                let msg = e.user_message();
                log.push(LogLine::error(msg.clone()));
                let status = if msg.contains("cancelled") { "cancelled" } else { "error" };
                (status.to_string(), msg.clone(), serde_json::json!({}), Some(msg))
            }
        };

        // Post-operation script gets the result.
        if op == "backup" {
            if let Some(cmd) = job.scripts.run_after.as_deref().filter(|c| !c.trim().is_empty()) {
                let mut env = Self::script_env(job, op, "AFTER");
                env.insert("UFOP_PARSED_RESULT".into(), status.clone());
                let rf = ufop_core::session::user_temp_subdir("results").join(format!("{}.txt", uuid::Uuid::new_v4().simple()));
                let body = if job.scripts.result_format == "text" { summary.clone() } else { report.to_string() };
                if std::fs::write(&rf, body).is_ok() {
                    env.insert("UFOP_RESULTFILE".into(), rf.to_string_lossy().into_owned());
                }
                if job.scripts.after_mode() == hooks::HookMode::Async {
                    // The run ends now: the outcome goes to the app log; the
                    // result file is removed once the script is done with it.
                    let h = hooks::spawn_script(cmd, &env, job.scripts.timeout_secs, "Post-backup script", hooks::AsyncHookLog::default());
                    tokio::spawn(async move {
                        let _ = h.await;
                        let _ = std::fs::remove_file(rf);
                    });
                    log.push(LogLine::info("Post-backup script started (async, not waited for)").tagged("script"));
                } else {
                    if let Ok(out) = hooks::run_script(cmd, &env, job.scripts.timeout_secs).await {
                        log.extend(out.log);
                    }
                    let _ = std::fs::remove_file(rf);
                }
            }
        }

        let (summary, err) = if job.masked_values.is_empty() {
            (summary, err)
        } else {
            let mask = |t: &str| job.masked_values.iter().fold(t.to_string(), |acc, v| acc.replace(v.as_str(), "***"));
            for l in &mut log {
                l.message = mask(&l.message);
            }
            (mask(&summary), err.map(|e| mask(&e)))
        };
        let run = BackupRun {
            id: uuid::Uuid::new_v4().to_string(),
            job_id: job.id.clone(),
            operation: op.to_string(),
            status: status.clone(),
            started_at: started.to_rfc3339(),
            finished_at: Some(finished.to_rfc3339()),
            summary: summary.clone(),
            report: report.clone(),
            log: log.clone(),
        };
        self.insert_run(&run).await?;
        let previous_result = self.inner.jobs.read().await.get(&job.id).and_then(|j| j.stats.last_result.clone());
        self.refresh_stats(&job.id, Some((&run, err.clone(), (finished - started).num_milliseconds().max(0) as u64))).await;

        // Reports and ledger are fail-open.
        let mut vars = BTreeMap::new();
        vars.insert("name".into(), job.name.clone());
        vars.insert("operation".into(), op.to_string());
        vars.insert("result".into(), status.clone());
        vars.insert("machine".into(), job.reports.machine_name.clone().unwrap_or_else(hooks::machine_name));
        vars.insert("backup_id".into(), job.reports.backup_id.clone().unwrap_or_else(|| job.id.clone()));
        vars.insert("duration".into(), format!("{}s", (finished - started).num_seconds()));
        vars.insert("files".into(), report.get("files_examined").map(|v| v.to_string()).unwrap_or_default());
        vars.insert("bytes".into(), report.get("bytes_uploaded").map(|v| v.to_string()).unwrap_or_default());
        vars.insert("errors".into(), err.clone().unwrap_or_default());
        vars.insert("job_id".into(), job.id.clone());
        vars.insert("previous_result".into(), previous_result.unwrap_or_default());
        for p in logs::write_run(&job.log_outputs, &job.name, op, &status, &summary, &log) {
            tracing::warn!("{p}");
        }
        let lines: Vec<&LogLine> = log.iter().filter(|l| l.level >= job.reports.min_log_level).take(job.reports.max_log_lines).collect();
        let mut payload = report.clone();
        if let Some(o) = payload.as_object_mut() {
            o.insert("log".into(), serde_json::to_value(&lines).unwrap_or_default());
        }
        let mut text = format!("{summary}\n");
        for l in &lines {
            text.push_str(&format!("{:?}: {}\n", l.level, l.message));
        }
        if job.reports.redact_paths {
            // Redact through the serialized form so every string is covered.
            let red = notify::redact_paths(&payload.to_string(), &job.sources);
            payload = serde_json::from_str(&red).unwrap_or(payload);
            text = notify::redact_paths(&text, &job.sources);
            for v in vars.values_mut() {
                *v = notify::redact_paths(v, &job.sources);
            }
        }
        if let Some(l) = self.inner.run_listener.get() {
            let mut data = payload.clone();
            if let Some(o) = data.as_object_mut() {
                o.remove("log");
            }
            let stats = self.inner.jobs.read().await.get(&job.id).map(|j| j.stats.clone()).unwrap_or_default();
            l(RunEvent {
                job_id: job.id.clone(),
                job_name: job.name.clone(),
                operation: op.to_string(),
                status: status.clone(),
                started_at: started.to_rfc3339(),
                finished_at: finished.to_rfc3339(),
                duration_ms: (finished - started).num_milliseconds().max(0) as u64,
                summary: summary.clone(),
                report: data,
                stats,
                schedule_interval_minutes: job.schedule.enabled.then(|| job.schedule.nominal_interval_minutes()),
            });
        }
        let reports = self.prepare_reports(&job.reports).await;
        for f in hooks::send_reports(&reports, &status, &vars, payload).await {
            tracing::warn!("{f}");
        }
        let (channels, warnings) = self.job_notifications(job).await;
        for w in warnings {
            tracing::warn!("{w}");
        }
        if !channels.is_empty() {
            let mut list = Vec::new();
            for n in channels {
                list.push(self.prepare_notification(n).await);
            }
            for f in notify::deliver(&list, &status, &vars, &text).await {
                tracing::warn!("Notification failed: {f}");
            }
        }
        if let Some(n) = self.inner.notifier.get() {
            if job.notify_on.iter().any(|s| s == &status) {
                n(&format!("Backup {status}: {}", job.name), &summary, &status);
            }
        }
        if let Some(ledger) = &self.inner.ledger {
            let st = match status.as_str() {
                "error" => LedgerStatus::Failed,
                "cancelled" => LedgerStatus::Cancelled,
                _ => LedgerStatus::Ok,
            };
            let mut ev = RecordEvent::new(LedgerEngine::Backup, format!("backup.{op}"))
                .status(st)
                .correlation(job.id.clone())
                .summary(format!("{}: {summary}", job.name))
                .target(job.destination.describe());
            if let Some(b) = report.get("bytes_uploaded").and_then(|v| v.as_i64()) {
                ev = ev.bytes(b);
            }
            ledger.record(ev).await;
        }

        match (result, err) {
            (Err(e), _) => Err(e),
            _ => Ok(OperationResult { run_id: run.id, status, summary, report, log }),
        }
    }

    async fn insert_run(&self, run: &BackupRun) -> Result<(), AppError> {
        let r = run.clone();
        let report = serde_json::to_string(&r.report)?;
        let log = serde_json::to_string(&r.log)?;
        self.inner
            .pool
            .execute(move |c| {
                c.execute(
                    "INSERT INTO backup_runs(id, job_id, operation, status, started_at, finished_at, summary, report_json, log_json)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![r.id, r.job_id, r.operation, r.status, r.started_at, r.finished_at, r.summary, report, log],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn run_history(&self, job_id: Option<String>, limit: u32) -> Result<Vec<BackupRun>, AppError> {
        self.inner
            .pool
            .execute(move |c| {
                let mut st = c.prepare(
                    "SELECT id, job_id, operation, status, started_at, finished_at, summary, report_json, log_json FROM backup_runs
                     WHERE (?1 IS NULL OR job_id = ?1) ORDER BY started_at DESC LIMIT ?2",
                )?;
                let v = st.query_map(params![job_id, limit.clamp(1, 1000)], run_row)?.collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })
            .await
    }

    /// [`Self::run_history`], or one page of the whole history (sorted by a
    /// [`paging::RUN_SORT_FIELDS`] column, newest first by default, text
    /// filter on the summary and operation) when `page` is given.
    pub async fn run_history_listing(&self, job_id: Option<String>, limit: u32, page: Option<paging::PageRequest>) -> Result<paging::Listing<BackupRun>, AppError> {
        let Some(page) = page else {
            return Ok(paging::Listing::All(self.run_history(job_id, limit).await?));
        };
        page.check_sort(paging::RUN_SORT_FIELDS)?;
        self.inner
            .pool
            .execute(move |c| {
                // The column comes from the whitelist above, never from the caller's text.
                let col = paging::RUN_SORT_FIELDS.iter().find(|f| page.sort.as_deref() == Some(**f)).copied().unwrap_or("started_at");
                let dir = if page.sort.is_none() || page.descending { "DESC" } else { "ASC" };
                let like = page.needle().map(|n| format!("%{}%", n.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")));
                let filter = "(?1 IS NULL OR job_id = ?1) AND (?2 IS NULL OR lower(summary) LIKE ?2 ESCAPE '\\' OR lower(operation) LIKE ?2 ESCAPE '\\')";
                let total: i64 = c.query_row(&format!("SELECT COUNT(*) FROM backup_runs WHERE {filter}"), params![job_id, like], |r| r.get(0))?;
                let take = page.take();
                let mut st = c.prepare(&format!(
                    "SELECT id, job_id, operation, status, started_at, finished_at, summary, report_json, log_json FROM backup_runs
                     WHERE {filter} ORDER BY {col} {dir}, started_at DESC, id LIMIT ?3 OFFSET ?4"
                ))?;
                let items = st.query_map(params![job_id, like, take as i64, page.offset as i64], run_row)?.collect::<Result<Vec<_>, _>>()?;
                Ok(paging::Listing::Page(paging::Page { items, total: total.max(0) as usize, offset: page.offset, limit: take }))
            })
            .await
    }

    /// Delete run history older than `days`.
    pub async fn prune_history(&self, days: u32) -> Result<usize, AppError> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        self.inner.pool.execute(move |c| Ok(c.execute("DELETE FROM backup_runs WHERE started_at < ?1", [cutoff])?)).await
    }

    async fn refresh_stats(&self, job_id: &str, run: Option<(&BackupRun, Option<String>, u64)>) {
        let path = self.job_db_path(job_id);
        let key = self.stored_db_key(job_id).await.ok().flatten();
        let identity = self.inner.jobs.read().await.get(job_id).map(versions::job_identity);
        let counts = blocking(move || {
            let db = BackupDb::open_with_key(&path, key.as_ref())?;
            // Shared repositories: this job's own versions.
            if let Some(i) = &identity {
                ufop_core::backup::shared::scope_db(&db, i)?;
            }
            let fs = db.filesets()?;
            let backend: u64 = db.volumes(None)?.iter().map(|v| v.size).sum();
            Ok((fs.len() as u64, fs.first().map(|f| f.total_size).unwrap_or(0), backend))
        })
        .await
        .unwrap_or((0, 0, 0));
        let snapshot = {
            let mut jobs = self.inner.jobs.write().await;
            let Some(job) = jobs.get_mut(job_id) else { return };
            job.stats.version_count = counts.0;
            job.stats.source_size = counts.1;
            job.stats.backend_size = counts.2;
            if let Some((r, err, ms)) = run {
                if r.operation == "backup" {
                    job.stats.last_run_at = r.finished_at.clone();
                    job.stats.last_result = Some(r.status.clone());
                    job.stats.last_duration_ms = Some(ms);
                    job.stats.last_error = err;
                }
            }
            job.clone()
        };
        if let Err(e) = self.persist(&snapshot).await {
            tracing::warn!("Failed to persist backup stats: {e}");
        }
    }

    // ── Direct restore without a configured job ──

    /// List versions of any repository (rebuilds a temporary database).
    pub async fn browse_remote(&self, dest: BackupDestination, passphrase: Option<String>, prefix: String) -> Result<Vec<Fileset>, AppError> {
        let backend = self.backend_for(&dest, Throttle::default(), RetryPolicy::default(), None).await?;
        blocking(move || {
            repo_lock::with_lock(backend.as_ref(), &prefix, LockKind::Shared, "list", &lock_options(&BackupOptions::default()), &OpContext::new(), || {
                let db = BackupDb::open_in_memory()?;
                maintenance::recreate_database(backend.as_ref(), &db, passphrase.as_deref(), &prefix, &OpContext::new())?;
                db.filesets()
            })
            .map(|(v, _)| v)
        })
        .await
    }

    // ── Local database tools ──

    /// Every job database in the data folder, with its owner job (if any).
    pub async fn database_inventory(&self) -> Result<Vec<DatabaseEntry>, AppError> {
        let jobs: HashMap<String, String> = self.inner.jobs.read().await.values().map(|j| (j.id.clone(), j.name.clone())).collect();
        let dir = self.inner.data_dir.clone();
        blocking(move || {
            let mut out = Vec::new();
            let rd = match std::fs::read_dir(&dir) {
                Ok(r) => r,
                Err(_) => return Ok(out),
            };
            for e in rd.flatten() {
                let name = e.file_name().to_string_lossy().into_owned();
                let Some(id) = name.strip_suffix(".sqlite") else { continue };
                let (info, error) = match BackupDb::inspect(&e.path()) {
                    Ok(i) => (Some(i), None),
                    Err(err) => (None, Some(err.user_message())),
                };
                out.push(DatabaseEntry { job_id: id.to_string(), job_name: jobs.get(id).cloned(), info, error });
            }
            out.sort_by(|a, b| a.job_name.cmp(&b.job_name).then(a.job_id.cmp(&b.job_id)));
            Ok(out)
        })
        .await
    }

    /// Delete orphan databases, upgrade old schemas and vacuum the rest.
    /// Databases of jobs with a running or queued task are left alone.
    pub async fn cleanup_databases(&self, delete_orphans: bool, vacuum: bool) -> Result<DatabaseCleanupReport, AppError> {
        let inv = self.database_inventory().await?;
        let busy: std::collections::HashSet<String> = lock(&self.inner.tasks).iter().map(|t| t.info.job_id.clone()).collect();
        let mut keys: HashMap<String, DbKey> = HashMap::new();
        for e in inv.iter().filter(|e| e.info.as_ref().is_some_and(|i| i.encrypted)) {
            if let Ok(Some(k)) = self.stored_db_key(&e.job_id).await {
                keys.insert(e.job_id.clone(), k);
            }
        }
        let dir = self.inner.data_dir.clone();
        blocking(move || {
            let mut r = DatabaseCleanupReport::default();
            for e in inv {
                let path = dir.join(format!("{}.sqlite", e.job_id));
                let size = |p: &Path| -> u64 { ["", "-wal", "-shm"].iter().filter_map(|x| std::fs::metadata(format!("{}{x}", p.display())).ok()).map(|m| m.len()).sum() };
                r.bytes_before += size(&path);
                if busy.contains(&e.job_id) {
                    r.bytes_after += size(&path);
                    continue;
                }
                if e.job_name.is_none() {
                    if delete_orphans {
                        for ext in ["", "-wal", "-shm"] {
                            let _ = std::fs::remove_file(format!("{}{ext}", path.display()));
                        }
                        r.orphans_deleted.push(e.job_id);
                    } else {
                        r.bytes_after += size(&path);
                    }
                    continue;
                }
                if e.info.as_ref().is_some_and(|i| i.schema_version < i.latest_schema) {
                    match BackupDb::migrate(&path, true) {
                        Ok(_) => r.migrated += 1,
                        Err(err) => r.errors.push(format!("{}: {}", e.job_id, err.user_message())),
                    }
                }
                if vacuum {
                    match BackupDb::open_with_key(&path, keys.get(&e.job_id)).and_then(|db| db.vacuum()) {
                        Ok(()) => r.vacuumed += 1,
                        Err(err) => r.errors.push(format!("{}: {}", e.job_id, err.user_message())),
                    }
                }
                r.bytes_after += size(&path);
            }
            Ok(r)
        })
        .await
    }

    /// Send one message through a notification channel, resolving its
    /// secret (password, bot token or webhook URL) from the credential
    /// store. Used by automation's SendMessage action.
    pub async fn send_message(&self, n: notify::Notification, title: &str, body: &str) -> Result<(), AppError> {
        let (mut n, secret) = self.prepare_notification(n).await;
        n.send_on = vec!["message".into()];
        n.template.clear();
        if let notify::Channel::Email { subject, .. } = &mut n.channel {
            *subject = title.to_string();
        }
        let vars: std::collections::BTreeMap<String, String> =
            [("name", title), ("operation", "automation"), ("result", "message")].into_iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let body = match &n.channel {
            // Slack/Teams show the title separately; email has it as subject.
            notify::Channel::Telegram { .. } | notify::Channel::Xmpp { .. } => format!("{title}\n\n{body}"),
            _ => body.to_string(),
        };
        match notify::deliver(&[(n, secret)], "message", &vars, &body).await.into_iter().next() {
            Some(e) => Err(AppError::connection(e, "Check the channel settings with Send test in Backup → Notifications.")),
            None => Ok(()),
        }
    }

    /// Send a test message through one notification channel.
    pub async fn test_notification(&self, n: notify::Notification) -> Result<(), AppError> {
        n.validate()?;
        let (mut n, secret) = self.prepare_notification(n).await;
        n.send_on = vec!["test".into()];
        let vars: std::collections::BTreeMap<String, String> = [("name", "Test"), ("operation", "test"), ("result", "test"), ("machine", "this computer"), ("backup_id", "test"), ("job_id", "test")]
            .into_iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        match notify::deliver(&[(n, secret)], "test", &vars, "This is a test notification from UFOP.").await.into_iter().next() {
            Some(e) => Err(AppError::validation(e)),
            None => Ok(()),
        }
    }

    // ── Support: bug reports and diagnostics ──

    /// Write a sanitized bug-report zip for a job (see [`bugreport`]).
    pub async fn bug_report(&self, job_id: &str, out_path: String) -> Result<bugreport::BugReport, AppError> {
        if out_path.trim().is_empty() {
            return Err(AppError::validation("Choose where to save the bug report"));
        }
        let job = self.get_job(job_id).await?;
        let runs = self.run_history(Some(job_id.to_string()), 20).await?;
        let db = self.job_db_path(job_id);
        let key = self.stored_db_key(job_id).await?;
        blocking(move || {
            let log = job.log_outputs.file.clone().filter(|f| !f.trim().is_empty()).map(PathBuf::from);
            bugreport::create(Path::new(&out_path), &job, &db, key.as_ref(), &runs, log.as_deref())
        })
        .await
    }

    /// The network endpoint behind a destination, for diagnostics.
    async fn destination_endpoint(&self, dest: &BackupDestination) -> Result<Option<(String, u16, bool)>, AppError> {
        Ok(match self.resolve_destination(dest).await? {
            BackupDestination::Cloud { config, .. } => config.network_endpoint(),
            BackupDestination::Connection { connection_id, .. } => {
                let Some(conns) = self.inner.conns.as_ref() else { return Ok(None) };
                let Ok(id) = uuid::Uuid::parse_str(&connection_id) else { return Ok(None) };
                conns.get_connection(id).await?.and_then(|p| {
                    use crate::core::types::ConnectionProtocol as P;
                    let (port, tls) = match p.protocol {
                        P::Sftp => (22, false),
                        P::Ftp => (21, false),
                        P::Ftps => (990, false),
                        P::WebDav | P::S3 | P::GoogleDrive | P::Dropbox | P::OneDrive | P::AzureBlob | P::AdlsGen2 | P::AzureFiles | P::Swift => (443, true),
                        P::Smb => (445, false),
                        P::Nfs => (2049, false),
                        P::Local | P::Rclone => return None,
                    };
                    let host = p.host.trim_start_matches("https://").trim_start_matches("http://").split('/').next().unwrap_or_default().to_string();
                    (!host.is_empty()).then(|| (host, p.port.unwrap_or(port), tls))
                })
            }
            _ => None,
        })
    }

    /// Check the job database against the destination and measure the
    /// network path. `throughput_kb` = 0 skips the upload/download test.
    pub async fn diagnose(&self, job_id: &str, network: bool, throughput_kb: u32) -> Result<diagnostics::Diagnostics, AppError> {
        let job = self.resolve_job_secrets(&self.effective_job(job_id).await?).await?;
        let mut d = diagnostics::Diagnostics { snapshot_providers: ufop_core::backup::snapshot::providers(), ..Default::default() };
        if network {
            if let Some((host, port, tls)) = self.destination_endpoint(&job.destination).await? {
                let timeout = std::time::Duration::from_secs(if job.network_timeout_secs > 0 { job.network_timeout_secs } else { 15 });
                d.network = Some(diagnostics::probe(&host, port, tls, &notify::CertPolicy::default(), timeout).await);
            }
        }
        let backend = self.build_backend(&job).await?;
        let path = self.job_db_path(job_id);
        let prefix = job.options.prefix.clone();
        let size = if job.read_only { 0 } else { (throughput_kb.min(64 * 1024) as usize) * 1024 };
        let key = self.stored_db_key(job_id).await?;
        let (db, thr) = blocking(move || {
            let db = if path.is_file() {
                BackupDb::open_with_key(&path, key.as_ref()).and_then(|db| diagnostics::check_database(&db, BackupDb::inspect(&path).ok(), backend.as_ref(), &prefix)).map_err(|e| e.user_message())
            } else {
                Err("This job has no local database yet; run a backup or repair it first".to_string())
            };
            let thr = (size > 0).then(|| diagnostics::throughput(backend.as_ref(), size));
            Ok((db, thr))
        })
        .await?;
        match db {
            Ok(c) => d.database = Some(c),
            Err(e) => d.database_error = Some(e),
        }
        d.throughput = thr;
        Ok(d)
    }

    // ── Recovery tool (no job database needed) ──

    pub async fn recovery_download(&self, dest: BackupDestination, passphrase: Option<String>, prefix: String, folder: String, opts: recovery::DownloadOptions) -> Result<recovery::DownloadReport, AppError> {
        let backend = self.backend_for(&dest, Throttle::default(), RetryPolicy::default(), None).await?;
        blocking(move || {
            repo_lock::with_lock(backend.as_ref(), &prefix, LockKind::Shared, "recovery_download", &lock_options(&BackupOptions::default()), &OpContext::new(), || {
                recovery::download(backend.as_ref(), passphrase.as_deref(), &prefix, Path::new(&folder), &opts, &OpContext::new())
            })
            .map(|(r, _)| r)
        })
        .await
    }

    pub async fn recovery_index(&self, folder: String) -> Result<recovery::IndexReport, AppError> {
        blocking(move || recovery::build_index(Path::new(&folder), &OpContext::new())).await
    }

    pub async fn recovery_list(&self, folder: String) -> Result<Vec<recovery::RecoveryVersion>, AppError> {
        blocking(move || recovery::list_versions(Path::new(&folder))).await
    }

    pub async fn recovery_restore(&self, folder: String, version: usize, paths: Vec<String>, target: String) -> Result<recovery::RecoveryRestoreReport, AppError> {
        blocking(move || recovery::restore(Path::new(&folder), version, &paths, Path::new(&target), &OpContext::new())).await
    }

    // ── Standalone file encryption ──

    pub async fn crypt_file(&self, decrypt: bool, input: String, output: String, passphrase: String, preset: Option<String>) -> Result<ufop_core::backup::filecrypt::CryptReport, AppError> {
        if passphrase.is_empty() {
            return Err(AppError::validation("Enter a passphrase"));
        }
        let kdf = match preset.as_deref() {
            None | Some("") => self.inner.kdf,
            Some(p) => KdfParams::preset(p).ok_or_else(|| AppError::validation(format!("Unknown key-derivation preset '{p}'")))?,
        };
        blocking(move || {
            let (i, o) = (Path::new(&input), Path::new(&output));
            if decrypt { ufop_core::backup::filecrypt::decrypt_file(i, o, &passphrase) } else { ufop_core::backup::filecrypt::encrypt_file(i, o, &passphrase, kdf) }
        })
        .await
    }

    pub async fn decrypt_volume(&self, volume: String, header: String, output: String, passphrase: Option<String>) -> Result<ufop_core::backup::filecrypt::CryptReport, AppError> {
        blocking(move || ufop_core::backup::filecrypt::decrypt_volume(Path::new(&volume), Path::new(&header), Path::new(&output), passphrase.as_deref().filter(|p| !p.is_empty()))).await
    }

    pub async fn recovery_convert(&self, folder: String, out: BackupDestination, opts: recovery::ConvertOptions) -> Result<recovery::ConvertReport, AppError> {
        let backend = self.backend_for(&out, Throttle::default(), RetryPolicy::default(), None).await?;
        let opts = recovery::ConvertOptions { kdf: self.inner.kdf, ..opts };
        blocking(move || {
            backend.create_folder()?;
            repo_lock::with_lock(backend.as_ref(), &opts.prefix, LockKind::Exclusive, "convert", &lock_options(&BackupOptions::default()), &OpContext::new(), || {
                recovery::convert(Path::new(&folder), backend.as_ref(), &opts, &OpContext::new())
            })
            .map(|(r, _)| r)
        })
        .await
    }

    /// Restore from any repository using only its credentials and passphrase.
    pub async fn direct_restore(&self, dest: BackupDestination, passphrase: Option<String>, prefix: String, options: RestoreOptions) -> Result<ufop_core::backup::RestoreReport, AppError> {
        let backend = self.backend_for(&dest, Throttle::default(), RetryPolicy::default(), None).await?;
        blocking(move || {
            let opts = BackupOptions { prefix: prefix.clone(), ..Default::default() };
            let (mut r, w) = repo_lock::with_lock(backend.as_ref(), &prefix, LockKind::Shared, "restore", &lock_options(&opts), &OpContext::new(), || {
                let db = BackupDb::open_in_memory()?;
                maintenance::recreate_database(backend.as_ref(), &db, passphrase.as_deref(), &prefix, &OpContext::new())?;
                let repo = Repo::open(backend.as_ref(), &db, passphrase.as_deref(), &opts, false, KdfParams::default())?;
                ufop_core::backup::run_restore(&repo, &options, &OpContext::new())
            })?;
            r.warnings.extend(w);
            Ok(r)
        })
        .await
    }

    /// Read the job configuration embedded in a remote repository.
    pub async fn discover_remote_config(&self, dest: BackupDestination, passphrase: Option<String>, prefix: String) -> Result<Option<serde_json::Value>, AppError> {
        let backend = self.backend_for(&dest, Throttle::default(), RetryPolicy::default(), None).await?;
        blocking(move || maintenance::read_remote_task_config(backend.as_ref(), passphrase.as_deref(), &prefix)).await
    }

    // ── Scheduler ──

    pub fn pause(&self, minutes: Option<u32>) {
        *lock(&self.inner.paused) = Some(minutes.map(|m| Utc::now() + chrono::Duration::minutes(m as i64)));
    }

    pub fn resume(&self) {
        *lock(&self.inner.paused) = None;
    }

    fn is_paused(&self) -> bool {
        let mut p = lock(&self.inner.paused);
        match *p {
            None => false,
            Some(None) => true,
            Some(Some(until)) if until > Utc::now() => true,
            Some(Some(_)) => {
                *p = None;
                false
            }
        }
    }

    pub async fn status(&self) -> SchedulerStatus {
        let tasks = lock(&self.inner.tasks)
            .iter()
            .map(|t| {
                let progress = lock(&t.progress).clone();
                let elapsed = t.info.started_at.as_deref().and_then(parse_ts).map(|s| (Utc::now() - s).num_milliseconds().max(0) as f64 / 1000.0).unwrap_or(0.0);
                let (bytes_per_sec, eta_secs) = progress.as_ref().map(|p| progress_rate(p, elapsed)).unwrap_or((0, None));
                TaskInfo { progress, elapsed_secs: elapsed as u64, bytes_per_sec, eta_secs, ..t.info.clone() }
            })
            .collect();
        let paused = self.is_paused();
        let paused_until = lock(&self.inner.paused).and_then(|p| p.map(|t| t.to_rfc3339()));
        let mut next_runs = Vec::new();
        let mut next_maintenance = Vec::new();
        let at = |n: DateTime<Utc>| n.max(Utc::now()).to_rfc3339();
        for (j, eff) in self.resolved_jobs().await {
            let last = j.stats.last_scheduled_at.as_deref().unwrap_or(j.created_at.as_str());
            if let Some(n) = parse_ts(last).and_then(|l| eff.schedule.next_after(l)) {
                next_runs.push(NextRun { job_id: j.id.clone(), job_name: j.name.clone(), next_run_at: at(n), operation: "backup".into() });
            }
            if j.read_only {
                continue;
            }
            for task in schedule::MAINTENANCE_TASKS {
                let Some(s) = eff.maintenance.schedule_of(task) else { continue };
                let last = j.stats.last_maintenance.get(task).map(String::as_str).unwrap_or(j.created_at.as_str());
                if let Some(n) = parse_ts(last).and_then(|l| s.next_after(l)) {
                    next_maintenance.push(NextRun { job_id: j.id.clone(), job_name: j.name.clone(), next_run_at: at(n), operation: task.to_string() });
                }
            }
        }
        next_runs.sort_by(|a, b| a.next_run_at.cmp(&b.next_run_at));
        next_maintenance.sort_by(|a, b| a.next_run_at.cmp(&b.next_run_at));
        SchedulerStatus { running: self.inner.scheduler_running.load(Ordering::SeqCst), paused, paused_until, tasks, next_runs, next_maintenance }
    }

    /// Jobs whose schedule slot has passed (missed runs included).
    pub async fn due_jobs(&self, now: DateTime<Utc>) -> Vec<String> {
        if self.is_paused() {
            return Vec::new();
        }
        let on_battery = power::on_battery() == Some(true);
        let mut out = Vec::new();
        for (j, eff) in self.resolved_jobs().await {
            if j.read_only {
                // Read-only connections never back up.
                continue;
            }
            let last = j.stats.last_scheduled_at.as_deref().or(Some(j.created_at.as_str())).and_then(parse_ts);
            let due = last.is_some_and(|l| eff.schedule.is_due(l, now)) && !self.is_job_busy(&j.id);
            if due && on_battery && eff.schedule.skip_on_battery {
                tracing::info!("Skipping scheduled backup '{}' while on battery", j.name);
                continue;
            }
            if due {
                out.push(j.id.clone());
            }
        }
        out
    }

    /// Every job with its resolved settings (schedules may come from templates).
    async fn resolved_jobs(&self) -> Vec<(BackupJob, BackupJob)> {
        let jobs: Vec<BackupJob> = self.inner.jobs.read().await.values().cloned().collect();
        let globals = self.global_defaults().await.unwrap_or_default();
        let templates = self.list_templates().await.unwrap_or_default();
        jobs.into_iter()
            .map(|j| {
                let eff = templates::resolve(&j, &globals, &templates).map(|r| r.job).unwrap_or_else(|_| j.clone());
                (j, eff)
            })
            .collect()
    }

    /// Maintenance tasks whose own schedule slot has passed: (job id, task).
    /// Missed slots run once, as for backups; read-only jobs never run them.
    pub async fn due_maintenance(&self, now: DateTime<Utc>) -> Vec<(String, &'static str)> {
        if self.is_paused() {
            return Vec::new();
        }
        let on_battery = power::on_battery() == Some(true);
        let mut out = Vec::new();
        // Templates can set the maintenance schedule: use the resolved one.
        for (j, eff) in self.resolved_jobs().await.into_iter().filter(|(j, _)| !j.read_only) {
            let m = eff.maintenance;
            for task in schedule::MAINTENANCE_TASKS {
                let Some(s) = m.schedule_of(task).filter(|s| s.enabled) else { continue };
                let last = j.stats.last_maintenance.get(task).map(String::as_str).or(Some(j.created_at.as_str())).and_then(parse_ts);
                if last.is_some_and(|l| s.is_due(l, now)) && !(on_battery && s.skip_on_battery) {
                    out.push((j.id.clone(), task));
                }
            }
        }
        out
    }

    /// The operation a maintenance task runs.
    fn maintenance_operation(m: &schedule::MaintenanceSchedule, task: &str) -> BackupOperation {
        match task {
            "compact" => BackupOperation::Compact,
            "verify" => BackupOperation::Verify { full: false, options: Some(m.verify.clone().unwrap_or_default().options) },
            _ => BackupOperation::ApplyRetention { policy: None, source: None },
        }
    }

    /// Start due maintenance tasks (one per job per pass; a busy job waits).
    async fn run_due_maintenance(&self, now: DateTime<Utc>) -> usize {
        let mut started = 0;
        let mut seen = std::collections::HashSet::new();
        for (id, task) in self.due_maintenance(now).await {
            if self.is_job_busy(&id) || !seen.insert(id.clone()) {
                continue;
            }
            let snapshot = {
                let mut jobs = self.inner.jobs.write().await;
                jobs.get_mut(&id).map(|j| {
                    j.stats.last_maintenance.insert(task.to_string(), now.to_rfc3339());
                    j.clone()
                })
            };
            let Some(j) = snapshot else { continue };
            let _ = self.persist(&j).await;
            let m = self.resolve_job(&j).await.map(|r| r.job.maintenance).unwrap_or_else(|_| j.maintenance.clone());
            if let Some(c) = m.schedule_of(task).and_then(|s| s.day_off(now)) {
                tracing::info!("Skipping scheduled {task} of '{}': today is a day off in '{c}'", j.name);
                continue;
            }
            match self.start_operation(&id, Self::maintenance_operation(&m, task)).await {
                Ok(_) => started += 1,
                Err(e) => tracing::warn!("Scheduled {task} of {id} could not start: {e}"),
            }
        }
        started
    }

    /// One scheduler pass: start every due job.
    pub async fn tick(&self) -> usize {
        if self.is_shutting_down() {
            return 0;
        }
        self.close_idle_drives().await;
        let now = Utc::now();
        let due = self.due_jobs(now).await;
        for id in &due {
            let snapshot = {
                let mut jobs = self.inner.jobs.write().await;
                jobs.get_mut(id).map(|j| {
                    j.stats.last_scheduled_at = Some(now.to_rfc3339());
                    j.clone()
                })
            };
            if let Some(j) = &snapshot {
                let _ = self.persist(j).await;
                // A day off in one of the job's holiday calendars uses up the slot.
                let schedule = self.resolve_job(j).await.map(|r| r.job.schedule).unwrap_or_else(|_| j.schedule.clone());
                if let Some(cal) = schedule.day_off(now) {
                    tracing::info!("Skipping scheduled backup '{}': today is a day off in '{cal}'", j.name);
                    continue;
                }
            }
            if let Err(e) = self.start_backup(id, false).await {
                tracing::warn!("Scheduled backup {id} could not start: {e}");
            }
        }
        let reps = self.due_replicas(now).await;
        for (job_id, rid) in &reps {
            if self.is_job_busy(job_id) {
                continue;
            }
            if let Err(e) = self.start_operation(job_id, BackupOperation::Replicate { replica_id: Some(rid.clone()) }).await {
                tracing::warn!("Scheduled replication of {job_id} could not start: {e}");
            }
        }
        let maint = self.run_due_maintenance(now).await;
        due.len() + reps.len() + maint
    }

    /// Start the background scheduler loop (idempotent).
    pub fn start_scheduler(&self) {
        if self.inner.scheduler_running.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        let fut = async move {
            // Small startup delay so app boot is not slowed by catch-up runs.
            tokio::time::sleep(std::time::Duration::from_secs(20)).await;
            loop {
                me.tick().await;
                tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            }
        };
        // Inside a Tokio runtime (server, tests) use it; otherwise (Tauri
        // `setup` on the main thread) use Tauri's long-lived runtime.
        match tokio::runtime::Handle::try_current() {
            Ok(h) => {
                h.spawn(fut);
            }
            Err(_) => {
                tauri::async_runtime::spawn(fut);
            }
        }
    }
}

/// The tracing span of one operation (exported as an OTLP span by ufop-server).
fn op_span(job: &BackupJob, op: &str, task_id: &str) -> tracing::Span {
    tracing::info_span!(
        "backup.operation",
        otel.kind = "internal",
        job.id = %job.id,
        job.name = %job.name,
        backup.operation = op,
        task.id = task_id,
        backup.result = tracing::field::Empty,
        backup.error = tracing::field::Empty,
    )
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, AppError> + Send + 'static) -> Result<T, AppError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::internal(format!("backup worker panicked: {e}")))?
}

fn summarize(op: &str, v: &serde_json::Value) -> String {
    let n = |k: &str| v.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
    match op {
        "backup" if v.get("skipped_unchanged").and_then(|b| b.as_bool()) == Some(true) => {
            format!("No changes in {} files; no new version", n("files_examined"))
        }
        "backup" => format!(
            "{} files examined, {} added, {} changed, {} deleted; {} uploaded{}",
            n("files_examined"),
            n("files_added"),
            n("files_changed"),
            n("files_deleted"),
            human_bytes(n("bytes_uploaded")),
            if v.get("dry_run").and_then(|b| b.as_bool()) == Some(true) { " (dry run)" } else { "" }
        ),
        "restore" => format!("{} files restored ({}), {} unchanged skipped", n("files_restored"), human_bytes(n("bytes_restored")), n("files_skipped_matching")),
        "verify" if n("files_checked") > 0 => format!("{} volume(s) and {} file(s) verified, {} problem(s)", n("checked"), n("files_checked"), v.get("problems").and_then(|p| p.as_array()).map(|a| a.len()).unwrap_or(0)),
        "verify" => format!("{} volume(s) verified", n("checked")),
        "compact" => format!("{} removed, {} rewritten, {} reclaimed", n("volumes_removed"), n("volumes_rewritten"), human_bytes(n("bytes_reclaimed"))),
        "rebuild_missing" => format!("{} volume(s) rebuilt, {} block(s) recovered, {} unrecoverable", n("volumes_rebuilt"), n("blocks_recovered"), n("blocks_unrecoverable")),
        "extract_control_files" => format!("{} control file(s) extracted", v.get("files").and_then(|f| f.as_array()).map(|a| a.len()).unwrap_or(0)),
        "apply_locks" => format!("{} volume(s) locked until {}", n("locked"), v.get("until").and_then(|u| u.as_str()).unwrap_or("")),
        "list_locks" => format!("{} locked volume(s)", v.get("locks").and_then(|l| l.as_array()).map(|a| a.len()).unwrap_or(0)),
        "replicate" => format!("{} replica(s) updated", v.get("replicas").and_then(|r| r.as_array()).map(|a| a.len()).unwrap_or(0)),
        "snapshot_test" if v.get("ok").and_then(|b| b.as_bool()) == Some(true) => format!("{} snapshot created and removed", v.get("method").and_then(|m| m.as_str()).unwrap_or("")),
        "snapshot_test" => "Snapshot test failed".to_string(),
        "autotune" => format!("Best restore download concurrency: {} thread(s)", n("download_threads")),
        "recreate_database" => format!("{} version(s), {} block volume(s) indexed", n("versions"), n("block_volumes")),
        "upgrade_format" | "rollback_format" => v.get("message").and_then(|m| m.as_str()).unwrap_or("Format updated").to_string(),
        "reshard" => format!(
            "{} volume(s) {}moved ({}), {} already in place",
            n("moved"),
            if v.get("dry_run").and_then(|b| b.as_bool()) == Some(true) { "would be " } else { "" },
            human_bytes(n("bytes_moved")),
            n("already_in_place")
        ),
        "format_info" => format!("Repository format {}", n("format_version")),
        "inspect_header" => format!("Repository header (format {})", v.pointer("/format/format_version").and_then(|x| x.as_u64()).unwrap_or(0)),
        "inspect_stats" => format!("{} block(s) in {} version(s), {} stored", n("blocks"), v.get("versions").and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0), human_bytes(n("total_bytes"))),
        "delete_versions" | "apply_retention" => format!("{} version(s) deleted", n("versions_deleted")),
        "set_version_metadata" => format!("Version {} updated", v.get("timestamp").and_then(|t| t.as_str()).unwrap_or("")),
        "copy_versions" => format!(
            "{} version(s) {}copied ({} new block(s), {}), {} already there",
            n("versions_copied"),
            if v.get("dry_run").and_then(|b| b.as_bool()) == Some(true) { "would be " } else { "" },
            n("blocks_copied"),
            human_bytes(n("bytes_uploaded")),
            n("versions_skipped")
        ),
        "enable_sharing" if v.get("already_shared").and_then(|b| b.as_bool()) == Some(true) => "The repository is already shared".to_string(),
        "enable_sharing" => format!("Repository shared; {} existing version(s) assigned to this job", n("versions_assigned")),
        "sync_sources" => format!("{} source(s)", v.get("sources").and_then(|r| r.as_array()).map(|a| a.len()).unwrap_or(0)),
        _ => format!("{op} completed"),
    }
}

pub fn human_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{b} B") } else { format!("{v:.1} {}", U[i]) }
}

/// Execute one operation against the core engine (blocking thread), under
/// the repository lock it needs, with the job's local database (opened,
/// migrated or rebuilt as the job's database options say).
#[allow(clippy::too_many_arguments)]
fn run_core(
    job: &BackupJob,
    op: BackupOperation,
    backend: &dyn Backend,
    db_path: &Path,
    dbo: &maintenance::JobDbOptions,
    pass: Option<&str>,
    kdf: KdfParams,
    cache: Option<Arc<ufop_core::backup::cache::VolumeCache>>,
    ctx: &OpContext,
) -> Result<serde_json::Value, AppError> {
    legal_hold::guard(job, &op)?;
    let job = &versions::with_identity(job);
    let prefix = job.options.prefix.as_str();
    let kind = op.lock_kind();
    let name = op.name();
    let work = || -> Result<serde_json::Value, AppError> {
        let (db, notices) = maintenance::open_job_db(db_path, dbo, Some(backend), pass, prefix, ctx)?;
        let mut v = dispatch(job, op, backend, &db, pass, kdf, cache.clone(), ctx)?;
        add_warnings(&mut v, notices);
        Ok(v)
    };
    match kind {
        None => work(),
        Some(kind) => {
            let (mut v, w) = repo_lock::with_lock(backend, prefix, kind, name, &lock_options(&job.options), ctx, work)?;
            add_warnings(&mut v, w);
            Ok(v)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn dispatch(
    job: &BackupJob,
    op: BackupOperation,
    backend: &dyn Backend,
    db: &BackupDb,
    pass: Option<&str>,
    kdf: KdfParams,
    cache: Option<Arc<ufop_core::backup::cache::VolumeCache>>,
    ctx: &OpContext,
) -> Result<serde_json::Value, AppError> {
    let open = |create: bool| Repo::open(backend, db, pass, &job.options, create, kdf).map(|r| r.with_cache(cache.clone()));
    let v = match op {
        BackupOperation::Backup { dry_run } => {
            let filter = job.filter.clone().compile()?;
            let mut opts = job.options.clone();
            opts.dry_run = dry_run;
            opts.task_config = match (job.store_config_remotely, job.remote_config_include_secrets && job.encrypted) {
                (false, _) => None,
                (true, false) => Some(portability::stripped_config(job)?),
                (true, true) => Some(portability::full_config(job)?),
            };
            let sources: Vec<PathBuf> = job.sources.iter().filter(|s| !s.trim().is_empty()).map(PathBuf::from).collect();
            let streams = match job.stdin_command.as_ref().filter(|c| c.is_set()) {
                Some(c) => vec![hooks::command_stream(c, &BackupManager::script_env(job, "backup", "STDIN"))?],
                None => Vec::new(),
            };
            // Maintenance on its own schedule does not also run after the backup.
            if job.maintenance.scheduled("compact") {
                opts.auto_compact = false;
            }
            let retention = if job.maintenance.scheduled("apply_retention") { RetentionPolicy::KeepAll } else { legal_hold::effective_retention(job) };
            serde_json::to_value(ufop_core::backup::run_backup_with_streams(backend, db, &sources, &filter, &opts, &retention, pass, None, kdf, ctx, streams)?)?
        }
        BackupOperation::Restore { options, .. } => serde_json::to_value(ufop_core::backup::run_restore(&open(false)?, &options, ctx)?)?,
        BackupOperation::Verify { full, options } => {
            let samples = if full { None } else { Some(job.options.verify_samples.max(1)) };
            let o = options.unwrap_or_else(|| ufop_core::backup::verify::VerifyOptions::legacy(samples, full));
            serde_json::to_value(ufop_core::backup::verify::verify(&open(false)?, &o, ctx)?)?
        }
        BackupOperation::Compact => serde_json::to_value(maintenance::compact(&open(false)?, &job.options, ctx)?)?,
        BackupOperation::Repair => serde_json::to_value(maintenance::repair(&open(false)?)?)?,
        BackupOperation::RecreateDatabase => serde_json::to_value(maintenance::recreate_database(backend, db, pass, &job.options.prefix, ctx)?)?,
        BackupOperation::ListBroken => serde_json::json!({ "files": maintenance::list_broken_files(&open(false)?)? }),
        BackupOperation::PurgeBroken { dry_run } => serde_json::to_value(maintenance::purge_broken_files(&open(false)?, dry_run)?)?,
        BackupOperation::Purge { paths, versions, dry_run } => {
            serde_json::to_value(maintenance::purge(&open(false)?, &paths, versions.as_deref(), dry_run)?)?
        }
        BackupOperation::DeleteVersions { versions, allow_full_removal, force } => {
            let n = maintenance::delete_versions_ex(&open(false)?, &versions, allow_full_removal || job.options.allow_full_removal, force)?;
            serde_json::json!({ "versions_deleted": n })
        }
        BackupOperation::SetVersionMetadata { version, source, patch } => {
            let repo = open(false)?;
            let _scope = ufop_core::backup::shared::selector_scope(source.as_deref()).map(|s| db.scoped(s));
            serde_json::to_value(ufop_core::backup::versions::set_version_metadata(&repo, version, &patch)?)?
        }
        BackupOperation::ApplyRetention { policy, source } => {
            let repo = open(false)?;
            let _scope = ufop_core::backup::shared::selector_scope(source.as_deref()).map(|s| db.scoped(s));
            let policy = policy.unwrap_or_else(|| job.retention.clone());
            let n = maintenance::apply_retention(&repo, &policy, job.options.allow_full_removal)?;
            serde_json::json!({ "versions_deleted": n })
        }
        BackupOperation::EnableSharing { claim_all } => {
            serde_json::to_value(ufop_core::backup::shared::enable_sharing(&open(false)?, &job.options.identity, claim_all)?)?
        }
        BackupOperation::SyncSources => {
            let repo = open(false)?;
            let own = job.options.identity.resolve().source_id();
            serde_json::json!({
                "shared": ufop_core::backup::shared::is_shared(&repo.header),
                "sources": ufop_core::backup::shared::list_sources(db, Some(&own))?,
            })
        }
        BackupOperation::CopyVersions { .. } => return Err(AppError::internal("copying runs in the manager")),
        BackupOperation::Consistency => serde_json::to_value(open(false)?.consistency_check()?)?,
        BackupOperation::RebuildMissing => serde_json::to_value(maintenance::rebuild_missing_block_volumes(&open(false)?, &job.options, ctx)?)?,
        BackupOperation::ExtractControlFiles { version, target } => {
            let files = maintenance::extract_control_files(&open(false)?, version, Path::new(&target))?;
            serde_json::json!({ "files": files.iter().map(|p| p.to_string_lossy()).collect::<Vec<_>>() })
        }
        BackupOperation::ApplyLocks { days, mode, all } => serde_json::to_value(maintenance::apply_locks(&open(false)?, days, mode, all)?)?,
        BackupOperation::ListLocks => {
            let l = maintenance::list_locks(&open(false)?)?;
            serde_json::json!({ "locks": l.into_iter().map(|(n, u)| serde_json::json!({ "name": n, "until": u })).collect::<Vec<_>>() })
        }
        BackupOperation::Replicate { .. } => return Err(AppError::internal("replication runs in the manager")),
        BackupOperation::SnapshotTest => {
            let srcs: Vec<PathBuf> = job.sources.iter().map(PathBuf::from).collect();
            let d = ufop_core::backup::snapshot::diagnose(&srcs, &job.options.snapshot);
            let mut v = serde_json::to_value(&d)?;
            if let (Some(e), Some(o)) = (&d.error, v.as_object_mut()) {
                o.insert("errors".into(), serde_json::json!([e]));
            }
            v
        }
        BackupOperation::AutoTune { sample_volumes } => serde_json::to_value(ufop_core::backup::restore::autotune(&open(false)?, sample_volumes.clamp(1, 50), ctx)?)?,
        BackupOperation::UpgradeFormat { rollback: false } => {
            serde_json::to_value(ufop_core::backup::format::upgrade_repository(backend, &job.options.prefix, pass, ufop_core::backup::REPO_FORMAT_VERSION)?)?
        }
        BackupOperation::UpgradeFormat { rollback: true } => serde_json::to_value(ufop_core::backup::format::rollback_upgrade(backend, &job.options.prefix)?)?,
        BackupOperation::Reshard { layout, dry_run } => serde_json::to_value(ufop_core::backup::layout::reshard(backend, &job.options.prefix, layout, dry_run, ctx)?)?,
        BackupOperation::FormatInfo => serde_json::to_value(ufop_core::backup::format::format_info(backend, &job.options.prefix)?)?,
        BackupOperation::InspectHeader => serde_json::to_value(ufop_core::backup::inspect::header(backend, &job.options.prefix)?)?,
        BackupOperation::InspectStats => serde_json::to_value(ufop_core::backup::inspect::stats(&open(false)?)?)?,
    };
    Ok(v)
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_policy;
