use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ──────────────────────────────────────────────
// File System Types
// ──────────────────────────────────────────────

/// Represents a file or directory entry returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub is_dir: bool,
    pub is_symlink: bool,
    pub size: u64,
    pub modified: Option<DateTime<Utc>>,
    pub created: Option<DateTime<Utc>>,
    pub is_hidden: bool,
    pub extension: Option<String>,
    pub permissions: Option<String>,
}

/// Requested file operation kind.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileOpKind {
    Copy,
    Move,
    Rename,
    Delete,
    CreateDir,
    CreateFile,
}

// ──────────────────────────────────────────────
// Transfer Types
// ──────────────────────────────────────────────

/// Transfer job status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Queued,
    Active,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

impl TransferStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Queued => "queued",
            Self::Active => "active",
            Self::Paused => "paused",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "queued" => Self::Queued,
            "active" => Self::Active,
            "paused" => Self::Paused,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Queued,
        }
    }
}

/// Transfer priority level for queue ordering.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum TransferPriority {
    Low = 0,
    #[default]
    Normal = 1,
    High = 2,
}

impl TransferPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "low" => Self::Low,
            "normal" => Self::Normal,
            "high" => Self::High,
            _ => Self::Normal,
        }
    }
}


/// Checksum algorithm for post-transfer verification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChecksumAlgorithm {
    Md5,
    Sha256,
    XxHash3,
}

impl ChecksumAlgorithm {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Md5 => "md5",
            Self::Sha256 => "sha256",
            Self::XxHash3 => "xxhash3",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "md5" => Self::Md5,
            "sha256" => Self::Sha256,
            "xxhash3" => Self::XxHash3,
            _ => Self::Sha256,
        }
    }
}

// ──────────────────────────────────────────────
// Transfer Integrity Types (Three-Layer Architecture)
// ──────────────────────────────────────────────

/// Verification tier for transfer integrity.
/// Controls the trade-off between speed and verification thoroughness.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum VerifyTier {
    /// Tier 1: xxHash3 per chunk + full-file xxHash3. Minimal overhead.
    #[default]
    Fast = 1,
    /// Tier 2: + write-back verify + SHA-256 + fsync/8 chunks.
    Verified = 2,
    /// Tier 3: + Merkle tree sidecar + fsync/chunk + audit trail.
    MissionCritical = 3,
}

impl VerifyTier {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Verified => "verified",
            Self::MissionCritical => "mission_critical",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "fast" | "1" => Self::Fast,
            "verified" | "2" => Self::Verified,
            "mission_critical" | "3" => Self::MissionCritical,
            _ => Self::Fast,
        }
    }

    pub fn as_int(&self) -> i32 {
        *self as i32
    }

    pub fn from_int(v: i32) -> Self {
        match v {
            1 => Self::Fast,
            2 => Self::Verified,
            3 => Self::MissionCritical,
            _ => Self::Fast,
        }
    }
}


/// Status of a single chunk within a transfer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ChunkStatus {
    Pending,
    Reading,
    Writing,
    Verifying,
    Verified,
    Failed { reason: String, attempts: u8 },
    Skipped,
}

impl ChunkStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Reading => "reading",
            Self::Writing => "writing",
            Self::Verifying => "verifying",
            Self::Verified => "verified",
            Self::Failed { .. } => "failed",
            Self::Skipped => "skipped",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "pending" => Self::Pending,
            "reading" => Self::Reading,
            "writing" => Self::Writing,
            "verifying" => Self::Verifying,
            "verified" => Self::Verified,
            "failed" => Self::Failed {
                reason: "unknown".to_string(),
                attempts: 0,
            },
            "skipped" => Self::Skipped,
            _ => Self::Pending,
        }
    }
}

/// Specification for a single chunk of a file transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChunkSpec {
    pub chunk_index: u32,
    pub offset: u64,
    pub length: u64,
    /// xxHash3 of source data (8 bytes stored as u64).
    pub source_hash: Option<u64>,
    /// xxHash3 of destination data after write-back read.
    pub dest_hash: Option<u64>,
    /// SHA-256 for Tier 2+ verification.
    pub sha256: Option<[u8; 32]>,
    pub status: ChunkStatus,
    pub write_time_ms: u32,
    pub verify_time_ms: u32,
}

/// Detected media type for adaptive chunk sizing.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    NvmeSsd,
    SataSsd,
    Hdd,
    Usb3,
    Usb2,
    NetworkSftp,
    NetworkS3,
    NetworkHighLatency,
    LanPeer,
    Unknown,
}

impl MediaType {
    /// Default chunk size in bytes for this media type.
    pub fn default_chunk_size(&self) -> u64 {
        match self {
            Self::NvmeSsd => 64 * 1024 * 1024,        // 64 MB
            Self::SataSsd => 16 * 1024 * 1024,         // 16 MB
            Self::Hdd => 4 * 1024 * 1024,              // 4 MB
            Self::Usb3 => 8 * 1024 * 1024,             // 8 MB
            Self::Usb2 => 2 * 1024 * 1024,             // 2 MB
            Self::NetworkSftp => 1024 * 1024,      // 1 MB
            Self::NetworkS3 => 8 * 1024 * 1024,        // 8 MB
            Self::NetworkHighLatency => 512 * 1024,     // 512 KB
            Self::LanPeer => 4 * 1024 * 1024,          // 4 MB
            Self::Unknown => 16 * 1024 * 1024,         // 16 MB default
        }
    }

    /// Default worker count for this media type.
    pub fn default_workers(&self) -> u32 {
        match self {
            Self::NvmeSsd | Self::SataSsd | Self::Hdd => 4,
            Self::Usb3 | Self::Usb2 => 4,
            Self::NetworkSftp | Self::NetworkHighLatency => 4,
            Self::NetworkS3 => 20,
            Self::LanPeer => 8,
            Self::Unknown => 4,
        }
    }
}

/// Merkle tree structure for Tier 3 verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleTree {
    /// SHA-256 per chunk (leaf hashes).
    pub leaf_hashes: Vec<[u8; 32]>,
    /// Internal tree nodes.
    pub tree_nodes: Vec<[u8; 32]>,
    /// Root hash of the Merkle tree.
    pub root: [u8; 32],
    pub chunk_count: u32,
    pub chunk_size: u64,
    pub file_size: u64,
    pub algorithm: String,
}

/// Serializable Merkle sidecar format for .ufop-merkle.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MerkleSidecar {
    pub version: u32,
    pub algorithm: String,
    pub file_size: u64,
    pub chunk_size: u64,
    pub chunk_count: u32,
    pub root_hash: String,
    pub leaf_hashes: Vec<String>,
    pub created_at: String,
}

/// Status of a transfer job in the journal.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferJobStatus {
    Active,
    Completed,
    Failed,
    Cancelled,
}

impl TransferJobStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "active" => Self::Active,
            "completed" => Self::Completed,
            "failed" => Self::Failed,
            "cancelled" => Self::Cancelled,
            _ => Self::Active,
        }
    }
}

/// Multi-file transfer manifest for .ufop-transfer.json.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferManifest {
    pub version: u32,
    pub job_id: String,
    pub source_root: String,
    pub dest_root: String,
    pub verify_tier: VerifyTier,
    pub files: Vec<ManifestFileEntry>,
    pub started_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestFileEntry {
    pub relative_path: String,
    pub size: u64,
    pub chunk_count: u32,
    pub source_hash: Option<String>,
    pub status: String,
    pub chunks_verified: Option<u32>,
    pub chunks_total: Option<u32>,
}

/// Transfer configuration for the three-layer engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferConfig {
    /// Manual chunk size override (None = adaptive).
    pub chunk_size_override: Option<u64>,
    /// Max parallel workers per job.
    pub max_workers_per_job: u32,
    /// Max workers across all jobs.
    pub max_workers_global: u32,
    /// Default verification tier.
    pub default_verify_tier: VerifyTier,
    /// Chunks between fsync calls (Tier 2).
    pub fsync_interval: u32,
    /// Read-ahead pipeline depth (2/3/4).
    pub pipeline_depth: u32,
    /// Per-chunk retry count.
    pub retry_max_attempts: u8,
    /// Exponential backoff base in ms.
    pub retry_backoff_base_ms: u64,
    /// Use direct I/O for files > 100MB.
    pub use_direct_io: bool,
    /// Pre-allocate destination file.
    pub preallocate: bool,
    /// Write Merkle tree sidecar (Tier 3).
    pub merkle_sidecar: bool,
    /// Days to keep completed journal entries.
    pub journal_retention_days: u32,
    /// Minimum file size for chunking (bytes).
    pub chunk_threshold: u64,
}

impl Default for TransferConfig {
    fn default() -> Self {
        Self {
            chunk_size_override: None,
            max_workers_per_job: 8,
            max_workers_global: 100,
            default_verify_tier: VerifyTier::Fast,
            fsync_interval: 8,
            pipeline_depth: 3,
            retry_max_attempts: 3,
            retry_backoff_base_ms: 1000,
            use_direct_io: true,
            preallocate: true,
            merkle_sidecar: true,
            journal_retention_days: 30,
            chunk_threshold: 10 * 1024 * 1024, // 10 MB
        }
    }
}

/// Conflict resolution policy.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum ConflictPolicy {
    OverwriteAlways,
    SkipAlways,
    OverwriteIfNewer,
    OverwriteIfLarger,
    Rename,
    #[default]
    AskEachTime,
}

impl ConflictPolicy {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::OverwriteAlways => "overwrite_always",
            Self::SkipAlways => "skip_always",
            Self::OverwriteIfNewer => "overwrite_if_newer",
            Self::OverwriteIfLarger => "overwrite_if_larger",
            Self::Rename => "rename",
            Self::AskEachTime => "ask_each_time",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "overwrite_always" => Self::OverwriteAlways,
            "skip_always" => Self::SkipAlways,
            "overwrite_if_newer" => Self::OverwriteIfNewer,
            "overwrite_if_larger" => Self::OverwriteIfLarger,
            "rename" => Self::Rename,
            "ask_each_time" => Self::AskEachTime,
            _ => Self::AskEachTime,
        }
    }
}


/// Retry policy configuration for transfers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryPolicy {
    pub max_retries: u32,
    /// Backoff durations in seconds for each retry attempt.
    pub backoff_seconds: Vec<u64>,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 3,
            backoff_seconds: vec![1, 5, 30],
        }
    }
}

/// Bandwidth throttle settings (bytes per second; 0 = unlimited).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct ThrottleConfig {
    /// Global bandwidth limit in bytes/sec (0 = unlimited).
    pub global_limit_bps: u64,
    /// Per-connection bandwidth limit in bytes/sec (0 = unlimited).
    pub per_connection_limit_bps: u64,
}


/// Represents a queued or active transfer job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferJob {
    pub id: Uuid,
    pub source_path: String,
    pub dest_path: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub status: TransferStatus,
    pub priority: TransferPriority,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub error_message: Option<String>,
    /// Number of retries attempted so far.
    pub retry_count: u32,
    /// Checksum of the transferred portion (for resume verification).
    pub partial_checksum: Option<String>,
    /// Conflict resolution policy for this specific job.
    pub conflict_policy: Option<ConflictPolicy>,
    /// Connection ID this transfer is associated with.
    pub connection_id: Option<Uuid>,
    /// Whether to verify checksum after transfer completes.
    pub verify_checksum: bool,
    /// Checksum algorithm to use for verification.
    pub checksum_algorithm: Option<ChecksumAlgorithm>,
    /// Computed source file checksum (set after verification).
    pub source_checksum: Option<String>,
    /// Computed destination file checksum (set after verification).
    pub dest_checksum: Option<String>,
    /// Transfer speed in bytes/sec (rolling average).
    pub speed_bps: u64,
    /// Estimated time remaining in seconds.
    pub eta_seconds: Option<u64>,
}

/// A resolved conflict with the decision made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictResolution {
    pub job_id: Uuid,
    pub source_path: String,
    pub dest_path: String,
    pub policy_applied: ConflictPolicy,
    pub resolved_dest_path: String,
    pub source_size: u64,
    pub dest_size: u64,
    pub source_modified: Option<DateTime<Utc>>,
    pub dest_modified: Option<DateTime<Utc>>,
    pub timestamp: DateTime<Utc>,
}

/// Transfer history record for T-018.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferHistoryRecord {
    pub id: Uuid,
    pub source_path: String,
    pub dest_path: String,
    pub total_bytes: u64,
    pub transferred_bytes: u64,
    pub status: TransferStatus,
    pub started_at: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub speed_bps: u64,
    pub error_message: Option<String>,
    pub connection_id: Option<Uuid>,
    pub checksum_verified: bool,
    pub checksum_match: Option<bool>,
    pub conflict_policy: Option<ConflictPolicy>,
    pub conflict_resolution: Option<String>,
    pub retry_count: u32,
}

/// Transfer history search filters.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TransferHistoryFilter {
    pub path_contains: Option<String>,
    pub status: Option<TransferStatus>,
    pub connection_id: Option<Uuid>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

/// Export format for transfer history.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExportFormat {
    Csv,
    Json,
}

/// Progress update emitted to the frontend via events.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferProgress {
    pub job_id: Uuid,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
    pub status: TransferStatus,
    pub error_message: Option<String>,
}

/// Connection group (folder) for organizing connections.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionGroup {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
}

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

// ──────────────────────────────────────────────
// Compatibility Types
// ──────────────────────────────────────────────

/// Result of a compatibility check on a filename.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatResult {
    pub original_name: String,
    pub translated_name: String,
    pub rules_applied: Vec<String>,
    pub is_modified: bool,
    pub reversible: bool,
}

/// A stored compatibility mapping.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatMapping {
    pub id: Uuid,
    pub source_pattern: String,
    pub target_replacement: String,
    pub platform: String,
    pub enabled: bool,
}

// ──────────────────────────────────────────────
// Governance / Audit Types
// ──────────────────────────────────────────────

/// Audit event for governance logging.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: Uuid,
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub actor: String,
    pub resource: String,
    pub details: String,
}

// ──────────────────────────────────────────────
// Connection / Bookmark Types
// ──────────────────────────────────────────────

/// Protocol for remote connections.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionProtocol {
    Sftp,
    Ftp,
    Ftps,
    WebDav,
    Smb,
    Nfs,
    S3,
    GoogleDrive,
    Dropbox,
    OneDrive,
    Local,
    AzureBlob,
    Swift,
}

impl ConnectionProtocol {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sftp => "sftp",
            Self::Ftp => "ftp",
            Self::Ftps => "ftps",
            Self::WebDav => "webdav",
            Self::Smb => "smb",
            Self::Nfs => "nfs",
            Self::S3 => "s3",
            Self::GoogleDrive => "google_drive",
            Self::Dropbox => "dropbox",
            Self::OneDrive => "onedrive",
            Self::Local => "local",
            Self::AzureBlob => "azure_blob",
            Self::Swift => "swift",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "sftp" => Self::Sftp,
            "ftp" => Self::Ftp,
            "ftps" => Self::Ftps,
            "webdav" => Self::WebDav,
            "smb" => Self::Smb,
            "nfs" => Self::Nfs,
            "s3" => Self::S3,
            "google_drive" => Self::GoogleDrive,
            "dropbox" => Self::Dropbox,
            "onedrive" => Self::OneDrive,
            "local" => Self::Local,
            "azure_blob" => Self::AzureBlob,
            "swift" => Self::Swift,
            _ => Self::Local,
        }
    }
}

/// A saved remote connection profile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnectionProfile {
    pub id: Uuid,
    pub name: String,
    pub protocol: ConnectionProtocol,
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    /// Credential reference (keychain ID), never stored in plaintext.
    pub credential_ref: Option<String>,
    pub remote_path: String,
    pub created_at: DateTime<Utc>,
    pub last_used: Option<DateTime<Utc>>,
    /// Group this connection belongs to.
    pub group_id: Option<Uuid>,
    /// Per-connection bandwidth limit (bytes/sec, 0 = unlimited).
    pub bandwidth_limit_bps: u64,
    /// Per-connection conflict resolution policy.
    pub conflict_policy: Option<ConflictPolicy>,
    /// Per-connection retry policy.
    pub retry_policy: Option<RetryPolicy>,
    /// Whether transfers on this connection verify checksums.
    pub verify_checksums: bool,
    /// Preferred checksum algorithm.
    pub checksum_algorithm: Option<ChecksumAlgorithm>,

    // ── Proxy configuration (per-connection) ──
    /// Proxy type: None, "http", "socks5".
    pub proxy_type: Option<String>,
    /// Proxy host.
    pub proxy_host: Option<String>,
    /// Proxy port.
    pub proxy_port: Option<u16>,
    /// Proxy username (optional).
    pub proxy_username: Option<String>,
    /// Proxy password (optional).
    pub proxy_password: Option<String>,

    // ── Default paths (per-connection) ──
    /// Default local directory to open when connecting.
    pub default_local_dir: Option<String>,
    /// Default remote directory to navigate to on connect.
    pub default_remote_dir: Option<String>,

    // ── Charset (per-connection) ──
    /// Character set for FTP/FTPS connections (default: "utf-8").
    pub charset: Option<String>,

    // ── Upload-time permission defaults (per-connection) ──
    /// Default file mode applied to uploaded files (octal, e.g. "644").
    /// Only used for SFTP/FTP connections. None = use server defaults.
    pub default_file_mode: Option<String>,
    /// Default directory mode applied to created directories (octal, e.g. "755").
    /// Only used for SFTP/FTP connections. None = use server defaults.
    pub default_dir_mode: Option<String>,

    // ── FTP advanced tuning (per-connection) ──
    /// Use MLSD instead of LIST for directory listings (more reliable parsing).
    /// None = auto-detect, Some(true) = force MLSD, Some(false) = force LIST.
    pub ftp_use_mlsd: Option<bool>,
    /// Force passive mode to use connection IP instead of server-reported IP.
    /// Fixes issues with servers behind NAT that report private IPs.
    pub ftp_force_passive_ip: Option<bool>,
    /// Commands to execute immediately after FTP login (e.g., "OPTS UTF8 ON").
    pub ftp_post_login_commands: Option<Vec<String>>,

    // ── Symlink handling (per-connection) ──
    /// How to handle symlinks during transfers and sync.
    /// "follow" = resolve and transfer target, "skip" = ignore symlinks,
    /// "preserve" = recreate symlinks at destination (default).
    pub symlink_policy: Option<String>,
}

/// Global proxy configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[derive(Default)]
pub struct ProxyConfig {
    pub proxy_type: Option<String>,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub password: Option<String>,
}


/// A filesystem bookmark.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bookmark {
    pub id: Uuid,
    pub name: String,
    pub path: String,
    pub icon: Option<String>,
    pub sort_order: i32,
    pub created_at: DateTime<Utc>,
}

// ──────────────────────────────────────────────
// Configuration Types
// ──────────────────────────────────────────────

/// A key-value configuration entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
    pub updated_at: DateTime<Utc>,
}

// ──────────────────────────────────────────────
// Workspace / App State Types
// ──────────────────────────────────────────────

/// Full workspace state that is persisted across restarts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceState {
    pub version: u32,
    pub tabs: Vec<TabState>,
    pub active_tab_index: usize,
    pub sidebar: SidebarState,
    pub theme: String,
    pub window: WindowState,
    pub panes: Vec<PaneState>,
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self {
            version: 1,
            tabs: vec![TabState {
                id: Uuid::new_v4(),
                path: dirs_home(),
                label: "Home".to_string(),
            }],
            active_tab_index: 0,
            sidebar: SidebarState::default(),
            theme: "system".to_string(),
            window: WindowState::default(),
            panes: vec![PaneState::default()],
        }
    }
}

/// Returns the user home directory as a string, or "/" as fallback.
fn dirs_home() -> String {
    directories::UserDirs::new()
        .map(|d| d.home_dir().to_string_lossy().to_string())
        .unwrap_or_else(|| "/".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabState {
    pub id: Uuid,
    pub path: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidebarState {
    pub visible: bool,
    pub width: u32,
    pub active_section: String,
}

impl Default for SidebarState {
    fn default() -> Self {
        Self {
            visible: true,
            width: 240,
            active_section: "bookmarks".to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowState {
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

impl Default for WindowState {
    fn default() -> Self {
        Self {
            x: None,
            y: None,
            width: 1280,
            height: 800,
            maximized: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PaneState {
    pub id: Uuid,
    pub path: String,
    pub view_mode: String,
    pub sort_by: String,
    pub sort_asc: bool,
    pub width_percent: f64,
}

impl Default for PaneState {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            path: dirs_home(),
            view_mode: "list".to_string(),
            sort_by: "name".to_string(),
            sort_asc: true,
            width_percent: 100.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_status_roundtrip() {
        let statuses = [
            TransferStatus::Queued,
            TransferStatus::Active,
            TransferStatus::Paused,
            TransferStatus::Completed,
            TransferStatus::Failed,
            TransferStatus::Cancelled,
        ];
        for s in &statuses {
            assert_eq!(*s, TransferStatus::from_str_lossy(s.as_str()));
        }
    }

    #[test]
    fn test_transfer_priority_roundtrip() {
        let priorities = [
            TransferPriority::Low,
            TransferPriority::Normal,
            TransferPriority::High,
        ];
        for p in &priorities {
            assert_eq!(*p, TransferPriority::from_str_lossy(p.as_str()));
        }
    }

    #[test]
    fn test_conflict_policy_roundtrip() {
        let policies = [
            ConflictPolicy::OverwriteAlways,
            ConflictPolicy::SkipAlways,
            ConflictPolicy::OverwriteIfNewer,
            ConflictPolicy::OverwriteIfLarger,
            ConflictPolicy::Rename,
            ConflictPolicy::AskEachTime,
        ];
        for p in &policies {
            assert_eq!(*p, ConflictPolicy::from_str_lossy(p.as_str()));
        }
    }

    #[test]
    fn test_checksum_algorithm_roundtrip() {
        let algs = [ChecksumAlgorithm::Md5, ChecksumAlgorithm::Sha256];
        for a in &algs {
            assert_eq!(*a, ChecksumAlgorithm::from_str_lossy(a.as_str()));
        }
    }

    #[test]
    fn test_sync_mode_roundtrip() {
        let modes = [
            SyncMode::OneWay,
            SyncMode::TwoWay,
            SyncMode::Mirror,
            SyncMode::VersionedBackup,
        ];
        for m in &modes {
            assert_eq!(*m, SyncMode::from_str_lossy(m.as_str()));
        }
    }

    #[test]
    fn test_connection_protocol_roundtrip() {
        let protocols = [
            ConnectionProtocol::Sftp,
            ConnectionProtocol::Ftp,
            ConnectionProtocol::Ftps,
            ConnectionProtocol::WebDav,
            ConnectionProtocol::Smb,
            ConnectionProtocol::Nfs,
            ConnectionProtocol::S3,
            ConnectionProtocol::GoogleDrive,
            ConnectionProtocol::Dropbox,
            ConnectionProtocol::OneDrive,
            ConnectionProtocol::Local,
            ConnectionProtocol::AzureBlob,
            ConnectionProtocol::Swift,
        ];
        for p in &protocols {
            assert_eq!(*p, ConnectionProtocol::from_str_lossy(p.as_str()));
        }
    }

    #[test]
    fn test_workspace_state_default() {
        let ws = WorkspaceState::default();
        assert_eq!(ws.version, 1);
        assert_eq!(ws.tabs.len(), 1);
        assert_eq!(ws.active_tab_index, 0);
        assert!(ws.sidebar.visible);
        assert_eq!(ws.theme, "system");
        assert_eq!(ws.panes.len(), 1);
    }

    #[test]
    fn test_workspace_state_serialization() {
        let ws = WorkspaceState::default();
        let json = serde_json::to_string(&ws).unwrap();
        let ws2: WorkspaceState = serde_json::from_str(&json).unwrap();
        assert_eq!(ws.version, ws2.version);
        assert_eq!(ws.tabs.len(), ws2.tabs.len());
        assert_eq!(ws.theme, ws2.theme);
    }

    #[test]
    fn test_file_entry_serialization() {
        let entry = FileEntry {
            name: "test.txt".to_string(),
            path: "/tmp/test.txt".to_string(),
            is_dir: false,
            is_symlink: false,
            size: 1024,
            modified: Some(Utc::now()),
            created: Some(Utc::now()),
            is_hidden: false,
            extension: Some("txt".to_string()),
            permissions: Some("644".to_string()),
        };
        let json = serde_json::to_string(&entry).unwrap();
        let entry2: FileEntry = serde_json::from_str(&json).unwrap();
        assert_eq!(entry.name, entry2.name);
        assert_eq!(entry.size, entry2.size);
    }
}
