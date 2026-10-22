//! Scheduler and filesystem watcher for automation triggers.
//!
//! - File watchers use the `notify` crate (same as sync_engine::watcher)
//! - Cron scheduling uses the `cron` crate (same as sync_engine::scheduler)
//! - Debounced events prevent rapid-fire triggers
//! - Cooldown window prevents circular trigger chains

use super::evaluator::evaluate_conditions;
use super::{FireContext, RuleTrigger};
use notify::{Config, Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Debounce window for filesystem events (prevents rapid-fire triggers).
const DEBOUNCE_MS: u64 = 2000;

/// Cooldown per file per rule to prevent circular triggers (seconds).
const COOLDOWN_SECS: u64 = 5;

/// Trigger event recorded for watcher-driven runs.
pub const EVENT_FILE: &str = "file";

/// A handle to an active filesystem watcher. Dropping it stops the watcher.
pub struct WatchHandle {
    pub rule_id: String,
    _watcher: RecommendedWatcher,
}

/// Start a filesystem watcher for an automation rule.
///
/// Each matching event runs the rule through [`FireContext`], which reads
/// the rule's *current* conditions and action, so edits apply without
/// restarting the watcher, and history is persisted like a manual run.
pub(crate) fn start_file_watcher(
    rule_id: String,
    watch_path: String,
    recursive: bool,
    trigger: RuleTrigger,
    exec_semaphore: Arc<tokio::sync::Semaphore>,
    cooldowns: Arc<RwLock<HashMap<String, Instant>>>,
    ctx: FireContext,
) -> Result<WatchHandle, crate::core::error::AppError> {
    let path = PathBuf::from(&watch_path);

    if !path.exists() {
        return Err(crate::core::error::AppError::FileOperation {
            message: format!("Watch path does not exist: {}", watch_path),
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

    let mode = if recursive {
        RecursiveMode::Recursive
    } else {
        RecursiveMode::NonRecursive
    };

    watcher
        .watch(&path, mode)
        .map_err(|e| crate::core::error::AppError::State {
            message: format!("Cannot watch path '{}': {e}", watch_path),
            advice: "Check that the path exists and is accessible.".to_string(),
        })?;

    // Spawn a background task to process events
    let rule_id_clone = rule_id.clone();
    tokio::spawn(async move {
        let mut last_event_time: Option<Instant> = None;

        loop {
            match rx.recv().await {
                Some(event) => {
                    // Debounce: skip if we just processed an event
                    if let Some(last) = last_event_time {
                        if last.elapsed() < Duration::from_millis(DEBOUNCE_MS) {
                            continue;
                        }
                    }

                    // Filter by trigger type
                    let is_relevant = matches!(
                        (&trigger, &event.kind),
                        (RuleTrigger::FileAppears { .. }, EventKind::Create(_))
                            | (RuleTrigger::FileModified { .. }, EventKind::Modify(_))
                            | (RuleTrigger::FileDeleted { .. }, EventKind::Remove(_))
                    );

                    if !is_relevant {
                        continue;
                    }

                    // Process each affected path
                    for path in &event.paths {
                        let path_str = path.to_string_lossy().to_string();

                        // Skip directories for file-based actions
                        if path.is_dir() {
                            continue;
                        }

                        // Check cooldown to prevent circular triggers
                        let cooldown_key = format!("{}:{}", rule_id_clone, path_str);
                        {
                            let cd = cooldowns.read().await;
                            if let Some(last) = cd.get(&cooldown_key) {
                                if last.elapsed() < Duration::from_secs(COOLDOWN_SECS) {
                                    tracing::debug!(
                                        "Skipping '{}' for rule '{}' (cooldown)",
                                        path_str, rule_id_clone
                                    );
                                    continue;
                                }
                            }
                        }

                        // Evaluate the rule's current conditions
                        let Some(rule) = ctx.rule(&rule_id_clone).await else {
                            tracing::info!("Rule '{}' no longer exists, stopping watcher", rule_id_clone);
                            return;
                        };
                        if !evaluate_conditions(&rule.conditions, &path_str) {
                            tracing::debug!(
                                "Conditions not met for '{}' in rule '{}'",
                                path_str, rule_id_clone
                            );
                            continue;
                        }

                        // Acquire semaphore permit for bounded concurrency
                        let Ok(_permit) = exec_semaphore.clone().acquire_owned().await else {
                            tracing::warn!("Automation semaphore closed, stopping watcher");
                            return;
                        };

                        // Set cooldown
                        cooldowns
                            .write()
                            .await
                            .insert(cooldown_key.clone(), Instant::now());

                        if let Some(log) = ctx.fire_if_enabled(&rule_id_clone, Some(&path_str), EVENT_FILE).await {
                            tracing::info!(
                                "Automation '{}' executed for '{}': {:?}",
                                rule_id_clone, path_str, log.status
                            );
                        }

                        last_event_time = Some(Instant::now());
                    }
                }
                None => {
                    tracing::info!("Watcher channel closed for rule '{}'", rule_id_clone);
                    return;
                }
            }
        }
    });

    Ok(WatchHandle {
        rule_id,
        _watcher: watcher,
    })
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
