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
    /// A cron expression, or `schedule:<id>` (`schedule_library`).
    pub cron_expr: String,
    /// The cron expression in force (a named schedule's, as last seen).
    pub resolved: String,
    pub next_run: Option<DateTime<Utc>>,
    pub enabled: bool,
}

/// Callback type for scheduled sync execution.
pub type ScheduleCallback = Arc<dyn Fn(Uuid) + Send + Sync>;

/// Callback receiving every id due in one check.
pub type BatchCallback = Arc<dyn Fn(Vec<Uuid>) + Send + Sync>;

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
        if crate::schedule_library::schedule_ref(cron_expr).is_some() {
            return crate::schedule_library::resolve(cron_expr).map(|_| ());
        }
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
        let (schedule, resolved) = if crate::schedule_library::schedule_ref(cron_expr).is_some() {
            let (s, text, _) = crate::schedule_library::resolve(cron_expr)?;
            (s, text)
        } else {
            let s = Schedule::from_str(cron_expr).map_err(|e| AppError::Sync {
                message: format!("Invalid cron expression: {}", e),
                advice: "Use standard cron format.".to_string(),
            })?;
            (s, cron_expr.to_string())
        };

        let next_run = schedule.upcoming(Utc).next();

        let entry = ScheduledPair {
            pair_id,
            cron_expr: cron_expr.to_string(),
            resolved,
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

                // Named schedules are resolved on every check: an edit applies
                // at once; a deleted one stops the entry until it is changed.
                let Ok((schedule, text, named)) = crate::schedule_library::resolve(&entry.cron_expr) else { continue };
                if text != entry.resolved {
                    entry.resolved = text;
                    entry.next_run = schedule.upcoming(Utc).next();
                    continue;
                }
                if let Some(next_run) = entry.next_run {
                    if now >= next_run {
                        entry.next_run = schedule.upcoming(Utc).next();
                        if let Some(cal) = crate::schedule_library::registry().run_blocked(named.as_deref(), &[], next_run) {
                            tracing::info!("Skipping scheduled run of {pair_id}: {} is a day off in '{cal}'", next_run.with_timezone(&chrono::Local).date_naive());
                            continue;
                        }
                        due.push(*pair_id);
                    }
                }
            }
        }

        due
    }

    /// Start the scheduler loop that checks for due schedules every 30 s.
    /// Returns immediately; the loop runs in a background thread.
    pub fn start(&self, callback: ScheduleCallback) {
        self.start_with_interval(callback, std::time::Duration::from_secs(30));
    }

    /// [`start`](Self::start) with a custom poll interval (tests use a short one).
    pub fn start_with_interval(&self, callback: ScheduleCallback, interval: std::time::Duration) {
        self.start_batch_with_interval(Arc::new(move |ids: Vec<Uuid>| ids.into_iter().for_each(|id| callback(id))), interval);
    }

    /// Like [`start_with_interval`](Self::start_with_interval), but hands
    /// every id due in the same check to `callback` at once, so the caller
    /// can order them.
    pub fn start_batch_with_interval(&self, callback: BatchCallback, interval: std::time::Duration) {
        {
            let mut r = match self.running.lock() {
                Ok(r) => r,
                Err(_) => return,
            };
            if *r {
                return; // Already running
            }
            *r = true;
        }

        let scheduler = self.clone_handles();
        std::thread::spawn(move || {
            tracing::info!("Cron scheduler started");
            while scheduler.is_running() {
                let due = scheduler.check_due();
                if !due.is_empty() {
                    tracing::info!("Scheduler triggering {} item(s)", due.len());
                    callback(due);
                }
                std::thread::sleep(interval);
            }
            tracing::info!("Cron scheduler stopped");
        });
    }

    /// A second handle onto the same schedule table and running flag.
    fn clone_handles(&self) -> Self {
        Self {
            schedules: self.schedules.clone(),
            running: self.running.clone(),
        }
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
    fn named_schedules_resolve_live_and_skip_days_off() {
        use crate::schedule_library::{registry, NamedSchedule};
        use ufop_core::calendar::HolidayCalendar;
        let tag = Uuid::new_v4().simple().to_string();
        let today = chrono::Local::now().date_naive();
        let cal = HolidayCalendar { id: format!("cal-{tag}"), name: "Today off".into(), dates: vec![today, today.succ_opt().unwrap()], ..Default::default() };
        let sched = NamedSchedule { id: format!("s-{tag}"), name: "Every second".into(), cron_expr: "* * * * * * *".into(), ..Default::default() };
        registry().insert_for_tests(Some(sched.clone()), Some(cal.clone()));
        let spec = format!("schedule:{}", sched.id);
        assert!(SyncScheduler::validate_cron(&spec).is_ok());
        assert!(SyncScheduler::validate_cron("schedule:missing").is_err());

        let scheduler = SyncScheduler::new();
        let pair = Uuid::new_v4();
        let entry = scheduler.add_schedule(pair, &spec).unwrap();
        assert_eq!(entry.resolved, "* * * * * * *");
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert_eq!(scheduler.check_due(), vec![pair], "no calendar: runs");

        // The schedule now skips the calendar's days (today): no run, live.
        registry().insert_for_tests(Some(NamedSchedule { calendar_ids: vec![cal.id.clone()], ..sched.clone() }), None);
        std::thread::sleep(std::time::Duration::from_millis(1100));
        assert!(scheduler.check_due().is_empty(), "today is a day off");

        // Editing the cron re-arms from now instead of firing.
        registry().insert_for_tests(Some(NamedSchedule { cron_expr: "0 0 0 1 1 *".into(), ..sched.clone() }), None);
        assert!(scheduler.check_due().is_empty());
        assert_eq!(scheduler.list_schedules()[0].resolved, "0 0 0 1 1 *");
        let next = scheduler.next_run_time(pair).unwrap();
        assert_eq!(next.format("%m-%d %H:%M").to_string(), "01-01 00:00");
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

    /// Regression: the loop must actually invoke the callback for due pairs.
    #[test]
    fn test_scheduler_fires_due_schedules() {
        let scheduler = SyncScheduler::new();
        let pair = Uuid::new_v4();
        scheduler.add_schedule(pair, "* * * * * * *").unwrap(); // every second
        let fired = Arc::new(Mutex::new(Vec::new()));
        let sink = fired.clone();
        scheduler.start_with_interval(
            Arc::new(move |id: Uuid| sink.lock().unwrap().push(id)),
            std::time::Duration::from_millis(100),
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while fired.lock().unwrap().is_empty() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        scheduler.stop();
        assert_eq!(fired.lock().unwrap().first(), Some(&pair));
    }
}
