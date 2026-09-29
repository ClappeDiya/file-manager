//! Sync engine data types: pairs, filters, copy options, previews, reports.
//!
//! Shared by the desktop app and the `ufop` CLI.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ──────────────────────────────────────────────
// Sync Types
// ──────────────────────────────────────────────

/// Sync mode for a SyncPair.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    OneWay,
    TwoWay,
    Mirror,
    VersionedBackup,
}

impl SyncMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OneWay => "one_way",
            Self::TwoWay => "two_way",
            Self::Mirror => "mirror",
            Self::VersionedBackup => "versioned_backup",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "one_way" => Self::OneWay,
            "two_way" => Self::TwoWay,
            "mirror" => Self::Mirror,
            "versioned_backup" => Self::VersionedBackup,
            _ => Self::OneWay,
        }
    }
}

/// Trigger mode for sync execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncTrigger {
    /// Manual execution only
    Manual,
    /// Cron-like scheduled execution (e.g., "0 */6 * * *")
    Scheduled { cron_expr: String },
    /// Filesystem watcher (real-time, within 5s)
    Watch,
    /// API/CLI triggered
    Api,
}

impl SyncTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Scheduled { .. } => "scheduled",
            Self::Watch => "watch",
            Self::Api => "api",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "manual" => Self::Manual,
            "watch" => Self::Watch,
            "api" => Self::Api,
            _ if s.starts_with("scheduled:") => Self::Scheduled {
                cron_expr: s.trim_start_matches("scheduled:").to_string(),
            },
            "scheduled" => Self::Scheduled {
                cron_expr: "0 * * * *".to_string(),
            },
            _ => Self::Manual,
        }
    }
}

/// Selective sync filter configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SyncFilter {
    /// Glob patterns to include (empty = include all)
    pub include_patterns: Vec<String>,
    /// Glob patterns to exclude
    pub exclude_patterns: Vec<String>,
    /// Regex patterns to include
    pub include_regex: Vec<String>,
    /// Regex patterns to exclude
    pub exclude_regex: Vec<String>,
    /// Minimum file size in bytes (0 = no minimum)
    pub min_size: u64,
    /// Maximum file size in bytes (0 = no maximum)
    pub max_size: u64,
    /// File type filters (extensions: "jpg", "pdf", etc.)
    pub file_types: Vec<String>,
    /// Exclude file type filters
    pub exclude_file_types: Vec<String>,
}

/// How symbolic links / junctions are treated during a sync (Robocopy /XJ, /SL).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum SymlinkMode {
    /// Follow the link and copy what it points to (Robocopy default).
    #[default]
    Follow,
    /// Skip links entirely (/XJ, /XJD, /XJF).
    Skip,
    /// Recreate the link itself at the destination (/SL). Unix only; falls
    /// back to `Follow` elsewhere.
    CopyLink,
}

/// Robocopy-equivalent copy options for a sync pair.
///
/// Every field defaults to "off", so a pair created before these existed
/// (or with `{}` stored) behaves exactly as it always has.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CopyOptions {
    // ── Selection ──
    /// Recurse into subdirectories (/S). `false` = top-level files only.
    pub copy_subdirs: bool,
    /// Also create empty directories at the destination (/E).
    pub include_empty_dirs: bool,
    /// Only descend this many levels (/LEV:n). 0 = unlimited.
    pub max_depth: u32,
    /// Directory names or relative-path globs to skip entirely (/XD).
    pub exclude_dirs: Vec<String>,
    /// Skip files last modified more than N days ago (/MAXAGE:n). 0 = off.
    pub max_age_days: u32,
    /// Skip files modified within the last N days (/MINAGE:n). 0 = off.
    pub min_age_days: u32,
    /// Skip hidden files — dot-files, or the Windows hidden attribute (/XA:H).
    pub exclude_hidden: bool,
    /// Skip read-only files (/XA:R).
    pub exclude_readonly: bool,
    /// Symbolic link handling (/XJ, /SL).
    pub symlinks: SymlinkMode,

    // ── Comparison ──
    /// Don't overwrite a destination file that is newer than the source (/XO).
    pub exclude_older: bool,
    /// Don't overwrite a destination file that is older than the source (/XN).
    pub exclude_newer: bool,
    /// Skip files whose timestamp matches but size differs (/XC).
    pub exclude_changed: bool,
    /// Only update files that already exist at the destination (/XL).
    pub exclude_lonely: bool,
    /// Re-copy files even when they look identical (/IS).
    pub include_same: bool,
    /// Treat timestamps within 2 seconds as equal — FAT/network shares (/FFT).
    pub fat_time_tolerance: bool,
    /// Treat an exact 1-hour timestamp difference as equal (/DST).
    pub dst_tolerance: bool,

    // ── Destination cleanup ──
    /// Delete destination files that no longer exist in the source (/PURGE).
    /// Mirror mode always does this.
    pub purge: bool,
    /// Never delete extra destination files, even in mirror mode (/XX).
    pub exclude_extra: bool,

    // ── Copy behaviour ──
    /// Preserve file modification times (/COPY:T). On by default.
    pub copy_timestamps: bool,
    /// Preserve directory modification times (/DCOPY:T).
    pub copy_dir_timestamps: bool,
    /// Delete each source file after it is copied (/MOV).
    pub move_files: bool,
    /// Like `move_files`, and also remove emptied source directories (/MOVE).
    pub move_dirs: bool,
    /// Create the directory tree and zero-length files only (/CREATE).
    pub create_only: bool,
    /// Copy through a `.ufop-partial` file that resumes after interruption (/Z).
    pub restartable: bool,

    // ── Reliability & performance ──
    /// Retries per failed file (/R:n).
    pub retries: u32,
    /// Seconds to wait between retries (/W:n).
    pub retry_wait_secs: u32,
    /// Parallel copy threads (/MT:n). 1 = sequential.
    pub threads: u32,
    /// Milliseconds to pause after every 64 KiB block to free bandwidth (/IPG:n).
    pub inter_packet_gap_ms: u32,
    /// Only copy between these local times, "HHMM-HHMM" (/RH). Empty = any time.
    pub run_hours: String,

    // ── Windows-only (ignored on other platforms) ──
    /// Copy only files with the archive attribute set (/A).
    pub archive_only: bool,
    /// Like /A, then clear the source's archive attribute (/M).
    pub archive_reset: bool,
    /// Only copy files with any of these attributes, RASHCNETO letters (/IA).
    pub include_attributes: String,
    /// Skip files with any of these attributes, RASHCNETO letters (/XA).
    /// Hidden and read-only also have their own portable switches.
    pub exclude_attributes: String,
    /// Attributes to set on copied files (/A+:), RASHCNET letters.
    pub add_attributes: String,
    /// Attributes to clear on copied files (/A-:), RASHCNET letters.
    pub remove_attributes: String,
    /// Copy NTFS access control lists (/COPY:S, /SEC).
    pub copy_security: bool,
    /// Copy file ownership (/COPY:O). Needs administrator rights.
    pub copy_owner: bool,
    /// Copy auditing information (/COPY:U). Needs administrator rights.
    pub copy_auditing: bool,
    /// Re-apply timestamps to files that are otherwise unchanged (/TIMFIX).
    pub fix_timestamps: bool,
    /// Re-apply security to files that are otherwise unchanged (/SECFIX).
    pub fix_security: bool,

    // ── Logging ──
    /// Write a Robocopy-style log to this file (/LOG). Empty = no log.
    pub log_file: String,
    /// Append to the log instead of overwriting it (/LOG+).
    pub log_append: bool,
}

impl Default for CopyOptions {
    fn default() -> Self {
        Self {
            copy_subdirs: true,
            include_empty_dirs: false,
            max_depth: 0,
            exclude_dirs: Vec::new(),
            max_age_days: 0,
            min_age_days: 0,
            exclude_hidden: false,
            exclude_readonly: false,
            symlinks: SymlinkMode::Follow,
            exclude_older: false,
            exclude_newer: false,
            exclude_changed: false,
            exclude_lonely: false,
            include_same: false,
            fat_time_tolerance: false,
            dst_tolerance: false,
            purge: false,
            exclude_extra: false,
            copy_timestamps: true,
            copy_dir_timestamps: false,
            move_files: false,
            move_dirs: false,
            create_only: false,
            restartable: false,
            retries: 0,
            retry_wait_secs: 30,
            threads: 1,
            inter_packet_gap_ms: 0,
            run_hours: String::new(),
            archive_only: false,
            archive_reset: false,
            include_attributes: String::new(),
            exclude_attributes: String::new(),
            add_attributes: String::new(),
            remove_attributes: String::new(),
            copy_security: false,
            copy_owner: false,
            copy_auditing: false,
            fix_timestamps: false,
            fix_security: false,
            log_file: String::new(),
            log_append: false,
        }
    }
}

/// Sync conflict resolution policy (7 policies per T-041).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SyncConflictPolicy {
    /// Prompt user for each conflict
    Ask,
    /// Source file always wins
    SourceWins,
    /// Destination file always wins
    DestWins,
    /// Newest file by modification time wins
    NewestWins,
    /// Create a conflict copy with (conflict YYYY-MM-DD) suffix
    #[default]
    CreateConflictCopy,
    /// Skip conflicted files
    Skip,
    /// Move conflicted files to quarantine review queue
    Quarantine,
}

impl SyncConflictPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::SourceWins => "source_wins",
            Self::DestWins => "dest_wins",
            Self::NewestWins => "newest_wins",
            Self::CreateConflictCopy => "create_conflict_copy",
            Self::Skip => "skip",
            Self::Quarantine => "quarantine",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "ask" => Self::Ask,
            "source_wins" => Self::SourceWins,
            "dest_wins" => Self::DestWins,
            "newest_wins" => Self::NewestWins,
            "create_conflict_copy" => Self::CreateConflictCopy,
            "skip" => Self::Skip,
            "quarantine" => Self::Quarantine,
            _ => Self::CreateConflictCopy,
        }
    }
}

/// Sync verification mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SyncVerifyMode {
    /// No verification
    None,
    /// Fast: compare size + modification time
    #[default]
    Fast,
    /// Full: byte-level or checksum comparison
    Full,
}

impl SyncVerifyMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Fast => "fast",
            Self::Full => "full",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "none" => Self::None,
            "fast" => Self::Fast,
            "full" => Self::Full,
            _ => Self::Fast,
        }
    }
}

/// Sync pair configuration (enhanced for T-039..T-042).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPair {
    pub id: Uuid,
    pub name: String,
    pub source_path: String,
    pub dest_path: String,
    pub mode: SyncMode,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
    /// Trigger mode: manual, scheduled, watch, api
    pub trigger: SyncTrigger,
    /// Selective sync filters
    pub filter: SyncFilter,
    /// Conflict resolution policy
    pub conflict_policy: SyncConflictPolicy,
    /// Verification mode (none/fast/full)
    pub verify_mode: SyncVerifyMode,
    /// Enable checksum verification toggle
    pub checksum_enabled: bool,
    /// Created timestamp
    pub created_at: DateTime<Utc>,
    /// Server time offset in seconds (for clock skew compensation).
    /// When set, remote mtime is adjusted by this value before comparison.
    /// Positive = server is ahead, negative = server is behind.
    #[serde(default)]
    pub time_offset_secs: Option<i64>,
    /// Robocopy-equivalent copy options.
    #[serde(default)]
    pub copy_options: CopyOptions,
}

impl Default for SyncPair {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            source_path: String::new(),
            dest_path: String::new(),
            mode: SyncMode::OneWay,
            enabled: true,
            last_run: None,
            trigger: SyncTrigger::Manual,
            filter: SyncFilter::default(),
            conflict_policy: SyncConflictPolicy::CreateConflictCopy,
            verify_mode: SyncVerifyMode::Fast,
            checksum_enabled: false,
            created_at: Utc::now(),
            time_offset_secs: None,
            copy_options: CopyOptions::default(),
        }
    }
}

/// Status of the last sync run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncState {
    pub pair_id: Uuid,
    pub last_sync_at: DateTime<Utc>,
    pub files_synced: u64,
    pub bytes_synced: u64,
    pub errors: u32,
    pub status: SyncRunStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncRunStatus {
    Success,
    PartialSuccess,
    Failed,
    Running,
    Cancelled,
}

/// Health indicator for a sync pair (T-042).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncHealth {
    /// All good - last sync successful
    Green,
    /// Warning - partial failures or stale
    Yellow,
    /// Error - last sync failed
    Red,
    /// Inactive - never run or disabled
    Gray,
}

/// A single file action in a sync plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncAction {
    Add,
    Modify,
    Delete,
    Skip,
    Conflict,
}

/// Represents one file diff entry in a dry-run preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncDiffEntry {
    pub relative_path: String,
    pub action: SyncAction,
    pub source_size: Option<u64>,
    pub dest_size: Option<u64>,
    pub source_modified: Option<DateTime<Utc>>,
    pub dest_modified: Option<DateTime<Utc>>,
    /// Reason for the action
    pub reason: String,
    /// Path length warning if applicable
    pub path_length_warning: Option<String>,
}

/// Dry-run preview result (T-040).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPreview {
    pub pair_id: Uuid,
    pub pair_name: String,
    pub computed_at: DateTime<Utc>,
    pub duration_ms: u64,
    /// Files to add (new at destination)
    pub additions: Vec<SyncDiffEntry>,
    /// Files to modify (changed)
    pub modifications: Vec<SyncDiffEntry>,
    /// Files to delete (extra at destination)
    pub deletions: Vec<SyncDiffEntry>,
    /// Skipped files (filtered out)
    pub skipped: Vec<SyncDiffEntry>,
    /// Conflict items needing resolution
    pub conflicts: Vec<SyncDiffEntry>,
    /// Path length warnings
    pub path_warnings: Vec<SyncDiffEntry>,
    /// Summary counts
    pub total_additions: u64,
    pub total_modifications: u64,
    pub total_deletions: u64,
    pub total_skipped: u64,
    pub total_conflicts: u64,
    /// Total bytes to transfer
    pub total_bytes: u64,
    /// Directories to create at the destination (/E empty dirs, /CREATE).
    #[serde(default)]
    pub dirs_to_create: Vec<String>,
    /// Extra destination directories to remove when purging (/PURGE, /MIR).
    #[serde(default)]
    pub dirs_to_remove: Vec<String>,
    /// Unchanged files whose timestamps or security get re-applied
    /// (/TIMFIX, /SECFIX).
    #[serde(default)]
    pub fixups: Vec<String>,
}

/// A conflict item for the ask-mode resolution UI (T-041).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncConflictItem {
    pub id: Uuid,
    pub pair_id: Uuid,
    pub relative_path: String,
    pub source_size: u64,
    pub dest_size: u64,
    pub source_modified: Option<DateTime<Utc>>,
    pub dest_modified: Option<DateTime<Utc>>,
    pub resolution: Option<SyncConflictPolicy>,
    pub created_at: DateTime<Utc>,
}

/// Quarantine queue entry (T-041).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarantineEntry {
    pub id: Uuid,
    pub pair_id: Uuid,
    pub original_path: String,
    pub quarantine_path: String,
    pub source_size: u64,
    pub dest_size: u64,
    pub source_modified: Option<DateTime<Utc>>,
    pub dest_modified: Option<DateTime<Utc>>,
    pub quarantined_at: DateTime<Utc>,
    pub resolved: bool,
}

/// Pre-destructive snapshot for rollback (T-042).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncSnapshot {
    pub id: Uuid,
    pub run_id: Uuid,
    pub pair_id: Uuid,
    pub relative_path: String,
    /// "delete" or "overwrite"
    pub action: String,
    /// Path to backed-up file
    pub backup_path: String,
    pub original_size: u64,
    pub original_modified: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

/// Per-run sync report (T-042).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReport {
    pub id: Uuid,
    pub pair_id: Uuid,
    pub pair_name: String,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    pub duration_ms: u64,
    pub files_added: u64,
    pub files_modified: u64,
    pub files_deleted: u64,
    pub files_skipped: u64,
    pub conflicts_resolved: u64,
    pub errors: u32,
    pub bytes_transferred: u64,
    pub status: SyncRunStatus,
    pub health: SyncHealth,
    /// Error details if any
    pub error_messages: Vec<String>,
    /// Whether the sync was resumed from interruption
    pub resumed: bool,
    /// Robocopy-compatible exit code bitmask: 1 = files copied,
    /// 2 = extra destination files, 4 = mismatches, 8 = failures, 16 = fatal.
    #[serde(default)]
    pub exit_code: u8,
}

/// Resumable sync state for crash recovery (T-042).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncResumeState {
    pub run_id: Uuid,
    pub pair_id: Uuid,
    /// Index of last successfully synced file
    pub last_completed_index: u64,
    /// Total files in plan
    pub total_files: u64,
    /// Serialized plan state
    pub plan_json: String,
    pub started_at: DateTime<Utc>,
    pub interrupted_at: DateTime<Utc>,
}
