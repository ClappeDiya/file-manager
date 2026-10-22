//! Automation engine - trigger → condition → action rules for file workflows.
//!
//! Provides `AutomationManager` for creating and running "Quickflow" rules:
//! - Filesystem watchers (file appears, modified, deleted)
//! - Cron-based scheduling
//! - Manual triggers
//! - Condition evaluation (extension, name pattern, size, age)
//! - Action dispatch to existing engines (fs, transfer, sync, archive)
//! - Execution logging and safety guards

pub mod evaluator;
pub mod executor;
pub mod pinner;
pub mod scheduler;
pub mod services;
pub mod suggester;
pub mod tauri_services;

use crate::core::error::AppError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Data Structures ──

/// A single automation rule: trigger → conditions → action.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationRule {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    pub trigger: RuleTrigger,
    pub conditions: Vec<RuleCondition>,
    pub action: RuleAction,
    pub created_at: String,
    pub updated_at: String,
    pub last_triggered: Option<String>,
    pub trigger_count: u64,
    pub error_count: u64,
    pub last_error: Option<String>,
}

impl AutomationRule {
    pub fn new(name: String, trigger: RuleTrigger, action: RuleAction) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            enabled: false, // disabled by default until user explicitly enables
            trigger,
            conditions: Vec::new(),
            action,
            created_at: now.clone(),
            updated_at: now,
            last_triggered: None,
            trigger_count: 0,
            error_count: 0,
            last_error: None,
        }
    }
}

/// What triggers the rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleTrigger {
    /// A new file appears in the watched directory.
    FileAppears {
        watch_path: String,
        recursive: bool,
    },
    /// A file is modified in the watched directory.
    FileModified {
        watch_path: String,
        recursive: bool,
    },
    /// A file is deleted from the watched directory.
    FileDeleted {
        watch_path: String,
        recursive: bool,
    },
    /// Cron-based schedule.
    Schedule {
        cron_expr: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cron_human: Option<String>,
    },
    /// Manual trigger only (run from UI or command palette).
    Manual,
}

/// Optional conditions that must ALL match (AND logic).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleCondition {
    /// File extension matches one of the given extensions (e.g., ["pdf", "doc"]).
    ExtensionIs { extensions: Vec<String> },
    /// Filename matches a glob pattern (e.g., "report_*").
    NameMatches { pattern: String },
    /// File size is greater than N bytes.
    SizeGreaterThan { bytes: u64 },
    /// File size is less than N bytes.
    SizeLessThan { bytes: u64 },
    /// File was created within the last N seconds.
    CreatedWithin { seconds: u64 },
    /// File path contains this substring.
    PathContains { substring: String },
}

/// What action to perform when triggered and conditions match.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleAction {
    /// Move file(s) to a destination directory.
    MoveFile { dest_path: String },
    /// Copy file(s) to a destination directory.
    CopyFile { dest_path: String },
    /// Rename file(s) using a pattern (e.g., "date_prefix", "lowercase").
    RenameFile { pattern: String },
    /// Delete file(s), optionally to trash.
    DeleteFile { to_trash: bool },
    /// Compress a folder into an archive.
    CompressFolder {
        dest_path: String,
        format: String,
    },
    /// Run an existing sync pair.
    RunSync { sync_pair_id: String },
    /// Transfer to a remote connector.
    TransferToConnector {
        connector_protocol: String,
        connection_id: String,
        remote_path: String,
    },
    /// Execute a saved custom command.
    ExecuteCustomCommand { command_id: String },
    /// Send an OS notification.
    Notify { title: String, body: String },
    /// Create a folder (and any missing parents).
    CreateFolder { path: String },
    /// Replay a captured set of file operations exactly as the user once
    /// performed them. Carries its own source list and destination, so the
    /// trigger doesn't need to provide a file. Created by the **Operation
    /// Pin** flow from a single ledger event — DRY by construction: source
    /// of truth is the original ledger row, the rule is just a saved
    /// re-runnable handle.
    ReplayOp {
        /// One of: `"copy"`, `"move"`. Drives the executor branch.
        kind: String,
        /// Absolute paths captured from the original ledger event.
        source_paths: Vec<String>,
        /// Destination directory captured from the original ledger event.
        dest_path: String,
    },
}

/// An execution log entry for a rule run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutomationLog {
    pub id: String,
    pub rule_id: String,
    pub rule_name: String,
    pub triggered_at: String,
    pub completed_at: Option<String>,
    pub trigger_event: String,
    pub status: AutomationStatus,
    pub files_affected: Vec<String>,
    pub error_message: Option<String>,
}

/// Status of an automation execution.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AutomationStatus {
    Success,
    Failed,
    Skipped,
    Running,
}

/// DRY helper that translates an [`AutomationLog`] into a unified ledger
/// event. Used by BOTH the manual `run_rule` path and the autonomous
/// scheduler watcher path so the cross-engine timeline captures every
/// fire — including the ones that happen while the user is asleep.
pub(crate) async fn record_automation_log(
    ledger: &crate::ledger::OperationLedger,
    log: &AutomationLog,
) {
    use crate::ledger::{LedgerEngine, LedgerStatus, RecordEvent};

    let status = match log.status {
        AutomationStatus::Success => LedgerStatus::Ok,
        AutomationStatus::Failed => LedgerStatus::Failed,
        AutomationStatus::Skipped => LedgerStatus::Skipped,
        AutomationStatus::Running => LedgerStatus::Ok,
    };

    let summary = match (&log.error_message, log.files_affected.len()) {
        (Some(err), _) => format!("automation '{}' failed: {}", log.rule_name, err),
        (None, 0) => format!("automation '{}' fired", log.rule_name),
        (None, n) => format!("automation '{}' fired ({} files)", log.rule_name, n),
    };

    let mut ev = RecordEvent::new(LedgerEngine::Automation, "rule_fire")
        .status(status)
        .correlation(&log.rule_id)
        .summary(summary);
    if let Some(first) = log.files_affected.first() {
        ev = ev.subject(first.clone());
    }
    ledger.record(ev).await;
}

// ── Manager ──

/// Maximum number of active filesystem watchers to prevent resource exhaustion.
const MAX_WATCHERS: usize = 50;

/// Maximum consecutive failures before auto-disabling a rule.
const MAX_CONSECUTIVE_FAILURES: u64 = 5;

/// In-memory run history kept per process (the full history is in SQLite).
const MAX_MEMORY_LOGS: usize = 500;

/// Trigger event recorded for runs started from the UI or command palette.
pub const EVENT_MANUAL: &str = "manual";
/// Trigger event recorded for cron-scheduled runs.
pub const EVENT_SCHEDULE: &str = "schedule";

type SharedServices = Arc<std::sync::RwLock<Option<Arc<dyn services::ActionServices>>>>;
type SharedRepo = Arc<std::sync::RwLock<Option<crate::storage::Repository>>>;

/// Everything one rule run needs. Manual runs, cron runs and watcher runs
/// all go through [`FireContext::fire`], so they execute the rule as it is
/// *now* (not a copy captured when a watcher started), update the same
/// stats, persist the same history and emit the same ledger event.
#[derive(Clone)]
pub(crate) struct FireContext {
    rules: Arc<RwLock<Vec<AutomationRule>>>,
    logs: Arc<RwLock<Vec<AutomationLog>>>,
    ledger: Option<crate::ledger::OperationLedger>,
    services: SharedServices,
    repo: SharedRepo,
}

impl FireContext {
    fn services(&self) -> Option<Arc<dyn services::ActionServices>> {
        self.services.read().ok().and_then(|g| g.clone())
    }

    fn repo(&self) -> Option<crate::storage::Repository> {
        self.repo.read().ok().and_then(|g| g.clone())
    }

    pub(crate) async fn rule(&self, rule_id: &str) -> Option<AutomationRule> {
        self.rules.read().await.iter().find(|r| r.id == rule_id).cloned()
    }

    /// Run a rule only if it is still enabled (watcher and cron paths: a rule
    /// that was disabled or auto-disabled since the trigger armed must not run).
    pub(crate) async fn fire_if_enabled(
        &self,
        rule_id: &str,
        triggered_file: Option<&str>,
        event: &str,
    ) -> Option<AutomationLog> {
        match self.rule(rule_id).await {
            Some(r) if r.enabled => self.fire(rule_id, triggered_file, event, None).await.ok(),
            _ => None,
        }
    }

    /// Execute a rule and record the outcome everywhere: rule stats (with
    /// auto-disable after repeated failures), SQLite history, the in-memory
    /// log and the operation ledger. `repo` overrides the attached repository.
    pub(crate) async fn fire(
        &self,
        rule_id: &str,
        triggered_file: Option<&str>,
        event: &str,
        repo: Option<&crate::storage::Repository>,
    ) -> Result<AutomationLog, AppError> {
        let rule = self.rule(rule_id).await.ok_or_else(|| AppError::State {
            message: format!("Automation rule '{}' not found", rule_id),
            advice: "Check the rule ID.".to_string(),
        })?;

        let services = self.services();
        let mut log =
            executor::execute_rule_action_with(&rule, triggered_file, false, services.as_deref()).await;
        log.trigger_event = match triggered_file {
            Some(f) if event != EVENT_MANUAL => format!("{event}: {f}"),
            _ => event.to_string(),
        };

        let updated = {
            let mut rules = self.rules.write().await;
            rules.iter_mut().find(|r| r.id == rule_id).map(|r| {
                r.trigger_count += 1;
                r.last_triggered = Some(Utc::now().to_rfc3339());
                if log.status == AutomationStatus::Failed {
                    r.error_count += 1;
                    r.last_error = log.error_message.clone();
                    if r.error_count >= MAX_CONSECUTIVE_FAILURES && r.enabled {
                        r.enabled = false;
                        tracing::warn!(
                            "Auto-disabled rule '{}' after {} consecutive failures",
                            r.name, r.error_count
                        );
                    }
                } else {
                    r.error_count = 0;
                    r.last_error = None;
                }
                r.clone()
            })
        };

        let attached = self.repo();
        if let Some(repo) = repo.or(attached.as_ref()) {
            if let Some(updated) = &updated {
                if let Err(e) = db_save_rule(repo, updated).await {
                    tracing::warn!("Failed to persist stats for rule '{}': {e}", updated.name);
                }
            }
            if let Err(e) = db_save_log(repo, &log).await {
                tracing::warn!("Failed to persist automation run history: {e}");
            }
        }

        {
            let mut logs = self.logs.write().await;
            logs.push(log.clone());
            let excess = logs.len().saturating_sub(MAX_MEMORY_LOGS);
            if excess > 0 {
                logs.drain(..excess);
            }
        }

        // Fail-open: ledger errors are swallowed inside `record`.
        if let Some(ledger) = &self.ledger {
            record_automation_log(ledger, &log).await;
        }
        Ok(log)
    }
}

/// Insert or update a rule row.
async fn db_save_rule(repo: &crate::storage::Repository, rule: &AutomationRule) -> Result<(), AppError> {
    let rule_clone = rule.clone();
    repo.pool().execute(move |conn| {
        let trigger_json = serde_json::to_string(&rule_clone.trigger).unwrap_or_default();
        let conditions_json = serde_json::to_string(&rule_clone.conditions).unwrap_or_default();
        let action_json = serde_json::to_string(&rule_clone.action).unwrap_or_default();

        conn.execute(
            "INSERT INTO automation_rules (id, name, enabled, trigger_json, conditions_json, action_json, \
             created_at, updated_at, last_triggered, trigger_count, error_count, last_error) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12) \
             ON CONFLICT(id) DO UPDATE SET \
             name = excluded.name, enabled = excluded.enabled, \
             trigger_json = excluded.trigger_json, conditions_json = excluded.conditions_json, \
             action_json = excluded.action_json, updated_at = excluded.updated_at, \
             last_triggered = excluded.last_triggered, trigger_count = excluded.trigger_count, \
             error_count = excluded.error_count, last_error = excluded.last_error",
            rusqlite::params![
                rule_clone.id,
                rule_clone.name,
                rule_clone.enabled as i32,
                trigger_json,
                conditions_json,
                action_json,
                rule_clone.created_at,
                rule_clone.updated_at,
                rule_clone.last_triggered,
                rule_clone.trigger_count as i64,
                rule_clone.error_count as i64,
                rule_clone.last_error,
            ],
        )?;
        Ok(())
    }).await
}

/// Insert a run-history row.
async fn db_save_log(repo: &crate::storage::Repository, log: &AutomationLog) -> Result<(), AppError> {
    let log_clone = log.clone();
    repo.pool().execute(move |conn| {
        let files_json = serde_json::to_string(&log_clone.files_affected).unwrap_or_default();
        let status_str = serde_json::to_string(&log_clone.status)
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();

        conn.execute(
            "INSERT INTO automation_logs (id, rule_id, rule_name, triggered_at, completed_at, \
             trigger_event, status, files_affected, error_message) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![
                log_clone.id,
                log_clone.rule_id,
                log_clone.rule_name,
                log_clone.triggered_at,
                log_clone.completed_at,
                log_clone.trigger_event,
                status_str,
                files_json,
                log_clone.error_message,
            ],
        )?;
        Ok(())
    }).await
}

/// Manages automation rules, watchers, schedulers, and execution.
pub struct AutomationManager {
    rules: Arc<RwLock<Vec<AutomationRule>>>,
    logs: Arc<RwLock<Vec<AutomationLog>>>,
    /// Track which rules have active watchers (rule_id -> watch handle).
    active_watchers: Arc<RwLock<HashMap<String, scheduler::WatchHandle>>>,
    /// Concurrency limiter for action execution.
    exec_semaphore: Arc<tokio::sync::Semaphore>,
    /// Cooldown tracker to prevent circular triggers (file_path -> last_trigger_time).
    cooldowns: Arc<RwLock<HashMap<String, std::time::Instant>>>,
    /// Optional unified Operation Ledger. When `Some`, every rule execution
    /// (manual via `run_rule` AND autonomous via the scheduler watcher path)
    /// emits a ledger event. Optional so existing tests / non-Tauri callers
    /// can construct the manager without a ledger.
    ledger: Option<crate::ledger::OperationLedger>,
    /// Engines that sync/transfer/command/notify actions delegate to.
    /// Attached once the Tauri app exists (see `set_services`).
    services: SharedServices,
    /// Repository used by autonomous runs to persist their history.
    repo: SharedRepo,
    /// Cron schedules for `RuleTrigger::Schedule` rules. Reuses the sync
    /// engine's cron scheduler, keyed by a per-rule UUID.
    cron: Arc<crate::sync_engine::scheduler::SyncScheduler>,
    /// Cron key → rule id.
    cron_ids: Arc<std::sync::Mutex<HashMap<Uuid, String>>>,
}

impl AutomationManager {
    pub fn new() -> Self {
        Self {
            rules: Arc::new(RwLock::new(Vec::new())),
            logs: Arc::new(RwLock::new(Vec::new())),
            active_watchers: Arc::new(RwLock::new(HashMap::new())),
            exec_semaphore: Arc::new(tokio::sync::Semaphore::new(5)),
            cooldowns: Arc::new(RwLock::new(HashMap::new())),
            ledger: None,
            services: Arc::new(std::sync::RwLock::new(None)),
            repo: Arc::new(std::sync::RwLock::new(None)),
            cron: Arc::new(crate::sync_engine::scheduler::SyncScheduler::new()),
            cron_ids: Arc::new(std::sync::Mutex::new(HashMap::new())),
        }
    }

    /// Inject the unified Operation Ledger. Idempotent; subsequent calls
    /// replace the previous handle. Should be called BEFORE
    /// [`start_all_watchers`] so autonomous fires are captured.
    pub fn set_ledger(&mut self, ledger: crate::ledger::OperationLedger) {
        self.ledger = Some(ledger);
    }

    /// Attach the engines that sync, transfer, command and notify actions
    /// use. Without them those actions fail with an explanation.
    pub fn set_services(&self, services: Arc<dyn services::ActionServices>) {
        if let Ok(mut g) = self.services.write() {
            *g = Some(services);
        }
    }

    /// Attach the repository autonomous (watcher and cron) runs persist to.
    pub fn set_repo(&self, repo: crate::storage::Repository) {
        if let Ok(mut g) = self.repo.write() {
            *g = Some(repo);
        }
    }

    pub(crate) fn fire_context(&self) -> FireContext {
        FireContext {
            rules: self.rules.clone(),
            logs: self.logs.clone(),
            ledger: self.ledger.clone(),
            services: self.services.clone(),
            repo: self.repo.clone(),
        }
    }

    /// Load rules from SQLite on startup.
    pub async fn load_rules_from_db(&self, repo: &crate::storage::Repository) -> Result<(), AppError> {
        let rules = repo.pool().execute(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, enabled, trigger_json, conditions_json, action_json, \
                 created_at, updated_at, last_triggered, trigger_count, error_count, last_error \
                 FROM automation_rules ORDER BY created_at DESC"
            )?;

            let rows = stmt
                .query_map([], |row| {
                    let trigger_json: String = row.get(3)?;
                    let conditions_json: String = row.get(4)?;
                    let action_json: String = row.get(5)?;

                    // A row this build can't read (e.g. written by a newer
                    // version) loads disabled with the reason, never as a
                    // different action that would run on the next trigger.
                    let parsed_action = serde_json::from_str::<RuleAction>(&action_json);
                    let parsed_trigger = serde_json::from_str::<RuleTrigger>(&trigger_json);
                    let unreadable = match (&parsed_action, &parsed_trigger) {
                        (Err(e), _) => Some(format!("Unreadable action configuration: {e}")),
                        (_, Err(e)) => Some(format!("Unreadable trigger configuration: {e}")),
                        _ => None,
                    };
                    Ok(AutomationRule {
                        id: row.get(0)?,
                        name: row.get(1)?,
                        enabled: unreadable.is_none() && row.get::<_, i32>(2)? != 0,
                        trigger: parsed_trigger.unwrap_or(RuleTrigger::Manual),
                        conditions: serde_json::from_str(&conditions_json).unwrap_or_default(),
                        action: parsed_action.unwrap_or(RuleAction::Notify {
                            title: "Error".to_string(),
                            body: "Invalid action configuration".to_string(),
                        }),
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                        last_triggered: row.get(8)?,
                        trigger_count: row.get::<_, i64>(9)? as u64,
                        error_count: row.get::<_, i64>(10)? as u64,
                        last_error: unreadable.or(row.get(11)?),
                    })
                })?
                .filter_map(|r| r.ok())
                .collect::<Vec<_>>();

            Ok(rows)
        }).await?;

        let count = rules.len();
        *self.rules.write().await = rules;
        tracing::info!("Loaded {} automation rule(s) from database", count);
        Ok(())
    }

    /// Save or update a rule in the database.
    pub async fn save_rule(&self, repo: &crate::storage::Repository, rule: &AutomationRule) -> Result<(), AppError> {
        db_save_rule(repo, rule).await?;

        // Update in-memory list
        let mut rules = self.rules.write().await;
        if let Some(existing) = rules.iter_mut().find(|r| r.id == rule.id) {
            *existing = rule.clone();
        } else {
            rules.push(rule.clone());
        }

        Ok(())
    }

    /// Re-arm a rule's trigger after it was created or edited: stop whatever
    /// watcher or schedule it had and start the current one if enabled.
    pub async fn rearm(&self, rule: &AutomationRule) -> Result<(), AppError> {
        self.stop_watcher(&rule.id).await;
        if rule.enabled {
            self.start_watcher_for_rule(rule).await?;
        }
        Ok(())
    }

    /// Delete a rule from the database and stop its watcher if active.
    pub async fn delete_rule(&self, repo: &crate::storage::Repository, rule_id: &str) -> Result<(), AppError> {
        // Stop watcher if active
        self.stop_watcher(rule_id).await;

        let rid = rule_id.to_string();
        repo.pool().execute(move |conn| {
            conn.execute("DELETE FROM automation_rules WHERE id = ?1", rusqlite::params![rid])?;
            Ok(())
        }).await?;

        // Remove from in-memory list
        let mut rules = self.rules.write().await;
        rules.retain(|r| r.id != rule_id);

        Ok(())
    }

    /// Get all rules.
    pub async fn list_rules(&self) -> Vec<AutomationRule> {
        self.rules.read().await.clone()
    }

    /// Get a single rule by ID.
    pub async fn get_rule(&self, rule_id: &str) -> Option<AutomationRule> {
        self.rules.read().await.iter().find(|r| r.id == rule_id).cloned()
    }

    /// Enable or disable a rule, starting/stopping its watcher/scheduler.
    pub async fn set_enabled(
        &self,
        repo: &crate::storage::Repository,
        rule_id: &str,
        enabled: bool,
    ) -> Result<(), AppError> {
        let mut rules = self.rules.write().await;
        let rule = rules.iter_mut().find(|r| r.id == rule_id).ok_or_else(|| AppError::State {
            message: format!("Automation rule '{}' not found", rule_id),
            advice: "Check the rule ID.".to_string(),
        })?;

        rule.enabled = enabled;
        rule.updated_at = Utc::now().to_rfc3339();

        // Reset error count when re-enabling
        if enabled {
            rule.error_count = 0;
            rule.last_error = None;
        }

        let rule_clone = rule.clone();
        drop(rules);

        self.save_rule(repo, &rule_clone).await?;

        if enabled {
            self.start_watcher_for_rule(&rule_clone).await?;
        } else {
            self.stop_watcher(rule_id).await;
        }

        Ok(())
    }

    /// Start filesystem watchers and cron schedules for all enabled rules.
    /// Must run on the long-lived app runtime: watcher tasks spawned on a
    /// short-lived runtime die with it.
    pub async fn start_all_watchers(&self) -> Result<(), AppError> {
        let rules = self.rules.read().await.clone();
        for rule in &rules {
            if rule.enabled {
                if let Err(e) = self.start_watcher_for_rule(rule).await {
                    tracing::warn!("Failed to start watcher for rule '{}': {}", rule.name, e);
                }
            }
        }
        Ok(())
    }

    /// Start the cron loop for `Schedule` rules. Call once from inside the
    /// long-lived tokio runtime; due rules run on that runtime.
    pub fn start_scheduler(&self) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("Automation scheduler not started: no tokio runtime");
            return;
        };
        let ctx = self.fire_context();
        let ids = self.cron_ids.clone();
        let semaphore = self.exec_semaphore.clone();
        self.cron.start(Arc::new(move |key: Uuid| {
            let Some(rule_id) = ids.lock().ok().and_then(|m| m.get(&key).cloned()) else {
                return;
            };
            let ctx = ctx.clone();
            let semaphore = semaphore.clone();
            handle.spawn(async move {
                let Ok(_permit) = semaphore.acquire_owned().await else { return };
                if let Some(log) = ctx.fire_if_enabled(&rule_id, None, EVENT_SCHEDULE).await {
                    tracing::info!("Scheduled automation '{}' ran: {:?}", log.rule_name, log.status);
                }
            });
        }));
    }

    /// Whether the cron loop is running.
    pub fn scheduler_running(&self) -> bool {
        self.cron.is_running()
    }

    /// Cron key for a rule: its own UUID, or a stable hash of a non-UUID id.
    fn cron_key(rule_id: &str) -> Uuid {
        Uuid::parse_str(rule_id).unwrap_or_else(|_| {
            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(rule_id.as_bytes());
            let mut bytes = [0u8; 16];
            bytes.copy_from_slice(&digest[..16]);
            Uuid::from_bytes(bytes)
        })
    }

    /// Next scheduled run of a `Schedule` rule, if armed.
    pub fn next_scheduled_run(&self, rule_id: &str) -> Option<chrono::DateTime<Utc>> {
        self.cron.next_run_time(Self::cron_key(rule_id))
    }

    /// Start a watcher (file triggers) or cron schedule (`Schedule`) for a rule.
    async fn start_watcher_for_rule(&self, rule: &AutomationRule) -> Result<(), AppError> {
        if let RuleTrigger::Schedule { cron_expr, .. } = &rule.trigger {
            scheduler::validate_cron(cron_expr)?;
            let key = Self::cron_key(&rule.id);
            self.cron.add_schedule(key, cron_expr).map_err(|e| AppError::State {
                message: format!("Cannot schedule rule '{}': {e}", rule.name),
                advice: "Check the cron expression.".to_string(),
            })?;
            if let Ok(mut ids) = self.cron_ids.lock() {
                ids.insert(key, rule.id.clone());
            }
            tracing::info!("Scheduled automation rule '{}' ({cron_expr})", rule.name);
            return Ok(());
        }

        let watch_path = match &rule.trigger {
            RuleTrigger::FileAppears { watch_path, .. } => Some(watch_path.clone()),
            RuleTrigger::FileModified { watch_path, .. } => Some(watch_path.clone()),
            RuleTrigger::FileDeleted { watch_path, .. } => Some(watch_path.clone()),
            RuleTrigger::Schedule { .. } | RuleTrigger::Manual => None,
        };

        let Some(path) = watch_path else {
            return Ok(());
        };

        // Check watcher limit
        let watchers = self.active_watchers.read().await;
        if watchers.len() >= MAX_WATCHERS {
            return Err(AppError::State {
                message: format!("Maximum watcher limit ({MAX_WATCHERS}) reached"),
                advice: "Disable some automation rules to free up watchers.".to_string(),
            });
        }
        if watchers.contains_key(&rule.id) {
            return Ok(()); // already watching
        }
        drop(watchers);

        let recursive = match &rule.trigger {
            RuleTrigger::FileAppears { recursive, .. }
            | RuleTrigger::FileModified { recursive, .. }
            | RuleTrigger::FileDeleted { recursive, .. } => *recursive,
            _ => false,
        };

        let handle = scheduler::start_file_watcher(
            rule.id.clone(),
            path,
            recursive,
            rule.trigger.clone(),
            self.exec_semaphore.clone(),
            self.cooldowns.clone(),
            self.fire_context(),
        )?;

        self.active_watchers.write().await.insert(rule.id.clone(), handle);
        tracing::info!("Started watcher for automation rule '{}'", rule.name);
        Ok(())
    }

    /// Stop a rule's watcher and cron schedule.
    async fn stop_watcher(&self, rule_id: &str) {
        let key = Self::cron_key(rule_id);
        let _ = self.cron.remove_schedule(key);
        if let Ok(mut ids) = self.cron_ids.lock() {
            ids.remove(&key);
        }
        if self.active_watchers.write().await.remove(rule_id).is_some() {
            tracing::info!("Stopped watcher for automation rule '{}'", rule_id);
        }
    }

    /// Stop all watchers and schedules.
    pub async fn stop_all(&self) {
        self.active_watchers.write().await.clear();
        let keys: Vec<Uuid> = self.cron_ids.lock().map(|m| m.keys().copied().collect()).unwrap_or_default();
        for key in keys {
            let _ = self.cron.remove_schedule(key);
        }
        if let Ok(mut ids) = self.cron_ids.lock() {
            ids.clear();
        }
        tracing::info!("Stopped all automation watchers");
    }

    /// Manually trigger a rule (for Manual triggers or testing).
    pub async fn run_rule(&self, repo: &crate::storage::Repository, rule_id: &str) -> Result<AutomationLog, AppError> {
        self.fire_context().fire(rule_id, None, EVENT_MANUAL, Some(repo)).await
    }

    /// Dry-run a rule without executing the action.
    pub async fn test_rule(&self, rule: &AutomationRule) -> Result<AutomationLog, AppError> {
        executor::execute_rule_action(rule, None, true).await;

        Ok(AutomationLog {
            id: Uuid::new_v4().to_string(),
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            triggered_at: Utc::now().to_rfc3339(),
            completed_at: Some(Utc::now().to_rfc3339()),
            trigger_event: "dry_run".to_string(),
            status: AutomationStatus::Success,
            files_affected: vec![],
            error_message: None,
        })
    }

    /// List automation logs, optionally filtered by rule ID.
    pub async fn list_logs(
        &self,
        repo: &crate::storage::Repository,
        rule_id: Option<&str>,
        limit: u32,
    ) -> Result<Vec<AutomationLog>, AppError> {
        let rid = rule_id.map(|s| s.to_string());
        let limit = limit as i64;

        repo.pool().execute(move |conn| {
            if let Some(ref rid) = rid {
                let mut stmt = conn.prepare(
                    "SELECT id, rule_id, rule_name, triggered_at, completed_at, trigger_event, status, files_affected, error_message \
                     FROM automation_logs WHERE rule_id = ?1 ORDER BY triggered_at DESC LIMIT ?2"
                )?;
                let logs: Vec<AutomationLog> = stmt.query_map(rusqlite::params![rid, limit], parse_log_row)?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(logs)
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, rule_id, rule_name, triggered_at, completed_at, trigger_event, status, files_affected, error_message \
                     FROM automation_logs ORDER BY triggered_at DESC LIMIT ?1"
                )?;
                let logs: Vec<AutomationLog> = stmt.query_map(rusqlite::params![limit], parse_log_row)?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(logs)
            }
        }).await
    }

    /// Clear logs, optionally for a specific rule.
    pub async fn clear_logs(&self, repo: &crate::storage::Repository, rule_id: Option<&str>) -> Result<(), AppError> {
        let rid = rule_id.map(|s| s.to_string());
        repo.pool().execute(move |conn| {
            if let Some(ref rid) = rid {
                conn.execute("DELETE FROM automation_logs WHERE rule_id = ?1", rusqlite::params![rid])?;
            } else {
                conn.execute("DELETE FROM automation_logs", [])?;
            }
            Ok(())
        }).await
    }
}

/// Parse a log row from SQLite into an AutomationLog.
fn parse_log_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<AutomationLog> {
    let status_str: String = row.get(6)?;
    let files_json: String = row.get(7)?;
    Ok(AutomationLog {
        id: row.get(0)?,
        rule_id: row.get(1)?,
        rule_name: row.get(2)?,
        triggered_at: row.get(3)?,
        completed_at: row.get(4)?,
        trigger_event: row.get(5)?,
        status: match status_str.as_str() {
            "success" => AutomationStatus::Success,
            "failed" => AutomationStatus::Failed,
            "skipped" => AutomationStatus::Skipped,
            "running" => AutomationStatus::Running,
            _ => AutomationStatus::Failed,
        },
        files_affected: serde_json::from_str(&files_json).unwrap_or_default(),
        error_message: row.get(8)?,
    })
}

impl Default for AutomationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation_engine::services::mock::RecordingServices;
    use crate::storage::Repository;
    use std::time::Duration;

    async fn manager() -> (AutomationManager, Repository) {
        let repo = Repository::open_in_memory().await.unwrap();
        let mgr = AutomationManager::new();
        mgr.set_repo(repo.clone());
        (mgr, repo)
    }

    async fn wait_for<F, Fut>(mut check: F) -> bool
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = bool>,
    {
        let deadline = std::time::Instant::now() + Duration::from_secs(8);
        while std::time::Instant::now() < deadline {
            if check().await {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        false
    }

    #[tokio::test]
    async fn schedule_rules_are_armed_and_disarmed() {
        let (mgr, repo) = manager().await;
        let rule = AutomationRule::new(
            "nightly".into(),
            RuleTrigger::Schedule { cron_expr: "0 0 2 * * * *".into(), cron_human: None },
            RuleAction::CreateFolder { path: "/tmp/never".into() },
        );
        mgr.save_rule(&repo, &rule).await.unwrap();
        assert!(mgr.next_scheduled_run(&rule.id).is_none(), "disabled rules are not scheduled");

        mgr.set_enabled(&repo, &rule.id, true).await.unwrap();
        assert!(mgr.next_scheduled_run(&rule.id).is_some(), "enabling arms the cron schedule");

        mgr.set_enabled(&repo, &rule.id, false).await.unwrap();
        assert!(mgr.next_scheduled_run(&rule.id).is_none(), "disabling removes it");
    }

    #[tokio::test]
    async fn invalid_cron_is_rejected_on_enable() {
        let (mgr, repo) = manager().await;
        let rule = AutomationRule::new(
            "bad".into(),
            RuleTrigger::Schedule { cron_expr: "every tuesday".into(), cron_human: None },
            RuleAction::CreateFolder { path: "/tmp/x".into() },
        );
        mgr.save_rule(&repo, &rule).await.unwrap();
        assert!(mgr.set_enabled(&repo, &rule.id, true).await.is_err());
    }

    /// Regression: cron rules used to validate but never run.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn scheduled_rule_fires_and_persists_history() {
        let (mgr, repo) = manager().await;
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("made-by-cron");
        let mut rule = AutomationRule::new(
            "every second".into(),
            RuleTrigger::Schedule { cron_expr: "* * * * * * *".into(), cron_human: None },
            RuleAction::CreateFolder { path: target.to_string_lossy().to_string() },
        );
        rule.enabled = true;
        mgr.save_rule(&repo, &rule).await.unwrap();
        mgr.rearm(&rule).await.unwrap();
        mgr.cron.start_with_interval(scheduler_callback(&mgr), Duration::from_millis(100));

        assert!(wait_for(|| async { target.is_dir() }).await, "cron rule never ran");
        let logged = wait_for(|| async {
            mgr.list_logs(&repo, Some(&rule.id), 10).await.unwrap()
                .iter().any(|l| l.trigger_event == EVENT_SCHEDULE && l.status == AutomationStatus::Success)
        }).await;
        mgr.cron.stop();
        assert!(logged, "scheduled run missing from persisted history");
    }

    /// The same callback `start_scheduler` installs, built on this runtime.
    fn scheduler_callback(mgr: &AutomationManager) -> crate::sync_engine::scheduler::ScheduleCallback {
        let handle = tokio::runtime::Handle::current();
        let ctx = mgr.fire_context();
        let ids = mgr.cron_ids.clone();
        Arc::new(move |key: Uuid| {
            let Some(rule_id) = ids.lock().unwrap().get(&key).cloned() else { return };
            let ctx = ctx.clone();
            handle.spawn(async move { ctx.fire_if_enabled(&rule_id, None, EVENT_SCHEDULE).await; });
        })
    }

    /// Regression: watcher-driven runs were kept in memory only, so they
    /// never appeared in the run history after a restart.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn watcher_runs_are_persisted_and_use_current_action() {
        let (mgr, repo) = manager().await;
        let watched = tempfile::tempdir().unwrap();
        let out_a = tempfile::tempdir().unwrap();
        let out_b = tempfile::tempdir().unwrap();
        let mut rule = AutomationRule::new(
            "copy pdfs".into(),
            RuleTrigger::FileAppears { watch_path: watched.path().to_string_lossy().to_string(), recursive: false },
            RuleAction::CopyFile { dest_path: out_a.path().to_string_lossy().to_string() },
        );
        mgr.save_rule(&repo, &rule).await.unwrap();
        mgr.set_enabled(&repo, &rule.id, true).await.unwrap();

        // Edit the action after the watcher started: the new one must apply.
        rule.enabled = true;
        rule.action = RuleAction::CopyFile { dest_path: out_b.path().to_string_lossy().to_string() };
        mgr.save_rule(&repo, &rule).await.unwrap();

        tokio::time::sleep(Duration::from_millis(300)).await;
        std::fs::write(watched.path().join("doc.pdf"), b"%PDF").unwrap();

        assert!(wait_for(|| async { out_b.path().join("doc.pdf").exists() }).await, "watcher did not run the current action");
        assert!(!out_a.path().join("doc.pdf").exists(), "stale action ran");
        // The run is recorded right after the action completes.
        let persisted = wait_for(|| async {
            mgr.list_logs(&repo, Some(&rule.id), 10).await.unwrap().iter().any(|l| l.trigger_event.starts_with("file: "))
        })
        .await;
        mgr.stop_all().await;
        assert!(persisted, "watcher run not persisted");
    }

    #[tokio::test]
    async fn repeated_failures_auto_disable_on_every_path() {
        let (mgr, repo) = manager().await;
        let mut rule = AutomationRule::new(
            "always fails".into(),
            RuleTrigger::Manual,
            RuleAction::RunSync { sync_pair_id: "missing".into() },
        );
        rule.enabled = true;
        mgr.save_rule(&repo, &rule).await.unwrap();
        mgr.set_services(Arc::new(RecordingServices { fail_with: Some("no such pair".into()), ..Default::default() }));
        let ctx = mgr.fire_context();
        for _ in 0..MAX_CONSECUTIVE_FAILURES {
            ctx.fire_if_enabled(&rule.id, None, EVENT_SCHEDULE).await;
        }
        let r = mgr.get_rule(&rule.id).await.unwrap();
        assert!(!r.enabled);
        assert_eq!(r.last_error.as_deref(), Some("no such pair"));
        // Disabled rules no longer fire from autonomous triggers.
        assert!(ctx.fire_if_enabled(&rule.id, None, EVENT_SCHEDULE).await.is_none());
    }

    #[tokio::test]
    async fn manual_run_uses_attached_services() {
        let (mgr, repo) = manager().await;
        let rule = AutomationRule::new(
            "sync now".into(),
            RuleTrigger::Manual,
            RuleAction::RunSync { sync_pair_id: "p".into() },
        );
        mgr.save_rule(&repo, &rule).await.unwrap();
        let svc = Arc::new(RecordingServices::default());
        mgr.set_services(svc.clone());
        let log = mgr.run_rule(&repo, &rule.id).await.unwrap();
        assert_eq!(log.status, AutomationStatus::Success);
        assert_eq!(log.trigger_event, EVENT_MANUAL);
        assert_eq!(svc.calls(), vec!["sync:p".to_string()]);
        assert_eq!(mgr.list_logs(&repo, Some(&rule.id), 5).await.unwrap().len(), 1);
    }

    /// A row this build can't parse loads disabled instead of turning into
    /// a different action.
    #[tokio::test]
    async fn unreadable_rows_load_disabled() {
        let (mgr, repo) = manager().await;
        repo.pool().execute(|conn| {
            conn.execute(
                "INSERT INTO automation_rules (id, name, enabled, trigger_json, conditions_json, action_json, \
                 created_at, updated_at, trigger_count, error_count) \
                 VALUES ('r1', 'future', 1, '{\"type\":\"manual\"}', '[]', '{\"type\":\"teleport\"}', 'now', 'now', 0, 0)",
                [],
            )?;
            Ok(())
        }).await.unwrap();
        mgr.load_rules_from_db(&repo).await.unwrap();
        let r = mgr.get_rule("r1").await.unwrap();
        assert!(!r.enabled);
        assert!(r.last_error.unwrap().contains("Unreadable action"));
    }
}
