//! Sync scheduler - cron-like scheduling for sync pair execution.
//!
//! Parses cron expressions and determines next run times.
//! Integrates with the SyncManager to trigger scheduled runs.

use crate::core::error::AppError;
use chrono::{DateTime, Utc};
use cron::Schedule;
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

/// A scheduled sync pair entry.
#[derive(Debug, Clone)]
pub struct ScheduledPair {
    pub pair_id: Uuid,
    pub cron_expr: String,
    pub next_run: Option<DateTime<Utc>>,
    pub enabled: bool,
}

/// Callback type for scheduled sync execution.
pub type ScheduleCallback = Arc<dyn Fn(Uuid) + Send + Sync>;

/// Manages scheduled sync pairs.
pub struct SyncScheduler {
    schedules: Arc<Mutex<HashMap<Uuid, ScheduledPair>>>,
    running: Arc<Mutex<bool>>,
}

impl SyncScheduler {
    pub fn new() -> Self {
        Self {
            schedules: Arc::new(Mutex::new(HashMap::new())),
            running: Arc::new(Mutex::new(false)),
        }
    }

    /// Parse and validate a cron expression.
    pub fn validate_cron(cron_expr: &str) -> Result<(), AppError> {
        Schedule::from_str(cron_expr).map_err(|e| AppError::Sync {
            message: format!("Invalid cron expression '{}': {}", cron_expr, e),
            advice: "Use standard cron format: 'sec min hour day_of_month month day_of_week year'. Example: '0 0 */6 * * * *' for every 6 hours.".to_string(),
        })?;
        Ok(())
    }

    /// Add a scheduled sync pair.
    pub fn add_schedule(
        &self,
        pair_id: Uuid,
        cron_expr: &str,
    ) -> Result<ScheduledPair, AppError> {
        let schedule = Schedule::from_str(cron_expr).map_err(|e| AppError::Sync {
            message: format!("Invalid cron expression: {}", e),
            advice: "Use standard cron format.".to_string(),
        })?;

        let next_run = schedule.upcoming(Utc).next();

        let entry = ScheduledPair {
            pair_id,
            cron_expr: cron_expr.to_string(),
            next_run,
            enabled: true,
        };

        let mut schedules = self.schedules.lock().map_err(|_| AppError::Sync {
            message: "Scheduler lock poisoned.".to_string(),
            advice: "Restart the application.".to_string(),
        })?;

        schedules.insert(pair_id, entry.clone());
        tracing::info!(
            "Scheduled sync pair {} with cron '{}', next run: {:?}",
            pair_id,
            cron_expr,
            next_run
        );

        Ok(entry)
    }

    /// Remove a scheduled sync pair.
    pub fn remove_schedule(&self, pair_id: Uuid) -> Result<(), AppError> {
        let mut schedules = self.schedules.lock().map_err(|_| AppError::Sync {
            message: "Scheduler lock poisoned.".to_string(),
            advice: "Restart the application.".to_string(),
        })?;

        schedules.remove(&pair_id);
        tracing::info!("Removed schedule for sync pair {}", pair_id);
        Ok(())
    }

    /// Get the next run time for a pair.
    pub fn next_run_time(&self, pair_id: Uuid) -> Option<DateTime<Utc>> {
        self.schedules
            .lock()
            .ok()
            .and_then(|s| s.get(&pair_id).and_then(|e| e.next_run))
    }

    /// List all scheduled pairs.
    pub fn list_schedules(&self) -> Vec<ScheduledPair> {
        self.schedules
            .lock()
            .map(|s| s.values().cloned().collect())
            .unwrap_or_default()
    }

    /// Check for due schedules and return pair IDs that should run now.
    pub fn check_due(&self) -> Vec<Uuid> {
        let now = Utc::now();
        let mut due = Vec::new();

        if let Ok(mut schedules) = self.schedules.lock() {
            for (pair_id, entry) in schedules.iter_mut() {
                if !entry.enabled {
                    continue;
                }

                if let Some(next_run) = entry.next_run {
                    if now >= next_run {
                        due.push(*pair_id);

                        // Compute next run time
                        if let Ok(schedule) = Schedule::from_str(&entry.cron_expr) {
                            entry.next_run = schedule.upcoming(Utc).next();
                        }
                    }
                }
            }
        }

        due
    }

    /// Start the scheduler loop that checks for due schedules periodically.
    /// Returns immediately; the loop runs in a background thread.
    pub fn start(&self, callback: ScheduleCallback) {
        let schedules = self.schedules.clone();
        let running = self.running.clone();

        {
            let mut r = running.lock().unwrap();
            if *r {
                return; // Already running
            }
            *r = true;
        }

        let running_clone = running.clone();

        std::thread::spawn(move || {
            tracing::info!("Sync scheduler started");

            loop {
                {
                    let r = running_clone.lock().unwrap();
                    if !*r {
                        break;
                    }
                }

                let now = Utc::now();
                let mut due_pairs = Vec::new();

                if let Ok(mut schedules) = schedules.lock() {
                    for (pair_id, entry) in schedules.iter_mut() {
                        if !entry.enabled {
                            continue;
                        }

                        if let Some(next_run) = entry.next_run {
                            if now >= next_run {
                                due_pairs.push(*pair_id);

                                // Advance to next run
                                if let Ok(schedule) = Schedule::from_str(&entry.cron_expr) {
                                    entry.next_run = schedule.upcoming(Utc).next();
                                }
                            }
                        }
                    }
                }

                for pair_id in due_pairs {
                    tracing::info!("Scheduler triggering sync for pair {}", pair_id);
                    callback(pair_id);
                }

                // Sleep for 30 seconds between checks
                std::thread::sleep(std::time::Duration::from_secs(30));
            }

            tracing::info!("Sync scheduler stopped");
        });
    }

    /// Stop the scheduler loop.
    pub fn stop(&self) {
        if let Ok(mut r) = self.running.lock() {
            *r = false;
        }
    }

    /// Check if the scheduler is running.
    pub fn is_running(&self) -> bool {
        self.running.lock().map(|r| *r).unwrap_or(false)
    }
}

impl Default for SyncScheduler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_cron_valid() {
        // Standard 7-field cron with seconds
        assert!(SyncScheduler::validate_cron("0 0 */6 * * * *").is_ok());
        assert!(SyncScheduler::validate_cron("0 30 9 * * 1-5 *").is_ok());
        assert!(SyncScheduler::validate_cron("0 0 0 * * * *").is_ok());
    }

    #[test]
    fn test_validate_cron_invalid() {
        assert!(SyncScheduler::validate_cron("not a cron").is_err());
        assert!(SyncScheduler::validate_cron("").is_err());
    }

    #[test]
    fn test_add_and_list_schedule() {
        let scheduler = SyncScheduler::new();
        let pair_id = Uuid::new_v4();

        let entry = scheduler.add_schedule(pair_id, "0 0 */6 * * * *").unwrap();
        assert_eq!(entry.pair_id, pair_id);
        assert!(entry.next_run.is_some());

        let schedules = scheduler.list_schedules();
        assert_eq!(schedules.len(), 1);
    }

    #[test]
    fn test_remove_schedule() {
        let scheduler = SyncScheduler::new();
        let pair_id = Uuid::new_v4();

        scheduler.add_schedule(pair_id, "0 0 * * * * *").unwrap();
        assert_eq!(scheduler.list_schedules().len(), 1);

        scheduler.remove_schedule(pair_id).unwrap();
        assert!(scheduler.list_schedules().is_empty());
    }

    #[test]
    fn test_next_run_time() {
        let scheduler = SyncScheduler::new();
        let pair_id = Uuid::new_v4();

        assert!(scheduler.next_run_time(pair_id).is_none());

        scheduler.add_schedule(pair_id, "0 0 * * * * *").unwrap();
        assert!(scheduler.next_run_time(pair_id).is_some());
    }

    #[test]
    fn test_check_due_no_schedules() {
        let scheduler = SyncScheduler::new();
        assert!(scheduler.check_due().is_empty());
    }

    #[test]
    fn test_scheduler_start_stop() {
        let scheduler = SyncScheduler::new();
        assert!(!scheduler.is_running());

        let callback = Arc::new(|_: Uuid| {});
        scheduler.start(callback);

        // Give thread time to start
        std::thread::sleep(std::time::Duration::from_millis(100));
        assert!(scheduler.is_running());

        scheduler.stop();
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}
