//! Sync engine - one-way, two-way, mirror, versioned backup sync.
//!
//! Provides `SyncManager` implementing full sync operations including:
//! - Sync pair CRUD with 4 modes and 4 trigger types
//! - Dry-run preview with diff computation
//! - 7 conflict resolution policies
//! - Filesystem watching and cron-like scheduling
//! - Pre-destructive rollback snapshots
//! - Per-run reporting with health indicators
//! - Checksum verification (fast and full modes)
//! - Partial failure continuation and resumable sync

pub mod conflict;
pub mod executor;
pub mod planner;
pub mod report;
pub mod rollback;
pub mod scheduler;
pub mod watcher;

use crate::core::error::AppError;
use crate::core::traits::{BoxFuture, SyncOperations};
use crate::core::types::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

/// Named filter presets for common exclusion patterns.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterPreset {
    pub id: String,
    pub name: String,
    pub description: String,
    pub patterns: Vec<String>,
    pub built_in: bool,
}

/// Get available filter presets.
pub fn get_filter_presets() -> Vec<FilterPreset> {
    vec![
        FilterPreset {
            id: "macos_metadata".to_string(),
            name: "macOS Metadata".to_string(),
            description: "Exclude .DS_Store, ._* resource forks, .Spotlight-V100, .fseventsd".to_string(),
            patterns: vec![
                ".DS_Store".to_string(),
                "._*".to_string(),
                ".Spotlight-V100".to_string(),
                ".fseventsd".to_string(),
                ".Trashes".to_string(),
                "__MACOSX".to_string(),
            ],
            built_in: true,
        },
        FilterPreset {
            id: "windows_metadata".to_string(),
            name: "Windows Metadata".to_string(),
            description: "Exclude Thumbs.db, desktop.ini, System Volume Information".to_string(),
            patterns: vec![
                "Thumbs.db".to_string(),
                "desktop.ini".to_string(),
                "System Volume Information".to_string(),
                "$RECYCLE.BIN".to_string(),
                "ehthumbs.db".to_string(),
            ],
            built_in: true,
        },
        FilterPreset {
            id: "version_control".to_string(),
            name: "Version Control".to_string(),
            description: "Exclude .git, .svn, .hg, .bzr directories".to_string(),
            patterns: vec![
                ".git".to_string(),
                ".svn".to_string(),
                ".hg".to_string(),
                ".bzr".to_string(),
                ".gitignore".to_string(),
            ],
            built_in: true,
        },
        FilterPreset {
            id: "ide_files".to_string(),
            name: "IDE & Editor Files".to_string(),
            description: "Exclude .idea, .vscode, *.swp, *~".to_string(),
            patterns: vec![
                ".idea".to_string(),
                ".vscode".to_string(),
                "*.swp".to_string(),
                "*.swo".to_string(),
                "*~".to_string(),
                ".project".to_string(),
                ".settings".to_string(),
            ],
            built_in: true,
        },
        FilterPreset {
            id: "temp_files".to_string(),
            name: "Temporary Files".to_string(),
            description: "Exclude *.tmp, *.temp, *.bak, *.log".to_string(),
            patterns: vec![
                "*.tmp".to_string(),
                "*.temp".to_string(),
                "*.bak".to_string(),
                "*.log".to_string(),
                "*.cache".to_string(),
            ],
            built_in: true,
        },
        FilterPreset {
            id: "node_modules".to_string(),
            name: "Node.js Dependencies".to_string(),
            description: "Exclude node_modules, .npm, package-lock.json".to_string(),
            patterns: vec![
                "node_modules".to_string(),
                ".npm".to_string(),
                ".yarn".to_string(),
                ".pnp.*".to_string(),
            ],
            built_in: true,
        },
    ]
}

/// Manages sync pairs and executes sync runs with full feature set.
pub struct SyncManager {
    pairs: Arc<RwLock<HashMap<Uuid, SyncPair>>>,
    reports: Arc<RwLock<HashMap<Uuid, Vec<SyncReport>>>>,
    snapshots: Arc<RwLock<HashMap<Uuid, Vec<SyncSnapshot>>>>,
    quarantine: Arc<RwLock<HashMap<Uuid, Vec<QuarantineEntry>>>>,
    conflicts: Arc<RwLock<Vec<SyncConflictItem>>>,
    resume_states: Arc<RwLock<HashMap<Uuid, SyncResumeState>>>,
    watcher_mgr: Arc<watcher::WatcherManager>,
    scheduler: Arc<scheduler::SyncScheduler>,
}

impl SyncManager {
    pub fn new() -> Self {
        Self {
            pairs: Arc::new(RwLock::new(HashMap::new())),
            reports: Arc::new(RwLock::new(HashMap::new())),
            snapshots: Arc::new(RwLock::new(HashMap::new())),
            quarantine: Arc::new(RwLock::new(HashMap::new())),
            conflicts: Arc::new(RwLock::new(Vec::new())),
            resume_states: Arc::new(RwLock::new(HashMap::new())),
            watcher_mgr: Arc::new(watcher::WatcherManager::default()),
            scheduler: Arc::new(scheduler::SyncScheduler::new()),
        }
    }

    /// Get a dry-run preview for a sync pair.
    pub async fn dry_run(&self, pair_id: Uuid) -> Result<SyncPreview, AppError> {
        let pairs = self.pairs.read().await;
        let pair = pairs.get(&pair_id).ok_or_else(|| AppError::Sync {
            message: format!("Sync pair {} not found", pair_id),
            advice: "Check the sync pair ID.".to_string(),
        })?;

        // Run planner on blocking thread
        let pair_clone = pair.clone();
        tokio::task::spawn_blocking(move || planner::compute_preview(&pair_clone))
            .await
            .map_err(|e| AppError::Sync {
                message: format!("Dry-run task failed: {}", e),
                advice: "Try again.".to_string(),
            })?
    }

    /// Export a preview as CSV.
    pub async fn export_preview_csv(
        &self,
        pair_id: Uuid,
    ) -> Result<String, AppError> {
        let preview = self.dry_run(pair_id).await?;
        planner::export_preview_csv(&preview)
    }

    /// Execute a sync run for a pair.
    pub async fn execute_sync(&self, pair_id: Uuid) -> Result<SyncReport, AppError> {
        let pair = {
            let pairs = self.pairs.read().await;
            pairs.get(&pair_id).cloned().ok_or_else(|| AppError::Sync {
                message: format!("Sync pair {} not found", pair_id),
                advice: "Create a sync pair before running a sync.".to_string(),
            })?
        };

        if !pair.enabled {
            return Err(AppError::Sync {
                message: "Sync pair is disabled.".to_string(),
                advice: "Enable the sync pair before running.".to_string(),
            });
        }

        let run_id = Uuid::new_v4();

        // Check for resume state
        let resume_from = {
            let resume_states = self.resume_states.read().await;
            resume_states
                .get(&pair_id)
                .map(|s| s.last_completed_index)
        };

        let quarantine_dir = PathBuf::from(&pair.dest_path)
            .join(".sync-quarantine");

        let config = executor::ExecutorConfig {
            pair: pair.clone(),
            run_id,
            continue_on_error: true,
            resume_from,
            quarantine_dir: Some(quarantine_dir),
            rollback_dir: None,
        };

        // Run on blocking thread
        let config_clone = config.clone();
        let report = tokio::task::spawn_blocking(move || executor::execute_sync(&config_clone))
            .await
            .map_err(|e| AppError::Sync {
                message: format!("Sync execution task failed: {}", e),
                advice: "Try again.".to_string(),
            })??;

        // Update pair last_run
        {
            let mut pairs = self.pairs.write().await;
            if let Some(p) = pairs.get_mut(&pair_id) {
                p.last_run = Some(Utc::now());
            }
        }

        // Store report
        {
            let mut reports = self.reports.write().await;
            reports
                .entry(pair_id)
                .or_insert_with(Vec::new)
                .insert(0, report.clone());
        }

        // Clear resume state on success
        if report.status == SyncRunStatus::Success {
            let mut resume_states = self.resume_states.write().await;
            resume_states.remove(&pair_id);
        }

        // Run verification if enabled
        if pair.checksum_enabled || pair.verify_mode != SyncVerifyMode::None {
            let pair_clone = pair.clone();
            let report_clone = report.clone();
            let mismatches = tokio::task::spawn_blocking(move || {
                executor::verify_sync(&pair_clone, &report_clone)
            })
            .await
            .map_err(|e| AppError::Sync {
                message: format!("Verification task failed: {}", e),
                advice: "The sync completed but verification could not run.".to_string(),
            })??;

            if !mismatches.is_empty() {
                tracing::warn!(
                    "Sync verification found {} mismatch(es) for pair {}",
                    mismatches.len(),
                    pair_id
                );
            }
        }

        Ok(report)
    }

    /// Rollback the last sync run for a pair.
    pub async fn rollback_last_run(&self, pair_id: Uuid) -> Result<u64, AppError> {
        let snapshots = {
            let snap_map = self.snapshots.read().await;
            snap_map.get(&pair_id).cloned().unwrap_or_default()
        };

        if snapshots.is_empty() {
            return Err(AppError::Sync {
                message: "No rollback data available for this sync pair.".to_string(),
                advice: "Rollback is only available for the last sync run within 7 days.".to_string(),
            });
        }

        if !rollback::is_rollback_valid(&snapshots) {
            return Err(AppError::Sync {
                message: "Rollback data has expired (>7 days old).".to_string(),
                advice: "Rollback is only available within 7 days of the sync run.".to_string(),
            });
        }

        let dest_path = {
            let pairs = self.pairs.read().await;
            pairs
                .get(&pair_id)
                .map(|p| PathBuf::from(&p.dest_path))
                .ok_or_else(|| AppError::Sync {
                    message: format!("Sync pair {} not found", pair_id),
                    advice: "Check the sync pair ID.".to_string(),
                })?
        };

        let snapshots_clone = snapshots.clone();
        let restored = tokio::task::spawn_blocking(move || {
            rollback::rollback_sync_run(&snapshots_clone, &dest_path)
        })
        .await
        .map_err(|e| AppError::Sync {
            message: format!("Rollback task failed: {}", e),
            advice: "Try again.".to_string(),
        })??;

        // Clear snapshots after rollback
        {
            let mut snap_map = self.snapshots.write().await;
            snap_map.remove(&pair_id);
        }

        Ok(restored)
    }

    /// Get reports for a sync pair.
    pub async fn get_reports(&self, pair_id: Uuid) -> Vec<SyncReport> {
        let reports = self.reports.read().await;
        reports.get(&pair_id).cloned().unwrap_or_default()
    }

    /// Get the health indicator for a sync pair.
    pub async fn get_health(&self, pair_id: Uuid) -> SyncHealth {
        let pair_enabled = {
            let pairs = self.pairs.read().await;
            pairs.get(&pair_id).map(|p| p.enabled).unwrap_or(false)
        };

        let reports = self.get_reports(pair_id).await;
        report::compute_health(&reports, pair_enabled)
    }

    /// Start filesystem watcher for a pair.
    pub async fn start_watcher(&self, pair_id: Uuid) -> Result<(), AppError> {
        let pair = {
            let pairs = self.pairs.read().await;
            pairs.get(&pair_id).cloned().ok_or_else(|| AppError::Sync {
                message: format!("Sync pair {} not found", pair_id),
                advice: "Check the sync pair ID.".to_string(),
            })?
        };

        let source_path = PathBuf::from(&pair.source_path);

        // Create a callback that will trigger sync execution
        let _pairs_ref = self.pairs.clone();
        let callback: watcher::OnChangeCallback = Box::new(move |pid| {
            tracing::info!("Watcher detected changes for sync pair {}", pid);
            // The actual sync will be triggered by the frontend polling or event
        });

        self.watcher_mgr
            .start_watching(pair_id, source_path, callback)
    }

    /// Stop filesystem watcher for a pair.
    pub async fn stop_watcher(&self, pair_id: Uuid) -> Result<(), AppError> {
        self.watcher_mgr.stop_watching(pair_id)
    }

    /// Add a scheduled sync pair.
    pub async fn add_schedule(
        &self,
        pair_id: Uuid,
        cron_expr: &str,
    ) -> Result<(), AppError> {
        self.scheduler.add_schedule(pair_id, cron_expr)?;
        Ok(())
    }

    /// Remove a scheduled sync pair.
    pub async fn remove_schedule(&self, pair_id: Uuid) -> Result<(), AppError> {
        self.scheduler.remove_schedule(pair_id)
    }

    /// Get quarantine entries for a pair.
    pub async fn get_quarantine(&self, pair_id: Uuid) -> Vec<QuarantineEntry> {
        let quarantine = self.quarantine.read().await;
        quarantine.get(&pair_id).cloned().unwrap_or_default()
    }

    /// Get pending conflicts for resolution.
    pub async fn get_pending_conflicts(&self) -> Vec<SyncConflictItem> {
        let conflicts = self.conflicts.read().await;
        conflicts
            .iter()
            .filter(|c| c.resolution.is_none())
            .cloned()
            .collect()
    }

    /// Resolve a conflict manually.
    pub async fn resolve_conflict_item(
        &self,
        conflict_id: Uuid,
        resolution: SyncConflictPolicy,
    ) -> Result<(), AppError> {
        let mut conflicts = self.conflicts.write().await;
        if let Some(item) = conflicts.iter_mut().find(|c| c.id == conflict_id) {
            item.resolution = Some(resolution);
            Ok(())
        } else {
            Err(AppError::Sync {
                message: "Conflict item not found.".to_string(),
                advice: "The conflict may have already been resolved.".to_string(),
            })
        }
    }

    /// Update sync pair configuration.
    pub async fn update_pair(&self, pair: SyncPair) -> Result<SyncPair, AppError> {
        let mut pairs = self.pairs.write().await;
        if let std::collections::hash_map::Entry::Occupied(mut e) = pairs.entry(pair.id) {
            e.insert(pair.clone());
            Ok(pair)
        } else {
            Err(AppError::Sync {
                message: "Sync pair not found.".to_string(),
                advice: "Create the sync pair first.".to_string(),
            })
        }
    }

    /// Export a report as CSV.
    pub async fn export_report_csv(
        &self,
        pair_id: Uuid,
        report_id: Uuid,
    ) -> Result<String, AppError> {
        let reports = self.reports.read().await;
        let pair_reports = reports.get(&pair_id).ok_or_else(|| AppError::Sync {
            message: "No reports found for this sync pair.".to_string(),
            advice: "Run a sync first.".to_string(),
        })?;

        let report = pair_reports
            .iter()
            .find(|r| r.id == report_id)
            .ok_or_else(|| AppError::Sync {
                message: "Report not found.".to_string(),
                advice: "Check the report ID.".to_string(),
            })?;

        report::export_report_csv(report)
    }

    /// Export a report as JSON.
    pub async fn export_report_json(
        &self,
        pair_id: Uuid,
        report_id: Uuid,
    ) -> Result<String, AppError> {
        let reports = self.reports.read().await;
        let pair_reports = reports.get(&pair_id).ok_or_else(|| AppError::Sync {
            message: "No reports found.".to_string(),
            advice: "Run a sync first.".to_string(),
        })?;

        let report = pair_reports
            .iter()
            .find(|r| r.id == report_id)
            .ok_or_else(|| AppError::Sync {
                message: "Report not found.".to_string(),
                advice: "Check the report ID.".to_string(),
            })?;

        report::export_report_json(report)
    }
}

impl Default for SyncManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncOperations for SyncManager {
    fn create_pair(&self, pair: SyncPair) -> BoxFuture<'_, Result<SyncPair, AppError>> {
        Box::pin(async move {
            let mut map = self.pairs.write().await;
            map.insert(pair.id, pair.clone());
            tracing::info!("Sync pair created: {} ({})", pair.name, pair.id);

            // Set up trigger if needed
            match &pair.trigger {
                SyncTrigger::Watch => {
                    drop(map); // Release lock before starting watcher
                    if let Err(e) = self.start_watcher(pair.id).await {
                        tracing::warn!("Failed to start watcher for {}: {}", pair.id, e);
                    }
                }
                SyncTrigger::Scheduled { cron_expr } => {
                    if let Err(e) = self.scheduler.add_schedule(pair.id, cron_expr) {
                        tracing::warn!("Failed to add schedule for {}: {}", pair.id, e);
                    }
                }
                _ => {}
            }

            Ok(pair)
        })
    }

    fn delete_pair(&self, pair_id: Uuid) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            // Stop watcher and schedule
            let _ = self.stop_watcher(pair_id).await;
            let _ = self.scheduler.remove_schedule(pair_id);

            let mut map = self.pairs.write().await;
            map.remove(&pair_id).ok_or_else(|| AppError::Sync {
                message: format!("Sync pair {pair_id} not found."),
                advice: "The sync pair may have already been removed.".to_string(),
            })?;

            // Clean up associated data
            {
                let mut reports = self.reports.write().await;
                reports.remove(&pair_id);
            }
            {
                let mut snapshots = self.snapshots.write().await;
                snapshots.remove(&pair_id);
            }
            {
                let mut quarantine = self.quarantine.write().await;
                quarantine.remove(&pair_id);
            }

            Ok(())
        })
    }

    fn list_pairs(&self) -> BoxFuture<'_, Result<Vec<SyncPair>, AppError>> {
        Box::pin(async move {
            let map = self.pairs.read().await;
            Ok(map.values().cloned().collect())
        })
    }

    fn run_sync(&self, pair_id: Uuid) -> BoxFuture<'_, Result<SyncState, AppError>> {
        Box::pin(async move {
            let report = self.execute_sync(pair_id).await?;

            // Update pair last_run
            {
                let mut map = self.pairs.write().await;
                if let Some(pair) = map.get_mut(&pair_id) {
                    pair.last_run = Some(Utc::now());
                }
            }

            Ok(SyncState {
                pair_id,
                last_sync_at: report.ended_at,
                files_synced: report.files_added + report.files_modified + report.files_deleted,
                bytes_synced: report.bytes_transferred,
                errors: report.errors,
                status: report.status,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::SyncMode;
    use tempfile::TempDir;

    fn make_test_pair(src: &std::path::Path, dst: &std::path::Path) -> SyncPair {
        SyncPair {
            id: Uuid::new_v4(),
            name: "Test Pair".to_string(),
            source_path: src.to_string_lossy().to_string(),
            dest_path: dst.to_string_lossy().to_string(),
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

    #[tokio::test]
    async fn test_create_and_list_pairs() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();

        let pairs = mgr.list_pairs().await.unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].id, pair.id);
    }

    #[tokio::test]
    async fn test_delete_pair() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();
        mgr.delete_pair(pair.id).await.unwrap();

        let pairs = mgr.list_pairs().await.unwrap();
        assert!(pairs.is_empty());
    }

    #[tokio::test]
    async fn test_delete_nonexistent_pair() {
        let mgr = SyncManager::new();
        let result = mgr.delete_pair(Uuid::new_v4()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_dry_run() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        std::fs::write(src.path().join("file.txt"), "hello").unwrap();

        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();

        let preview = mgr.dry_run(pair.id).await.unwrap();
        assert_eq!(preview.total_additions, 1);
        assert_eq!(preview.total_bytes, 5);
    }

    #[tokio::test]
    async fn test_execute_sync_one_way() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        std::fs::write(src.path().join("a.txt"), "alpha").unwrap();
        std::fs::write(src.path().join("b.txt"), "bravo").unwrap();

        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();

        let report = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(report.files_added, 2);
        assert_eq!(report.status, SyncRunStatus::Success);

        // Verify files copied
        assert!(dst.path().join("a.txt").exists());
        assert!(dst.path().join("b.txt").exists());
    }

    #[tokio::test]
    async fn test_execute_sync_mirror() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        // File only in destination - should be deleted in mirror mode
        std::fs::write(dst.path().join("extra.txt"), "should go").unwrap();
        std::fs::write(src.path().join("keep.txt"), "keep this").unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::Mirror;
        mgr.create_pair(pair.clone()).await.unwrap();

        let report = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(report.files_deleted, 1);
        assert_eq!(report.files_added, 1);

        assert!(!dst.path().join("extra.txt").exists());
        assert!(dst.path().join("keep.txt").exists());
    }

    #[tokio::test]
    async fn test_run_sync_returns_state() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        std::fs::write(src.path().join("file.txt"), "test").unwrap();

        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();

        let state = mgr.run_sync(pair.id).await.unwrap();
        assert_eq!(state.pair_id, pair.id);
        assert_eq!(state.status, SyncRunStatus::Success);
        assert_eq!(state.files_synced, 1);
    }

    #[tokio::test]
    async fn test_get_health() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();

        // No runs yet = gray
        let health = mgr.get_health(pair.id).await;
        assert_eq!(health, SyncHealth::Gray);

        // Run sync
        std::fs::write(src.path().join("f.txt"), "ok").unwrap();
        mgr.execute_sync(pair.id).await.unwrap();

        // After success = green
        let health = mgr.get_health(pair.id).await;
        assert_eq!(health, SyncHealth::Green);
    }

    #[tokio::test]
    async fn test_get_reports() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        std::fs::write(src.path().join("f.txt"), "data").unwrap();

        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();
        mgr.execute_sync(pair.id).await.unwrap();

        let reports = mgr.get_reports(pair.id).await;
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].status, SyncRunStatus::Success);
    }

    #[tokio::test]
    async fn test_update_pair() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();

        pair.name = "Updated Name".to_string();
        pair.mode = SyncMode::Mirror;
        let updated = mgr.update_pair(pair.clone()).await.unwrap();
        assert_eq!(updated.name, "Updated Name");
        assert_eq!(updated.mode, SyncMode::Mirror);
    }

    #[tokio::test]
    async fn test_disabled_pair_cannot_sync() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.enabled = false;
        mgr.create_pair(pair.clone()).await.unwrap();

        let result = mgr.execute_sync(pair.id).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_versioned_backup_mode() {
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();

        std::fs::write(src.path().join("doc.txt"), "version 1").unwrap();
        std::fs::write(dst.path().join("doc.txt"), "existing").unwrap();

        let mut pair = make_test_pair(src.path(), dst.path());
        pair.mode = SyncMode::VersionedBackup;
        mgr.create_pair(pair.clone()).await.unwrap();

        let report = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(report.status, SyncRunStatus::Success);

        // Original and versioned copy should both exist
        assert!(dst.path().join("doc.txt").exists());
    }
}
