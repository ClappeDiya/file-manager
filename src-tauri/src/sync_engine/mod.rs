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

pub mod filter_presets;
pub mod live_files;
pub mod portable;
pub mod report;
pub mod scheduler;
pub mod watcher;

// The engine itself lives in `ufop-core` (shared with the CLI).
pub use ufop_core::sync::{compare, conflict, copier, executor, planner, robocopy, rollback, winattr};

use crate::core::error::AppError;
use crate::core::traits::{BoxFuture, SyncOperations};
use crate::core::types::*;
use chrono::{DateTime, Utc};
use ufop_core::run_trigger::RunTrigger;
use ufop_core::sync_types::{SyncItemAction, SyncRunItem};
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
    .into_iter()
    // File-type groups come from the shared rule engine's groups, so sync
    // and backup offer the same lists.
    .chain(MEDIA_GROUPS.iter().filter_map(|(id, name)| {
        let (_, description, patterns) = ufop_core::filter_rules::FILTER_GROUPS.iter().find(|(n, _, _)| n == id)?;
        Some(FilterPreset {
            id: (*id).to_string(),
            name: (*name).to_string(),
            description: format!("{description} ({})", patterns.iter().step_by(2).take(6).cloned().collect::<Vec<_>>().join(", ")),
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
            built_in: true,
        })
    }))
    .collect()
}

/// Shared groups offered as sync presets: (group name, label).
const MEDIA_GROUPS: [(&str, &str); 5] =
    [("images", "Images"), ("video", "Video"), ("audio", "Audio"), ("documents", "Documents"), ("archives", "Archives")];

/// Manages sync pairs and executes sync runs with full feature set.
pub struct SyncManager {
    pairs: Arc<RwLock<HashMap<Uuid, SyncPair>>>,
    reports: Arc<RwLock<HashMap<Uuid, Vec<SyncReport>>>>,
    snapshots: Arc<RwLock<HashMap<Uuid, Vec<SyncSnapshot>>>>,
    quarantine: Arc<RwLock<HashMap<Uuid, Vec<QuarantineEntry>>>>,
    conflicts: Arc<RwLock<Vec<SyncConflictItem>>>,
    watcher_mgr: Arc<watcher::WatcherManager>,
    scheduler: Arc<scheduler::SyncScheduler>,
    /// Where rollback snapshots and quarantined files live. Always outside
    /// every sync root, so a mirror run can never purge them.
    state_dir: PathBuf,
    /// Cancel flags of runs in progress, by pair.
    running: Arc<RwLock<HashMap<Uuid, Arc<std::sync::atomic::AtomicBool>>>>,
    /// Latest progress of each run in progress, by pair.
    progress: Arc<std::sync::Mutex<HashMap<Uuid, executor::SyncProgress>>>,
    /// The files each run in progress is working on and has finished, by pair.
    live: Arc<std::sync::Mutex<HashMap<Uuid, live_files::LiveRun>>>,
    /// Told about every finished run (desktop and profile notifications).
    run_listeners: Arc<std::sync::RwLock<Vec<RunListener>>>,
    /// Database for two-way baselines; without one (unit tests) they are
    /// kept in memory for the manager's lifetime.
    repo: Arc<std::sync::RwLock<Option<crate::storage::Repository>>>,
    baselines: Arc<RwLock<HashMap<Uuid, SyncBaseline>>>,
    /// Global default copy options: a pair with `option_overrides` takes
    /// every option it does not set from here.
    default_options: Arc<std::sync::RwLock<CopyOptions>>,
}

/// Config-table key of the global default copy options.
pub const DEFAULT_OPTIONS_KEY: &str = "sync.default_copy_options";

/// What the UI shows about a two-way pair's baseline.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BaselineInfo {
    /// Files both sides agreed on after the last successful run.
    pub files: u64,
    pub recorded_at: Option<DateTime<Utc>>,
    pub run_id: Option<Uuid>,
}

/// Compare a pair's two sides on demand (blocking): `Ok` with a one-line
/// outcome when they agree, `Err` naming the first differences otherwise.
/// What the automation step "Verify a sync pair" reports.
pub fn verify_outcome(pair: &SyncPair, full: bool) -> Result<String, String> {
    let diffs = executor::verify_now(pair, full).map_err(|e| e.user_message())?;
    if diffs.is_empty() {
        return Ok(format!("{} and {} agree", pair.source_path, pair.dest_path));
    }
    let shown: Vec<&str> = diffs.iter().take(5).map(String::as_str).collect();
    let more = if diffs.len() > shown.len() { format!(" (and {} more)", diffs.len() - shown.len()) } else { String::new() };
    Err(format!("{} difference(s) between {} and {}: {}{more}", diffs.len(), pair.source_path, pair.dest_path, shown.join("; ")))
}

/// How a sync run ended, for notifications.
#[derive(Debug, Clone)]
pub struct SyncRunEvent {
    /// The pair as it was when the run started.
    pub pair: SyncPair,
    /// The report, or the reason the run could not finish.
    pub outcome: Result<SyncReport, String>,
    /// Files the post-run verification found different (0 when it did not run).
    pub verify_mismatches: usize,
}

/// Listener for finished sync runs.
pub type RunListener = Arc<dyn Fn(&SyncRunEvent) + Send + Sync>;

/// Default location for sync state: the app data dir, else the temp dir.
fn default_state_dir() -> PathBuf {
    // Unit tests must never write into the developer's real app data.
    if cfg!(test) {
        return std::env::temp_dir().join(format!("ufop-sync-test-{}", std::process::id()));
    }
    crate::paths::data_dir().join("sync")
}

impl SyncManager {
    pub fn new() -> Self {
        Self::with_state_dir(default_state_dir())
    }

    /// A manager that keeps rollback and quarantine data under `state_dir`.
    pub fn with_state_dir(state_dir: PathBuf) -> Self {
        Self {
            state_dir,
            running: Arc::new(RwLock::new(HashMap::new())),
            progress: Arc::new(std::sync::Mutex::new(HashMap::new())),
            live: Arc::new(std::sync::Mutex::new(HashMap::new())),
            run_listeners: Arc::new(std::sync::RwLock::new(Vec::new())),
            pairs: Arc::new(RwLock::new(HashMap::new())),
            reports: Arc::new(RwLock::new(HashMap::new())),
            snapshots: Arc::new(RwLock::new(HashMap::new())),
            quarantine: Arc::new(RwLock::new(HashMap::new())),
            conflicts: Arc::new(RwLock::new(Vec::new())),
            watcher_mgr: Arc::new(watcher::WatcherManager::default()),
            scheduler: Arc::new(scheduler::SyncScheduler::new()),
            repo: Arc::new(std::sync::RwLock::new(None)),
            baselines: Arc::new(RwLock::new(HashMap::new())),
            default_options: Arc::new(std::sync::RwLock::new(CopyOptions::default())),
        }
    }

    /// Listen for finished runs, whoever started them.
    pub fn add_run_listener(&self, listener: RunListener) {
        self.run_listeners.write().unwrap_or_else(|e| e.into_inner()).push(listener);
    }

    /// The global default copy options.
    pub fn default_options(&self) -> CopyOptions {
        self.default_options.read().map(|d| d.clone()).unwrap_or_default()
    }

    /// Read the saved global defaults (startup).
    pub async fn load_default_options(&self, repo: &crate::storage::Repository) -> Result<(), AppError> {
        if let Some(json) = repo.get_config(DEFAULT_OPTIONS_KEY).await? {
            let d: CopyOptions = serde_json::from_str(&json)?;
            if let Ok(mut cur) = self.default_options.write() {
                *cur = d;
            }
        }
        Ok(())
    }

    /// Validate, save and apply new global defaults.
    pub async fn set_default_options(&self, repo: &crate::storage::Repository, options: CopyOptions) -> Result<CopyOptions, AppError> {
        copier::validate_options(&options)?;
        repo.set_config(DEFAULT_OPTIONS_KEY, &serde_json::to_string(&options)?).await?;
        if let Ok(mut cur) = self.default_options.write() {
            *cur = options.clone();
        }
        Ok(options)
    }

    /// The pair as it runs: its own options over the global defaults.
    pub fn effective(&self, pair: &SyncPair) -> SyncPair {
        pair.effective(&self.default_options())
    }

    /// Copy a pair's definition under a new id: "<name> (copy)", no run
    /// history or two-way baseline, and never on a schedule or watcher
    /// until the user turns one on.
    pub async fn duplicate_pair(&self, pair_id: Uuid) -> Result<SyncPair, AppError> {
        let original = self.pairs.read().await.get(&pair_id).cloned().ok_or_else(|| AppError::Sync {
            message: format!("Sync pair {} not found", pair_id),
            advice: "Check the sync pair ID.".to_string(),
        })?;
        // The trigger (cron expression included) is copied, but a copy that
        // would run on its own starts disabled: nothing arms it until the
        // user turns it on in Edit, after pointing it at its own folders.
        let automatic = matches!(original.trigger, SyncTrigger::Scheduled { .. } | SyncTrigger::Watch);
        let copy = SyncPair {
            id: Uuid::new_v4(),
            name: format!("{} (copy)", original.name),
            last_run: None,
            created_at: Utc::now(),
            enabled: original.enabled && !automatic,
            ..original
        };
        self.create_pair(copy).await
    }

    /// One pair as a `ufop-sync-pair` JSON export, naming the saved
    /// connections either folder is mounted from.
    pub async fn export_pair(&self, repo: &crate::storage::Repository, pair_id: Uuid) -> Result<String, AppError> {
        let pair = self.pairs.read().await.get(&pair_id).cloned().ok_or_else(|| AppError::Sync {
            message: format!("Sync pair {} not found", pair_id),
            advice: "Check the sync pair ID.".to_string(),
        })?;
        let (mounts, profiles) = Self::mounts_and_profiles(repo).await?;
        portable::export(&pair, portable::connection_refs(&pair, &mounts, &profiles))
    }

    /// Create a pair from an export. Anything this computer lacks (a saved
    /// connection, a folder) is listed, and such a pair stays disabled.
    pub async fn import_pair(&self, repo: &crate::storage::Repository, json: &str) -> Result<portable::ImportResult, AppError> {
        let doc = portable::parse(json)?;
        let (_, profiles) = Self::mounts_and_profiles(repo).await?;
        let mut warnings = portable::warnings(&doc, &profiles, |p| std::path::Path::new(p).exists());
        // As with a duplicate, a pair that would run on its own (and could
        // mirror-delete at once) is never armed by the import itself.
        let automatic = matches!(doc.pair.trigger, SyncTrigger::Scheduled { .. } | SyncTrigger::Watch);
        let enabled = warnings.is_empty() && doc.pair.enabled && !automatic;
        if !warnings.is_empty() {
            warnings.push("The pair is disabled until you fix this; enable it from Edit.".to_string());
        } else if doc.pair.enabled && automatic {
            warnings.push("It runs on a schedule or when files change, so it starts turned off: check its folders, then turn it on in Edit.".to_string());
        }
        let pair = SyncPair { id: Uuid::new_v4(), last_run: None, created_at: Utc::now(), enabled, ..doc.pair };
        let pair = self.create_pair(pair).await?;
        self.save_pair_to_db(repo, &pair).await?;
        Ok(portable::ImportResult { pair, warnings })
    }

    /// Saved mounts (connection id, mount point) and connection profiles.
    async fn mounts_and_profiles(repo: &crate::storage::Repository) -> Result<(Vec<(String, String)>, Vec<ConnectionProfile>), AppError> {
        let mounts = repo
            .pool()
            .execute(|conn| {
                let mut st = conn.prepare("SELECT connection_id, mount_point FROM mount_configs")?;
                let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
                Ok(rows.filter_map(|r| r.ok()).collect::<Vec<_>>())
            })
            .await?;
        Ok((mounts, repo.list_connections().await?))
    }

    /// Keep two-way baselines in this database (both app init paths call it).
    pub fn attach_repository(&self, repo: crate::storage::Repository) {
        if let Ok(mut r) = self.repo.write() {
            *r = Some(repo);
        }
    }

    fn repository(&self) -> Option<crate::storage::Repository> {
        self.repo.read().ok().and_then(|r| r.clone())
    }

    /// The baseline a two-way run of `pair` compares against: the one
    /// recorded after its last successful run, for these same folders.
    pub async fn load_baseline(&self, pair: &SyncPair) -> Result<Option<SyncBaseline>, AppError> {
        if pair.mode != SyncMode::TwoWay {
            return Ok(None);
        }
        let found = match self.repository() {
            Some(repo) => baseline_db::load(&repo, pair.id).await?,
            None => self.baselines.read().await.get(&pair.id).cloned(),
        };
        Ok(found.filter(|b| b.matches(pair)))
    }

    /// Record `baseline` for `pair_id`, replacing the previous one in one
    /// transaction.
    pub async fn save_baseline(&self, pair_id: Uuid, baseline: SyncBaseline) -> Result<(), AppError> {
        match self.repository() {
            Some(repo) => baseline_db::save(&repo, pair_id, &baseline).await,
            None => {
                self.baselines.write().await.insert(pair_id, baseline);
                Ok(())
            }
        }
    }

    /// Forget a pair's baseline: its next two-way run combines both sides
    /// again and deletes nothing.
    pub async fn reset_baseline(&self, pair_id: Uuid) -> Result<(), AppError> {
        self.baselines.write().await.remove(&pair_id);
        match self.repository() {
            Some(repo) => baseline_db::delete(&repo, pair_id).await,
            None => Ok(()),
        }
    }

    /// Size and age of a pair's baseline, `None` when it has none.
    pub async fn baseline_info(&self, pair_id: Uuid) -> Result<Option<BaselineInfo>, AppError> {
        let pair = self.pairs.read().await.get(&pair_id).cloned().ok_or_else(|| AppError::Sync {
            message: format!("Sync pair {} not found", pair_id),
            advice: "Check the sync pair ID.".to_string(),
        })?;
        Ok(self.load_baseline(&pair).await?.map(|b| BaselineInfo {
            files: b.entries.len() as u64,
            recorded_at: b.recorded_at,
            run_id: b.run_id,
        }))
    }

    /// Get a dry-run preview for a sync pair.
    pub async fn dry_run(&self, pair_id: Uuid) -> Result<SyncPreview, AppError> {
        let pairs = self.pairs.read().await;
        let pair = pairs.get(&pair_id).ok_or_else(|| AppError::Sync {
            message: format!("Sync pair {} not found", pair_id),
            advice: "Check the sync pair ID.".to_string(),
        })?;

        // Run planner on blocking thread
        let pair_clone = self.effective(pair);
        drop(pairs);
        let baseline = self.load_baseline(&pair_clone).await?;
        // Newest first: the preview notes when that run kept the old baseline.
        let last_run_id = self.reports.read().await.get(&pair_id).and_then(|r| r.first()).map(|r| r.id);
        tokio::task::spawn_blocking(move || {
            let ctx = planner::PlanContext { baseline: baseline.as_ref(), last_run_id, ..Default::default() };
            planner::plan(&pair_clone, &ctx).map(|p| p.preview)
        })
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

    fn rollback_root(&self, pair_id: Uuid) -> PathBuf {
        self.state_dir.join("rollback").join(pair_id.to_string())
    }

    fn quarantine_root(&self, pair_id: Uuid) -> PathBuf {
        self.state_dir.join("quarantine").join(pair_id.to_string())
    }

    /// Steps of the pair's current run, written as they finish.
    fn checkpoint_path(&self, pair_id: Uuid) -> PathBuf {
        self.state_dir.join("checkpoints").join(format!("{pair_id}.jsonl"))
    }

    /// Latest progress of a running sync, or `None` when it isn't running.
    pub fn get_progress(&self, pair_id: Uuid) -> Option<executor::SyncProgress> {
        self.progress.lock().ok().and_then(|m| m.get(&pair_id).cloned())
    }

    /// Progress of every running sync.
    /// The files a pair's running sync is working on and has finished (a
    /// bounded list); `None` when the pair is not syncing.
    pub fn live_files(&self, pair_id: Uuid, limit: usize) -> Option<live_files::LiveFiles> {
        self.live.lock().ok()?.get(&pair_id).map(|run| run.snapshot(limit))
    }

    pub fn all_progress(&self) -> Vec<executor::SyncProgress> {
        self.progress.lock().map(|m| m.values().cloned().collect()).unwrap_or_default()
    }

    /// Ask a running sync to stop after the file it is copying.
    /// Returns false when the pair isn't running.
    pub async fn cancel_sync(&self, pair_id: Uuid) -> bool {
        match self.running.read().await.get(&pair_id) {
            Some(flag) => {
                flag.store(true, std::sync::atomic::Ordering::Relaxed);
                true
            }
            None => false,
        }
    }

    /// Execute a sync run for a pair, started by hand.
    pub async fn execute_sync(&self, pair_id: Uuid) -> Result<SyncReport, AppError> {
        self.execute_sync_as(pair_id, RunTrigger::Manual).await
    }

    /// Execute a sync run for a pair; `trigger` is what the run history
    /// says started it. The returned report carries the run's changed items
    /// (store them with [`Self::save_report_to_db`]); the copy kept in
    /// memory for health and reports does not, to keep the history light.
    pub async fn execute_sync_as(&self, pair_id: Uuid, trigger: RunTrigger) -> Result<SyncReport, AppError> {
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

        // One run per pair at a time: two runs over the same folders would
        // race on every file.
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        {
            let mut running = self.running.write().await;
            if running.contains_key(&pair_id) {
                return Err(AppError::Sync {
                    message: format!("\"{}\" is already syncing.", pair.name),
                    advice: "Wait for the current run to finish, or cancel it.".to_string(),
                });
            }
            running.insert(pair_id, cancel.clone());
        }

        let pair = self.effective(&pair);
        let result = self.run_pair(pair.clone(), cancel).await;

        self.running.write().await.remove(&pair_id);
        if let Ok(mut p) = self.progress.lock() {
            p.remove(&pair_id);
        }
        if let Ok(mut l) = self.live.lock() {
            l.remove(&pair_id);
        }
        let mut report = match result {
            Ok(report) => report,
            Err(e) => {
                self.announce(&pair, Err(e.user_message()), 0);
                return Err(e);
            }
        };
        report.trigger = trigger;
        // Everything below passes the report around (history in memory,
        // notifications): only the caller that stores the run needs the items.
        let items = std::mem::take(&mut report.items);

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

        // Run verification if enabled
        let mut verify_mismatches = 0;
        if pair.checksum_enabled || pair.verify_mode != SyncVerifyMode::None {
            let pair_clone = pair.clone();
            let report_clone = report.clone();
            let verified = tokio::task::spawn_blocking(move || {
                executor::verify_sync(&pair_clone, &report_clone)
            })
            .await
            .map_err(|e| AppError::Sync {
                message: format!("Verification task failed: {}", e),
                advice: "The sync completed but verification could not run.".to_string(),
            })
            .and_then(|r| r);
            let mismatches = match verified {
                Ok(m) => m,
                Err(e) => {
                    self.announce(&pair, Ok(report.clone()), 0);
                    return Err(e);
                }
            };

            if !mismatches.is_empty() {
                tracing::warn!(
                    "Sync verification found {} mismatch(es) for pair {}",
                    mismatches.len(),
                    pair_id
                );
            }
            verify_mismatches = mismatches.len();
        }

        self.announce(&pair, Ok(report.clone()), verify_mismatches);
        Ok(SyncReport { items, ..report })
    }

    /// Tell the run listener (notifications) how a run ended.
    fn announce(&self, pair: &SyncPair, outcome: Result<SyncReport, String>, verify_mismatches: usize) {
        let listeners = self.run_listeners.read().unwrap_or_else(|e| e.into_inner());
        if listeners.is_empty() {
            return;
        }
        let event = SyncRunEvent { pair: pair.clone(), outcome, verify_mismatches };
        for listener in listeners.iter() {
            listener(&event);
        }
    }

    /// Run the executor on a blocking thread and file away what it produced.
    async fn run_pair(
        &self,
        pair: SyncPair,
        cancel: Arc<std::sync::atomic::AtomicBool>,
    ) -> Result<SyncReport, AppError> {
        let pair_id = pair.id;
        let run_id = Uuid::new_v4();

        // A checkpoint left by a run that never finished (crash, power
        // loss, cancel): this run skips the steps it had completed. One
        // written for other folders or another mode says nothing about this
        // run: drop it.
        let cp_path = self.checkpoint_path(pair_id);
        let resume_from = checkpoint::load(&cp_path).filter(|c| {
            let ok = c.matches(&pair);
            if !ok {
                checkpoint::clear(&cp_path);
            }
            ok
        });
        let resuming = resume_from.is_some();
        let journal = checkpoint::Journal::open(&cp_path, &pair, run_id, resuming);

        let config = executor::ExecutorConfig {
            pair: pair.clone(),
            run_id,
            continue_on_error: true,
            resume_from,
            quarantine_dir: Some(self.quarantine_root(pair_id)),
            rollback_dir: Some(self.rollback_root(pair_id).join(run_id.to_string())),
            baseline: self.load_baseline(&pair).await?,
        };

        // Rollback is only offered for 7 days; don't keep data past that.
        if let Err(e) = rollback::cleanup_rollback_data(&self.rollback_root(pair_id), 7) {
            tracing::warn!("Rollback cleanup failed for {pair_id}: {e}");
        }

        let progress_map = self.progress.clone();
        let live_map = self.live.clone();
        // A fresh list for this run.
        if let Ok(mut m) = self.live.lock() {
            m.insert(pair_id, live_files::LiveRun::default());
        }
        let hooks = executor::RunHooks {
            progress: Some(Arc::new(move |p: &executor::SyncProgress| {
                if let Ok(mut m) = progress_map.lock() {
                    m.insert(p.pair_id, p.clone());
                }
            })),
            cancel: Some(cancel),
            on_step: journal.as_ref().map(|j| j.recorder()),
            on_file: Some(Arc::new(move |event: &executor::FileEvent| {
                if let Ok(mut m) = live_map.lock() {
                    m.entry(pair_id).or_default().apply(event);
                }
            })),
        };

        let result = tokio::task::spawn_blocking(move || executor::execute_sync_with(&config, &hooks))
            .await
            .map_err(|e| AppError::Sync {
                message: format!("Sync execution task failed: {}", e),
                advice: "Try again.".to_string(),
            })?;
        // The checkpoint outlives only runs that stopped part-way: a
        // cancelled run (resumed next time) or one that never got going
        // while an older checkpoint was waiting.
        let keep = match &result {
            Ok(o) => o.report.status == SyncRunStatus::Cancelled,
            Err(_) => resuming,
        };
        if !keep {
            checkpoint::clear(&self.checkpoint_path(pair_id));
        }
        let outcome = result?;

        // Keep the newest run's snapshots so "Rollback" undoes exactly that
        // run; older runs' snapshot folders can never be restored, so drop them.
        if !outcome.snapshots.is_empty() {
            let root = self.rollback_root(pair_id);
            let keep = run_id.to_string();
            if let Ok(entries) = std::fs::read_dir(&root) {
                for entry in entries.flatten() {
                    if entry.file_name().to_string_lossy() != keep {
                        let _ = std::fs::remove_dir_all(entry.path());
                    }
                }
            }
            self.snapshots
                .write()
                .await
                .insert(pair_id, outcome.snapshots.clone());
        }
        if !outcome.quarantined.is_empty() {
            self.quarantine
                .write()
                .await
                .entry(pair_id)
                .or_default()
                .extend(outcome.quarantined.clone());
        }
        // Two-way: the executor only hands back a baseline when every step
        // ran cleanly. Failing to record it is not a failed sync — the next
        // run compares against the previous baseline and redoes the work.
        if let Some(baseline) = outcome.baseline.clone() {
            if let Err(e) = self.save_baseline(pair_id, baseline).await {
                tracing::warn!("Could not record the two-way baseline of {pair_id}: {e}");
            }
        }
        if !outcome.pending_conflicts.is_empty() {
            let mut conflicts = self.conflicts.write().await;
            for item in &outcome.pending_conflicts {
                // Don't queue the same unresolved file twice.
                let exists = conflicts.iter().any(|c| {
                    c.pair_id == item.pair_id
                        && c.relative_path == item.relative_path
                        && c.resolution.is_none()
                });
                if !exists {
                    conflicts.push(item.clone());
                }
            }
        }
        Ok(outcome.report)
    }

    /// Rollback the last sync run for a pair.
    ///
    /// Never while the pair is syncing: the restore would race the run's
    /// writes, and the run would then save a two-way baseline recording the
    /// restored paths as in sync again. The rollback holds the pair's run
    /// slot itself, so no run starts until it is done.
    pub async fn rollback_last_run(&self, pair_id: Uuid) -> Result<u64, AppError> {
        {
            let mut running = self.running.write().await;
            if running.contains_key(&pair_id) {
                return Err(AppError::Sync {
                    message: "This pair is syncing; its last run cannot be rolled back now.".to_string(),
                    advice: "Wait for the run to finish, or cancel it, then roll back.".to_string(),
                });
            }
            running.insert(pair_id, Arc::new(std::sync::atomic::AtomicBool::new(false)));
        }
        let result = self.rollback_last_run_unlocked(pair_id).await;
        self.running.write().await.remove(&pair_id);
        result
    }

    async fn rollback_last_run_unlocked(&self, pair_id: Uuid) -> Result<u64, AppError> {
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

        let pair = self.pairs.read().await.get(&pair_id).cloned().ok_or_else(|| AppError::Sync {
            message: format!("Sync pair {} not found", pair_id),
            advice: "Check the sync pair ID.".to_string(),
        })?;
        let (source_path, dest_path) = (PathBuf::from(&pair.source_path), PathBuf::from(&pair.dest_path));

        let snapshots_clone = snapshots.clone();
        let restored = tokio::task::spawn_blocking(move || {
            rollback::rollback_pair_run(&snapshots_clone, &source_path, &dest_path)
        })
        .await
        .map_err(|e| AppError::Sync {
            message: format!("Rollback task failed: {}", e),
            advice: "Try again.".to_string(),
        })?;

        // Two-way: the baseline the rolled-back run saved says the restored
        // files are in sync with the other side, which they no longer are.
        // Forget their history (also when only some could be restored): the
        // next run sees two different files and handles them as a conflict
        // under the pair's policy, instead of copying the restored file over
        // the other side's newer version.
        self.forget_baseline_paths(&pair, snapshots.iter().map(|s| s.relative_path.as_str())).await?;
        let restored = restored?;

        // Clear snapshots after rollback
        {
            let mut snap_map = self.snapshots.write().await;
            snap_map.remove(&pair_id);
        }

        Ok(restored)
    }

    /// Drop `paths` from `pair`'s two-way baseline. When the shortened
    /// baseline cannot be saved, the whole baseline is reset instead (the
    /// next run then combines both sides and deletes nothing).
    async fn forget_baseline_paths<'a>(&self, pair: &SyncPair, paths: impl Iterator<Item = &'a str>) -> Result<(), AppError> {
        let Some(mut baseline) = self.load_baseline(pair).await? else {
            return Ok(());
        };
        let before = baseline.entries.len();
        for path in paths {
            baseline.entries.remove(path);
        }
        if baseline.entries.len() == before {
            return Ok(());
        }
        if let Err(e) = self.save_baseline(pair.id, baseline).await {
            tracing::warn!("Could not update the two-way baseline of {} after a rollback: {e}", pair.id);
            self.reset_baseline(pair.id).await.map_err(|_| AppError::Sync {
                message: "The files were restored, but the pair's two-way sync history could not be updated.".to_string(),
                advice: "Reset the pair's sync history before its next run, so the restored files are not copied over newer versions.".to_string(),
            })?;
        }
        Ok(())
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
    /// Start the cron loop that runs scheduled pairs through `run`. Without
    /// this, `Scheduled` triggers are armed but never fire.
    pub fn start_scheduler(&self, run: scheduler::ScheduleCallback) {
        self.scheduler.start(run);
    }

    /// Whether the cron loop is running.
    /// Next scheduled run of a pair, if its schedule is armed.
    pub fn next_scheduled_run(&self, pair_id: Uuid) -> Option<DateTime<Utc>> {
        self.scheduler.next_run_time(pair_id)
    }

    pub fn scheduler_running(&self) -> bool {
        self.scheduler.is_running()
    }

    #[cfg(test)]
    fn start_scheduler_every(&self, run: scheduler::ScheduleCallback, interval: std::time::Duration) {
        self.scheduler.start_with_interval(run, interval);
    }

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

    /// Resolve a conflict manually: apply the chosen policy to the file now.
    pub async fn resolve_conflict_item(
        &self,
        conflict_id: Uuid,
        resolution: SyncConflictPolicy,
    ) -> Result<(), AppError> {
        let item = {
            let conflicts = self.conflicts.read().await;
            conflicts
                .iter()
                .find(|c| c.id == conflict_id && c.resolution.is_none())
                .cloned()
                .ok_or_else(|| AppError::Sync {
                    message: "Conflict item not found.".to_string(),
                    advice: "The conflict may have already been resolved.".to_string(),
                })?
        };
        let pair = self
            .pairs
            .read()
            .await
            .get(&item.pair_id)
            .cloned()
            .ok_or_else(|| AppError::Sync {
                message: "The sync pair for this conflict no longer exists.".to_string(),
                advice: "Dismiss the conflict; there is nothing left to sync.".to_string(),
            })?;

        let quarantine_dir = self.quarantine_root(pair.id);
        let rollback_dir = self.rollback_root(pair.id);
        let item_clone = item.clone();
        let (snapshots, quarantined) = tokio::task::spawn_blocking(move || {
            executor::apply_manual_resolution(
                &pair,
                &item_clone,
                resolution,
                &quarantine_dir,
                &rollback_dir,
            )
        })
        .await
        .map_err(|e| AppError::Sync {
            message: format!("Conflict resolution task failed: {}", e),
            advice: "Try again.".to_string(),
        })??;

        if !snapshots.is_empty() {
            self.snapshots.write().await.insert(item.pair_id, snapshots);
        }
        if !quarantined.is_empty() {
            self.quarantine
                .write()
                .await
                .entry(item.pair_id)
                .or_default()
                .extend(quarantined);
        }
        let mut conflicts = self.conflicts.write().await;
        if let Some(c) = conflicts.iter_mut().find(|c| c.id == conflict_id) {
            c.resolution = Some(resolution);
        }
        Ok(())
    }

    /// Update sync pair configuration. Changing the mode or either folder
    /// forgets the two-way baseline: it describes other files. Changing
    /// what a run copies (folders, mode, filter, options) also drops a
    /// pending crash-recovery checkpoint: its steps were planned for the old
    /// definition.
    pub async fn update_pair(&self, pair: SyncPair) -> Result<SyncPair, AppError> {
        // Rejected before anything changes: re-arming below must not fail
        // after the pair was replaced.
        if let SyncTrigger::Scheduled { cron_expr } = &pair.trigger {
            if pair.enabled {
                scheduler::SyncScheduler::validate_cron(cron_expr)?;
            }
        }
        let not_found = || AppError::Sync {
            message: "Sync pair not found.".to_string(),
            advice: "Create the sync pair first.".to_string(),
        };
        let (reset, rearm, stale_checkpoint) = {
            let pairs = self.pairs.read().await;
            let old = pairs.get(&pair.id).ok_or_else(not_found)?;
            let reset = old.mode != pair.mode || old.source_path != pair.source_path || old.dest_path != pair.dest_path;
            let rearm = old.trigger != pair.trigger || old.enabled != pair.enabled || old.source_path != pair.source_path;
            let stale_checkpoint = reset
                || old.dest_state != pair.dest_state
                || old.copy_options != pair.copy_options
                || old.option_overrides != pair.option_overrides
                || old.run_options != pair.run_options
                || serde_json::to_value(&old.filter).ok() != serde_json::to_value(&pair.filter).ok();
            (reset, rearm, stale_checkpoint)
        };
        // The fallible baseline reset comes before the pair is replaced, so
        // a failure leaves the old pair in place everywhere.
        if reset {
            self.reset_baseline(pair.id).await?;
        }
        match self.pairs.write().await.get_mut(&pair.id) {
            Some(slot) => *slot = pair.clone(),
            None => return Err(not_found()),
        }
        if stale_checkpoint {
            checkpoint::clear(&self.checkpoint_path(pair.id));
        }
        // An edited trigger takes effect now, not after a restart.
        if rearm {
            let _ = self.stop_watcher(pair.id).await;
            let _ = self.scheduler.remove_schedule(pair.id);
            if pair.enabled {
                match &pair.trigger {
                    // As in `create_pair`: the edit stands (the command saves
                    // it) and the watcher is retried at the next start.
                    SyncTrigger::Watch => {
                        if let Err(e) = self.start_watcher(pair.id).await {
                            tracing::warn!("Failed to start watcher for {}: {}", pair.id, e);
                        }
                    }
                    SyncTrigger::Scheduled { cron_expr } => {
                        self.scheduler.add_schedule(pair.id, cron_expr)?;
                    }
                    _ => {}
                }
            }
        }
        Ok(pair)
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

/// One page of a stored run's changed items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunItemsPage {
    pub items: Vec<SyncRunItem>,
    /// Items stored for the run (at most [`ufop_core::sync_types::MAX_RUN_ITEMS`]).
    pub total: u64,
    /// The run changed more files than were kept.
    pub truncated: bool,
}

// ── Persistence ──────────────────────────────────────────────────────────
//
// Sync pairs were pure in-memory state: `create_pair` inserted into a HashMap
// and nothing ever wrote to SQLite, so every pair a user configured vanished
// on the next launch — silently, with no error and no export
// (`export_all_settings` reads `list_pairs()`, so it exported nothing too).
//
// The `sync_pairs` table has existed since schema v1 and was deliberately
// evolved twice since (v4 added trigger_mode / cron_expr / filter_json /
// conflict_policy / verify_mode / checksum_enabled / created_at; v8 added
// time_offset_secs) — its columns map 1:1 onto `SyncPair`, and every enum
// involved already carries the `as_str()` / `from_str_lossy()` pair this
// needs. Everything was in place except the read and the write.
//
// Mirrors `AutomationManager::{save_rule, delete_rule, load_rules_from_db}`:
// the manager holds no pool, the caller passes the `Repository` in. That keeps
// `SyncManager::new()` unchanged, so every existing caller and test still
// compiles, and persistence stays opt-in per call site.
impl SyncManager {
    /// Insert-or-update one pair. Called by the sync commands after any
    /// mutation so the DB is the source of truth across restarts.
    /// Record a finished run in the run history and trim each pair's history
    /// to the newest `keep` runs (0 = keep all).
    pub async fn save_report_to_db(&self, repo: &crate::storage::Repository, report: &SyncReport, keep: u32) -> Result<(), AppError> {
        let r = report.clone();
        let json = serde_json::to_string(&r)?;
        repo.pool()
            .execute(move |conn| {
                let status = serde_json::to_value(r.status).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
                let tx = conn.unchecked_transaction()?;
                // Replacing a run drops its items (ON DELETE CASCADE); they are written again below.
                tx.execute(
                    "INSERT OR REPLACE INTO sync_runs (id, pair_id, pair_name, status, started_at, ended_at, report_json, run_trigger, items_truncated) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    rusqlite::params![r.id.to_string(), r.pair_id.to_string(), r.pair_name, status, r.started_at.to_rfc3339(), r.ended_at.to_rfc3339(), json, r.trigger.as_str(), r.items_truncated],
                )?;
                {
                    let mut ins = tx.prepare("INSERT INTO sync_run_items (run_id, rel_path, action, bytes, error) VALUES (?1, ?2, ?3, ?4, ?5)")?;
                    for item in r.items.iter().take(ufop_core::sync_types::MAX_RUN_ITEMS) {
                        ins.execute(rusqlite::params![r.id.to_string(), item.rel_path, item.action.as_str(), item.bytes as i64, item.error])?;
                    }
                }
                if keep > 0 {
                    tx.execute(
                        "DELETE FROM sync_runs WHERE pair_id = ?1 AND id NOT IN (SELECT id FROM sync_runs WHERE pair_id = ?1 ORDER BY started_at DESC LIMIT ?2)",
                        rusqlite::params![r.pair_id.to_string(), keep],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// Stored runs, newest first, optionally for one pair.
    pub async fn run_history(&self, repo: &crate::storage::Repository, pair_id: Option<Uuid>, limit: u32) -> Result<Vec<SyncReport>, AppError> {
        self.run_history_between(repo, pair_id, limit, None, None).await
    }

    /// [`Self::run_history`] limited to runs that started in `[from, to]`.
    pub async fn run_history_between(
        &self,
        repo: &crate::storage::Repository,
        pair_id: Option<Uuid>,
        limit: u32,
        from: Option<DateTime<Utc>>,
        to: Option<DateTime<Utc>>,
    ) -> Result<Vec<SyncReport>, AppError> {
        repo.pool()
            .execute(move |conn| {
                let mut st = conn.prepare(
                    "SELECT report_json, run_trigger, items_truncated FROM sync_runs
                     WHERE (?1 IS NULL OR pair_id = ?1) AND (?3 IS NULL OR started_at >= ?3) AND (?4 IS NULL OR started_at <= ?4)
                     ORDER BY started_at DESC LIMIT ?2",
                )?;
                let rows = st.query_map(
                    rusqlite::params![pair_id.map(|p| p.to_string()), limit.clamp(1, 10_000), from.map(|d| d.to_rfc3339()), to.map(|d| d.to_rfc3339())],
                    |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, bool>(2)?)),
                )?;
                // The columns are the source of truth for what the report JSON
                // did not know when an older version stored it.
                Ok(rows
                    .filter_map(|r| r.ok())
                    .filter_map(|(j, trig, trunc)| {
                        serde_json::from_str::<SyncReport>(&j).ok().map(|rep| SyncReport { trigger: RunTrigger::from_str_lossy(&trig), items_truncated: trunc, ..rep })
                    })
                    .collect())
            })
            .await
    }

    /// The files a stored run changed, in the order they were handled, one
    /// page at a time. `None` when no such run is stored.
    pub async fn run_items(&self, repo: &crate::storage::Repository, run_id: Uuid, offset: u32, limit: u32) -> Result<Option<RunItemsPage>, AppError> {
        repo.pool()
            .execute(move |conn| {
                use rusqlite::OptionalExtension;
                let id = run_id.to_string();
                let truncated: Option<bool> = conn.query_row("SELECT items_truncated FROM sync_runs WHERE id = ?1", [&id], |r| r.get(0)).optional()?;
                let Some(truncated) = truncated else { return Ok(None) };
                let total: i64 = conn.query_row("SELECT COUNT(*) FROM sync_run_items WHERE run_id = ?1", [&id], |r| r.get(0))?;
                let mut st = conn.prepare("SELECT rel_path, action, bytes, error FROM sync_run_items WHERE run_id = ?1 ORDER BY rowid LIMIT ?2 OFFSET ?3")?;
                let items = st
                    .query_map(rusqlite::params![id, limit.clamp(1, 5_000), offset], |r| {
                        Ok(SyncRunItem {
                            rel_path: r.get(0)?,
                            action: SyncItemAction::from_str_lossy(&r.get::<_, String>(1)?),
                            bytes: r.get::<_, i64>(2)?.max(0) as u64,
                            error: r.get(3)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(Some(RunItemsPage { items, total: total.max(0) as u64, truncated }))
            })
            .await
    }

    /// One stored run by id.
    pub async fn stored_report(&self, repo: &crate::storage::Repository, report_id: Uuid) -> Result<Option<SyncReport>, AppError> {
        repo.pool()
            .execute(move |conn| {
                use rusqlite::OptionalExtension;
                let j: Option<String> = conn.query_row("SELECT report_json FROM sync_runs WHERE id = ?1", [report_id.to_string()], |r| r.get(0)).optional()?;
                Ok(j.and_then(|j| serde_json::from_str(&j).ok()))
            })
            .await
    }

    /// Delete stored runs older than `days`; returns how many were removed.
    pub async fn prune_run_history(&self, repo: &crate::storage::Repository, days: u32) -> Result<usize, AppError> {
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        repo.pool().execute(move |conn| Ok(conn.execute("DELETE FROM sync_runs WHERE started_at < ?1", [cutoff])?)).await
    }

    /// Load the newest stored runs into memory (health and report views).
    pub async fn load_reports_from_db(&self, repo: &crate::storage::Repository, per_pair: usize) -> Result<usize, AppError> {
        let all = self.run_history(repo, None, 10_000).await?; // newest first
        let mut map = self.reports.write().await;
        let mut n = 0;
        for r in all {
            let v = map.entry(r.pair_id).or_default();
            if v.len() >= per_pair || v.iter().any(|x| x.id == r.id) {
                continue;
            }
            v.push(r);
            n += 1;
        }
        Ok(n)
    }

    pub async fn save_pair_to_db(
        &self,
        repo: &crate::storage::Repository,
        pair: &SyncPair,
    ) -> Result<(), AppError> {
        let p = pair.clone();
        repo.pool()
            .execute(move |conn| {
                let filter_json = serde_json::to_string(&p.filter).unwrap_or_else(|_| "{}".into());
                let options_json =
                    serde_json::to_string(&p.copy_options).unwrap_or_else(|_| "{}".into());
                // `trigger_mode` stores the discriminant and `cron_expr` the
                // payload, matching the two columns v4 created for exactly this.
                let cron_expr = match &p.trigger {
                    SyncTrigger::Scheduled { cron_expr } => Some(cron_expr.clone()),
                    _ => None,
                };
                conn.execute(
                    "INSERT INTO sync_pairs (id, name, source_path, dest_path, mode, enabled, \
                     last_run, trigger_mode, cron_expr, filter_json, conflict_policy, \
                     verify_mode, checksum_enabled, created_at, time_offset_secs, options_json, \
                     dest_state, checksum_algorithm, run_options_json, option_overrides_json) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20) \
                     ON CONFLICT(id) DO UPDATE SET \
                     name = excluded.name, source_path = excluded.source_path, \
                     dest_path = excluded.dest_path, mode = excluded.mode, \
                     enabled = excluded.enabled, last_run = excluded.last_run, \
                     trigger_mode = excluded.trigger_mode, cron_expr = excluded.cron_expr, \
                     filter_json = excluded.filter_json, \
                     conflict_policy = excluded.conflict_policy, \
                     verify_mode = excluded.verify_mode, \
                     checksum_enabled = excluded.checksum_enabled, \
                     time_offset_secs = excluded.time_offset_secs, \
                     options_json = excluded.options_json, \
                     dest_state = excluded.dest_state, \
                     checksum_algorithm = excluded.checksum_algorithm, \
                     run_options_json = excluded.run_options_json, \
                     option_overrides_json = excluded.option_overrides_json",
                    rusqlite::params![
                        p.id.to_string(),
                        p.name,
                        p.source_path,
                        p.dest_path,
                        p.mode.as_str(),
                        p.enabled as i32,
                        p.last_run.map(|t| t.to_rfc3339()),
                        p.trigger.as_str(),
                        cron_expr,
                        filter_json,
                        p.conflict_policy.as_str(),
                        p.verify_mode.as_str(),
                        p.checksum_enabled as i32,
                        p.created_at.to_rfc3339(),
                        p.time_offset_secs,
                        options_json,
                        serde_json::to_value(p.dest_state).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "scan".into()),
                        serde_json::to_value(p.checksum_algorithm).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "xxh3".into()),
                        serde_json::to_string(&p.run_options).unwrap_or_else(|_| "{}".into()),
                        p.option_overrides.as_ref().and_then(|o| serde_json::to_string(o).ok()),
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Remove one pair's row. The `ON DELETE CASCADE` on `sync_state` /
    /// `sync_reports` clears their history with it.
    pub async fn delete_pair_from_db(
        &self,
        repo: &crate::storage::Repository,
        pair_id: Uuid,
    ) -> Result<(), AppError> {
        let id = pair_id.to_string();
        repo.pool()
            .execute(move |conn| {
                conn.execute("DELETE FROM sync_pairs WHERE id = ?1", rusqlite::params![id])?;
                Ok(())
            })
            .await?;
        baseline_db::delete(repo, pair_id).await
    }

    /// Restore every saved pair into memory at startup and re-arm its trigger,
    /// so a watch/scheduled pair is as live after a restart as it was when the
    /// user created it. Returns how many were loaded.
    pub async fn load_pairs_from_db(
        &self,
        repo: &crate::storage::Repository,
    ) -> Result<usize, AppError> {
        let pairs: Vec<SyncPair> = repo
            .pool()
            .execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, source_path, dest_path, mode, enabled, last_run, \
                     trigger_mode, cron_expr, filter_json, conflict_policy, verify_mode, \
                     checksum_enabled, created_at, time_offset_secs, options_json, \
                     dest_state, checksum_algorithm, run_options_json, option_overrides_json \
                     FROM sync_pairs ORDER BY created_at ASC",
                )?;
                let rows = stmt.query_map([], |row| {
                    let id: String = row.get(0)?;
                    let mode: String = row.get(4)?;
                    let last_run: Option<String> = row.get(6)?;
                    let trigger_mode: String = row.get(7)?;
                    let cron_expr: Option<String> = row.get(8)?;
                    let filter_json: String = row.get(9)?;
                    let conflict_policy: String = row.get(10)?;
                    let verify_mode: String = row.get(11)?;
                    let created_at: String = row.get(13)?;
                    let options_json: String = row.get(15)?;
                    let dest_state: String = row.get(16)?;
                    let checksum_algorithm: String = row.get(17)?;
                    let run_options_json: String = row.get(18)?;
                    let option_overrides_json: Option<String> = row.get(19)?;

                    Ok(SyncPair {
                        // A row we cannot parse is a row we cannot honour, but
                        // one bad pair must not cost the user all the others —
                        // fall back per-field rather than dropping the row.
                        id: Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::new_v4()),
                        name: row.get(1)?,
                        source_path: row.get(2)?,
                        dest_path: row.get(3)?,
                        mode: SyncMode::from_str_lossy(&mode),
                        enabled: row.get::<_, i32>(5)? != 0,
                        last_run: last_run
                            .and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
                            .map(|t| t.with_timezone(&Utc)),
                        trigger: match trigger_mode.as_str() {
                            "scheduled" => SyncTrigger::Scheduled {
                                cron_expr: cron_expr.unwrap_or_else(|| "0 * * * *".to_string()),
                            },
                            other => SyncTrigger::from_str_lossy(other),
                        },
                        filter: serde_json::from_str(&filter_json).unwrap_or_default(),
                        conflict_policy: SyncConflictPolicy::from_str_lossy(&conflict_policy),
                        verify_mode: SyncVerifyMode::from_str_lossy(&verify_mode),
                        checksum_enabled: row.get::<_, i32>(12)? != 0,
                        created_at: DateTime::parse_from_rfc3339(&created_at)
                            .map(|t| t.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                        time_offset_secs: row.get(14)?,
                        copy_options: serde_json::from_str(&options_json).unwrap_or_default(),
                        dest_state: serde_json::from_value(serde_json::Value::String(dest_state)).unwrap_or_default(),
                        checksum_algorithm: serde_json::from_value(serde_json::Value::String(checksum_algorithm)).unwrap_or_default(),
                        run_options: serde_json::from_str(&run_options_json).unwrap_or_default(),
                        option_overrides: option_overrides_json.and_then(|j| serde_json::from_str(&j).ok()),
                    })
                })?;
                let mut out = Vec::new();
                for r in rows {
                    out.push(r?);
                }
                Ok(out)
            })
            .await?;

        let count = pairs.len();
        {
            let mut map = self.pairs.write().await;
            for pair in &pairs {
                map.insert(pair.id, pair.clone());
            }
        }

        // Re-arm triggers outside the lock, exactly as `create_pair` does.
        for pair in &pairs {
            if !pair.enabled {
                continue;
            }
            match &pair.trigger {
                SyncTrigger::Watch => {
                    if let Err(e) = self.start_watcher(pair.id).await {
                        tracing::warn!("Failed to restore watcher for {}: {}", pair.id, e);
                    }
                }
                SyncTrigger::Scheduled { cron_expr } => {
                    if let Err(e) = self.scheduler.add_schedule(pair.id, cron_expr) {
                        tracing::warn!("Failed to restore schedule for {}: {}", pair.id, e);
                    }
                }
                _ => {}
            }
        }

        tracing::info!("Loaded {} sync pair(s) from database", count);
        Ok(count)
    }
}

/// Crash-recovery checkpoints: one JSON-lines file per pair in the state
/// dir. The first line names the run; every finished step appends a line
/// (written straight through, so a crash loses at most the step in flight).
/// The file is removed when a run ends normally, so one that exists at the
/// start of a run belongs to a run that never finished.
mod checkpoint {
    use super::*;
    use std::io::Write;

    #[derive(Serialize, Deserialize)]
    struct Header {
        pair_id: Uuid,
        run_id: Uuid,
        started_at: DateTime<Utc>,
        /// The folders and mode the run worked on; a checkpoint without them
        /// matches no pair.
        #[serde(default)]
        source_root: String,
        #[serde(default)]
        dest_root: String,
        #[serde(default)]
        mode: Option<SyncMode>,
    }

    #[derive(Serialize, Deserialize)]
    struct Line {
        path: String,
        step: CompletedStep,
    }

    /// The checkpoint at `path`, if a run left one. A torn last line (the
    /// crash happened mid-write) is ignored.
    pub fn load(path: &std::path::Path) -> Option<SyncCheckpoint> {
        let text = std::fs::read_to_string(path).ok()?;
        let mut lines = text.lines();
        let header: Header = serde_json::from_str(lines.next()?).ok()?;
        let mut cp = SyncCheckpoint {
            pair_id: header.pair_id,
            run_id: header.run_id,
            started_at: Some(header.started_at),
            source_root: header.source_root,
            dest_root: header.dest_root,
            mode: header.mode,
            completed: Default::default(),
        };
        for l in lines {
            if let Ok(line) = serde_json::from_str::<Line>(l) {
                cp.completed.insert(line.path, line.step);
            }
        }
        Some(cp)
    }

    pub fn clear(path: &std::path::Path) {
        if let Err(e) = std::fs::remove_file(path) {
            if e.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!("Could not remove sync checkpoint {}: {e}", path.display());
            }
        }
    }

    fn ends_without_newline(f: &mut std::fs::File) -> std::io::Result<bool> {
        use std::io::{Read, Seek, SeekFrom};
        if f.metadata()?.len() == 0 {
            return Ok(false);
        }
        f.seek(SeekFrom::End(-1))?;
        let mut last = [0u8; 1];
        f.read_exact(&mut last)?;
        Ok(last[0] != b'\n')
    }

    /// The open checkpoint of a run.
    pub struct Journal(Arc<std::sync::Mutex<std::fs::File>>);

    impl Journal {
        /// Start (or, when resuming, continue) the checkpoint at `path`.
        /// `None` when it cannot be written: the run goes ahead without one.
        pub fn open(path: &std::path::Path, pair: &SyncPair, run_id: Uuid, resume: bool) -> Option<Self> {
            let open = || -> std::io::Result<std::fs::File> {
                if let Some(dir) = path.parent() {
                    std::fs::create_dir_all(dir)?;
                }
                if resume {
                    let mut f = std::fs::OpenOptions::new().read(true).append(true).open(path)?;
                    // A crash mid-write leaves a torn last line; end it so the
                    // next step is not glued onto it (`load` skips the fragment).
                    if ends_without_newline(&mut f)? {
                        writeln!(f)?;
                    }
                    return Ok(f);
                }
                let mut f = std::fs::File::create(path)?;
                let header = Header {
                    pair_id: pair.id,
                    run_id,
                    started_at: Utc::now(),
                    source_root: pair.source_path.clone(),
                    dest_root: pair.dest_path.clone(),
                    mode: Some(pair.mode),
                };
                writeln!(f, "{}", serde_json::to_string(&header).map_err(std::io::Error::other)?)?;
                Ok(f)
            };
            match open() {
                Ok(f) => Some(Self(Arc::new(std::sync::Mutex::new(f)))),
                Err(e) => {
                    tracing::warn!("Could not start a sync checkpoint at {}: {e}", path.display());
                    None
                }
            }
        }

        /// The executor's finished-step callback.
        pub fn recorder(&self) -> executor::StepFn {
            let file = self.0.clone();
            Arc::new(move |path: &str, step: &CompletedStep| {
                let Ok(line) = serde_json::to_string(&Line { path: path.to_string(), step: step.clone() }) else { return };
                if let Ok(mut f) = file.lock() {
                    let _ = writeln!(f, "{line}");
                }
            })
        }
    }
}

/// Two-way baselines in SQLite (schema v33): a header row per pair and one
/// row per file. Saving replaces the whole baseline in one transaction, so
/// a crash mid-save leaves the previous one intact.
mod baseline_db {
    use super::*;
    use std::collections::BTreeMap;

    fn ns(t: Option<DateTime<Utc>>) -> Option<i64> {
        t.and_then(|t| t.timestamp_nanos_opt())
    }

    fn from_ns(n: Option<i64>) -> Option<DateTime<Utc>> {
        n.map(DateTime::<Utc>::from_timestamp_nanos)
    }

    pub async fn load(repo: &crate::storage::Repository, pair_id: Uuid) -> Result<Option<SyncBaseline>, AppError> {
        let id = pair_id.to_string();
        repo.pool()
            .execute(move |conn| {
                use rusqlite::OptionalExtension;
                let head: Option<(Option<String>, String, String, String)> = conn
                    .query_row(
                        "SELECT run_id, source_root, dest_root, recorded_at FROM sync_baseline_runs WHERE pair_id = ?1",
                        [&id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                    )
                    .optional()?;
                let Some((run_id, source_root, dest_root, recorded_at)) = head else {
                    return Ok(None);
                };
                let mut st = conn.prepare("SELECT rel_path, size, source_mtime_ns, dest_mtime_ns, hash FROM sync_baseline WHERE pair_id = ?1")?;
                let rows = st.query_map([&id], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        BaselineEntry {
                            size: r.get::<_, i64>(1)?.max(0) as u64,
                            source_modified: from_ns(r.get(2)?),
                            dest_modified: from_ns(r.get(3)?),
                            hash: r.get(4)?,
                        },
                    ))
                })?;
                let mut entries = BTreeMap::new();
                for row in rows {
                    let (path, entry) = row?;
                    entries.insert(path, entry);
                }
                Ok(Some(SyncBaseline {
                    source_root,
                    dest_root,
                    run_id: run_id.and_then(|r| Uuid::parse_str(&r).ok()),
                    recorded_at: DateTime::parse_from_rfc3339(&recorded_at).ok().map(|t| t.with_timezone(&Utc)),
                    entries,
                }))
            })
            .await
    }

    pub async fn save(repo: &crate::storage::Repository, pair_id: Uuid, b: &SyncBaseline) -> Result<(), AppError> {
        let id = pair_id.to_string();
        let b = b.clone();
        repo.pool()
            .execute(move |conn| {
                let tx = conn.unchecked_transaction()?;
                tx.execute("DELETE FROM sync_baseline_runs WHERE pair_id = ?1", [&id])?;
                tx.execute(
                    "INSERT INTO sync_baseline_runs (pair_id, run_id, source_root, dest_root, recorded_at) VALUES (?1, ?2, ?3, ?4, ?5)",
                    rusqlite::params![id, b.run_id.map(|r| r.to_string()), b.source_root, b.dest_root, b.recorded_at.unwrap_or_else(Utc::now).to_rfc3339()],
                )?;
                {
                    let mut st = tx.prepare(
                        "INSERT INTO sync_baseline (pair_id, rel_path, size, source_mtime_ns, dest_mtime_ns, hash) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    )?;
                    for (path, e) in &b.entries {
                        st.execute(rusqlite::params![id, path, e.size as i64, ns(e.source_modified), ns(e.dest_modified), e.hash])?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }

    pub async fn delete(repo: &crate::storage::Repository, pair_id: Uuid) -> Result<(), AppError> {
        let id = pair_id.to_string();
        repo.pool()
            .execute(move |conn| {
                conn.execute("DELETE FROM sync_baseline_runs WHERE pair_id = ?1", [&id])?;
                Ok(())
            })
            .await
    }
}

impl SyncOperations for SyncManager {
    fn create_pair(&self, pair: SyncPair) -> BoxFuture<'_, Result<SyncPair, AppError>> {
        Box::pin(async move {
            let mut map = self.pairs.write().await;
            map.insert(pair.id, pair.clone());
            tracing::info!("Sync pair created: {} ({})", pair.name, pair.id);

            // Set up trigger if needed. A disabled pair (a duplicate, an
            // import missing its folders) is not armed: its runs would
            // only be refused.
            match &pair.trigger {
                _ if !pair.enabled => {}
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
            self.reset_baseline(pair_id).await?;
            checkpoint::clear(&self.checkpoint_path(pair_id));
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

    /// The regression this guards: sync pairs lived only in a HashMap, so every
    /// pair a user configured was gone on the next launch — silently, while the
    /// `sync_pairs` table sat empty beside a schema built and twice-migrated for
    /// exactly this data.
    ///
    /// Round-trips through a real SQLite DB and asserts every column, so it also
    /// pins the SyncPair↔schema mapping rather than just "a row came back".
    #[tokio::test]
    async fn sync_run_history_survives_a_restart_and_is_trimmed() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "alpha").unwrap();
        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();
        let mut ids = Vec::new();
        for i in 0..3 {
            std::fs::write(src.path().join(format!("f{i}.txt")), "x").unwrap();
            let r = mgr.execute_sync(pair.id).await.unwrap();
            mgr.save_report_to_db(&repo, &r, 2).await.unwrap();
            ids.push(r.id);
        }
        let hist = mgr.run_history(&repo, Some(pair.id), 10).await.unwrap();
        assert_eq!(hist.iter().map(|r| r.id).collect::<Vec<_>>(), vec![ids[2], ids[1]], "newest first, trimmed to 2");
        assert!(mgr.run_history(&repo, Some(Uuid::new_v4()), 10).await.unwrap().is_empty());
        assert_eq!(mgr.run_history(&repo, None, 1).await.unwrap().len(), 1);

        let restarted = SyncManager::new();
        assert_eq!(restarted.load_reports_from_db(&repo, 50).await.unwrap(), 2);
        assert_eq!(restarted.stored_report(&repo, ids[1]).await.unwrap().unwrap().id, ids[1]);
        assert!(restarted.stored_report(&repo, ids[0]).await.unwrap().is_none(), "trimmed");
        let mem = restarted.get_reports(pair.id).await;
        assert_eq!(mem[0].id, ids[2], "in-memory order matches live runs (newest first)");
        assert_eq!(mem[0].files_added, hist[0].files_added);

        assert_eq!(mgr.prune_run_history(&repo, 0).await.unwrap(), 2);
        assert!(mgr.run_history(&repo, None, 10).await.unwrap().is_empty());
    }

    /// Row 73: a run says what started it and what it did to each file,
    /// and both survive the history round trip.
    #[tokio::test]
    async fn run_history_keeps_the_trigger_and_the_changed_items() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "alpha").unwrap();
        std::fs::write(src.path().join("b.txt"), "bravo!").unwrap();
        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();

        let manual = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(manual.trigger, RunTrigger::Manual);
        assert_eq!(manual.items.len(), 2, "the caller gets the items to store");
        mgr.save_report_to_db(&repo, &manual, 0).await.unwrap();

        std::fs::write(src.path().join("c.txt"), "charlie").unwrap();
        let scheduled = mgr.execute_sync_as(pair.id, RunTrigger::Scheduled).await.unwrap();
        mgr.save_report_to_db(&repo, &scheduled, 0).await.unwrap();

        let hist = mgr.run_history(&repo, Some(pair.id), 10).await.unwrap();
        assert_eq!(hist.iter().map(|r| r.trigger).collect::<Vec<_>>(), vec![RunTrigger::Scheduled, RunTrigger::Manual]);
        assert!(hist.iter().all(|r| r.items.is_empty()), "history rows are light");
        assert!(mgr.get_reports(pair.id).await.iter().all(|r| r.items.is_empty()), "so is the in-memory copy");

        let page = mgr.run_items(&repo, manual.id, 0, 100).await.unwrap().unwrap();
        let mut got: Vec<(String, &str, u64)> = page.items.iter().map(|i| (i.rel_path.clone(), i.action.as_str(), i.bytes)).collect();
        got.sort();
        assert_eq!(got, vec![("a.txt".into(), "copied", 5), ("b.txt".into(), "copied", 6)]);
        assert_eq!((page.total, page.truncated), (2, false));
        let second = mgr.run_items(&repo, scheduled.id, 0, 100).await.unwrap().unwrap();
        assert_eq!(second.items.len(), 1);
        assert_eq!(second.items[0].rel_path, "c.txt");
        let paged = mgr.run_items(&repo, manual.id, 1, 1).await.unwrap().unwrap();
        assert_eq!((paged.items.len(), paged.total), (1, 2));
        assert!(mgr.run_items(&repo, Uuid::new_v4(), 0, 10).await.unwrap().is_none());

        // Trimming a run takes its items with it.
        mgr.save_report_to_db(&repo, &scheduled, 1).await.unwrap();
        assert!(mgr.run_items(&repo, manual.id, 0, 10).await.unwrap().is_none());
        let left: i64 = repo.pool().execute(|c| Ok(c.query_row("SELECT COUNT(*) FROM sync_run_items", [], |r| r.get(0))?)).await.unwrap();
        assert_eq!(left, 1);
    }

    #[tokio::test]
    async fn run_history_can_be_limited_to_a_date_range() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = SyncManager::new();
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        let pair = make_test_pair(src.path(), dst.path());
        mgr.create_pair(pair.clone()).await.unwrap();
        let base = mgr.execute_sync(pair.id).await.unwrap();
        let day = |n: i64| Utc::now() - chrono::Duration::days(n);
        for (i, age) in [0i64, 2, 10, 40].into_iter().enumerate() {
            let r = SyncReport { id: Uuid::new_v4(), started_at: day(age), ended_at: day(age), pair_name: format!("run{i}"), ..base.clone() };
            mgr.save_report_to_db(&repo, &r, 0).await.unwrap();
        }
        let names = |v: Vec<SyncReport>| v.into_iter().map(|r| r.pair_name).collect::<Vec<_>>();
        let all = mgr.run_history_between(&repo, None, 50, None, None).await.unwrap();
        assert_eq!(names(all), vec!["run0", "run1", "run2", "run3"]);
        let week = mgr.run_history_between(&repo, None, 50, Some(day(7)), None).await.unwrap();
        assert_eq!(names(week), vec!["run0", "run1"]);
        let window = mgr.run_history_between(&repo, None, 50, Some(day(30)), Some(day(5))).await.unwrap();
        assert_eq!(names(window), vec!["run2"]);
    }

    /// Row 72: a running sync exposes the files it is on, bounded; once it
    /// ends the list is gone.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_running_sync_lists_its_files_and_the_list_goes_with_the_run() {
        let mgr = Arc::new(SyncManager::new());
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        for i in 0..600 {
            std::fs::write(src.path().join(format!("f{i:03}.txt")), format!("file number {i}")).unwrap();
        }
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.copy_options.threads = 1;
        mgr.create_pair(pair.clone()).await.unwrap();
        assert!(mgr.live_files(pair.id, 50).is_none(), "idle pairs have no live list");

        let poller = {
            let mgr = mgr.clone();
            let id = pair.id;
            tokio::spawn(async move {
                let mut best: Option<live_files::LiveFiles> = None;
                for _ in 0..20_000 {
                    if let Some(snap) = mgr.live_files(id, 50) {
                        if snap.finished_total > 0 && best.as_ref().map_or(true, |b| snap.finished_total > b.finished_total) {
                            best = Some(snap);
                        }
                    }
                    tokio::time::sleep(std::time::Duration::from_micros(500)).await;
                    if best.as_ref().is_some_and(|b| b.finished_total >= 300) {
                        break;
                    }
                }
                best
            })
        };
        let report = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(report.files_added, 600);
        let seen = poller.await.unwrap().expect("a snapshot taken while the run was going");
        assert!(seen.files.len() <= 50, "bounded by the limit");
        assert!(seen.files.iter().any(|f| f.state == "done" && f.action.as_deref() == Some("copied")));
        assert!(mgr.live_files(pair.id, 50).is_none(), "the list goes with the run");
    }

    #[tokio::test]
    async fn sync_pairs_survive_a_restart() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let pair = SyncPair {
            name: "Photos backup".to_string(),
            source_path: "/src/photos".to_string(),
            dest_path: "/dst/photos".to_string(),
            mode: SyncMode::Mirror,
            enabled: true,
            trigger: SyncTrigger::Scheduled {
                cron_expr: "0 0 */6 * * * *".to_string(),
            },
            conflict_policy: SyncConflictPolicy::NewestWins,
            verify_mode: SyncVerifyMode::Full,
            checksum_enabled: true,
            time_offset_secs: Some(-42),
            copy_options: CopyOptions {
                threads: 8,
                retries: 3,
                exclude_dirs: vec!["node_modules".to_string()],
                run_hours: "2200-0600".to_string(),
                ..CopyOptions::default()
            },
            filter: SyncFilter {
                exclude_patterns: vec!["*.tmp".to_string()],
                ..SyncFilter::default()
            },
            ..SyncPair::default()
        };

        SyncManager::new().save_pair_to_db(&repo, &pair).await.unwrap();

        // A brand-new manager stands in for the next app launch.
        let restarted = SyncManager::new();
        assert_eq!(restarted.load_pairs_from_db(&repo).await.unwrap(), 1);

        let loaded = restarted.pairs.read().await;
        let got = loaded.get(&pair.id).expect("pair missing after restart");
        assert_eq!(got.name, "Photos backup");
        assert_eq!(got.source_path, "/src/photos");
        assert_eq!(got.dest_path, "/dst/photos");
        assert_eq!(got.mode, SyncMode::Mirror);
        assert!(got.enabled);
        assert!(got.checksum_enabled);
        assert_eq!(got.time_offset_secs, Some(-42));
        assert_eq!(got.copy_options, pair.copy_options);
        assert_eq!(got.conflict_policy.as_str(), "newest_wins");
        assert_eq!(got.verify_mode.as_str(), "full");
        assert_eq!(got.filter.exclude_patterns, vec!["*.tmp".to_string()]);
        // The cron payload lives in its own column; losing it would silently
        // downgrade a 6-hourly sync to the "0 * * * *" fallback.
        match &got.trigger {
            SyncTrigger::Scheduled { cron_expr } => assert_eq!(cron_expr, "0 0 */6 * * * *"),
            other => panic!("trigger did not round-trip: {other:?}"),
        }
    }

    #[tokio::test]
    async fn run_options_survive_a_restart() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = SyncManager::new();
        let pair = SyncPair {
            run_options: ufop_core::sync_types::SyncRunOptions {
                settle_secs: 30,
                completion_marker: Some("_DONE".into()),
                version_template: Some("{name}~{num}.{ext}".into()),
                // Result notifications survive a restart with the pair.
                notify: ufop_core::run_notify::RunNotify { notify_profile_id: Some("p1".into()), notify_on: vec!["error".into()], min_bytes: Some(10) },
                keep_versions: Some(5),
            },
            ..SyncPair::default()
        };
        mgr.save_pair_to_db(&repo, &pair).await.unwrap();
        let restarted = SyncManager::new();
        restarted.load_pairs_from_db(&repo).await.unwrap();
        assert_eq!(restarted.pairs.read().await.get(&pair.id).unwrap().run_options, pair.run_options);
    }

    #[tokio::test]
    async fn saving_the_same_pair_twice_updates_rather_than_duplicates() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = SyncManager::new();
        let mut pair = SyncPair {
            name: "First".to_string(),
            ..SyncPair::default()
        };
        mgr.save_pair_to_db(&repo, &pair).await.unwrap();
        pair.name = "Renamed".to_string();
        mgr.save_pair_to_db(&repo, &pair).await.unwrap();

        let restarted = SyncManager::new();
        assert_eq!(restarted.load_pairs_from_db(&repo).await.unwrap(), 1);
        assert_eq!(
            restarted.pairs.read().await.get(&pair.id).unwrap().name,
            "Renamed"
        );
    }

    /// `last_run` is stamped in memory by `execute_sync`, so it is the field
    /// most likely to drift from the DB. `run_sync` saves the pair afterwards;
    /// this pins that the column survives the round-trip at all.
    #[tokio::test]
    async fn last_run_round_trips() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let ran_at = Utc::now();
        let pair = SyncPair {
            last_run: Some(ran_at),
            ..SyncPair::default()
        };
        SyncManager::new().save_pair_to_db(&repo, &pair).await.unwrap();

        let restarted = SyncManager::new();
        restarted.load_pairs_from_db(&repo).await.unwrap();
        let loaded = restarted.pairs.read().await;
        let got = loaded.get(&pair.id).unwrap();

        // RFC3339 keeps sub-second precision, but compare at second resolution
        // so the assertion cannot hinge on formatting round-off.
        assert_eq!(
            got.last_run.map(|t| t.timestamp()),
            Some(ran_at.timestamp())
        );
    }

    #[tokio::test]
    async fn deleted_pairs_do_not_come_back_on_restart() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = SyncManager::new();
        let pair = SyncPair::default();
        mgr.save_pair_to_db(&repo, &pair).await.unwrap();
        mgr.delete_pair_from_db(&repo, pair.id).await.unwrap();

        let restarted = SyncManager::new();
        assert_eq!(restarted.load_pairs_from_db(&repo).await.unwrap(), 0);
    }

    #[test]
    fn filter_presets_include_the_shared_file_type_groups() {
        let presets = get_filter_presets();
        for id in ["macos_metadata", "images", "video", "audio", "documents", "archives"] {
            assert!(presets.iter().any(|p| p.id == id), "{id}");
        }
        let images = presets.iter().find(|p| p.id == "images").unwrap();
        assert!(images.patterns.contains(&"*.jpg".to_string()) && images.patterns.contains(&"*.JPG".to_string()));
    }

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
            copy_options: CopyOptions::default(),
            ..Default::default()
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

    /// Regression: `Scheduled` pairs were armed but the cron loop was never
    /// started, so they never ran. The loop now drives the run callback.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scheduled_pair_runs_through_the_scheduler() {
        let mgr = Arc::new(SyncManager::new());
        let src = TempDir::new().unwrap();
        let dst = TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "alpha").unwrap();
        let mut pair = make_test_pair(src.path(), dst.path());
        pair.trigger = SyncTrigger::Scheduled { cron_expr: "* * * * * * *".into() };
        mgr.create_pair(pair.clone()).await.unwrap();

        let handle = tokio::runtime::Handle::current();
        let runner = mgr.clone();
        mgr.start_scheduler_every(
            Arc::new(move |id: Uuid| {
                let runner = runner.clone();
                handle.spawn(async move { let _ = runner.execute_sync(id).await; });
            }),
            std::time::Duration::from_millis(100),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(8);
        while !dst.path().join("a.txt").exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        mgr.scheduler.stop();
        assert!(dst.path().join("a.txt").exists(), "scheduled sync never ran");
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

    // ── Rollback / quarantine / conflicts / progress (regressions) ──

    fn temp_pair(src: &std::path::Path, dst: &std::path::Path, mode: SyncMode) -> SyncPair {
        SyncPair {
            name: "Regression".to_string(),
            source_path: src.to_string_lossy().to_string(),
            dest_path: dst.to_string_lossy().to_string(),
            mode,
            ..SyncPair::default()
        }
    }

    async fn mgr_with(pair: &SyncPair) -> (SyncManager, tempfile::TempDir) {
        let state = tempfile::TempDir::new().unwrap();
        let mgr = SyncManager::with_state_dir(state.path().to_path_buf());
        mgr.create_pair(pair.clone()).await.unwrap();
        (mgr, state)
    }

    #[tokio::test]
    async fn mirror_rollback_survives_and_restores() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(dst.path().join("extra.txt"), "precious").unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::Mirror);
        let (mgr, state) = mgr_with(&pair).await;

        let report = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(report.files_deleted, 1);
        // Snapshots live in the state dir, never inside the destination.
        assert!(!dst.path().join(".sync-rollback").exists());
        assert!(state.path().join("rollback").exists());

        // A second mirror run must not destroy what rollback needs.
        mgr.execute_sync(pair.id).await.unwrap();
        let restored = mgr.rollback_last_run(pair.id).await.unwrap();
        assert_eq!(restored, 1);
        assert_eq!(std::fs::read_to_string(dst.path().join("extra.txt")).unwrap(), "precious");
    }

    /// Regression (review): a rollback during a run of the same pair raced
    /// the run's file writes, and the run then saved a two-way baseline
    /// recording the rolled-back paths as in sync again. Rollback waits for
    /// the run, and a run cannot start during a rollback.
    #[tokio::test]
    async fn rollback_is_refused_while_the_pair_is_syncing() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(dst.path().join("extra.txt"), "precious").unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::Mirror);
        let (mgr, _state) = mgr_with(&pair).await;
        mgr.execute_sync(pair.id).await.unwrap();
        assert!(!dst.path().join("extra.txt").exists());

        mgr.running.write().await.insert(pair.id, Arc::new(std::sync::atomic::AtomicBool::new(false)));
        let err = mgr.rollback_last_run(pair.id).await.unwrap_err();
        assert!(matches!(&err, AppError::Sync { message, .. } if message.contains("syncing")), "{err:?}");
        assert!(!dst.path().join("extra.txt").exists(), "nothing restored during the run");
        mgr.running.write().await.remove(&pair.id);

        // Once the run is over the rollback works, and frees the pair again.
        assert_eq!(mgr.rollback_last_run(pair.id).await.unwrap(), 1);
        assert_eq!(std::fs::read_to_string(dst.path().join("extra.txt")).unwrap(), "precious");
        assert!(mgr.running.read().await.get(&pair.id).is_none());
        assert!(!mgr.cancel_sync(pair.id).await, "a finished rollback leaves no run behind");
    }

    #[tokio::test]
    async fn mirror_never_purges_internal_folders_or_partials() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::create_dir_all(dst.path().join(".sync-rollback/old-run")).unwrap();
        std::fs::write(dst.path().join(".sync-rollback/old-run/a.txt"), "snap").unwrap();
        std::fs::write(dst.path().join("big.bin.ufop-partial"), "half").unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::Mirror);
        let (mgr, _state) = mgr_with(&pair).await;
        let report = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(report.files_deleted, 0);
        assert!(dst.path().join(".sync-rollback/old-run/a.txt").exists());
        assert!(dst.path().join("big.bin.ufop-partial").exists());
    }

    fn make_two_way_conflict(src: &std::path::Path, dst: &std::path::Path) {
        std::fs::write(src.join("doc.txt"), "source version, longer").unwrap();
        std::fs::write(dst.join("doc.txt"), "dest version").unwrap();
        let t = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        copier::set_mtime(&dst.join("doc.txt"), t);
    }

    #[tokio::test]
    async fn ask_conflicts_are_queued_and_resolution_is_applied() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        make_two_way_conflict(src.path(), dst.path());
        let mut pair = temp_pair(src.path(), dst.path(), SyncMode::TwoWay);
        pair.conflict_policy = SyncConflictPolicy::Ask;
        let (mgr, _state) = mgr_with(&pair).await;

        mgr.execute_sync(pair.id).await.unwrap();
        let pending = mgr.get_pending_conflicts().await;
        assert_eq!(pending.len(), 1);
        // Re-running doesn't queue the same file twice.
        mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(mgr.get_pending_conflicts().await.len(), 1);

        mgr.resolve_conflict_item(pending[0].id, SyncConflictPolicy::SourceWins)
            .await
            .unwrap();
        assert_eq!(
            std::fs::read_to_string(dst.path().join("doc.txt")).unwrap(),
            "source version, longer"
        );
        assert!(mgr.get_pending_conflicts().await.is_empty());
        // The overwrite can be undone.
        assert_eq!(mgr.rollback_last_run(pair.id).await.unwrap(), 1);
        assert_eq!(std::fs::read_to_string(dst.path().join("doc.txt")).unwrap(), "dest version");
    }

    #[tokio::test]
    async fn quarantine_entries_are_recorded_outside_destination() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        make_two_way_conflict(src.path(), dst.path());
        let mut pair = temp_pair(src.path(), dst.path(), SyncMode::TwoWay);
        pair.conflict_policy = SyncConflictPolicy::Quarantine;
        let (mgr, state) = mgr_with(&pair).await;
        mgr.execute_sync(pair.id).await.unwrap();
        let q = mgr.get_quarantine(pair.id).await;
        assert_eq!(q.len(), 1);
        assert!(q[0].quarantine_path.starts_with(&*state.path().to_string_lossy()));
        assert!(!dst.path().join(".sync-quarantine").exists());
    }

    // ── Two-way baseline (item 47) ──

    /// Regression: two-way sync had no memory of the last run, so a file
    /// deleted on one side was seen as "new on the other side" and copied
    /// straight back — deletions never stuck.
    #[tokio::test]
    async fn two_way_deletion_sticks_and_propagates() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(src.path().join("keep.txt"), "keep").unwrap();
        std::fs::write(src.path().join("gone.txt"), "delete me").unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::TwoWay);
        let (mgr, _state) = mgr_with(&pair).await;

        let first = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(first.files_added, 2);
        assert!(dst.path().join("gone.txt").exists());

        std::fs::remove_file(src.path().join("gone.txt")).unwrap();
        let second = mgr.execute_sync(pair.id).await.unwrap();
        assert!(!src.path().join("gone.txt").exists(), "the deleted file came back");
        assert!(!dst.path().join("gone.txt").exists(), "the deletion was not propagated");
        assert_eq!(second.files_deleted, 1);
        assert!(dst.path().join("keep.txt").exists() && src.path().join("keep.txt").exists());
        // The deletion can be undone like any other destructive step.
        assert_eq!(mgr.rollback_last_run(pair.id).await.unwrap(), 1);
        assert!(dst.path().join("gone.txt").exists());
    }

    /// Regression (review): rolling back a two-way run restored the files
    /// but kept the baseline that run saved, so the restored file read as a
    /// fresh edit and the next run overwrote the other side's newer version
    /// with the rolled-back content.
    #[tokio::test]
    async fn two_way_rollback_never_loses_the_other_sides_edit() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "a").unwrap();
        std::fs::write(src.path().join("b.txt"), "b").unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::TwoWay);
        let (mgr, _state) = mgr_with(&pair).await;
        mgr.execute_sync(pair.id).await.unwrap();

        let edit = "b edited in the destination";
        std::fs::write(dst.path().join("b.txt"), edit).unwrap();
        let second = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(second.files_modified, 1);
        assert_eq!(std::fs::read_to_string(src.path().join("b.txt")).unwrap(), edit);
        assert_eq!(mgr.rollback_last_run(pair.id).await.unwrap(), 1);
        assert_eq!(std::fs::read_to_string(src.path().join("b.txt")).unwrap(), "b");

        let preview = mgr.dry_run(pair.id).await.unwrap();
        assert!(
            !preview.modifications.iter().any(|m| m.relative_path == "b.txt" && m.direction == SyncDirection::ToDest),
            "the rolled-back file must not be copied over the destination's edit: {:?}",
            preview.modifications
        );
        let third = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(third.errors, 0, "{:?}", third.error_messages);
        assert_eq!(std::fs::read_to_string(dst.path().join("b.txt")).unwrap(), edit, "the destination edit was overwritten after a rollback");
        // The difference is a conflict under the pair's policy (keep both):
        // the rolled-back content is kept too.
        let kept = std::fs::read_dir(dst.path()).unwrap().flatten().any(|e| std::fs::read_to_string(e.path()).is_ok_and(|c| c == "b"));
        assert!(kept, "the rolled-back version was lost");
        // Untouched files keep their history.
        assert!(mgr.load_baseline(&pair).await.unwrap().unwrap().entries.contains_key("a.txt"));
    }

    /// Review: a run with errors keeps the old baseline; the dry run says so.
    #[tokio::test]
    async fn two_way_dry_run_notes_a_baseline_held_back_by_a_failed_run() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "a").unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::TwoWay);
        let (mgr, _state) = mgr_with(&pair).await;
        let held_back = |p: &SyncPreview| p.notes.iter().any(|n| n.contains("did not finish cleanly"));
        mgr.execute_sync(pair.id).await.unwrap();
        assert!(!held_back(&mgr.dry_run(pair.id).await.unwrap()));

        std::fs::write(src.path().join("blocked"), "b").unwrap();
        std::fs::create_dir(dst.path().join("blocked")).unwrap();
        assert!(mgr.execute_sync(pair.id).await.unwrap().errors > 0);
        assert!(held_back(&mgr.dry_run(pair.id).await.unwrap()));
    }

    /// The baseline lives in SQLite: a restart in between does not turn the
    /// deletion back into an addition.
    #[tokio::test]
    async fn two_way_baseline_survives_a_restart() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "a").unwrap();
        std::fs::write(src.path().join("b.txt"), "b").unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::TwoWay);
        let (mgr, _state) = mgr_with(&pair).await;
        mgr.attach_repository(repo.clone());
        mgr.save_pair_to_db(&repo, &pair).await.unwrap();
        mgr.execute_sync(pair.id).await.unwrap();
        let info = mgr.baseline_info(pair.id).await.unwrap().unwrap();
        assert_eq!(info.files, 2);
        assert!(info.recorded_at.is_some());

        let state = tempfile::TempDir::new().unwrap();
        let restarted = SyncManager::with_state_dir(state.path().to_path_buf());
        restarted.attach_repository(repo.clone());
        restarted.load_pairs_from_db(&repo).await.unwrap();
        std::fs::remove_file(dst.path().join("a.txt")).unwrap();
        let preview = restarted.dry_run(pair.id).await.unwrap();
        assert_eq!(preview.total_deletions, 1, "dry run uses the stored baseline");
        assert_eq!(preview.deletions[0].direction, SyncDirection::ToSource);
        restarted.execute_sync(pair.id).await.unwrap();
        assert!(!src.path().join("a.txt").exists());

        // Changing the mode or folders forgets it; so does an explicit reset.
        let mut moved = restarted.pairs.read().await.get(&pair.id).cloned().unwrap();
        assert!(restarted.baseline_info(pair.id).await.unwrap().is_some());
        moved.mode = SyncMode::OneWay;
        restarted.update_pair(moved.clone()).await.unwrap();
        moved.mode = SyncMode::TwoWay;
        restarted.update_pair(moved).await.unwrap();
        assert!(restarted.baseline_info(pair.id).await.unwrap().is_none());
        restarted.execute_sync(pair.id).await.unwrap();
        assert!(restarted.baseline_info(pair.id).await.unwrap().is_some());
        restarted.reset_baseline(pair.id).await.unwrap();
        assert!(restarted.baseline_info(pair.id).await.unwrap().is_none());
    }

    /// Regression (dead resume path): `resume_states` was read on every run
    /// but never written, so an interrupted run always started over. Steps
    /// are now checkpointed as they finish and a rerun skips them.
    #[tokio::test]
    async fn interrupted_run_resumes_from_its_checkpoint() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "alpha").unwrap();
        std::fs::write(src.path().join("b.txt"), "bravo").unwrap();
        let mut pair = temp_pair(src.path(), dst.path(), SyncMode::OneWay);
        pair.copy_options.copy_timestamps = false;
        let (mgr, _state) = mgr_with(&pair).await;
        let first = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!(first.files_added, 2);
        assert!(!first.resumed);
        assert!(!mgr.checkpoint_path(pair.id).exists(), "a finished run leaves no checkpoint");

        // A run that died after copying a.txt left this behind.
        let preview = mgr.dry_run(pair.id).await.unwrap();
        let a = preview.modifications.iter().find(|e| e.relative_path == "a.txt").unwrap();
        let journal = checkpoint::Journal::open(&mgr.checkpoint_path(pair.id), &pair, Uuid::new_v4(), false).unwrap();
        (journal.recorder())("a.txt", &CompletedStep::of(a));
        drop(journal);
        assert_eq!(checkpoint::load(&mgr.checkpoint_path(pair.id)).unwrap().completed.len(), 1);

        let resumed = mgr.execute_sync(pair.id).await.unwrap();
        assert!(resumed.resumed);
        assert_eq!(resumed.files_modified, 1, "a.txt was already done");
        assert!(!mgr.checkpoint_path(pair.id).exists());

        // A cancelled run keeps its checkpoint for the next one.
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let cancelled = mgr.run_pair(pair.clone(), flag).await.unwrap();
        assert_eq!(cancelled.status, SyncRunStatus::Cancelled);
        assert!(mgr.checkpoint_path(pair.id).exists());
        assert!(mgr.execute_sync(pair.id).await.unwrap().resumed);
        assert!(!mgr.checkpoint_path(pair.id).exists());
    }

    /// Regression: a resumed run appended its first step to the torn last
    /// line a crash left behind, so that step was lost as well.
    #[test]
    fn resumed_checkpoint_does_not_glue_onto_a_torn_line() {
        let dir = tempfile::TempDir::new().unwrap();
        let pair = temp_pair(dir.path(), dir.path(), SyncMode::OneWay);
        let path = dir.path().join("cp.jsonl");
        let step = CompletedStep { action: SyncAction::Add, direction: Default::default(), size: Some(1), modified: None, done_at: None, version: None };
        let journal = checkpoint::Journal::open(&path, &pair, Uuid::new_v4(), false).unwrap();
        (journal.recorder())("a.txt", &step);
        drop(journal);
        // The crash cut the second line short.
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        std::io::Write::write_all(&mut f, br#"{"path":"b.tx"#).unwrap();
        drop(f);
        assert_eq!(checkpoint::load(&path).unwrap().completed.len(), 1);

        let journal = checkpoint::Journal::open(&path, &pair, Uuid::new_v4(), true).unwrap();
        (journal.recorder())("c.txt", &step);
        drop(journal);
        let cp = checkpoint::load(&path).unwrap();
        assert!(cp.completed.contains_key("a.txt") && cp.completed.contains_key("c.txt"), "{:?}", cp.completed.keys().collect::<Vec<_>>());
    }

    /// Regression: the checkpoint of a cancelled run made the next run skip
    /// its copies after the pair's destination was changed, reporting Success.
    #[tokio::test]
    async fn checkpoint_does_not_survive_a_change_of_folders() {
        let src = tempfile::TempDir::new().unwrap();
        let d1 = tempfile::TempDir::new().unwrap();
        let d2 = tempfile::TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "alpha").unwrap();
        let mut pair = temp_pair(src.path(), d1.path(), SyncMode::OneWay);
        pair.copy_options.copy_timestamps = false;
        let (mgr, _state) = mgr_with(&pair).await;
        mgr.execute_sync(pair.id).await.unwrap();
        let record = |mgr: &SyncManager, pair: &SyncPair| {
            let preview = planner::compute_preview(pair).unwrap();
            let a = preview.modifications.iter().chain(&preview.additions).find(|e| e.relative_path == "a.txt").unwrap().clone();
            let journal = checkpoint::Journal::open(&mgr.checkpoint_path(pair.id), pair, Uuid::new_v4(), false).unwrap();
            (journal.recorder())("a.txt", &CompletedStep::of(&a));
        };

        // Editing the folders forgets the checkpoint.
        record(&mgr, &pair);
        let moved = SyncPair { dest_path: d2.path().to_string_lossy().to_string(), ..pair.clone() };
        mgr.update_pair(moved.clone()).await.unwrap();
        assert!(!mgr.checkpoint_path(pair.id).exists(), "update_pair keeps a stale checkpoint");
        let r = mgr.execute_sync(pair.id).await.unwrap();
        assert_eq!((r.status, r.files_added, r.resumed), (SyncRunStatus::Success, 1, false));
        assert!(d2.path().join("a.txt").exists(), "a.txt missing at the new destination");

        // A checkpoint written for other folders is ignored and removed.
        record(&mgr, &pair);
        let r = mgr.execute_sync(pair.id).await.unwrap();
        assert!(!r.resumed);
        assert_eq!(r.files_modified, 1);
        assert!(!mgr.checkpoint_path(pair.id).exists());

        // Deleting the pair removes its checkpoint too.
        record(&mgr, &moved);
        mgr.delete_pair(pair.id).await.unwrap();
        assert!(!mgr.checkpoint_path(pair.id).exists());
    }

    #[tokio::test]
    async fn duplicate_copies_the_definition_without_history_or_schedule() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "a").unwrap();
        let mut pair = temp_pair(src.path(), dst.path(), SyncMode::TwoWay);
        pair.trigger = SyncTrigger::Scheduled { cron_expr: "0 0 * * * * *".into() };
        pair.copy_options.threads = 4;
        let (mgr, _state) = mgr_with(&pair).await;
        mgr.attach_repository(repo.clone());
        mgr.execute_sync(pair.id).await.unwrap();
        let copy = mgr.duplicate_pair(pair.id).await.unwrap();
        assert_ne!(copy.id, pair.id);
        assert_eq!(copy.name, "Regression (copy)");
        // The schedule is copied (cron included) but off: the copy is disabled and not armed.
        assert_eq!(copy.trigger, pair.trigger, "the cron expression is kept");
        assert!(!copy.enabled, "the copy is never scheduled");
        assert!(mgr.scheduler.next_run_time(copy.id).is_none(), "the copy's schedule is armed");
        assert!(mgr.scheduler.next_run_time(pair.id).is_some());
        assert_eq!(copy.copy_options.threads, 4);
        assert!(copy.last_run.is_none());
        assert!(mgr.get_reports(copy.id).await.is_empty());
        assert!(mgr.baseline_info(copy.id).await.unwrap().is_none(), "no two-way history");
        assert!(mgr.baseline_info(pair.id).await.unwrap().is_some());
        assert_eq!(mgr.list_pairs().await.unwrap().len(), 2);
        assert!(mgr.duplicate_pair(Uuid::new_v4()).await.is_err());
    }

    #[tokio::test]
    async fn global_defaults_apply_to_layered_pairs_only() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = SyncManager::new();
        let defaults = CopyOptions { exclude_hidden: true, ..CopyOptions::default() };
        mgr.set_default_options(&repo, defaults.clone()).await.unwrap();
        assert!(mgr.set_default_options(&repo, CopyOptions { threads: 0, ..CopyOptions::default() }).await.is_err());
        let restarted = SyncManager::new();
        restarted.load_default_options(&repo).await.unwrap();
        assert_eq!(restarted.default_options(), defaults);

        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(src.path().join(".hidden"), "h").unwrap();
        std::fs::write(src.path().join("seen.txt"), "s").unwrap();
        // A pair from before defaults existed keeps its own options …
        let legacy = temp_pair(src.path(), dst.path(), SyncMode::OneWay);
        restarted.create_pair(legacy.clone()).await.unwrap();
        assert_eq!(restarted.dry_run(legacy.id).await.unwrap().total_additions, 2);
        // … a layered pair follows the defaults for what it does not set.
        let layered = SyncPair { id: Uuid::new_v4(), option_overrides: Some(vec!["threads".into()]), ..legacy.clone() };
        restarted.create_pair(layered.clone()).await.unwrap();
        assert_eq!(restarted.dry_run(layered.id).await.unwrap().total_additions, 1);
        let own = SyncPair { id: Uuid::new_v4(), option_overrides: Some(vec!["exclude_hidden".into()]), ..legacy };
        restarted.create_pair(own.clone()).await.unwrap();
        assert_eq!(restarted.dry_run(own.id).await.unwrap().total_additions, 2, "the pair's own value wins");
        // Overrides survive a restart.
        restarted.save_pair_to_db(&repo, &own).await.unwrap();
        let again = SyncManager::new();
        again.load_pairs_from_db(&repo).await.unwrap();
        assert_eq!(again.pairs.read().await.get(&own.id).unwrap().option_overrides, Some(vec!["exclude_hidden".to_string()]));
    }

    #[tokio::test]
    async fn export_and_import_a_pair() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        let mut pair = temp_pair(src.path(), dst.path(), SyncMode::Mirror);
        pair.filter.exclude_patterns = vec!["*.tmp".into()];
        let (mgr, _state) = mgr_with(&pair).await;
        let json = mgr.export_pair(&repo, pair.id).await.unwrap();
        assert!(json.contains("\"format\": \"ufop-sync-pair\""));
        let imported = mgr.import_pair(&repo, &json).await.unwrap();
        assert!(imported.warnings.is_empty(), "{:?}", imported.warnings);
        assert_ne!(imported.pair.id, pair.id);
        assert!(imported.pair.enabled);
        assert_eq!(imported.pair.filter.exclude_patterns, vec!["*.tmp".to_string()]);
        // On a computer without the folders it is flagged and left disabled.
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["pair"]["dest_path"] = "/nonexistent/ufop/dest".into();
        let flagged = mgr.import_pair(&repo, &v.to_string()).await.unwrap();
        assert!(!flagged.pair.enabled);
        assert!(flagged.warnings.iter().any(|w| w.contains("/nonexistent/ufop/dest")));
        assert!(mgr.import_pair(&repo, "{}").await.is_err());

        // Regression: an enabled scheduled (or watched) pair from another
        // computer was armed on import, so a mirror could start purging a
        // destination before anyone looked at it. It now arrives off.
        let armed = mgr.scheduler.list_schedules().len();
        let mut v: serde_json::Value = serde_json::from_str(&json).unwrap();
        v["pair"]["trigger"] = serde_json::to_value(SyncTrigger::Scheduled { cron_expr: "0 0 * * * * *".into() }).unwrap();
        let scheduled = mgr.import_pair(&repo, &v.to_string()).await.unwrap();
        assert!(!scheduled.pair.enabled, "an imported scheduled pair starts disabled");
        assert!(matches!(scheduled.pair.trigger, SyncTrigger::Scheduled { .. }), "the trigger itself is kept");
        assert_eq!(mgr.scheduler.list_schedules().len(), armed, "an imported schedule is not armed");
        assert!(scheduled.warnings.iter().any(|w| w.contains("Edit")), "{:?}", scheduled.warnings);
        v["pair"]["trigger"] = serde_json::to_value(SyncTrigger::Watch).unwrap();
        let watched = mgr.import_pair(&repo, &v.to_string()).await.unwrap();
        assert!(!watched.pair.enabled, "an imported watched pair starts disabled");
    }

    #[tokio::test]
    async fn editing_the_trigger_rearms_it() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::OneWay);
        let (mgr, _state) = mgr_with(&pair).await;
        assert!(mgr.scheduler.list_schedules().is_empty());
        let scheduled = SyncPair { trigger: SyncTrigger::Scheduled { cron_expr: "0 0 * * * * *".into() }, ..pair.clone() };
        mgr.update_pair(scheduled).await.unwrap();
        assert_eq!(mgr.scheduler.list_schedules().len(), 1);
        mgr.update_pair(pair).await.unwrap();
        assert!(mgr.scheduler.list_schedules().is_empty());
    }

    /// Regression: an edit to a watched pair whose folder cannot be watched
    /// failed after the pair was replaced in memory (the command then never
    /// saved it), so memory, database and triggers disagreed. A watcher
    /// failure is now logged, as when a pair is created.
    #[tokio::test]
    async fn unwatchable_edit_is_still_applied() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::OneWay);
        let (mgr, _state) = mgr_with(&pair).await;
        let gone = src.path().join("missing");
        let watched = SyncPair {
            name: "Watched".into(),
            trigger: SyncTrigger::Watch,
            source_path: gone.to_string_lossy().to_string(),
            ..pair.clone()
        };
        let saved = mgr.update_pair(watched.clone()).await.expect("the edit is applied");
        assert_eq!(saved.name, "Watched");
        let kept = mgr.pairs.read().await.get(&pair.id).cloned().unwrap();
        assert_eq!((kept.name.as_str(), &kept.trigger), ("Watched", &SyncTrigger::Watch));
    }

    /// Regression: an edit with an invalid cron expression replaced the pair
    /// in memory and dropped its schedule before failing, so memory, the
    /// database (never saved) and the scheduler disagreed.
    #[tokio::test]
    async fn invalid_cron_edit_changes_nothing() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        let pair = temp_pair(src.path(), dst.path(), SyncMode::OneWay);
        let (mgr, _state) = mgr_with(&pair).await;
        let scheduled = SyncPair { trigger: SyncTrigger::Scheduled { cron_expr: "0 0 * * * * *".into() }, ..pair.clone() };
        mgr.update_pair(scheduled.clone()).await.unwrap();
        let bad = SyncPair { name: "Renamed".into(), trigger: SyncTrigger::Scheduled { cron_expr: "not a cron".into() }, ..scheduled.clone() };
        assert!(mgr.update_pair(bad).await.is_err());
        let kept = mgr.pairs.read().await.get(&pair.id).cloned().unwrap();
        assert_eq!(kept.name, scheduled.name);
        assert_eq!(kept.trigger, scheduled.trigger);
        assert_eq!(mgr.scheduler.list_schedules().len(), 1, "the old schedule stays armed");
    }

    #[tokio::test]
    async fn cancel_and_progress_and_single_run() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        for i in 0..5 {
            std::fs::write(src.path().join(format!("f{i}.txt")), "x").unwrap();
        }
        let pair = temp_pair(src.path(), dst.path(), SyncMode::OneWay);
        let (mgr, _state) = mgr_with(&pair).await;
        assert!(!mgr.cancel_sync(pair.id).await, "nothing running yet");
        assert!(mgr.get_progress(pair.id).is_none());

        // A pre-set cancel flag stops the run before any file is copied.
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        mgr.running.write().await.insert(pair.id, flag.clone());
        assert!(mgr.execute_sync(pair.id).await.is_err(), "second run is refused");
        mgr.running.write().await.remove(&pair.id);
        let report = mgr.run_pair(pair.clone(), flag).await.unwrap();
        assert_eq!(report.status, SyncRunStatus::Cancelled);
        assert_eq!(report.files_added, 0);

        // A normal run reports progress with the right totals.
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen2 = seen.clone();
        let config = executor::ExecutorConfig {
            pair: pair.clone(),
            run_id: Uuid::new_v4(),
            continue_on_error: true,
            resume_from: None,
            quarantine_dir: None,
            rollback_dir: None,
            baseline: None,
        };
        let hooks = executor::RunHooks {
            progress: Some(Arc::new(move |p: &executor::SyncProgress| {
                seen2.lock().unwrap().push(p.clone());
            })),
            cancel: None,
            on_step: None,
            on_file: None,
        };
        let outcome = executor::execute_sync_with(&config, &hooks).unwrap();
        assert_eq!(outcome.report.files_added, 5);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.first().unwrap().phase, "planning");
        assert_eq!(seen.last().unwrap().phase, "finishing");
        assert!(seen.iter().any(|p| p.files_total == 5 && p.files_done == 5));
    }
}
