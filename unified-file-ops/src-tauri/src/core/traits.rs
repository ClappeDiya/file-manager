//! Trait definitions for dependency injection and mock testing.
//!
//! Each subsystem exposes a trait so that the command layer (and tests) can
//! work against abstractions rather than concrete implementations.

use crate::core::error::AppError;
use crate::core::types::*;
use std::future::Future;
use std::pin::Pin;
use uuid::Uuid;

/// A boxed future that is Send – used for trait async methods.
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

// ── File-system operations ──

/// Trait for local file-system operations.
pub trait FsOperations: Send + Sync {
    fn list_directory(&self, path: &str) -> BoxFuture<'_, Result<Vec<FileEntry>, AppError>>;
    fn create_directory(&self, path: &str) -> BoxFuture<'_, Result<(), AppError>>;
    fn delete_entry(&self, path: &str, recursive: bool) -> BoxFuture<'_, Result<(), AppError>>;
    fn rename_entry(&self, from: &str, to: &str) -> BoxFuture<'_, Result<(), AppError>>;
    fn get_metadata(&self, path: &str) -> BoxFuture<'_, Result<FileEntry, AppError>>;
}

// ── Transfer engine ──

/// Trait for the async file-transfer queue.
pub trait TransferOperations: Send + Sync {
    fn enqueue(
        &self,
        source: &str,
        dest: &str,
        total_bytes: u64,
    ) -> BoxFuture<'_, Result<TransferJob, AppError>>;
    fn pause(&self, job_id: Uuid) -> BoxFuture<'_, Result<(), AppError>>;
    fn resume(&self, job_id: Uuid) -> BoxFuture<'_, Result<(), AppError>>;
    fn cancel(&self, job_id: Uuid) -> BoxFuture<'_, Result<(), AppError>>;
    fn list_jobs(&self) -> BoxFuture<'_, Result<Vec<TransferJob>, AppError>>;
    fn get_job(&self, job_id: Uuid) -> BoxFuture<'_, Result<Option<TransferJob>, AppError>>;
}

// ── Sync engine ──

/// Trait for folder synchronization.
pub trait SyncOperations: Send + Sync {
    fn create_pair(&self, pair: SyncPair) -> BoxFuture<'_, Result<SyncPair, AppError>>;
    fn delete_pair(&self, pair_id: Uuid) -> BoxFuture<'_, Result<(), AppError>>;
    fn list_pairs(&self) -> BoxFuture<'_, Result<Vec<SyncPair>, AppError>>;
    fn run_sync(&self, pair_id: Uuid) -> BoxFuture<'_, Result<SyncState, AppError>>;
}

// ── Compatibility engine ──

/// Trait for filename compatibility checking.
pub trait CompatOperations: Send + Sync {
    fn check_name(
        &self,
        name: &str,
        target_platform: &str,
    ) -> BoxFuture<'_, Result<CompatResult, AppError>>;
    fn list_mappings(&self) -> BoxFuture<'_, Result<Vec<CompatMapping>, AppError>>;
}

// ── Governance / Audit ──

/// Trait for governance and audit logging.
pub trait AuditOperations: Send + Sync {
    fn log_event(&self, event: AuditEvent) -> BoxFuture<'_, Result<(), AppError>>;
    fn query_events(
        &self,
        limit: u32,
        offset: u32,
    ) -> BoxFuture<'_, Result<Vec<AuditEvent>, AppError>>;
}

// ── Storage / Persistence ──

/// Trait for the persistence layer.
pub trait StorageOperations: Send + Sync {
    fn get_config(&self, key: &str) -> BoxFuture<'_, Result<Option<String>, AppError>>;
    fn set_config(&self, key: &str, value: &str) -> BoxFuture<'_, Result<(), AppError>>;
    fn save_workspace_state(
        &self,
        state: &WorkspaceState,
    ) -> BoxFuture<'_, Result<(), AppError>>;
    fn load_workspace_state(&self) -> BoxFuture<'_, Result<WorkspaceState, AppError>>;
    fn save_transfer_queue(
        &self,
        jobs: &[TransferJob],
    ) -> BoxFuture<'_, Result<(), AppError>>;
    fn load_transfer_queue(&self) -> BoxFuture<'_, Result<Vec<TransferJob>, AppError>>;
}

#[cfg(test)]
mod tests {
    use super::*;

    // Verify traits are object-safe (can be used as dyn Trait).
    fn _assert_object_safe(
        _fs: &dyn FsOperations,
        _transfer: &dyn TransferOperations,
        _sync: &dyn SyncOperations,
        _compat: &dyn CompatOperations,
        _audit: &dyn AuditOperations,
        _storage: &dyn StorageOperations,
    ) {
    }

    #[test]
    fn traits_are_defined() {
        // Compile-time check: if the module compiles, traits exist.
    }
}
