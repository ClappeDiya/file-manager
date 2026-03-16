use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

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

/// Transfer job state.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferStatus {
    Queued,
    Active,
    Paused,
    Completed,
    Failed,
    Cancelled,
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
    pub created_at: DateTime<Utc>,
    pub error_message: Option<String>,
}

/// Sync pair configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncPair {
    pub id: Uuid,
    pub name: String,
    pub source_path: String,
    pub dest_path: String,
    pub mode: SyncMode,
    pub enabled: bool,
    pub last_run: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SyncMode {
    OneWay,
    TwoWay,
    Mirror,
    VersionedBackup,
}

/// Result of a compatibility check on a filename.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatResult {
    pub original_name: String,
    pub translated_name: String,
    pub rules_applied: Vec<String>,
    pub is_modified: bool,
    pub reversible: bool,
}

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
