//! Backup schedule math. Two modes:
//!
//! - **Interval**: a local start time, a repeat interval and allowed weekdays.
//! - **Cron**: one or more cron expressions (`cron`); the earliest next
//!   match of any of them is the next run. Parsed by the schedule library's
//!   cron parser (5, 6 or 7 fields).
//!
//! Times are evaluated in the machine's local timezone, so a 02:00 backup
//! stays at 02:00 across daylight-saving changes; stored timestamps are UTC.
//! The `*_in` functions take any timezone, for callers that have one.

use chrono::{DateTime, Datelike, Duration, Local, NaiveTime, TimeZone, Utc};
use crate::core::error::AppError;
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
    /// Holiday calendars (`schedule_library`) whose days off skip runs.
    pub calendar_ids: Vec<String>,
    /// Cron mode: when not empty, these expressions replace the start time,
    /// interval and weekdays (5 fields `min hour day month weekday`, or 6/7
    /// with seconds and year), evaluated in local time.
    pub cron: Vec<String>,
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self { enabled: false, start_time: "02:00".into(), interval_minutes: 1440, weekdays: Vec::new(), skip_on_battery: false, calendar_ids: Vec::new(), cron: Vec::new() }
    }
}

/// Cron schedules may not run more often than this (like interval schedules).
pub const MIN_GAP_MINUTES: i64 = 5;

impl BackupSchedule {
    /// A cron-mode schedule (enabled).
    pub fn cron(exprs: &[&str]) -> Self {
        Self { enabled: true, cron: exprs.iter().map(|s| s.to_string()).collect(), ..Default::default() }
    }

    pub fn is_cron(&self) -> bool {
        self.cron.iter().any(|c| !c.trim().is_empty())
    }

    fn parsed_cron(&self) -> Result<Vec<cron::Schedule>, AppError> {
        self.cron.iter().filter(|c| !c.trim().is_empty()).map(|c| crate::schedule_library::parse_cron_any(c)).collect()
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.is_cron() {
            let parsed = self.parsed_cron()?;
            // Too frequent: two of the next runs closer than the minimum gap.
            let mut next: Vec<DateTime<Utc>> = parsed.iter().flat_map(|s| s.after(&Utc::now()).take(12).collect::<Vec<_>>()).collect();
            next.sort();
            next.dedup();
            if next.is_empty() {
                return Err(AppError::validation("The cron schedule never runs (no future matches)"));
            }
            if next.windows(2).any(|w| (w[1] - w[0]).num_minutes() < MIN_GAP_MINUTES) {
                return Err(AppError::validation(format!("The cron schedule runs more often than every {MIN_GAP_MINUTES} minutes")));
            }
        } else {
            NaiveTime::parse_from_str(&self.start_time, "%H:%M").map_err(|_| AppError::validation("Schedule start time must be HH:MM (24-hour)"))?;
            if self.interval_minutes < 5 {
                return Err(AppError::validation("Schedule interval must be at least 5 minutes"));
            }
        }
        if self.weekdays.iter().any(|d| *d > 6) {
            return Err(AppError::validation("Weekdays must be 0 (Mon) to 6 (Sun)"));
        }
        if let Some(id) = self.calendar_ids.iter().find(|id| crate::schedule_library::registry().calendar(id).is_none()) {
            return Err(AppError::validation(format!("Holiday calendar {id} does not exist")));
        }
        Ok(())
    }

    fn day_allowed<Tz: TimeZone>(&self, t: &DateTime<Tz>) -> bool {
        self.weekdays.is_empty() || self.weekdays.contains(&(t.weekday().num_days_from_monday() as u8))
    }

    /// First slot strictly after `after`, in local time.
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        self.next_after_in(after, &Local)
    }

    /// First slot strictly after `after`, evaluating times of day in `tz`.
    /// Interval mode: slots are `anchor + k * interval`, where the anchor is
    /// the start time on the day before `after`. Cron mode: the earliest
    /// match of any expression.
    pub fn next_after_in<Tz: TimeZone>(&self, after: DateTime<Utc>, tz: &Tz) -> Option<DateTime<Utc>> {
        if !self.enabled {
            return None;
        }
        if self.is_cron() {
            let local_after = after.with_timezone(tz);
            return self.parsed_cron().ok()?.iter().filter_map(|s| s.after(&local_after).next()).map(|t| t.with_timezone(&Utc)).min();
        }
        let start = NaiveTime::parse_from_str(&self.start_time, "%H:%M").ok()?;
        let interval = Duration::minutes(self.interval_minutes.max(5) as i64);
        let local_after = after.with_timezone(tz);
        let day = local_after.date_naive() - Duration::days(1);
        let anchor = tz.from_local_datetime(&day.and_time(start)).earliest()?;
        let mut t = anchor;
        // Jump close to `after`, then walk forward (bounded: two weeks of slots).
        if t < local_after {
            let steps = (local_after.clone() - t.clone()).num_minutes() / interval.num_minutes();
            t += interval * steps as i32;
        }
        let limit = local_after.clone() + Duration::days(15);
        while t <= limit {
            if t > local_after && self.day_allowed(&t) {
                return Some(t.with_timezone(&Utc));
            }
            t += interval;
        }
        None
    }

    /// The next `n` runs after `after` (local time).
    pub fn upcoming(&self, after: DateTime<Utc>, n: usize) -> Vec<DateTime<Utc>> {
        self.upcoming_in(after, n, &Local)
    }

    pub fn upcoming_in<Tz: TimeZone>(&self, after: DateTime<Utc>, n: usize, tz: &Tz) -> Vec<DateTime<Utc>> {
        let mut out = Vec::with_capacity(n);
        let mut t = after;
        while out.len() < n {
            match self.next_after_in(t, tz) {
                Some(x) => {
                    out.push(x);
                    t = x;
                }
                None => break,
            }
        }
        out
    }

    /// Validate and list the next `n` runs (1–100), marking those a holiday
    /// calendar skips (the "next runs" preview of the editor).
    pub fn preview(&self, after: DateTime<Utc>, n: usize) -> Result<Vec<crate::schedule_library::PreviewRun>, AppError> {
        self.validate()?;
        let on = Self { enabled: true, ..self.clone() };
        Ok(on
            .upcoming(after, n.clamp(1, 100))
            .into_iter()
            .map(|t| crate::schedule_library::PreviewRun {
                at: t.to_rfc3339(),
                skipped_by: crate::schedule_library::registry().blocked_by(&self.calendar_ids, t.with_timezone(&Local).date_naive()),
            })
            .collect())
    }

    /// Typical minutes between runs (interval mode: the interval; cron: the
    /// gap between the next two runs), for "overdue" checks elsewhere.
    pub fn nominal_interval_minutes(&self) -> u32 {
        if !self.is_cron() {
            return self.interval_minutes;
        }
        match self.upcoming(Utc::now(), 2).as_slice() {
            [a, b] => (*b - *a).num_minutes().clamp(MIN_GAP_MINUTES, u32::MAX as i64) as u32,
            _ => self.interval_minutes,
        }
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
        let s = BackupSchedule { enabled: true, start_time: "02:00".into(), interval_minutes: 1440, ..Default::default() };
        assert_eq!(s.next_after(local(2026, 3, 10, 1, 0)), Some(local(2026, 3, 10, 2, 0)));
        assert_eq!(s.next_after(local(2026, 3, 10, 2, 0)), Some(local(2026, 3, 11, 2, 0)));
        // Missed while offline: due immediately.
        assert!(s.is_due(local(2026, 3, 1, 3, 0), local(2026, 3, 10, 9, 0)));
        assert!(!s.is_due(local(2026, 3, 10, 3, 0), local(2026, 3, 10, 9, 0)));
    }

    #[test]
    fn hourly_with_weekdays() {
        // 2026-03-14 is a Saturday.
        let s = BackupSchedule { enabled: true, start_time: "00:30".into(), interval_minutes: 60, weekdays: vec![0, 1, 2, 3, 4], ..Default::default() };
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

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    #[test]
    fn cron_next_runs_in_fixed_zones() {
        // Weekdays 02:30, evaluated in UTC and in UTC+2: no DST involved.
        let s = BackupSchedule::cron(&["30 2 * * Mon-Fri"]);
        s.validate().unwrap();
        // 2026-03-13 is a Friday.
        let runs = s.upcoming_in(utc(2026, 3, 13, 3, 0), 3, &Utc);
        assert_eq!(runs, vec![utc(2026, 3, 16, 2, 30), utc(2026, 3, 17, 2, 30), utc(2026, 3, 18, 2, 30)]);
        let plus2 = chrono::FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(s.next_after_in(utc(2026, 3, 16, 0, 0), &plus2), Some(utc(2026, 3, 16, 0, 30)), "02:30 at UTC+2 is 00:30 UTC");
        // 6-field (seconds) form is accepted too.
        assert_eq!(BackupSchedule::cron(&["0 30 2 * * Mon-Fri"]).next_after_in(utc(2026, 3, 13, 3, 0), &Utc), Some(utc(2026, 3, 16, 2, 30)));
    }

    #[test]
    fn several_cron_expressions_take_the_earliest() {
        let s = BackupSchedule::cron(&["0 9 * * *", "0 18 * * Sat,Sun"]);
        // Saturday 2026-03-14 10:00 → 18:00 the same day beats 09:00 Sunday.
        assert_eq!(s.next_after_in(utc(2026, 3, 14, 10, 0), &Utc), Some(utc(2026, 3, 14, 18, 0)));
        assert_eq!(s.upcoming_in(utc(2026, 3, 14, 10, 0), 4, &Utc), vec![utc(2026, 3, 14, 18, 0), utc(2026, 3, 15, 9, 0), utc(2026, 3, 15, 18, 0), utc(2026, 3, 16, 9, 0)]);
        assert_eq!(BackupSchedule::cron(&["0 3 * * *"]).nominal_interval_minutes(), 1440);
    }

    #[test]
    fn cron_missed_runs_catch_up_once() {
        let s = BackupSchedule::cron(&["0 2 * * *"]);
        // Last run a week ago: one run is due now (not seven), then none until the next 02:00.
        let (last, now) = (local(2026, 6, 1, 3, 0), local(2026, 6, 8, 9, 0));
        assert!(s.is_due(last, now));
        assert!(!s.is_due(now, local(2026, 6, 8, 23, 0)));
        assert!(s.is_due(now, local(2026, 6, 9, 2, 0)));
        // Disabled cron schedules never run.
        assert!(BackupSchedule { enabled: false, ..s.clone() }.next_after(last).is_none());
    }

    #[test]
    fn cron_validation_is_friendly() {
        let e = BackupSchedule::cron(&["61 * * * *"]).validate().unwrap_err().user_message();
        assert!(e.contains("Invalid cron expression '61 * * * *'"), "{e}");
        assert!(BackupSchedule::cron(&["* * * * *"]).validate().unwrap_err().user_message().contains("more often than every 5 minutes"));
        assert!(BackupSchedule::cron(&["schedule:abc"]).validate().unwrap_err().user_message().contains("named schedule"));
        assert!(BackupSchedule::cron(&["*/5 * * * *"]).validate().is_ok());
        // Interval fields are ignored in cron mode.
        assert!(BackupSchedule { interval_minutes: 1, ..BackupSchedule::cron(&["0 * * * *"]) }.validate().is_ok());
    }
}
