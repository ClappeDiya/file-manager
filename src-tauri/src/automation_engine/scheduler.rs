//! Scheduler and filesystem watcher for automation triggers.
//!
//! - File watchers use the `notify` crate (same as sync_engine::watcher)
//! - Cron scheduling uses the `cron` crate (same as sync_engine::scheduler)
//! - Debounced events prevent rapid-fire triggers
//! - Cooldown window prevents circular trigger chains

use super::evaluator::evaluate_rule;
use super::{FireContext, RuleTrigger};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use ufop_core::stability::Stability;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Cooldown per file per rule to prevent circular triggers (seconds).
const COOLDOWN_SECS: u64 = 5;

/// Trigger event recorded for watcher-driven runs.
pub const EVENT_FILE: &str = "file";

/// A handle to an active filesystem watcher. Dropping it stops the watcher.
pub struct WatchHandle {
    pub folder: String,
    _watcher: RecommendedWatcher,
}

/// Whether `trigger` reacts to an event of `kind`.
fn trigger_matches(trigger: &RuleTrigger, kind: &EventKind) -> bool {
    matches!(
        (trigger, kind),
        (RuleTrigger::FileAppears { .. }, EventKind::Create(_))
            | (RuleTrigger::FileModified { .. }, EventKind::Modify(_))
            | (RuleTrigger::FileDeleted { .. }, EventKind::Remove(_))
    )
}

fn trigger_recursive(trigger: &RuleTrigger) -> bool {
    matches!(
        trigger,
        RuleTrigger::FileAppears { recursive: true, .. }
            | RuleTrigger::FileModified { recursive: true, .. }
            | RuleTrigger::FileDeleted { recursive: true, .. }
    )
}

/// Watch one folder for every rule in `members`.
///
/// For each event, the rules it concerns run one after another in
/// priority order (lower first, then oldest rule), so several rules on one
/// intake folder act in a predictable sequence. Rules are read fresh for
/// every event, so edits apply without restarting the watcher.
pub(crate) fn start_folder_watcher(
    folder: String,
    recursive: bool,
    members: Arc<RwLock<Vec<String>>>,
    exec_semaphore: Arc<tokio::sync::Semaphore>,
    cooldowns: Arc<RwLock<HashMap<String, Instant>>>,
    ctx: FireContext,
) -> Result<WatchHandle, crate::core::error::AppError> {
    let path = PathBuf::from(&folder);
    if !path.exists() {
        return Err(crate::core::error::AppError::FileOperation {
            message: format!("Watch path does not exist: {}", folder),
            advice: "Create the directory or update the automation rule.".to_string(),
        });
    }

    // Async channel: the event loop awaits instead of blocking a worker
    // thread, and ends when the watcher (and with it `tx`) is dropped.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Event>();
    let mut watcher = RecommendedWatcher::new(
        move |result: Result<Event, notify::Error>| {
            if let Ok(event) = result {
                let _ = tx.send(event);
            }
        },
        Config::default(),
    )
    .map_err(|e| crate::core::error::AppError::State {
        message: format!("Cannot create filesystem watcher: {e}"),
        advice: "Check OS filesystem notification support.".to_string(),
    })?;
    let mode = if recursive { RecursiveMode::Recursive } else { RecursiveMode::NonRecursive };
    watcher.watch(&path, mode).map_err(|e| crate::core::error::AppError::State {
        message: format!("Cannot watch path '{}': {e}", folder),
        advice: "Check that the path exists and is accessible.".to_string(),
    })?;

    let root = path.clone();
    tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            for file in &event.paths {
                if file.is_dir() {
                    continue;
                }
                let file_str = file.to_string_lossy().to_string();
                let direct_child = file.parent() == Some(root.as_path());

                // Rules this event concerns, in run order.
                let ids = members.read().await.clone();
                let mut rules = Vec::new();
                for id in ids {
                    let Some(rule) = ctx.rule(&id).await else { continue };
                    if !rule.enabled || !trigger_matches(&rule.trigger, &event.kind) {
                        continue;
                    }
                    if !direct_child && !trigger_recursive(&rule.trigger) {
                        continue;
                    }
                    if !evaluate_rule(&rule, &file_str).matched {
                        continue;
                    }
                    rules.push(rule);
                }
                if rules.is_empty() {
                    continue;
                }
                rules.sort_by(|a, b| a.options.priority.cmp(&b.options.priority).then_with(|| a.created_at.cmp(&b.created_at)));

                // Cooldown per rule and file stops a rule re-triggering on
                // its own output (and the burst of events one write makes).
                let mut due = Vec::new();
                {
                    let mut cd = cooldowns.write().await;
                    for rule in rules {
                        let key = format!("{}:{}", rule.id, file_str);
                        if cd.get(&key).is_some_and(|t| t.elapsed() < Duration::from_secs(COOLDOWN_SECS)) {
                            continue;
                        }
                        cd.insert(key, Instant::now());
                        due.push(rule);
                    }
                }
                if due.is_empty() {
                    continue;
                }

                let (ctx, semaphore, file_str) = (ctx.clone(), exec_semaphore.clone(), file_str.clone());
                tokio::spawn(async move {
                    let Ok(_permit) = semaphore.acquire_owned().await else { return };
                    for rule in due {
                        if rule.options.settle_secs > 0 && !matches!(rule.trigger, RuleTrigger::FileDeleted { .. }) {
                            let quiet = Duration::from_secs(rule.options.settle_secs);
                            let limit = (quiet * 20).min(Duration::from_secs(1800));
                            match ufop_core::stability::wait_until_stable(Path::new(&file_str), quiet, Duration::from_millis(250), limit).await {
                                Stability::Stable => {}
                                Stability::Gone => continue,
                                Stability::StillChanging => {
                                    ctx.record_skip(&rule.id, Some(&file_str), EVENT_FILE, "The file was still changing; it was left alone").await;
                                    continue;
                                }
                            }
                        }
                        if let Some(log) = ctx.fire_if_enabled(&rule.id, Some(&file_str), EVENT_FILE).await {
                            tracing::info!("Automation '{}' executed for '{}': {:?}", log.rule_name, file_str, log.status);
                        }
                    }
                });
            }
        }
        tracing::info!("Watcher channel closed for '{}'", root.display());
    });

    Ok(WatchHandle { folder, _watcher: watcher })
}

/// Validate a cron expression.
pub fn validate_cron(cron_expr: &str) -> Result<(), crate::core::error::AppError> {
    use cron::Schedule;
    use std::str::FromStr;

    Schedule::from_str(cron_expr).map_err(|e| crate::core::error::AppError::State {
        message: format!("Invalid cron expression '{}': {e}", cron_expr),
        advice: "Use standard cron format: 'sec min hour day_of_month month day_of_week year'. Example: '0 0 */6 * * * *' for every 6 hours.".to_string(),
    })?;
    Ok(())
}
