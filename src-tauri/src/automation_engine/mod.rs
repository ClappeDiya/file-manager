//! Automation engine - trigger → condition → action rules for file workflows.
//!
//! Provides `AutomationManager` for creating and running "Quickflow" rules:
//! - Filesystem watchers (file appears, modified, deleted)
//! - Cron-based scheduling
//! - Manual triggers
//! - Condition evaluation (extension, name pattern, size, age)
//! - Action dispatch to existing engines (fs, transfer, sync, archive)
//! - Execution logging and safety guards

pub mod download;
pub mod evaluator;
pub mod executor;
pub mod malware;
pub mod outputs;
pub mod pinner;
pub mod risk;
pub mod scheduler;
pub mod services;
pub mod suggester;
pub mod tauri_services;
pub mod validate;
pub mod webhook;
pub mod workflow;

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
    /// Name under which `action`'s result is saved for later steps
    /// (`{steps.<name>.path}`…); each further step names its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_as: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub last_triggered: Option<String>,
    pub trigger_count: u64,
    pub error_count: u64,
    pub last_error: Option<String>,
    /// AND/OR/NOT condition tree; when set it replaces `conditions`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_tree: Option<ConditionNode>,
    /// Further actions run after `action`, in order (also how one rule
    /// delivers to several destinations).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<ActionStep>,
    #[serde(default)]
    pub options: RuleOptions,
    /// When set, the rule runs this graph instead of `action` + `steps`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workflow: Option<workflow::Workflow>,
    /// Organization that owns the rule (pushed from the console; read-only here).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_by: Option<String>,
}

/// A rule from its portable definition (what "Copy as JSON" exports): the
/// trigger, conditions and actions, with identity and run stats optional.
pub fn rule_from_definition(definition: &serde_json::Value) -> Result<AutomationRule, serde_json::Error> {
    let mut v = definition.clone();
    if let Some(o) = v.as_object_mut() {
        let now = Utc::now().to_rfc3339();
        for (k, d) in [("id", "".into()), ("name", "".into()), ("enabled", true.into()), ("conditions", serde_json::json!([])), ("created_at", now.clone().into()), ("updated_at", now.clone().into()), ("last_triggered", serde_json::Value::Null), ("trigger_count", 0.into()), ("error_count", 0.into()), ("last_error", serde_json::Value::Null)] {
            o.entry(k).or_insert(d);
        }
    }
    serde_json::from_value(v)
}

fn default_ai_suffix() -> String {
    "ai.md".into()
}

fn default_ai_chars() -> usize {
    20_000
}

fn default_fail_on_invalid() -> bool {
    true
}

/// The parts of a rule stored in `automation_rules.extra_json`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct RuleExtra {
    #[serde(default)]
    output_as: Option<String>,
    #[serde(default)]
    condition_tree: Option<ConditionNode>,
    #[serde(default)]
    steps: Vec<ActionStep>,
    #[serde(default)]
    options: RuleOptions,
    #[serde(default)]
    workflow: Option<workflow::Workflow>,
    #[serde(default)]
    managed_by: Option<String>,
}

/// One extra action of a multi-step rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionStep {
    pub action: RuleAction,
    /// Keep going with later steps if this one fails.
    #[serde(default)]
    pub continue_on_error: bool,
    /// Name under which the step's result is saved for later steps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_as: Option<String>,
}

/// A boolean tree of conditions.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ConditionNode {
    All { nodes: Vec<ConditionNode> },
    Any { nodes: Vec<ConditionNode> },
    Not { node: Box<ConditionNode> },
    Condition { condition: RuleCondition },
}

/// Which files a rule without a triggering file (schedule, manual, webhook)
/// works on.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceSelection {
    pub path: String,
    #[serde(default)]
    pub recursive: bool,
    /// Only the N most recently modified matching files.
    #[serde(default)]
    pub newest: Option<usize>,
    /// Include folders as well as files (e.g. to compress them).
    #[serde(default)]
    pub include_dirs: bool,
}

fn default_overwrite() -> crate::core::types::ConflictPolicy {
    crate::core::types::ConflictPolicy::Rename
}

/// How a rule runs. Every field defaults to the behaviour rules had before
/// it existed, so stored rules are unaffected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RuleOptions {
    /// What a copy/move/extract does when the target exists.
    #[serde(default = "default_overwrite")]
    pub overwrite: crate::core::types::ConflictPolicy,
    /// Retries for transient failures (network, locked files).
    #[serde(default = "no_retry")]
    pub retry: ufop_core::retry::RetryPolicy,
    /// Lower runs first when several rules fire for the same event or tick.
    #[serde(default)]
    pub priority: i32,
    /// Let a new run start while the previous one is still going.
    #[serde(default)]
    pub allow_overlap: bool,
    /// For file triggers: wait until the file has not changed for this many
    /// seconds before acting (a file still being written is not touched).
    #[serde(default)]
    pub settle_secs: u64,
    /// Files to act on when nothing triggered with a file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<SourceSelection>,
    /// Approval of the exact command a `RunCommand` step runs (hash of its
    /// program and arguments), set only through `approve_automation_command`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_approval: Option<String>,
    /// SHA-256 of the secret in this rule's webhook URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub webhook_secret_hash: Option<String>,
    /// Where the result of a run is reported (a notification profile).
    #[serde(flatten)]
    pub notify: ufop_core::run_notify::RunNotify,
}

fn no_retry() -> ufop_core::retry::RetryPolicy {
    ufop_core::retry::RetryPolicy::NONE
}

impl Default for RuleOptions {
    fn default() -> Self {
        Self {
            overwrite: default_overwrite(),
            retry: no_retry(),
            priority: 0,
            allow_overlap: false,
            settle_secs: 0,
            source: None,
            command_approval: None,
            webhook_secret_hash: None,
            notify: Default::default(),
        }
    }
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
            output_as: None,
            created_at: now.clone(),
            updated_at: now,
            last_triggered: None,
            trigger_count: 0,
            error_count: 0,
            last_error: None,
            condition_tree: None,
            steps: Vec::new(),
            options: RuleOptions::default(),
            workflow: None,
            managed_by: None,
        }
    }

    /// Everything that can be checked before the rule runs: its graph, its
    /// notification settings and the step results its templates use.
    pub fn validate(&self) -> Result<(), AppError> {
        if let Some(wf) = &self.workflow {
            wf.validate().map_err(AppError::validation)?;
        }
        self.options.notify.validate().map_err(AppError::validation)?;
        outputs::validate_rule(self).map_err(AppError::validation)
    }

    /// Every action the rule can run: its workflow's action nodes, or
    /// `action` + `steps` (for approvals and checks that must see them all).
    pub fn all_actions(&self) -> Vec<&RuleAction> {
        match &self.workflow {
            Some(wf) => wf
                .nodes
                .iter()
                .filter_map(|n| match &n.kind {
                    workflow::NodeKind::Action { action } => Some(action.as_ref()),
                    _ => None,
                })
                .collect(),
            None => self.all_steps().into_iter().map(|(a, _)| a).collect(),
        }
    }

    /// Every action of the rule in order, with its continue-on-error flag.
    pub fn all_steps(&self) -> Vec<(&RuleAction, bool)> {
        std::iter::once((&self.action, false))
            .chain(self.steps.iter().map(|s| (&s.action, s.continue_on_error)))
            .collect()
    }
}

/// What triggers the rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// An HTTP request to the rule's webhook URL (see `webhook`).
    Webhook,
}

/// Optional conditions that must ALL match (AND logic).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// File was last modified more than N seconds ago (expiry rules).
    OlderThan { seconds: u64 },
    /// File was modified within the last N seconds.
    ModifiedWithin { seconds: u64 },
    /// File name matches a regular expression; captures feed `{1}`… tokens.
    NameRegex { pattern: String },
}

/// What action to perform when triggered and conditions match.
// Rule configuration (a handful per rule, cloned and serialized), not hot
// data: boxing the notification would change every place that builds one.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// Extract an archive (zip, tar, tar.gz, 7z) into a folder.
    ExtractArchive {
        dest_path: String,
        /// Refuse archives with more members than this (zip bombs).
        #[serde(default = "default_max_entries")]
        max_entries: usize,
        /// Delete the archive after a successful extraction.
        #[serde(default)]
        delete_archive: bool,
    },
    /// Run a program directly (no shell) with templated arguments.
    RunCommand {
        program: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        working_dir: Option<String>,
        #[serde(default = "default_command_timeout")]
        timeout_secs: u64,
    },
    /// Send a message to email, Slack, Microsoft Teams, Telegram or XMPP
    /// (title and body are templates). The channel is configured like a
    /// backup notification; its secret stays in the credential store.
    SendMessage {
        notification: crate::backup_engine::notify::Notification,
        title: String,
        body: String,
    },
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
    /// Resize an image and/or change its format (`convert_engine`). The
    /// result is written to `dest_path` (a folder template; empty = next to
    /// the file) and is the file later steps work on.
    ConvertImage {
        /// `jpeg`, `png`, `webp`, `gif`, `bmp`, `tiff`; empty = keep.
        #[serde(default)]
        format: Option<String>,
        #[serde(default)]
        max_width: Option<u32>,
        #[serde(default)]
        max_height: Option<u32>,
        #[serde(default)]
        quality: Option<u8>,
        #[serde(default)]
        dest_path: String,
    },
    /// Convert an office document with LibreOffice (e.g. to PDF); like
    /// `ConvertImage`, the result is handed on.
    ConvertDocument {
        to: String,
        #[serde(default)]
        dest_path: String,
    },
    /// Check a delivered batch (`automation_engine::validate`): the files in
    /// `folder` (a template; empty = the triggering file's folder) against
    /// member and content rules. Writes a report into `report_folder` when
    /// set, and fails the step when the batch is invalid unless
    /// `fail_on_invalid` is off.
    ValidateBatch {
        #[serde(default)]
        folder: String,
        rules: validate::BatchRules,
        #[serde(default)]
        report_folder: Option<String>,
        #[serde(default = "default_fail_on_invalid")]
        fail_on_invalid: bool,
    },
    /// Ask the local AI model to process the file (summarize, classify,
    /// extract…): `instruction` (a template) is applied to the file's text
    /// (at most `max_chars`) and the answer saved as `<file name>.<suffix>`
    /// in `dest_path` (a folder template; empty = next to the file). Needs
    /// AI and content analysis allowed in the AI settings.
    AiTask {
        instruction: String,
        #[serde(default)]
        dest_path: String,
        #[serde(default = "default_ai_suffix")]
        suffix: String,
        #[serde(default = "default_ai_chars")]
        max_chars: usize,
    },
    /// Scan the file with ClamAV (`automation_engine::malware`). A clean
    /// file goes on to later steps; an infected one is moved to
    /// `quarantine_path` (a folder template), a notification is shown when
    /// `notify`, and the step fails so nothing else touches it.
    ScanForMalware {
        quarantine_path: String,
        #[serde(default = "default_fail_on_invalid")]
        notify: bool,
    },
    /// Download a URL into a folder (`automation_engine::download`). URL,
    /// folder and file name are templates.
    DownloadUrl {
        url: String,
        dest_path: String,
        /// File name (default: from the server or the URL).
        #[serde(default)]
        file_name: Option<String>,
        /// Replace an existing file instead of adding a number.
        #[serde(default)]
        overwrite: bool,
        #[serde(default = "default_download_max_mb")]
        max_mb: u64,
        /// Expected SHA-256 (hex; templates allowed).
        #[serde(default)]
        sha256: Option<String>,
        /// Allow loopback, private and link-local addresses (intranet servers).
        #[serde(default)]
        allow_private: bool,
        /// Send the download straight to a saved connection instead of a
        /// local folder: it is fetched to a temporary file, uploaded with the
        /// same path as `TransferToConnector`, and the temporary file removed.
        #[serde(default)]
        destination_connection_id: Option<String>,
        /// The connection's protocol (as for `TransferToConnector`).
        #[serde(default)]
        destination_protocol: Option<String>,
        /// Remote folder (a template) the file is uploaded into.
        #[serde(default)]
        remote_path: Option<String>,
    },
    /// Compare a sync pair's source and destination (`verify_sync`) and fail
    /// the step when files differ; `full` compares contents, not just sizes.
    VerifyPair {
        sync_pair_id: String,
        #[serde(default)]
        full: bool,
    },
    /// Remove empty folders under `path` (a template; empty = the
    /// triggering folder), deepest first. Folders with anything in them stay.
    RemoveEmptyDirs {
        #[serde(default)]
        path: String,
        /// Also remove `path` itself when it ends up empty.
        #[serde(default)]
        remove_root: bool,
    },
    /// Write a listing of the files under `path` (a template; empty = the
    /// triggering folder) to `output`, as CSV or JSON.
    ListFiles {
        #[serde(default)]
        path: String,
        output: String,
        #[serde(default)]
        format: ListFormat,
        #[serde(default)]
        recursive: bool,
        #[serde(default)]
        include_dirs: bool,
        /// Replace an existing listing instead of adding a number.
        #[serde(default = "default_true")]
        overwrite: bool,
    },
    /// Remove a folder and everything in it. High risk: it refuses paths
    /// near a drive root or home folder, and the safety interlock refuses a
    /// purge far larger than usual.
    Purge {
        path: String,
        /// Refuse when the folder holds more files than this.
        #[serde(default)]
        max_files: Option<u64>,
    },
}

/// Format of a `ListFiles` listing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListFormat {
    #[default]
    Csv,
    Json,
}

fn default_true() -> bool {
    true
}

fn default_download_max_mb() -> u64 {
    1024
}

fn default_max_entries() -> usize {
    10_000
}

fn default_command_timeout() -> u64 {
    300
}

/// Outcome of one step of one run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StepResult {
    pub step: usize,
    pub action: String,
    pub file: Option<String>,
    pub ok: bool,
    pub attempts: u32,
    pub message: Option<String>,
    /// Workflow step id (workflow runs), for drawing the run on the graph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
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
    /// Attempts the run took (more than 1 when retried).
    #[serde(default = "one")]
    pub attempts: u32,
    /// Per-step, per-file results.
    #[serde(default)]
    pub steps: Vec<StepResult>,
}

fn one() -> u32 {
    1
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

/// Approval string covering every `RunCommand` step of `rule` (comma-joined
/// fingerprints), or `None` if it runs no commands.
pub fn command_approvals(rule: &AutomationRule) -> Option<String> {
    let fps: Vec<String> = rule
        .all_actions()
        .into_iter()
        .filter_map(|a| match a {
            RuleAction::RunCommand { program, args, working_dir, .. } => Some(executor::command_fingerprint(program, args, working_dir.as_deref())),
            _ => None,
        })
        .collect();
    (!fps.is_empty()).then(|| fps.join(","))
}

// ── Manager ──

/// Revisions kept per rule.
const MAX_REVISIONS: i64 = 50;

/// An earlier version of a rule.
#[derive(Debug, Clone, Serialize)]
pub struct RuleRevision {
    pub revision: i64,
    pub saved_at: String,
    pub name: String,
    pub workflow: bool,
    pub actions: usize,
    pub rule: AutomationRule,
}

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
    /// Per-rule run locks: runs of one rule are serialized in trigger
    /// order unless it allows overlap (tokio's Mutex is FIFO-fair).
    locks: Arc<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
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

    fn lock_for(&self, rule_id: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self.locks.lock().unwrap_or_else(|e| e.into_inner());
        locks.entry(rule_id.to_string()).or_default().clone()
    }

    /// Record a run that was decided without executing (e.g. a file that
    /// never stopped changing), so the user sees why nothing happened.
    pub(crate) async fn record_skip(&self, rule_id: &str, file: Option<&str>, event: &str, reason: &str) {
        let Some(rule) = self.rule(rule_id).await else { return };
        let now = Utc::now().to_rfc3339();
        let log = AutomationLog {
            id: Uuid::new_v4().to_string(),
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            triggered_at: now.clone(),
            completed_at: Some(now),
            trigger_event: match file {
                Some(f) => format!("{event}: {f}"),
                None => event.to_string(),
            },
            status: AutomationStatus::Skipped,
            files_affected: file.map(|f| vec![f.to_string()]).unwrap_or_default(),
            error_message: Some(reason.to_string()),
            attempts: 0,
            steps: vec![],
        };
        if let Some(repo) = self.repo() {
            if let Err(e) = db_save_log(&repo, &log).await {
                tracing::warn!("Failed to persist automation run history: {e}");
            }
        }
        self.logs.write().await.push(log);
    }

    /// [`fire`](Self::fire) with extra template values (webhook payload).
    pub(crate) async fn fire(
        &self,
        rule_id: &str,
        triggered_file: Option<&str>,
        event: &str,
        repo: Option<&crate::storage::Repository>,
    ) -> Result<AutomationLog, AppError> {
        self.fire_with(rule_id, triggered_file, event, repo, HashMap::new()).await
    }

    /// Execute a rule and record the outcome everywhere: rule stats (with
    /// auto-disable after repeated failures), SQLite history, the in-memory
    /// log and the operation ledger. `repo` overrides the attached repository.
    pub(crate) async fn fire_with(
        &self,
        rule_id: &str,
        triggered_file: Option<&str>,
        event: &str,
        repo: Option<&crate::storage::Repository>,
        extra: HashMap<String, String>,
    ) -> Result<AutomationLog, AppError> {
        let rule = self.rule(rule_id).await.ok_or_else(|| AppError::State {
            message: format!("Automation rule '{}' not found", rule_id),
            advice: "Check the rule ID.".to_string(),
        })?;

        let lock = self.lock_for(rule_id);
        let _serial = if rule.options.allow_overlap { None } else { Some(lock.lock().await) };
        // The rule may have changed while this run waited its turn.
        let rule = self.rule(rule_id).await.unwrap_or(rule);

        let services = self.services();
        let mut log = executor::execute_rule(&rule, triggered_file, false, services.as_deref(), extra).await;
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
        // Result notifications (profile channels) are fail-open too.
        if let (Some(services), Some(rule)) = (&services, &updated) {
            services.run_finished(rule, &log).await;
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
        let extra_json = serde_json::to_string(&RuleExtra {
            output_as: rule_clone.output_as.clone(),
            condition_tree: rule_clone.condition_tree.clone(),
            steps: rule_clone.steps.clone(),
            options: rule_clone.options.clone(),
            workflow: rule_clone.workflow.clone(),
            managed_by: rule_clone.managed_by.clone(),
        })
        .unwrap_or_else(|_| "{}".into());

        conn.execute(
            "INSERT INTO automation_rules (id, name, enabled, trigger_json, conditions_json, action_json, \
             created_at, updated_at, last_triggered, trigger_count, error_count, last_error, extra_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) \
             ON CONFLICT(id) DO UPDATE SET \
             name = excluded.name, enabled = excluded.enabled, \
             trigger_json = excluded.trigger_json, conditions_json = excluded.conditions_json, \
             action_json = excluded.action_json, updated_at = excluded.updated_at, \
             last_triggered = excluded.last_triggered, trigger_count = excluded.trigger_count, \
             error_count = excluded.error_count, last_error = excluded.last_error, \
             extra_json = excluded.extra_json",
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
                extra_json,
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

        let steps_json = serde_json::to_string(&log_clone.steps).unwrap_or_else(|_| "[]".into());
        conn.execute(
            "INSERT INTO automation_logs (id, rule_id, rule_name, triggered_at, completed_at, \
             trigger_event, status, files_affected, error_message, attempts, steps_json) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
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
                log_clone.attempts as i64,
                steps_json,
            ],
        )?;
        Ok(())
    }).await
}

/// A watched folder and the rules listening on it.
struct FolderWatch {
    _handle: scheduler::WatchHandle,
    members: Arc<RwLock<Vec<String>>>,
    recursive: bool,
}

/// Key identifying a watched folder (canonical path when resolvable).
fn folder_key(path: &str) -> String {
    std::fs::canonicalize(path).map(|p| p.to_string_lossy().to_string()).unwrap_or_else(|_| path.trim_end_matches(['/', '\\']).to_string())
}

/// Manages automation rules, watchers, schedulers, and execution.
pub struct AutomationManager {
    rules: Arc<RwLock<Vec<AutomationRule>>>,
    logs: Arc<RwLock<Vec<AutomationLog>>>,
    /// One OS watcher per watched folder, shared by every rule on it.
    active_watchers: Arc<RwLock<HashMap<String, FolderWatch>>>,
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
    run_locks: Arc<std::sync::Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>>,
    webhook_addr: Arc<std::sync::RwLock<Option<std::net::SocketAddr>>>,
}

/// Existing files a dry run of a folder-watching rule tries (newest first).
pub const DRY_RUN_SAMPLE: usize = 20;

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
            run_locks: Arc::new(std::sync::Mutex::new(HashMap::new())),
            webhook_addr: Arc::new(std::sync::RwLock::new(None)),
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
            locks: self.run_locks.clone(),
        }
    }

    /// Load rules from SQLite on startup.
    pub async fn load_rules_from_db(&self, repo: &crate::storage::Repository) -> Result<(), AppError> {
        let rules = repo.pool().execute(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, name, enabled, trigger_json, conditions_json, action_json, \
                 created_at, updated_at, last_triggered, trigger_count, error_count, last_error, extra_json \
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
                    let extra_json: String = row.get(12)?;
                    let parsed_extra = serde_json::from_str::<RuleExtra>(&extra_json);
                    let unreadable = match (&parsed_action, &parsed_trigger, &parsed_extra) {
                        (Err(e), _, _) => Some(format!("Unreadable action configuration: {e}")),
                        (_, Err(e), _) => Some(format!("Unreadable trigger configuration: {e}")),
                        (_, _, Err(e)) => Some(format!("Unreadable rule options: {e}")),
                        _ => None,
                    };
                    let extra = parsed_extra.unwrap_or_default();
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
                        output_as: extra.output_as,
                        created_at: row.get(6)?,
                        updated_at: row.get(7)?,
                        last_triggered: row.get(8)?,
                        trigger_count: row.get::<_, i64>(9)? as u64,
                        error_count: row.get::<_, i64>(10)? as u64,
                        last_error: unreadable.or(row.get(11)?),
                        condition_tree: extra.condition_tree,
                        steps: extra.steps,
                        options: extra.options,
                        workflow: extra.workflow,
                        managed_by: extra.managed_by,
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
        rule.validate()?;
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
            conn.execute("DELETE FROM automation_rule_revisions WHERE rule_id = ?1", rusqlite::params![rid])?;
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
        self.start_scheduler_every(std::time::Duration::from_secs(30));
    }

    /// [`start_scheduler`](Self::start_scheduler) with a custom poll interval.
    pub fn start_scheduler_every(&self, interval: std::time::Duration) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else {
            tracing::warn!("Automation scheduler not started: no tokio runtime");
            return;
        };
        let ctx = self.fire_context();
        let ids = self.cron_ids.clone();
        let semaphore = self.exec_semaphore.clone();
        self.cron.start_batch_with_interval(
            Arc::new(move |keys: Vec<Uuid>| {
                let rule_ids: Vec<String> = {
                    let map = ids.lock().unwrap_or_else(|e| e.into_inner());
                    keys.iter().filter_map(|k| map.get(k).cloned()).collect()
                };
                let (ctx, semaphore) = (ctx.clone(), semaphore.clone());
                // Rules due on the same tick run one after another, lowest
                // priority number first.
                handle.spawn(async move {
                    let mut due = Vec::new();
                    for id in rule_ids {
                        if let Some(r) = ctx.rule(&id).await {
                            due.push(r);
                        }
                    }
                    due.sort_by(|a, b| a.options.priority.cmp(&b.options.priority).then_with(|| a.created_at.cmp(&b.created_at)));
                    let Ok(_permit) = semaphore.acquire_owned().await else { return };
                    for rule in due {
                        if let Some(log) = ctx.fire_if_enabled(&rule.id, None, EVENT_SCHEDULE).await {
                            tracing::info!("Scheduled automation '{}' ran: {:?}", log.rule_name, log.status);
                        }
                    }
                });
            }),
            interval,
        );
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
            RuleTrigger::FileAppears { watch_path, .. }
            | RuleTrigger::FileModified { watch_path, .. }
            | RuleTrigger::FileDeleted { watch_path, .. } => watch_path.clone(),
            RuleTrigger::Schedule { .. } | RuleTrigger::Manual | RuleTrigger::Webhook => return Ok(()),
        };
        let recursive = matches!(
            rule.trigger,
            RuleTrigger::FileAppears { recursive: true, .. }
                | RuleTrigger::FileModified { recursive: true, .. }
                | RuleTrigger::FileDeleted { recursive: true, .. }
        );
        let key = folder_key(&watch_path);

        let mut watchers = self.active_watchers.write().await;
        if let Some(existing) = watchers.get(&key) {
            let mut members = existing.members.write().await;
            if !members.contains(&rule.id) {
                members.push(rule.id.clone());
            }
            if existing.recursive || !recursive {
                return Ok(());
            }
        } else if watchers.len() >= MAX_WATCHERS {
            return Err(AppError::State {
                message: format!("Maximum watcher limit ({MAX_WATCHERS} folders) reached"),
                advice: "Disable some automation rules to free up watchers.".to_string(),
            });
        }

        // New folder, or an existing one that now needs a recursive watch:
        // (re)start the OS watcher with the folder's member list.
        let members = match watchers.get(&key) {
            Some(w) => w.members.clone(),
            None => Arc::new(RwLock::new(vec![rule.id.clone()])),
        };
        let recursive = recursive || watchers.get(&key).is_some_and(|w| w.recursive);
        let handle = scheduler::start_folder_watcher(
            watch_path,
            recursive,
            members.clone(),
            self.exec_semaphore.clone(),
            self.cooldowns.clone(),
            self.fire_context(),
        )?;
        watchers.insert(key, FolderWatch { _handle: handle, members, recursive });
        tracing::info!("Watching for automation rule '{}'", rule.name);
        Ok(())
    }

    /// Stop a rule's watcher membership and cron schedule.
    async fn stop_watcher(&self, rule_id: &str) {
        let key = Self::cron_key(rule_id);
        let _ = self.cron.remove_schedule(key);
        if let Ok(mut ids) = self.cron_ids.lock() {
            ids.remove(&key);
        }
        let mut watchers = self.active_watchers.write().await;
        let mut emptied = Vec::new();
        for (folder, w) in watchers.iter() {
            let mut members = w.members.write().await;
            if let Some(i) = members.iter().position(|m| m == rule_id) {
                members.remove(i);
                tracing::info!("Stopped watching for automation rule '{}'", rule_id);
            }
            if members.is_empty() {
                emptied.push(folder.clone());
            }
        }
        for folder in emptied {
            watchers.remove(&folder);
        }
    }

    /// Folders currently watched, with how many rules use each.
    pub async fn watched_folders(&self) -> Vec<(String, usize)> {
        let watchers = self.active_watchers.read().await;
        let mut out = Vec::new();
        for (k, w) in watchers.iter() {
            out.push((k.clone(), w.members.read().await.len()));
        }
        out.sort();
        out
    }

    /// Security fields can't arrive through create/update: command
    /// approval and the webhook secret hash are only ever set by
    /// [`approve_commands`](Self::approve_commands) and
    /// [`rotate_webhook_secret`](Self::rotate_webhook_secret). An edit keeps
    /// the stored approval only while the command lines are unchanged.
    pub async fn sanitize_incoming(&self, rule: &mut AutomationRule) {
        let stored = self.get_rule(&rule.id).await;
        let still_approved = stored.as_ref().and_then(|s| s.options.command_approval.clone()).filter(|a| {
            command_approvals(rule).is_some_and(|wanted| wanted.split(',').all(|fp| a.split(',').any(|x| x == fp)))
        });
        rule.options.command_approval = still_approved;
        rule.options.webhook_secret_hash = stored.and_then(|s| s.options.webhook_secret_hash);
        // Only managed settings make a rule the organization's.
        rule.managed_by = None;
    }

    /// Approve the exact command lines a rule runs (after the user confirmed
    /// them in the UI).
    pub async fn approve_commands(&self, repo: &crate::storage::Repository, rule_id: &str) -> Result<AutomationRule, AppError> {
        let mut rule = self.get_rule(rule_id).await.ok_or_else(|| AppError::not_found(format!("Automation rule '{rule_id}' not found")))?;
        rule.options.command_approval = command_approvals(&rule);
        if rule.options.command_approval.is_none() {
            return Err(AppError::validation("This rule doesn't run any commands"));
        }
        rule.updated_at = Utc::now().to_rfc3339();
        self.save_rule(repo, &rule).await?;
        Ok(rule)
    }

    /// Copy a rule: new id, "(copy)" name, disabled, fresh statistics, and
    /// without the original's command approval or webhook secret.
    pub async fn clone_rule(&self, repo: &crate::storage::Repository, rule_id: &str) -> Result<AutomationRule, AppError> {
        let src = self.get_rule(rule_id).await.ok_or_else(|| AppError::not_found(format!("Automation rule '{rule_id}' not found")))?;
        let mut copy = AutomationRule::new(format!("{} (copy)", src.name), src.trigger.clone(), src.action.clone());
        copy.conditions = src.conditions.clone();
        copy.condition_tree = src.condition_tree.clone();
        copy.steps = src.steps.clone();
        copy.output_as = src.output_as.clone();
        copy.workflow = src.workflow.clone();
        copy.options = RuleOptions { command_approval: None, webhook_secret_hash: None, ..src.options.clone() };
        self.save_rule(repo, &copy).await?;
        Ok(copy)
    }

    /// Refuse changing a rule the organization owns.
    pub async fn ensure_editable(&self, rule_id: &str) -> Result<(), AppError> {
        match self.get_rule(rule_id).await.and_then(|r| r.managed_by) {
            Some(org) => Err(AppError::PermissionDenied {
                message: format!("This rule is managed by {org} and cannot be changed here"),
                advice: "Run it as it is, or duplicate it to make your own version.".into(),
            }),
            None => Ok(()),
        }
    }

    /// Install the organization's workflows (console-managed settings):
    /// add or update each as a read-only rule `org-<id>`, and remove org rules
    /// that are no longer listed. Returns problems with individual workflows
    /// (those are skipped; the others still apply).
    pub async fn apply_org_workflows(&self, repo: &crate::storage::Repository, workflows: &[ufop_core::agent::ManagedWorkflow], org: &str) -> Result<Vec<String>, AppError> {
        let org = if org.trim().is_empty() { "your organization" } else { org.trim() };
        let mut problems = Vec::new();
        let mut wanted = std::collections::HashSet::new();
        for wf in workflows {
            let id = format!("org-{}", wf.id);
            wanted.insert(id.clone());
            if let Err(e) = wf.validate() {
                problems.push(e);
                continue;
            }
            let mut rule = match rule_from_definition(&wf.rule) {
                Ok(r) => r,
                Err(e) => {
                    problems.push(format!("Workflow '{}' could not be read: {e}", wf.name));
                    continue;
                }
            };
            let existing = self.get_rule(&id).await;
            rule.id = id.clone();
            rule.name = wf.name.clone();
            rule.enabled = wf.enabled;
            rule.managed_by = Some(org.to_string());
            rule.options.command_approval = None;
            rule.options.webhook_secret_hash = None;
            if command_approvals(&rule).is_some() {
                problems.push(format!("Workflow '{}' runs a program; organization workflows cannot", wf.name));
                continue;
            }
            if let Some(w) = &rule.workflow {
                if let Err(e) = w.validate() {
                    problems.push(format!("Workflow '{}': {e}", wf.name));
                    continue;
                }
            }
            if let Some(old) = &existing {
                rule.created_at = old.created_at.clone();
                (rule.trigger_count, rule.error_count, rule.last_triggered, rule.last_error) = (old.trigger_count, old.error_count, old.last_triggered.clone(), old.last_error.clone());
                let same = |r: &AutomationRule| serde_json::to_value(AutomationRule { updated_at: String::new(), ..r.clone() }).ok();
                if same(old) == same(&rule) {
                    continue;
                }
                self.record_revision(repo, &id).await?;
            }
            rule.updated_at = Utc::now().to_rfc3339();
            self.save_rule(repo, &rule).await?;
            self.rearm(&rule).await?;
        }
        let gone: Vec<String> = self.rules.read().await.iter().filter(|r| r.managed_by.is_some() && !wanted.contains(&r.id)).map(|r| r.id.clone()).collect();
        for id in gone {
            self.delete_rule(repo, &id).await?;
        }
        Ok(problems)
    }

    /// Keep the stored version of `rule_id` as a revision before an edit
    /// replaces it (the newest `MAX_REVISIONS` are kept).
    pub async fn record_revision(&self, repo: &crate::storage::Repository, rule_id: &str) -> Result<(), AppError> {
        let Some(old) = self.get_rule(rule_id).await else { return Ok(()) };
        let (id, data, at) = (old.id.clone(), serde_json::to_string(&old)?, old.updated_at.clone());
        repo.pool()
            .execute(move |c| {
                let next: i64 = c.query_row("SELECT COALESCE(MAX(revision), 0) + 1 FROM automation_rule_revisions WHERE rule_id = ?1", rusqlite::params![id], |r| r.get(0))?;
                c.execute("INSERT INTO automation_rule_revisions (rule_id, revision, data, saved_at) VALUES (?1, ?2, ?3, ?4)", rusqlite::params![id, next, data, at])?;
                c.execute("DELETE FROM automation_rule_revisions WHERE rule_id = ?1 AND revision <= ?2", rusqlite::params![id, next - MAX_REVISIONS])?;
                Ok(())
            })
            .await
    }

    /// Earlier versions of a rule, newest first.
    pub async fn list_revisions(&self, repo: &crate::storage::Repository, rule_id: &str) -> Result<Vec<RuleRevision>, AppError> {
        let id = rule_id.to_string();
        repo.pool()
            .execute(move |c| {
                let mut st = c.prepare("SELECT revision, data, saved_at FROM automation_rule_revisions WHERE rule_id = ?1 ORDER BY revision DESC")?;
                let rows = st
                    .query_map(rusqlite::params![id], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?)))?
                    .filter_map(Result::ok)
                    .filter_map(|(revision, data, saved_at)| {
                        let rule: AutomationRule = serde_json::from_str(&data).ok()?;
                        Some(RuleRevision { revision, saved_at, name: rule.name.clone(), workflow: rule.workflow.is_some(), actions: rule.all_actions().len(), rule })
                    })
                    .collect();
                Ok(rows)
            })
            .await
    }

    /// Put an earlier version back (the current one becomes a revision).
    /// Statistics stay current; command approvals must be given again.
    pub async fn restore_revision(&self, repo: &crate::storage::Repository, rule_id: &str, revision: i64) -> Result<AutomationRule, AppError> {
        let current = self.get_rule(rule_id).await.ok_or_else(|| AppError::not_found(format!("Automation rule '{rule_id}' not found")))?;
        let old = self.list_revisions(repo, rule_id).await?.into_iter().find(|r| r.revision == revision).ok_or_else(|| AppError::not_found(format!("Revision {revision} not found")))?;
        let mut rule = old.rule;
        rule.id = current.id.clone();
        rule.created_at = current.created_at.clone();
        (rule.trigger_count, rule.error_count, rule.last_triggered, rule.last_error) = (current.trigger_count, current.error_count, current.last_triggered.clone(), current.last_error.clone());
        rule.updated_at = Utc::now().to_rfc3339();
        self.sanitize_incoming(&mut rule).await;
        self.record_revision(repo, rule_id).await?;
        self.save_rule(repo, &rule).await?;
        Ok(rule)
    }

    /// Issue a new webhook secret for a rule; returns it once (only its hash
    /// is stored, so it can't be shown again).
    pub async fn rotate_webhook_secret(&self, repo: &crate::storage::Repository, rule_id: &str) -> Result<String, AppError> {
        let mut rule = self.get_rule(rule_id).await.ok_or_else(|| AppError::not_found(format!("Automation rule '{rule_id}' not found")))?;
        if rule.trigger != RuleTrigger::Webhook {
            return Err(AppError::validation("Only webhook-triggered rules have a webhook URL"));
        }
        let secret = webhook::new_secret();
        rule.options.webhook_secret_hash = Some(webhook::hash_secret(&secret));
        rule.updated_at = Utc::now().to_rfc3339();
        self.save_rule(repo, &rule).await?;
        Ok(secret)
    }

    /// Start the webhook listener; returns the bound address.
    /// Start the webhook listener on `addr`. When the default port is held by
    /// another session on this machine (Remote Desktop, shared servers), a
    /// stable per-user port is used instead.
    pub async fn start_webhook_listener(&self, addr: std::net::SocketAddr) -> std::io::Result<std::net::SocketAddr> {
        let bound = match webhook::serve(self.fire_context(), addr).await {
            Err(e) if e.kind() == std::io::ErrorKind::AddrInUse && addr.port() == webhook::DEFAULT_PORT => {
                let alt = std::net::SocketAddr::new(addr.ip(), ufop_core::session::per_user_port(webhook::DEFAULT_PORT, 500));
                tracing::warn!("Webhook port {} is in use (another session?); using {} for this user", addr.port(), alt.port());
                webhook::serve(self.fire_context(), alt).await?
            }
            r => r?,
        };
        if let Ok(mut a) = self.webhook_addr.write() {
            *a = Some(bound);
        }
        tracing::info!("Automation webhooks listening on http://{bound}/hooks/<rule-id>");
        Ok(bound)
    }

    /// Where the webhook listener is bound, if running.
    pub fn webhook_addr(&self) -> Option<std::net::SocketAddr> {
        self.webhook_addr.read().ok().and_then(|a| *a)
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

    /// Dry-run a rule: which files it would act on and which steps would
    /// run, without touching anything.
    ///
    /// A rule without a source selection that watches a folder for new or
    /// changed files is tried against the files already there (the most
    /// recent [`DRY_RUN_SAMPLE`]): each matching file stands for an event
    /// the rule would act on. Nothing is changed, nothing is saved.
    pub async fn test_rule(&self, rule: &AutomationRule) -> Result<AutomationLog, AppError> {
        let mut probe = rule.clone();
        if probe.options.source.is_none() {
            if let RuleTrigger::FileAppears { watch_path, recursive } | RuleTrigger::FileModified { watch_path, recursive } = &rule.trigger {
                if std::path::Path::new(watch_path).is_dir() {
                    probe.options.source = Some(SourceSelection { path: watch_path.clone(), recursive: *recursive, newest: Some(DRY_RUN_SAMPLE), include_dirs: false });
                }
            }
        }
        let mut log = executor::execute_rule_action(&probe, None, true).await;
        log.trigger_event = "dry_run".to_string();
        Ok(log)
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
                    "SELECT id, rule_id, rule_name, triggered_at, completed_at, trigger_event, status, files_affected, error_message, attempts, steps_json \
                     FROM automation_logs WHERE rule_id = ?1 ORDER BY triggered_at DESC LIMIT ?2"
                )?;
                let logs: Vec<AutomationLog> = stmt.query_map(rusqlite::params![rid, limit], parse_log_row)?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(logs)
            } else {
                let mut stmt = conn.prepare(
                    "SELECT id, rule_id, rule_name, triggered_at, completed_at, trigger_event, status, files_affected, error_message, attempts, steps_json \
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
        attempts: row.get::<_, i64>(9)? as u32,
        steps: serde_json::from_str(&row.get::<_, String>(10)?).unwrap_or_default(),
    })
}

impl Default for AutomationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn dry_run_of_an_unsaved_folder_rule_tries_the_files_already_there() {
        let dir = tempfile::tempdir().unwrap();
        for f in ["a.pdf", "b.txt", "c.pdf"] {
            std::fs::write(dir.path().join(f), f).unwrap();
        }
        let mgr = AutomationManager::new();
        let watch = dir.path().to_string_lossy().to_string();
        let mut rule = AutomationRule::new("pdfs".into(), RuleTrigger::FileAppears { watch_path: watch.clone(), recursive: false }, RuleAction::DeleteFile { to_trash: true });
        rule.conditions = vec![RuleCondition::ExtensionIs { extensions: vec!["pdf".into()] }];
        let log = mgr.test_rule(&rule).await.unwrap();
        let mut files = log.files_affected.clone();
        files.sort();
        assert_eq!(files, vec![format!("{watch}/a.pdf"), format!("{watch}/c.pdf")]);
        assert_eq!((log.trigger_event.as_str(), log.status.clone()), ("dry_run", AutomationStatus::Success));
        assert_eq!(log.steps.len(), 2, "one step per sample file");
        // Nothing happened, and the rule was not saved.
        assert!(dir.path().join("a.pdf").exists() && mgr.get_rule(&rule.id).await.is_none());
        // No match: skipped, with the reason.
        rule.conditions = vec![RuleCondition::ExtensionIs { extensions: vec!["docx".into()] }];
        let log = mgr.test_rule(&rule).await.unwrap();
        assert_eq!((log.status, log.error_message.as_deref()), (AutomationStatus::Skipped, Some("No files matched")));
        // A manual rule without a source is a single step-only preview.
        let manual = AutomationRule::new("m".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/x".into() });
        let log = mgr.test_rule(&manual).await.unwrap();
        assert!(log.files_affected.is_empty() && log.steps.len() == 1);
    }

    #[tokio::test]
    async fn org_workflows_install_update_refuse_programs_and_uninstall() {
        use ufop_core::agent::ManagedWorkflow;
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = AutomationManager::new();
        let mine = AutomationRule::new("mine".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/m".into() });
        mgr.save_rule(&repo, &mine).await.unwrap();
        let rule = |p: &str| serde_json::to_value(AutomationRule::new("ignored".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: p.into() })).unwrap();
        let wf = |id: &str, r: serde_json::Value| ManagedWorkflow { id: id.into(), name: format!("Org {id}"), enabled: true, rule: r };
        let cmd = serde_json::to_value(AutomationRule::new("c".into(), RuleTrigger::Manual, RuleAction::RunCommand { program: "sh".into(), args: vec![], working_dir: None, timeout_secs: 5 })).unwrap();

        let problems = mgr.apply_org_workflows(&repo, &[wf("a", rule("/a")), wf("b", rule("/b")), wf("evil", cmd)], "Acme").await.unwrap();
        assert_eq!(problems.len(), 1, "{problems:?}");
        assert!(problems[0].contains("runs a program"));
        let a = mgr.get_rule("org-a").await.unwrap();
        assert_eq!((a.name.as_str(), a.enabled, a.managed_by.as_deref()), ("Org a", true, Some("Acme")));
        assert!(mgr.get_rule("org-evil").await.is_none());
        assert!(mgr.ensure_editable("org-a").await.unwrap_err().to_string().contains("managed by Acme"));
        assert!(mgr.ensure_editable(&mine.id).await.is_ok());

        // An update keeps the run statistics (and the old version as a revision); an unchanged one is not rewritten.
        mgr.rules.write().await.iter_mut().find(|r| r.id == "org-a").unwrap().trigger_count = 7;
        let before = mgr.get_rule("org-b").await.unwrap().updated_at;
        mgr.apply_org_workflows(&repo, &[wf("a", rule("/a2")), wf("b", rule("/b"))], "Acme").await.unwrap();
        let a2 = mgr.get_rule("org-a").await.unwrap();
        assert!(matches!(&a2.action, RuleAction::CreateFolder { path } if path == "/a2"));
        assert_eq!(a2.trigger_count, 7);
        assert_eq!(mgr.list_revisions(&repo, "org-a").await.unwrap().len(), 1);
        assert_eq!(mgr.get_rule("org-b").await.unwrap().updated_at, before);

        // Dropped by the organization: removed; the user's own rule stays.
        mgr.apply_org_workflows(&repo, &[wf("b", rule("/b"))], "Acme").await.unwrap();
        assert!(mgr.get_rule("org-a").await.is_none() && mgr.get_rule("org-b").await.is_some());
        mgr.apply_org_workflows(&repo, &[], "Acme").await.unwrap();
        assert!(mgr.get_rule("org-b").await.is_none());
        assert!(mgr.get_rule(&mine.id).await.is_some());
        // Persisted with its owner.
        mgr.apply_org_workflows(&repo, &[wf("c", rule("/c"))], "").await.unwrap();
        let reloaded = AutomationManager::new();
        reloaded.load_rules_from_db(&repo).await.unwrap();
        assert_eq!(reloaded.get_rule("org-c").await.unwrap().managed_by.as_deref(), Some("your organization"));

        // The portable definition "Copy as JSON" exports (no identity or stats) installs too.
        let portable = serde_json::json!({ "trigger": { "type": "manual" }, "action": { "type": "create_folder", "path": "/p" } });
        assert!(mgr.apply_org_workflows(&repo, &[wf("p", portable)], "Acme").await.unwrap().is_empty());
        let p = mgr.get_rule("org-p").await.unwrap();
        assert_eq!((p.name.as_str(), p.trigger_count, p.conditions.len()), ("Org p", 0, 0));

        // A rule the user sends can't claim an owner, or take an org rule's id.
        let mut spoof = AutomationRule::new("spoof".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/s".into() });
        spoof.managed_by = Some("Acme".into());
        mgr.sanitize_incoming(&mut spoof).await;
        assert!(spoof.managed_by.is_none());
        assert!(mgr.ensure_editable("org-p").await.is_err());
    }

    #[tokio::test]
    async fn a_finished_run_is_reported_to_the_services_for_notifications() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = AutomationManager::new();
        let svc = Arc::new(services::mock::RecordingServices::default());
        mgr.set_services(svc.clone());
        let mut ok = AutomationRule::new("tells".into(), RuleTrigger::Manual, RuleAction::Notify { title: "t".into(), body: "b".into() });
        ok.options.notify = ufop_core::run_notify::RunNotify { notify_profile_id: Some("p".into()), notify_on: vec!["error".into()], min_bytes: Some(5) };
        mgr.save_rule(&repo, &ok).await.unwrap();
        let bad = AutomationRule::new("breaks".into(), RuleTrigger::Manual, RuleAction::RunSync { sync_pair_id: "p".into() });
        mgr.save_rule(&repo, &bad).await.unwrap();
        mgr.run_rule(&repo, &ok.id).await.unwrap();
        assert_eq!(*svc.finished.lock().unwrap(), vec!["tells:Success".to_string()]);
        let failing = Arc::new(services::mock::RecordingServices { fail_with: Some("boom".into()), ..Default::default() });
        mgr.set_services(failing.clone());
        let log = mgr.run_rule(&repo, &bad.id).await.unwrap();
        assert_eq!(log.status, AutomationStatus::Failed);
        assert_eq!(*failing.finished.lock().unwrap(), vec!["breaks:Failed".to_string()]);
        // The notification settings are stored with the rule's options.
        let reloaded = AutomationManager::new();
        reloaded.load_rules_from_db(&repo).await.unwrap();
        assert_eq!(reloaded.get_rule(&ok.id).await.unwrap().options.notify, ok.options.notify);
    }

    #[tokio::test]
    async fn revisions_are_kept_listed_and_restored() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = AutomationManager::new();
        let mut rule = AutomationRule::new("v1".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/a".into() });
        mgr.save_rule(&repo, &rule).await.unwrap();
        for name in ["v2", "v3"] {
            mgr.record_revision(&repo, &rule.id).await.unwrap();
            rule.name = name.into();
            rule.workflow = (name == "v3").then(|| workflow::Workflow::from_rule(&rule));
            mgr.save_rule(&repo, &rule).await.unwrap();
        }
        let revs = mgr.list_revisions(&repo, &rule.id).await.unwrap();
        assert_eq!(revs.iter().map(|r| (r.revision, r.name.as_str(), r.workflow)).collect::<Vec<_>>(), vec![(2, "v2", false), (1, "v1", false)]);
        let back = mgr.restore_revision(&repo, &rule.id, 1).await.unwrap();
        assert_eq!((back.name.as_str(), back.workflow.is_none(), back.id == rule.id), ("v1", true, true));
        assert_eq!(mgr.get_rule(&rule.id).await.unwrap().name, "v1");
        assert_eq!(mgr.list_revisions(&repo, &rule.id).await.unwrap()[0].name, "v3", "the replaced version is kept");
        assert!(mgr.restore_revision(&repo, &rule.id, 99).await.is_err());
        // An invalid workflow is refused on save.
        rule.workflow = Some(workflow::Workflow::default());
        assert!(mgr.save_rule(&repo, &rule).await.is_err());
        mgr.delete_rule(&repo, &rule.id).await.unwrap();
        assert!(mgr.list_revisions(&repo, &rule.id).await.unwrap().is_empty());
    }

    /// Rules exactly as the desktop rule editor emits them (generated by
    /// `src/__tests__/automation-rule-model.test.ts`): every one must load,
    /// and the shapes the editor relies on must come through.
    #[test]
    fn editor_rules_deserialize() {
        let rules: Vec<AutomationRule> =
            serde_json::from_str(include_str!("fixtures/editor-rules.json")).expect("editor JSON matches the engine");
        assert!(rules.len() >= 5);
        // The workflow rule the builder emits.
        let wf_rule = rules.last().unwrap();
        let wf = wf_rule.workflow.as_ref().expect("workflow survives the round trip");
        assert!(wf.validate().is_ok());
        assert_eq!(wf.nodes.len(), 5);
        assert!(matches!(wf.nodes[0].kind, workflow::NodeKind::Condition { .. }));
        assert!(matches!(&wf.nodes[1].kind, workflow::NodeKind::Action { action } if matches!(**action, RuleAction::MoveFile { .. })));
        assert!(matches!(wf.nodes[3].kind, workflow::NodeKind::Join { all: true }));
        assert_eq!(wf.edges.iter().map(|e| e.on).collect::<Vec<_>>(), vec![workflow::EdgeOn::True, workflow::EdgeOn::False, workflow::EdgeOn::Success, workflow::EdgeOn::Always, workflow::EdgeOn::Failure]);
        assert!(matches!(wf_rule.action, RuleAction::MoveFile { .. }), "action = the first action step");
        let last = &rules[rules.len() - 2];
        assert_eq!(last.trigger, RuleTrigger::Webhook);
        assert!(matches!(last.condition_tree, Some(ConditionNode::Any { ref nodes }) if nodes.len() == 2));
        assert_eq!(last.steps.len(), 13);
        // Steps, result names and notification settings added with the Quickflows parity work.
        assert!(matches!(&last.steps[7].action, RuleAction::ListFiles { format: ListFormat::Json, recursive: true, include_dirs: true, overwrite: false, .. }));
        assert_eq!(last.steps[7].output_as.as_deref(), Some("listing"));
        assert!(matches!(&last.steps[8].action, RuleAction::VerifyPair { sync_pair_id, full: true } if sync_pair_id == "pair-1"));
        assert!(matches!(&last.steps[9].action, RuleAction::RemoveEmptyDirs { remove_root: true, .. }));
        assert!(matches!(&last.steps[10].action, RuleAction::Purge { path, max_files: Some(500) } if path == "/in/tmp"));
        assert!(matches!(&last.steps[11].action, RuleAction::DownloadUrl { destination_connection_id: Some(c), destination_protocol: Some(p), remote_path: Some(r), .. } if c == "conn-1" && p == "sftp" && r == "/in/{yyyy}"));
        assert!(outputs::validate_rule(last).is_ok(), "{{steps.listing.*}} resolves to the earlier step");
        assert_eq!(last.options.notify.notify_profile_id.as_deref(), Some("profile-1"));
        assert_eq!(last.options.notify.notify_on, vec!["warning".to_string(), "error".to_string()]);
        assert_eq!(last.options.notify.min_bytes, Some(1_048_576));
        assert!(matches!(&last.steps[6].action, RuleAction::ScanForMalware { quarantine_path, notify: false } if quarantine_path == "/quarantine/{yyyy}"));
        assert!(matches!(&last.steps[5].action, RuleAction::AiTask { instruction, suffix, max_chars: 5000, .. } if instruction == "Classify {filename}" && suffix == "class.txt"));
        assert!(matches!(&last.steps[2].action, RuleAction::ConvertImage { format: Some(f), max_width: Some(800), max_height: None, quality: None, dest_path } if f == "webp" && dest_path == "/in/small"));
        assert!(matches!(&last.steps[3].action, RuleAction::ConvertDocument { to, dest_path } if to == "pdf" && dest_path.is_empty()));
        let RuleAction::ValidateBatch { rules: batch, report_folder: Some(_), fail_on_invalid: true, .. } = &last.steps[4].action else { panic!("validate_batch") };
        assert!(batch.no_extra_files && batch.members[0].max == Some(10) && batch.content.len() == 4);
        assert!(matches!(&batch.content[0].check, validate::ContentCheck::Csv { required_columns, min_rows: Some(1), delimiter: Some(d) } if required_columns.len() == 2 && d == ";"));
        assert!(batch.validate().is_ok());
        assert!(matches!(last.action, RuleAction::SendMessage { .. }));
        assert!(matches!(last.steps[0].action, RuleAction::RunCommand { .. }));
        assert!(matches!(&last.steps[1].action, RuleAction::DownloadUrl { url, overwrite: true, max_mb: 50, allow_private: false, .. } if url.contains("{customer}")));
        assert_eq!(last.options.priority, -1);
        assert_eq!(last.options.source.as_ref().map(|s| s.newest), Some(Some(5)));
        assert!(command_approvals(last).is_some());
        // Round trip: what the engine sends back the editor reads the same.
        for r in &rules {
            let json = serde_json::to_value(r).unwrap();
            let again: AutomationRule = serde_json::from_value(json.clone()).unwrap();
            assert_eq!(serde_json::to_value(&again).unwrap(), json);
        }
    }

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

    // ── Automation v1.5: manager level ──

    /// Rules on one folder run in priority order (lower first) for each file.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn shared_folder_watch_runs_rules_in_priority_order() {
        let (mgr, repo) = manager().await;
        let watched = tempfile::tempdir().unwrap();
        let journal = watched.path().parent().unwrap().join(format!("journal-{}", Uuid::new_v4()));
        let mk = |name: &str, priority: i32| {
            let mut r = AutomationRule::new(
                name.into(),
                RuleTrigger::FileAppears { watch_path: watched.path().to_string_lossy().to_string(), recursive: false },
                RuleAction::RunCommand { program: "sh".into(), args: vec!["-c".into(), format!("echo {name} >> {}", journal.display())], working_dir: None, timeout_secs: 10 },
            );
            r.options.priority = priority;
            r.options.command_approval = command_approvals(&r);
            r
        };
        for r in [mk("third", 9), mk("first", -1), mk("second", 0)] {
            mgr.save_rule(&repo, &r).await.unwrap();
            mgr.set_enabled(&repo, &r.id, true).await.unwrap();
        }
        assert_eq!(mgr.watched_folders().await.len(), 1, "one OS watcher for the folder");
        tokio::time::sleep(Duration::from_millis(300)).await;
        std::fs::write(watched.path().join("a.csv"), b"x").unwrap();
        let done = wait_for(|| async { std::fs::read_to_string(&journal).map(|s| s.lines().count() == 3).unwrap_or(false) }).await;
        mgr.stop_all().await;
        assert!(done, "all three rules ran");
        assert_eq!(std::fs::read_to_string(&journal).unwrap(), "first\nsecond\nthird\n");
        std::fs::remove_file(journal).ok();
    }

    /// Regression: the watcher dropped every event within 2 s of the
    /// previous one, so the second of two files arriving together was missed.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn files_arriving_together_are_all_handled() {
        let (mgr, repo) = manager().await;
        let watched = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let rule = AutomationRule::new(
            "copy all".into(),
            RuleTrigger::FileAppears { watch_path: watched.path().to_string_lossy().to_string(), recursive: false },
            RuleAction::CopyFile { dest_path: out.path().to_string_lossy().to_string() },
        );
        mgr.save_rule(&repo, &rule).await.unwrap();
        mgr.set_enabled(&repo, &rule.id, true).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;
        for i in 0..5 {
            std::fs::write(watched.path().join(format!("f{i}.txt")), b"x").unwrap();
        }
        let all = wait_for(|| async { (0..5).all(|i| out.path().join(format!("f{i}.txt")).exists()) }).await;
        mgr.stop_all().await;
        assert!(all, "every file was copied");
    }

    #[tokio::test]
    async fn runs_of_one_rule_never_overlap_unless_allowed() {
        let (mgr, repo) = manager().await;
        let mut rule = AutomationRule::new(
            "slow".into(),
            RuleTrigger::Manual,
            RuleAction::RunCommand { program: "sleep".into(), args: vec!["0.3".into()], working_dir: None, timeout_secs: 5 },
        );
        rule.options.command_approval = command_approvals(&rule);
        mgr.save_rule(&repo, &rule).await.unwrap();
        let ctx = mgr.fire_context();
        let (a, b) = tokio::join!(ctx.fire(&rule.id, None, EVENT_MANUAL, None), ctx.fire(&rule.id, None, EVENT_MANUAL, None));
        let (a, b) = (a.unwrap(), b.unwrap());
        let (first, second) = if a.triggered_at <= b.triggered_at { (a, b) } else { (b, a) };
        assert!(first.completed_at.as_deref().unwrap() <= second.triggered_at.as_str(), "second run started after the first finished");

        rule.options.allow_overlap = true;
        mgr.save_rule(&repo, &rule).await.unwrap();
        let started = std::time::Instant::now();
        let _ = tokio::join!(ctx.fire(&rule.id, None, EVENT_MANUAL, None), ctx.fire(&rule.id, None, EVENT_MANUAL, None));
        assert!(started.elapsed() < Duration::from_millis(550), "overlapping runs went in parallel");
    }

    #[tokio::test]
    async fn incoming_rules_cannot_forge_approval_or_webhook_secret() {
        let (mgr, repo) = manager().await;
        let mut rule = AutomationRule::new(
            "cmd".into(),
            RuleTrigger::Manual,
            RuleAction::RunCommand { program: "touch".into(), args: vec!["/tmp/x".into()], working_dir: None, timeout_secs: 5 },
        );
        rule.options.command_approval = command_approvals(&rule); // forged by the caller
        rule.options.webhook_secret_hash = Some("forged".into());
        mgr.sanitize_incoming(&mut rule).await;
        assert!(rule.options.command_approval.is_none() && rule.options.webhook_secret_hash.is_none());
        mgr.save_rule(&repo, &rule).await.unwrap();

        let approved = mgr.approve_commands(&repo, &rule.id).await.unwrap();
        assert!(approved.options.command_approval.is_some());
        // Unchanged edit keeps the approval; changing the command drops it.
        let mut same = approved.clone();
        same.name = "renamed".into();
        mgr.sanitize_incoming(&mut same).await;
        assert_eq!(same.options.command_approval, approved.options.command_approval);
        let mut changed = approved.clone();
        changed.action = RuleAction::RunCommand { program: "rm".into(), args: vec!["-rf".into(), "/".into()], working_dir: None, timeout_secs: 5 };
        mgr.sanitize_incoming(&mut changed).await;
        assert!(changed.options.command_approval.is_none());
    }

    #[tokio::test]
    async fn clone_copies_everything_but_identity_and_secrets() {
        let (mgr, repo) = manager().await;
        let mut rule = AutomationRule::new("orig".into(), RuleTrigger::Webhook, RuleAction::CreateFolder { path: "/tmp/{customer}".into() });
        rule.enabled = true;
        rule.trigger_count = 9;
        rule.steps = vec![ActionStep { action: RuleAction::CreateFolder { path: "/tmp/b".into() }, continue_on_error: true, output_as: None }];
        rule.options.priority = 3;
        mgr.save_rule(&repo, &rule).await.unwrap();
        mgr.rotate_webhook_secret(&repo, &rule.id).await.unwrap();
        let copy = mgr.clone_rule(&repo, &rule.id).await.unwrap();
        assert_ne!(copy.id, rule.id);
        assert_eq!(copy.name, "orig (copy)");
        assert!(!copy.enabled && copy.trigger_count == 0);
        assert_eq!((copy.steps.len(), copy.options.priority), (1, 3));
        assert!(copy.options.webhook_secret_hash.is_none());
        // Extras persist across a reload.
        let reloaded = AutomationManager::new();
        reloaded.load_rules_from_db(&repo).await.unwrap();
        let back = reloaded.get_rule(&copy.id).await.unwrap();
        assert_eq!(back.steps, copy.steps);
        assert_eq!(back.options, copy.options);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn webhook_runs_the_rule_with_payload_values() {
        let (mgr, repo) = manager().await;
        let base = tempfile::tempdir().unwrap();
        let mut rule = AutomationRule::new(
            "hook".into(),
            RuleTrigger::Webhook,
            RuleAction::CreateFolder { path: format!("{}/{{customer}}/{{order}}", base.path().display()) },
        );
        rule.enabled = true;
        mgr.save_rule(&repo, &rule).await.unwrap();
        let secret = mgr.rotate_webhook_secret(&repo, &rule.id).await.unwrap();
        let addr = mgr.start_webhook_listener("127.0.0.1:0".parse().unwrap()).await.unwrap();
        let url = format!("http://{addr}/hooks/{}", rule.id);
        // Another session holding the default port: this user's own port is used.
        if let Ok(_held) = std::net::TcpListener::bind(("127.0.0.1", webhook::DEFAULT_PORT)) {
            let (other, _) = manager().await;
            let alt = other.start_webhook_listener(std::net::SocketAddr::from(([127, 0, 0, 1], webhook::DEFAULT_PORT))).await.unwrap();
            assert_eq!(alt.port(), ufop_core::session::per_user_port(webhook::DEFAULT_PORT, 500));
        }
        let http = reqwest::Client::new();

        let denied = http.post(&url).bearer_auth("wrong").body("{}").send().await.unwrap();
        assert_eq!(denied.status(), 401);
        let unknown = http.post(format!("http://{addr}/hooks/nope")).bearer_auth(&secret).send().await.unwrap();
        assert_eq!(unknown.status(), 404);

        let ok = http.post(&url).bearer_auth(&secret).body(r#"{"customer":"acme","order":42}"#).send().await.unwrap();
        assert_eq!(ok.status(), 202);
        let target = base.path().join("acme/42");
        assert!(wait_for(|| async { target.is_dir() }).await, "rule ran with payload values");
        let via_query = http.get(format!("{url}?token={secret}")).send().await.unwrap();
        assert_eq!(via_query.status(), 202);

        // A rotated secret invalidates the old one.
        mgr.rotate_webhook_secret(&repo, &rule.id).await.unwrap();
        assert_eq!(http.post(&url).bearer_auth(&secret).send().await.unwrap().status(), 401);
    }

    /// A file still being written is copied only once it stops changing.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn settle_waits_for_writes_to_finish() {
        use std::io::Write;
        let (mgr, repo) = manager().await;
        let watched = tempfile::tempdir().unwrap();
        let out = tempfile::tempdir().unwrap();
        let mut rule = AutomationRule::new(
            "settled copy".into(),
            RuleTrigger::FileAppears { watch_path: watched.path().to_string_lossy().to_string(), recursive: false },
            RuleAction::CopyFile { dest_path: out.path().to_string_lossy().to_string() },
        );
        rule.options.settle_secs = 1;
        mgr.save_rule(&repo, &rule).await.unwrap();
        mgr.set_enabled(&repo, &rule.id, true).await.unwrap();
        tokio::time::sleep(Duration::from_millis(300)).await;

        let path = watched.path().join("big.bin");
        let writer = std::thread::spawn(move || {
            let mut f = std::fs::File::create(&path).unwrap();
            for _ in 0..6 {
                f.write_all(&[7u8; 4096]).unwrap();
                f.flush().unwrap();
                std::thread::sleep(Duration::from_millis(300));
            }
        });
        let copied = out.path().join("big.bin");
        assert!(wait_for(|| async { copied.exists() }).await);
        writer.join().unwrap();
        mgr.stop_all().await;
        assert_eq!(std::fs::metadata(&copied).unwrap().len(), 6 * 4096, "copied only after the last write");
    }
}
