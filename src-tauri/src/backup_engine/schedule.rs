//! Backup schedule math: a local start time, a repeat interval and allowed
//! weekdays. Times are evaluated in the machine's local timezone, so a 02:00
//! backup stays at 02:00 across daylight-saving changes; stored timestamps
//! are UTC.

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BackupSchedule {
    pub enabled: bool,
    /// Local time of day, `HH:MM`.
    pub start_time: String,
    /// Repeat interval in minutes (1440 = daily).
    pub interval_minutes: u32,
    /// Allowed weekdays, 0 = Monday … 6 = Sunday. Empty = every day.
    pub weekdays: Vec<u8>,
    /// Skip scheduled runs while the computer runs on battery.
    pub skip_on_battery: bool,
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self { enabled: false, start_time: "02:00".into(), interval_minutes: 1440, weekdays: Vec::new(), skip_on_battery: false }
    }
}

impl BackupSchedule {
    pub fn validate(&self) -> Result<(), crate::core::error::AppError> {
        NaiveTime::parse_from_str(&self.start_time, "%H:%M").map_err(|_| {
            crate::core::error::AppError::validation("Schedule start time must be HH:MM (24-hour)")
        })?;
        if self.interval_minutes < 5 {
            return Err(crate::core::error::AppError::validation("Schedule interval must be at least 5 minutes"));
        }
        if self.weekdays.iter().any(|d| *d > 6) {
            return Err(crate::core::error::AppError::validation("Weekdays must be 0 (Mon) to 6 (Sun)"));
        }
        Ok(())
    }

    fn day_allowed(&self, t: DateTime<Local>) -> bool {
        self.weekdays.is_empty() || self.weekdays.contains(&(t.weekday().num_days_from_monday() as u8))
    }

    /// First slot strictly after `after` (or the anchor itself when `after`
    /// precedes it). Slots are `anchor + k * interval`, where the anchor is
    /// the start time on the day of `after`.
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        if !self.enabled {
            return None;
        }
        let start = NaiveTime::parse_from_str(&self.start_time, "%H:%M").ok()?;
        let interval = Duration::minutes(self.interval_minutes.max(5) as i64);
        let local_after = after.with_timezone(&Local);
        let day = local_after.date_naive() - Duration::days(1);
        let anchor = Local.from_local_datetime(&day.and_time(start)).earliest()?;
        let mut t = anchor;
        // Jump close to `after`, then walk forward (bounded: two weeks of slots).
        if t < local_after {
            let steps = (local_after - t).num_minutes() / interval.num_minutes();
            t += interval * steps as i32;
        }
        let limit = local_after + Duration::days(15);
        while t <= limit {
            if t > local_after && self.day_allowed(t) {
                return Some(t.with_timezone(&Utc));
            }
            t += interval;
        }
        None
    }

    /// Whether a run is due: the first slot after the last run (or creation)
    /// has passed. A slot missed while the app was closed is therefore run
    /// as soon as the scheduler starts again.
    pub fn is_due(&self, last: DateTime<Utc>, now: DateTime<Utc>) -> bool {
        self.next_after(last).is_some_and(|n| n <= now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn local(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Local.with_ymd_and_hms(y, m, d, h, min, 0).earliest().unwrap().with_timezone(&Utc)
    }

    #[test]
    fn daily_next_and_catch_up() {
        let s = BackupSchedule { enabled: true, start_time: "02:00".into(), interval_minutes: 1440, weekdays: vec![], skip_on_battery: false };
        assert_eq!(s.next_after(local(2026, 3, 10, 1, 0)), Some(local(2026, 3, 10, 2, 0)));
        assert_eq!(s.next_after(local(2026, 3, 10, 2, 0)), Some(local(2026, 3, 11, 2, 0)));
        // Missed while offline: due immediately.
        assert!(s.is_due(local(2026, 3, 1, 3, 0), local(2026, 3, 10, 9, 0)));
        assert!(!s.is_due(local(2026, 3, 10, 3, 0), local(2026, 3, 10, 9, 0)));
    }

    #[test]
    fn hourly_with_weekdays() {
        // 2026-03-14 is a Saturday.
        let s = BackupSchedule { enabled: true, start_time: "00:30".into(), interval_minutes: 60, weekdays: vec![0, 1, 2, 3, 4], skip_on_battery: false };
        let n = s.next_after(local(2026, 3, 14, 10, 0)).unwrap();
        assert_eq!(n, local(2026, 3, 16, 0, 30));
        assert_eq!(s.next_after(local(2026, 3, 16, 10, 0)), Some(local(2026, 3, 16, 10, 30)));
    }

    #[test]
    fn disabled_and_validation() {
        let s = BackupSchedule::default();
        assert!(s.next_after(Utc::now()).is_none());
        assert!(BackupSchedule { start_time: "25:00".into(), ..Default::default() }.validate().is_err());
        assert!(BackupSchedule { interval_minutes: 1, ..Default::default() }.validate().is_err());
        assert!(BackupSchedule { weekdays: vec![7], ..Default::default() }.validate().is_err());
    }
}
