//! Scheduler and filesystem watcher for automation triggers.
//!
//! - File watchers use the `notify` crate (same as sync_engine::watcher)
//! - Cron scheduling uses the `cron` crate (same as sync_engine::scheduler)
//! - Debounced events prevent rapid-fire triggers
//! - Cooldown window prevents circular trigger chains

use super::evaluator::evaluate_conditions;
use super::executor::execute_rule_action;
use super::{AutomationLog, AutomationRule, AutomationStatus, RuleAction, RuleCondition, RuleTrigger};
use chrono::Utc;
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

/// A handle to an active filesystem watcher. Dropping it stops the watcher.
pub struct WatchHandle {
    pub rule_id: String,
    _watcher: RecommendedWatcher,
}

/// Start a filesystem watcher for an automation rule.
#[allow(clippy::too_many_arguments)]
pub fn start_file_watcher(
    rule_id: String,
    watch_path: String,
    recursive: bool,
    trigger: RuleTrigger,
    conditions: Vec<RuleCondition>,
    action: RuleAction,
    exec_semaphore: Arc<tokio::sync::Semaphore>,
    cooldowns: Arc<RwLock<HashMap<String, Instant>>>,
    rules: Arc<RwLock<Vec<AutomationRule>>>,
    logs: Arc<RwLock<Vec<AutomationLog>>>,
    ledger: Option<crate::ledger::OperationLedger>,
) -> Result<WatchHandle, crate::core::error::AppError> {
    let path = PathBuf::from(&watch_path);

    if !path.exists() {
        return Err(crate::core::error::AppError::FileOperation {
            message: format!("Watch path does not exist: {}", watch_path),
            advice: "Create the directory or update the automation rule.".to_string(),
        });
    }

    let (tx, rx) = std::sync::mpsc::channel::<Event>();

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
            // Check for events with a timeout
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(event) => {
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

                        // Evaluate conditions
                        if !evaluate_conditions(&conditions, &path_str) {
                            tracing::debug!(
                                "Conditions not met for '{}' in rule '{}'",
                                path_str, rule_id_clone
                            );
                            continue;
                        }

                        // Acquire semaphore permit for bounded concurrency
                        let permit = exec_semaphore.clone().acquire_owned().await;
                        if permit.is_err() {
                            tracing::warn!("Automation semaphore closed, stopping watcher");
                            return;
                        }
                        let _permit = permit.unwrap();

                        // Set cooldown
                        cooldowns
                            .write()
                            .await
                            .insert(cooldown_key.clone(), Instant::now());

                        // Build a temporary rule for execution
                        let temp_rule = AutomationRule {
                            id: rule_id_clone.clone(),
                            name: format!("auto-{}", rule_id_clone),
                            enabled: true,
                            trigger: trigger.clone(),
                            conditions: conditions.clone(),
                            action: action.clone(),
                            created_at: String::new(),
                            updated_at: String::new(),
                            last_triggered: None,
                            trigger_count: 0,
                            error_count: 0,
                            last_error: None,
                        };

                        // Execute the action
                        let log =
                            execute_rule_action(&temp_rule, Some(&path_str), false).await;

                        // Update rule stats
                        {
                            let mut rules_guard = rules.write().await;
                            if let Some(r) = rules_guard.iter_mut().find(|r| r.id == rule_id_clone)
                            {
                                r.trigger_count += 1;
                                r.last_triggered = Some(Utc::now().to_rfc3339());
                                if log.status == AutomationStatus::Failed {
                                    r.error_count += 1;
                                    r.last_error = log.error_message.clone();
                                } else {
                                    r.error_count = 0;
                                    r.last_error = None;
                                }
                            }
                        }

                        // Store log
                        logs.write().await.push(log.clone());

                        // Emit unified-ledger event for the autonomous fire.
                        // This is the ONLY way users see autonomous activity
                        // in the cross-engine timeline. Fail-open.
                        if let Some(led) = &ledger {
                            super::record_automation_log(led, &log).await;
                        }

                        tracing::info!(
                            "Automation '{}' executed for '{}': {:?}",
                            rule_id_clone, path_str, log.status
                        );

                        last_event_time = Some(Instant::now());
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // No events, continue loop (allows checking if watcher should stop)
                    continue;
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
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
