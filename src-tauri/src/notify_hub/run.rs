//! Result notifications of sync pairs and automation rules through the
//! notification profiles kept in the backup settings.
//!
//! A pair or rule carries a [`RunNotify`]: the profile to send to, which
//! results send (`notify_on`) and an optional size threshold. This module
//! builds the message values for a finished run and hands the profile's
//! channel to the shared dispatcher ([`super::deliver`]), exactly as a
//! backup job's result is delivered.

use super::profiles::NotificationProfile;
use super::Notification;
use crate::automation_engine::AutomationManager;
use crate::backup_engine::{human_bytes, BackupManager};
use crate::core::error::AppError;
use crate::core::traits::{BoxFuture, SyncOperations};
use crate::storage::Repository;
use crate::sync_engine::SyncManager;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;
use ufop_core::run_notify::{RunNotify, RunResult};

/// What a finished run reports. `kind` is `sync` or `automation` and fills
/// `{{operation}}` in message templates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunSummary {
    pub kind: &'static str,
    pub id: String,
    pub name: String,
    pub result: RunResult,
    /// One line: what the run did.
    pub summary: String,
    /// Error lines (empty on success).
    pub errors: Vec<String>,
    pub files: u64,
    /// Bytes moved; `None` when the engine cannot tell.
    pub bytes: Option<u64>,
    pub duration_secs: u64,
}

/// Results of the previous run of each pair/rule in this session, so
/// recovery-aware channels (PagerDuty) know a problem was cleared.
static PREVIOUS: Mutex<Option<HashMap<String, RunResult>>> = Mutex::new(None);

/// Record `run`'s result; returns the one before it.
fn remember(run: &RunSummary) -> Option<RunResult> {
    let mut g = PREVIOUS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(format!("{}:{}", run.kind, run.id), run.result)
}

/// The values a channel's templates can use for `run`.
pub fn vars(run: &RunSummary, previous: Option<RunResult>) -> BTreeMap<String, String> {
    let mut v = BTreeMap::new();
    v.insert("name".into(), run.name.clone());
    v.insert("operation".into(), run.kind.to_string());
    v.insert("result".into(), run.result.as_str().to_string());
    v.insert("machine".into(), crate::backup_engine::hooks::machine_name());
    v.insert("job_id".into(), run.id.clone());
    v.insert("backup_id".into(), run.id.clone());
    v.insert("duration".into(), format!("{}s", run.duration_secs));
    v.insert("files".into(), run.files.to_string());
    v.insert("bytes".into(), run.bytes.map(|b| b.to_string()).unwrap_or_default());
    v.insert("errors".into(), run.errors.join("\n"));
    v.insert("previous_result".into(), previous.map(|p| p.as_str().to_string()).unwrap_or_default());
    v
}

/// The report text: the summary, the size when known, then the errors.
pub fn body(run: &RunSummary) -> String {
    let mut text = format!("{}\n", run.summary);
    if let Some(b) = run.bytes {
        text.push_str(&format!("{} moved in {} file(s), {}s\n", human_bytes(b), run.files, run.duration_secs));
    }
    for e in run.errors.iter().take(20) {
        text.push_str(&format!("Error: {e}\n"));
    }
    if run.errors.len() > 20 {
        text.push_str(&format!("… and {} more\n", run.errors.len() - 20));
    }
    text
}

/// The profile's channel as this pair or rule sends it: `notify_on`, when
/// set, replaces the profile's own result filter.
pub fn effective(profile: &NotificationProfile, cfg: &RunNotify) -> Notification {
    let mut n = profile.effective();
    if !cfg.notify_on.is_empty() {
        n.send_on = cfg.notify_on.clone();
    }
    n
}

/// Send the report of a finished run if `cfg` asks for it. Returns what
/// went wrong (empty when sent, when not wanted, or when no profile is
/// chosen). Fails open: the caller only logs these lines.
pub async fn report(backup: &BackupManager, cfg: &RunNotify, run: &RunSummary) -> Vec<String> {
    let previous = remember(run);
    if !cfg.wants(run.result, run.bytes) {
        return Vec::new();
    }
    let Some(id) = cfg.profile() else { return Vec::new() };
    let profile = match backup.get_notification_profile(id).await {
        Ok(p) => p,
        Err(_) => return vec![format!("Notification profile {id} no longer exists; \"{}\" was not reported", run.name)],
    };
    let (n, secret) = backup.prepare_notification(effective(&profile, cfg)).await;
    super::deliver(&[(n, secret)], run.result.as_str(), &vars(run, previous), &body(run)).await
}

/// What a finished sync run reports; `None` for a cancelled run.
pub fn sync_summary(run: &crate::sync_engine::SyncRunEvent) -> Option<RunSummary> {
    use ufop_core::sync_types::SyncRunStatus;
    let name = run.pair.name.clone();
    let base = |result, summary, errors, files, bytes, duration_secs| RunSummary { kind: "sync", id: run.pair.id.to_string(), name: name.clone(), result, summary, errors, files, bytes, duration_secs };
    match &run.outcome {
        Err(e) => Some(base(RunResult::Error, format!("Sync of {name} failed"), vec![e.clone()], 0, None, 0)),
        Ok(r) => {
            let result = match r.status {
                SyncRunStatus::Failed => RunResult::Error,
                SyncRunStatus::PartialSuccess => RunResult::Warning,
                SyncRunStatus::Success if run.verify_mismatches > 0 => RunResult::Warning,
                SyncRunStatus::Success => RunResult::Success,
                SyncRunStatus::Cancelled | SyncRunStatus::Running => return None,
            };
            let mut errors = r.error_messages.clone();
            if run.verify_mismatches > 0 {
                errors.push(format!("Verification found {} file(s) that differ after the run", run.verify_mismatches));
            }
            let summary = format!("Sync of {name}: {} added, {} changed, {} deleted, {} skipped", r.files_added, r.files_modified, r.files_deleted, r.files_skipped);
            Some(base(result, summary, errors, r.files_added + r.files_modified + r.files_deleted, Some(r.bytes_transferred), r.duration_ms / 1000))
        }
    }
}

/// Bytes of the files a run produced or touched that still exist. `None`
/// when none of them does (deleted or purged paths, remote results, runs that
/// only notify or verify): the run's size is unknown and a size threshold
/// must not mute it.
pub fn affected_bytes(paths: &[String]) -> Option<u64> {
    let sizes: Vec<u64> = paths.iter().filter_map(|f| std::fs::metadata(f).ok()).filter(|m| m.is_file()).map(|m| m.len()).collect();
    if sizes.is_empty() {
        None
    } else {
        Some(sizes.iter().sum())
    }
}

/// What a finished automation run reports; `None` for a run that did nothing
/// (skipped: no file matched) or is still going.
pub fn automation_summary(rule: &crate::automation_engine::AutomationRule, log: &crate::automation_engine::AutomationLog) -> Option<RunSummary> {
    use crate::automation_engine::AutomationStatus;
    let result = match log.status {
        AutomationStatus::Success => RunResult::Success,
        AutomationStatus::Failed => RunResult::Error,
        AutomationStatus::Skipped | AutomationStatus::Running => return None,
    };
    let duration_secs = match (chrono::DateTime::parse_from_rfc3339(&log.triggered_at), log.completed_at.as_deref().map(chrono::DateTime::parse_from_rfc3339)) {
        (Ok(a), Some(Ok(b))) => (b - a).num_seconds().max(0) as u64,
        _ => 0,
    };
    let bytes = affected_bytes(&log.files_affected);
    let summary = match (&log.error_message, log.files_affected.len()) {
        (Some(e), _) if result == RunResult::Error => format!("Automation \"{}\" failed: {e}", rule.name),
        (_, 0) => format!("Automation \"{}\" ran", rule.name),
        (_, n) => format!("Automation \"{}\" ran on {n} file(s)", rule.name),
    };
    let errors: Vec<String> = log.steps.iter().filter(|s| !s.ok).map(|s| s.message.clone().unwrap_or_else(|| format!("{} failed", s.action))).collect();
    let errors = if errors.is_empty() { log.error_message.clone().into_iter().collect() } else { errors };
    Some(RunSummary { kind: "automation", id: rule.id.clone(), name: rule.name.clone(), result, summary, errors, files: log.files_affected.len() as u64, bytes, duration_secs })
}

/// Who else names a notification profile: sync pairs and automation rules
/// (backup jobs and templates are the profile store's own). Lets deleting a
/// profile see, and with force clean up, those references.
pub trait ProfileUsers: Send + Sync {
    /// Labels of the pairs and rules that send to `profile_id`.
    fn users<'a>(&'a self, profile_id: &'a str) -> BoxFuture<'a, Vec<String>>;
    /// Stop them sending to it; the labels of what changed.
    fn remove<'a>(&'a self, profile_id: &'a str) -> BoxFuture<'a, Result<Vec<String>, AppError>>;
}

/// `sync pair 'Photos'`, `automation rule 'Nightly'` for everything naming `profile_id`.
pub async fn run_users(profile_id: &str, sync: &SyncManager, automation: &AutomationManager) -> Vec<String> {
    let names = |cfg: &RunNotify| cfg.profile() == Some(profile_id);
    let mut out: Vec<String> = sync.list_pairs().await.unwrap_or_default().iter().filter(|p| names(&p.run_options.notify)).map(|p| format!("sync pair '{}'", p.name)).collect();
    out.extend(automation.list_rules().await.iter().filter(|r| names(&r.options.notify)).map(|r| format!("automation rule '{}'", r.name)));
    out
}

/// Clear the notification settings of every pair and rule that names `profile_id`.
pub async fn remove_run_users(profile_id: &str, sync: &SyncManager, automation: &AutomationManager, repo: &Repository) -> Result<Vec<String>, AppError> {
    let mut changed = Vec::new();
    for mut pair in sync.list_pairs().await.unwrap_or_default() {
        if pair.run_options.notify.profile() != Some(profile_id) {
            continue;
        }
        pair.run_options.notify = RunNotify::default();
        let label = format!("sync pair '{}'", pair.name);
        let saved = sync.update_pair(pair).await?;
        sync.save_pair_to_db(repo, &saved).await?;
        changed.push(label);
    }
    for mut rule in automation.list_rules().await {
        if rule.options.notify.profile() != Some(profile_id) {
            continue;
        }
        // A rule an organization manages is theirs to change.
        if rule.managed_by.is_some() {
            continue;
        }
        automation.record_revision(repo, &rule.id).await?;
        rule.options.notify = RunNotify::default();
        automation.save_rule(repo, &rule).await?;
        changed.push(format!("automation rule '{}'", rule.name));
    }
    Ok(changed)
}

/// The desktop app's view of [`ProfileUsers`]: the managers live in Tauri's state.
struct AppProfileUsers(tauri::AppHandle);

impl ProfileUsers for AppProfileUsers {
    fn users<'a>(&'a self, profile_id: &'a str) -> BoxFuture<'a, Vec<String>> {
        Box::pin(async move {
            use tauri::Manager;
            run_users(profile_id, &self.0.state::<SyncManager>(), &self.0.state::<AutomationManager>()).await
        })
    }

    fn remove<'a>(&'a self, profile_id: &'a str) -> BoxFuture<'a, Result<Vec<String>, AppError>> {
        Box::pin(async move {
            use tauri::Manager;
            remove_run_users(profile_id, &self.0.state::<SyncManager>(), &self.0.state::<AutomationManager>(), &self.0.state::<Repository>()).await
        })
    }
}

/// Let deleting a notification profile see the sync pairs and rules using it.
pub fn wire_profile_users(app: &tauri::AppHandle) {
    use tauri::Manager;
    app.state::<BackupManager>().set_profile_users(std::sync::Arc::new(AppProfileUsers(app.clone())));
}

/// Report finished sync runs of pairs that name a notification profile.
pub fn wire_sync_reports(app: &tauri::AppHandle) {
    use tauri::Manager;
    let handle = app.clone();
    app.state::<crate::sync_engine::SyncManager>().add_run_listener(std::sync::Arc::new(move |run| {
        if run.pair.run_options.notify.profile().is_none() {
            return;
        }
        let Some(summary) = sync_summary(run) else { return };
        let (handle, cfg) = (handle.clone(), run.pair.run_options.notify.clone());
        tauri::async_runtime::spawn(async move {
            let backup = handle.state::<BackupManager>();
            for problem in report(&backup, &cfg, &summary).await {
                tracing::warn!("Sync notification for '{}' failed: {problem}", summary.name);
            }
        });
    }));
}

#[cfg(test)]
mod tests {
    use super::super::tests::{manager, recorder, webhook_profile};
    use super::*;

    fn run(result: RunResult, bytes: Option<u64>) -> RunSummary {
        RunSummary {
            kind: "sync",
            id: "pair-1".into(),
            name: "Photos".into(),
            result,
            summary: "Photos: +2 ~1 -0".into(),
            errors: if result == RunResult::Error { vec!["locked file".into()] } else { vec![] },
            files: 3,
            bytes,
            duration_secs: 4,
        }
    }

    fn cfg(profile: &str, on: &[&str], min: Option<u64>) -> RunNotify {
        RunNotify { notify_profile_id: Some(profile.into()), notify_on: on.iter().map(|s| s.to_string()).collect(), min_bytes: min }
    }

    #[test]
    fn values_and_text_describe_the_run() {
        let r = run(RunResult::Error, Some(2048));
        let v = vars(&r, Some(RunResult::Success));
        assert_eq!((v["name"].as_str(), v["operation"].as_str(), v["result"].as_str()), ("Photos", "sync", "error"));
        assert_eq!((v["files"].as_str(), v["bytes"].as_str(), v["previous_result"].as_str(), v["errors"].as_str()), ("3", "2048", "success", "locked file"));
        assert_eq!(vars(&run(RunResult::Success, None), None)["bytes"], "");
        let text = body(&r);
        assert!(text.starts_with("Photos: +2 ~1 -0\n") && text.contains("2.0 KB moved in 3 file(s)") && text.contains("Error: locked file"), "{text}");
        // Long error lists are cut.
        let many = RunSummary { errors: (0..25).map(|i| format!("e{i}")).collect(), ..r };
        assert!(body(&many).contains("… and 5 more"));
    }

    #[test]
    fn notify_on_replaces_the_profiles_own_filter() {
        let mut p = webhook_profile("Ops", "https://h.example/x");
        p.notification.send_on = vec!["error".into()];
        assert_eq!(effective(&p, &cfg("p", &[], None)).send_on, vec!["error"], "empty: the profile decides");
        assert_eq!(effective(&p, &cfg("p", &["success", "warning"], None)).send_on, vec!["success", "warning"]);
        p.min_severity = Some(super::super::profiles::Severity::Warning);
        assert_eq!(effective(&p, &cfg("p", &[], None)).send_on, vec!["warning", "error"], "min severity applies");
    }

    #[tokio::test]
    async fn a_finished_run_is_delivered_through_the_chosen_profile() {
        let t = tempfile::tempdir().unwrap();
        let m = manager(t.path()).await;
        let (base, seen) = recorder().await;
        let p = m.save_notification_profile(webhook_profile("Ops", &format!("{base}/ops"))).await.unwrap();

        // Errors reach the profile; the body carries the run's values.
        assert!(report(&m, &cfg(&p.id, &[], None), &run(RunResult::Error, Some(10))).await.is_empty());
        let got = seen.lock().unwrap().remove(0);
        assert_eq!(got.1, "/ops");
        let body = got.3.to_string();
        assert!(body.contains("Photos") && body.contains("locked file"), "{body}");

        // The profile only sends errors; a success is filtered by the channel.
        assert!(report(&m, &cfg(&p.id, &[], None), &run(RunResult::Success, Some(10))).await.is_empty());
        assert!(seen.lock().unwrap().is_empty());
        // `notify_on` widens it to successes.
        assert!(report(&m, &cfg(&p.id, &["success"], None), &run(RunResult::Success, Some(10))).await.is_empty());
        assert_eq!(seen.lock().unwrap().len(), 1);
        seen.lock().unwrap().clear();

        // Size threshold: a small success is muted, an error never is.
        assert!(report(&m, &cfg(&p.id, &["success", "error"], Some(1000)), &run(RunResult::Success, Some(999))).await.is_empty());
        assert!(seen.lock().unwrap().is_empty(), "below the threshold");
        assert!(report(&m, &cfg(&p.id, &["success", "error"], Some(1000)), &run(RunResult::Success, Some(1000))).await.is_empty());
        assert_eq!(seen.lock().unwrap().len(), 1, "at the threshold");
        assert!(report(&m, &cfg(&p.id, &["success", "error"], Some(1000)), &run(RunResult::Error, Some(0))).await.is_empty());
        assert_eq!(seen.lock().unwrap().len(), 2, "errors always notify");

        // No profile, or one that was deleted: nothing sent, and the second says why.
        seen.lock().unwrap().clear();
        assert!(report(&m, &RunNotify::default(), &run(RunResult::Error, None)).await.is_empty());
        let lines = report(&m, &cfg("gone", &[], None), &run(RunResult::Error, None)).await;
        assert!(lines.len() == 1 && lines[0].contains("no longer exists"), "{lines:?}");
        assert!(seen.lock().unwrap().is_empty());
    }

    fn pair_run(outcome: Result<ufop_core::sync_types::SyncReport, String>, mismatches: usize) -> crate::sync_engine::SyncRunEvent {
        crate::sync_engine::SyncRunEvent { pair: ufop_core::sync_types::SyncPair { name: "Photos".into(), ..Default::default() }, outcome, verify_mismatches: mismatches }
    }

    fn sync_report(status: ufop_core::sync_types::SyncRunStatus, errors: u32) -> ufop_core::sync_types::SyncReport {
        ufop_core::sync_types::SyncReport {
            id: uuid::Uuid::new_v4(),
            pair_id: uuid::Uuid::new_v4(),
            pair_name: "Photos".into(),
            started_at: chrono::Utc::now(),
            ended_at: chrono::Utc::now(),
            duration_ms: 4_500,
            files_added: 2,
            files_modified: 1,
            files_deleted: 0,
            files_skipped: 3,
            conflicts_resolved: 0,
            errors,
            bytes_transferred: 4096,
            status,
            health: ufop_core::sync_types::SyncHealth::Green,
            error_messages: if errors > 0 { vec!["locked".into()] } else { vec![] },
            resumed: false,
            exit_code: 0,
            versions_pruned: 0,
        }
    }

    #[test]
    fn sync_runs_map_to_results() {
        use ufop_core::sync_types::SyncRunStatus::*;
        let ok = sync_summary(&pair_run(Ok(sync_report(Success, 0)), 0)).unwrap();
        assert_eq!((ok.kind, ok.result, ok.files, ok.bytes, ok.duration_secs), ("sync", RunResult::Success, 3, Some(4096), 4));
        assert_eq!(ok.summary, "Sync of Photos: 2 added, 1 changed, 0 deleted, 3 skipped");
        assert_eq!(sync_summary(&pair_run(Ok(sync_report(PartialSuccess, 1)), 0)).unwrap().result, RunResult::Warning);
        assert_eq!(sync_summary(&pair_run(Ok(sync_report(Failed, 2)), 0)).unwrap().result, RunResult::Error);
        let v = sync_summary(&pair_run(Ok(sync_report(Success, 0)), 2)).unwrap();
        assert_eq!(v.result, RunResult::Warning, "differences after the run are a warning");
        assert!(v.errors[0].contains("2 file(s) that differ"));
        let e = sync_summary(&pair_run(Err("source missing".into()), 0)).unwrap();
        assert_eq!((e.result, e.errors.as_slice()), (RunResult::Error, &["source missing".to_string()][..]));
        assert!(sync_summary(&pair_run(Ok(sync_report(Cancelled, 0)), 0)).is_none());
    }

    #[test]
    fn automation_runs_map_to_results() {
        use crate::automation_engine::{AutomationLog, AutomationRule, AutomationStatus, RuleAction, RuleTrigger, StepResult};
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("a.bin");
        std::fs::write(&f, vec![0u8; 1500]).unwrap();
        let rule = AutomationRule::new("Archive".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/x".into() });
        let log = |status, error: Option<&str>, steps: Vec<StepResult>| AutomationLog {
            id: "l".into(),
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            triggered_at: "2026-01-01T00:00:00+00:00".into(),
            completed_at: Some("2026-01-01T00:00:07+00:00".into()),
            trigger_event: "manual".into(),
            status,
            files_affected: vec![f.to_string_lossy().into_owned(), "/gone".into()],
            error_message: error.map(Into::into),
            attempts: 1,
            steps,
        };
        let ok = automation_summary(&rule, &log(AutomationStatus::Success, None, vec![])).unwrap();
        assert_eq!((ok.kind, ok.result, ok.files, ok.bytes, ok.duration_secs), ("automation", RunResult::Success, 2, Some(1500), 7));
        assert_eq!(ok.summary, "Automation \"Archive\" ran on 2 file(s)");
        let step = StepResult { step: 0, action: "move_file".into(), file: None, ok: false, attempts: 1, message: Some("disk full".into()), node: None };
        let bad = automation_summary(&rule, &log(AutomationStatus::Failed, Some("a.bin: disk full"), vec![step])).unwrap();
        assert_eq!((bad.result, bad.errors.as_slice()), (RunResult::Error, &["disk full".to_string()][..]));
        assert!(bad.summary.contains("failed: a.bin: disk full"));
        assert!(automation_summary(&rule, &log(AutomationStatus::Skipped, Some("No files matched"), vec![])).is_none());
        assert!(automation_summary(&rule, &log(AutomationStatus::Running, None, vec![])).is_none());

        // Results that are not local files (deleted, purged, a sync pair, a remote path): the size is
        // unknown, so a size threshold must not mute the run.
        let mut gone = log(AutomationStatus::Success, None, vec![]);
        gone.files_affected = vec!["/gone".into(), "sync_pair:abc (ok)".into(), t.path().to_string_lossy().into_owned()];
        let unknown = automation_summary(&rule, &gone).unwrap();
        assert_eq!(unknown.bytes, None, "no existing local file");
        assert!(cfg("p", &[], Some(1_000_000)).wants(unknown.result, unknown.bytes), "a threshold does not mute a run whose size is unknown");
        assert_eq!(affected_bytes(&[]), None);
        // Known sizes still count against the threshold.
        assert!(!cfg("p", &[], Some(1_000_000)).wants(ok.result, ok.bytes));
    }

    /// Reaches the managers the way the desktop app does, without a window.
    struct Direct {
        sync: std::sync::Arc<SyncManager>,
        automation: std::sync::Arc<AutomationManager>,
        repo: Repository,
    }

    impl ProfileUsers for Direct {
        fn users<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Vec<String>> {
            Box::pin(run_users(id, &self.sync, &self.automation))
        }
        fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Vec<String>, AppError>> {
            Box::pin(remove_run_users(id, &self.sync, &self.automation, &self.repo))
        }
    }

    #[tokio::test]
    async fn deleting_a_profile_sees_and_cleans_the_sync_pairs_and_rules_using_it() {
        use crate::automation_engine::{AutomationRule, RuleAction, RuleTrigger};
        use ufop_core::sync_types::SyncPair;
        let t = tempfile::tempdir().unwrap();
        let m = manager(t.path()).await;
        let profile = m.save_notification_profile(super::super::tests::webhook_profile("Ops", "https://h.example/x")).await.unwrap();
        let repo = Repository::open_in_memory().await.unwrap();
        let sync = std::sync::Arc::new(SyncManager::with_state_dir(t.path().join("sync")));
        let automation = std::sync::Arc::new(AutomationManager::new());
        m.set_profile_users(std::sync::Arc::new(Direct { sync: sync.clone(), automation: automation.clone(), repo: repo.clone() }));

        let mut notifying = SyncPair { name: "Photos".into(), ..Default::default() };
        notifying.run_options.notify = cfg(&profile.id, &["error"], Some(10));
        let quiet = SyncPair { name: "Quiet".into(), ..Default::default() };
        for p in [&notifying, &quiet] {
            sync.create_pair(p.clone()).await.unwrap();
        }
        let mut rule = AutomationRule::new("Nightly".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/x".into() });
        rule.options.notify = cfg(&profile.id, &[], None);
        let other = AutomationRule::new("Other".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/y".into() });
        for r in [&rule, &other] {
            automation.save_rule(&repo, r).await.unwrap();
        }

        // Refused while in use, naming them.
        let e = m.delete_notification_profile(&profile.id, false).await.unwrap_err().user_message();
        assert!(e.contains("sync pair 'Photos'") && e.contains("automation rule 'Nightly'"), "{e}");
        assert!(!e.contains("Quiet") && !e.contains("Other"), "{e}");
        assert!(m.get_notification_profile(&profile.id).await.is_ok(), "still there");

        // Forced: the settings are cleared (not left dangling) and reported.
        let out = m.delete_notification_profile(&profile.id, true).await.unwrap();
        assert_eq!(out.removed_from, vec!["sync pair 'Photos'", "automation rule 'Nightly'"]);
        assert_eq!(out.warnings.len(), 2);
        let pair = sync.list_pairs().await.unwrap().into_iter().find(|p| p.id == notifying.id).unwrap();
        assert_eq!(pair.run_options.notify, RunNotify::default());
        assert_eq!(automation.get_rule(&rule.id).await.unwrap().options.notify, RunNotify::default());
        assert!(pair.run_options.notify.validate().is_ok(), "what is left can be saved again");
        assert!(m.get_notification_profile(&profile.id).await.is_err(), "deleted");
        assert!(run_users(&profile.id, &sync, &automation).await.is_empty());
    }

    #[tokio::test]
    async fn the_previous_result_is_remembered_per_run_target() {
        let t = tempfile::tempdir().unwrap();
        let m = manager(t.path()).await;
        let a = RunSummary { id: "prev-a".into(), ..run(RunResult::Error, None) };
        report(&m, &RunNotify::default(), &a).await;
        assert_eq!(remember(&RunSummary { result: RunResult::Success, ..a.clone() }), Some(RunResult::Error));
        let other = RunSummary { id: "prev-b".into(), kind: "automation", ..a };
        assert_eq!(remember(&other), None);
    }
}
