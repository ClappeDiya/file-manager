//! Backup schedule math. Two modes:
//!
//! - **Interval**: a local start time, a repeat interval and allowed weekdays.
//! - **Cron**: one or more cron expressions (`cron`); the earliest next
//!   match of any of them is the next run. Parsed by the schedule library's
//!   cron parser (5, 6 or 7 fields).
//!
//! Times are evaluated in the schedule's time zone (`timezone`): the
//! machine's local zone by default, so a 02:00 backup stays at 02:00 across
//! daylight-saving changes, or a fixed UTC offset (`UTC`, `+02:00`, …; see
//! [`ScheduleZone`]). Stored timestamps are UTC. The `*_in` functions take
//! any timezone, for callers that have one.
//!
//! Shared by the desktop app, `ufop-server` and the standalone `ufop agent`
//! host, so a managed job runs on the same slots everywhere. Holiday
//! calendars come from the process-wide [`crate::calendar::registry`].

use chrono::{DateTime, Datelike, Duration, FixedOffset, Local, NaiveDate, NaiveTime, Offset, TimeZone, Utc};
use crate::error::AppError;
use std::str::FromStr;
use serde::{Deserialize, Serialize};

/// A schedule field (sync pair, automation rule) can hold a reference to a
/// named schedule instead of a cron expression: `schedule:<id>`.
pub const REF_PREFIX: &str = "schedule:";

/// The named schedule a schedule field refers to.
pub fn schedule_ref(expr: &str) -> Option<&str> {
    expr.trim().strip_prefix(REF_PREFIX).map(str::trim).filter(|s| !s.is_empty())
}

/// Parse a cron expression written with 5 fields (`min hour day month
/// weekday`, as in crontab), 6 (with seconds) or 7 (with year). Five-field
/// expressions run at second 0.
pub fn parse_cron_any(expr: &str) -> Result<cron::Schedule, AppError> {
    let t = expr.trim();
    if t.is_empty() {
        return Err(AppError::validation("The cron expression is empty"));
    }
    if schedule_ref(t).is_some() {
        return Err(AppError::validation(format!("'{t}' refers to a named schedule; enter its cron expression here instead")));
    }
    let fields = t.split_whitespace().count();
    let full = if fields == 5 { format!("0 {t}") } else { t.to_string() };
    cron::Schedule::from_str(&full).map_err(|e| {
        AppError::validation(format!(
            "Invalid cron expression '{t}': {e}. Use `min hour day month weekday` (e.g. `30 2 * * Mon-Fri`), optionally with seconds first and a year last."
        ))
    })
}

/// One upcoming run in a preview.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct PreviewRun {
    pub at: String,
    /// Calendar that skips this run, if any.
    pub skipped_by: Option<String>,
    /// Backup schedules: `at` with the schedule time zone's offset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub zoned: Option<String>,
}

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
    /// with seconds and year), evaluated in the schedule's time zone.
    pub cron: Vec<String>,
    /// Time zone of the start time, weekdays, cron fields and holiday
    /// calendars: `None` / `local` = this computer's zone, or a fixed offset
    /// (`UTC`, `+02:00`, `UTC-05:30`). See [`ScheduleZone::parse`].
    pub timezone: Option<String>,
}

/// The zone a schedule is evaluated in. Named IANA zones (`Europe/Berlin`)
/// need a time-zone database the app does not ship, so only the local zone
/// (which follows daylight saving) and fixed UTC offsets are supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScheduleZone {
    Local,
    Fixed(FixedOffset),
}

/// Largest accepted UTC offset (real zones span −12:00 to +14:00).
const MAX_OFFSET_MINUTES: i32 = 14 * 60;

impl ScheduleZone {
    /// Accepts empty / `local`, `UTC` / `GMT` / `Z` / `Etc/UTC`, and offsets
    /// `+02:00`, `-0530`, `+2`, optionally after `UTC` or `GMT` (`UTC+1`).
    pub fn parse(name: Option<&str>) -> Result<Self, AppError> {
        let raw = name.map(str::trim).unwrap_or("");
        let lower = raw.to_ascii_lowercase();
        if lower.is_empty() || lower == "local" {
            return Ok(Self::Local);
        }
        if matches!(lower.as_str(), "utc" | "gmt" | "z" | "etc/utc" | "etc/gmt" | "zulu" | "universal") {
            return Ok(Self::Fixed(Utc.fix()));
        }
        let off = lower.strip_prefix("utc").or_else(|| lower.strip_prefix("gmt")).unwrap_or(&lower).trim();
        Self::parse_offset(off).map(Self::Fixed).ok_or_else(|| {
            AppError::validation(format!(
                "Unknown time zone \"{raw}\". Use local (this computer's zone, with daylight saving), UTC, or a fixed offset such as +02:00 or UTC-05:00; named zones like Europe/Berlin are not supported"
            ))
        })
    }

    fn parse_offset(s: &str) -> Option<FixedOffset> {
        let (sign, rest) = match s.chars().next()? {
            '+' => (1, &s[1..]),
            '-' => (-1, &s[1..]),
            _ => return None,
        };
        let (h, m) = match rest.split_once(':') {
            Some((h, m)) => (h, m),
            None if rest.len() == 4 => rest.split_at(2),
            None => (rest, "0"),
        };
        if h.is_empty() || h.len() > 2 || m.is_empty() || m.len() > 2 || !h.chars().chain(m.chars()).all(|c| c.is_ascii_digit()) {
            return None;
        }
        let (h, m): (i32, i32) = (h.parse().ok()?, m.parse().ok()?);
        let total = h * 60 + m;
        if m >= 60 || total > MAX_OFFSET_MINUTES {
            return None;
        }
        FixedOffset::east_opt(sign * total * 60)
    }

    /// The calendar date of `t` in this zone (for weekdays and holidays).
    pub fn date_of(&self, t: DateTime<Utc>) -> NaiveDate {
        match self {
            Self::Local => t.with_timezone(&Local).date_naive(),
            Self::Fixed(o) => t.with_timezone(o).date_naive(),
        }
    }

    /// `t` as RFC 3339 with this zone's offset.
    pub fn rfc3339(&self, t: DateTime<Utc>) -> String {
        match self {
            Self::Local => t.with_timezone(&Local).to_rfc3339(),
            Self::Fixed(o) => t.with_timezone(o).to_rfc3339(),
        }
    }
}

impl Default for BackupSchedule {
    fn default() -> Self {
        Self { enabled: false, start_time: "02:00".into(), interval_minutes: 1440, weekdays: Vec::new(), skip_on_battery: false, calendar_ids: Vec::new(), cron: Vec::new(), timezone: None }
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
        self.cron.iter().filter(|c| !c.trim().is_empty()).map(|c| parse_cron_any(c)).collect()
    }

    /// The schedule's zone; an unparsable stored name (rejected on save)
    /// falls back to the local zone.
    pub fn zone(&self) -> ScheduleZone {
        ScheduleZone::parse(self.timezone.as_deref()).unwrap_or(ScheduleZone::Local)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        ScheduleZone::parse(self.timezone.as_deref())?;
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
        if let Some(id) = self.calendar_ids.iter().find(|id| !crate::calendar::registry().contains(id)) {
            return Err(AppError::validation(format!("Holiday calendar {id} does not exist")));
        }
        Ok(())
    }

    fn day_allowed<Tz: TimeZone>(&self, t: &DateTime<Tz>) -> bool {
        self.weekdays.is_empty() || self.weekdays.contains(&(t.weekday().num_days_from_monday() as u8))
    }

    /// First slot strictly after `after`, in the schedule's zone.
    pub fn next_after(&self, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
        match self.zone() {
            ScheduleZone::Local => self.next_after_in(after, &Local),
            ScheduleZone::Fixed(o) => self.next_after_in(after, &o),
        }
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

    /// The next `n` runs after `after` (in the schedule's zone).
    pub fn upcoming(&self, after: DateTime<Utc>, n: usize) -> Vec<DateTime<Utc>> {
        match self.zone() {
            ScheduleZone::Local => self.upcoming_in(after, n, &Local),
            ScheduleZone::Fixed(o) => self.upcoming_in(after, n, &o),
        }
    }

    /// The holiday calendar that makes the day of `at` (in the schedule's
    /// zone) a day off, if any.
    pub fn day_off(&self, at: DateTime<Utc>) -> Option<String> {
        crate::calendar::registry().blocked_by(&self.calendar_ids, self.zone().date_of(at))
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
    /// calendar skips (the "next runs" preview of the editor). `zoned` is the
    /// same instant with the schedule zone's offset.
    pub fn preview(&self, after: DateTime<Utc>, n: usize) -> Result<Vec<PreviewRun>, AppError> {
        self.validate()?;
        let on = Self { enabled: true, ..self.clone() };
        let zone = self.zone();
        Ok(on
            .upcoming(after, n.clamp(1, 100))
            .into_iter()
            .map(|t| PreviewRun { at: t.to_rfc3339(), skipped_by: self.day_off(t), zoned: Some(zone.rfc3339(t)) })
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

/// Scheduled verification: when, and how much (scope, sampling).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VerifyTask {
    pub schedule: BackupSchedule,
    pub options: super::verify::VerifyOptions,
}

/// Maintenance that runs on its own schedule instead of after backups.
/// A compaction schedule replaces compaction after each backup (and
/// `auto_compact_interval_days`); a retention schedule replaces retention
/// after each backup. Unset tasks keep the after-backup behaviour.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct MaintenanceSchedule {
    pub compact: Option<BackupSchedule>,
    pub verify: Option<VerifyTask>,
    pub retention: Option<BackupSchedule>,
}

/// Maintenance task names (also the operation names in the run history).
pub const MAINTENANCE_TASKS: [&str; 3] = ["compact", "verify", "apply_retention"];

impl MaintenanceSchedule {
    /// The schedule of a task (`compact`, `verify`, `apply_retention`), when set.
    pub fn schedule_of(&self, task: &str) -> Option<&BackupSchedule> {
        match task {
            "compact" => self.compact.as_ref(),
            "verify" => self.verify.as_ref().map(|v| &v.schedule),
            "apply_retention" => self.retention.as_ref(),
            _ => None,
        }
    }

    /// Whether a task runs on its own (enabled) schedule.
    pub fn scheduled(&self, task: &str) -> bool {
        self.schedule_of(task).is_some_and(|s| s.enabled)
    }

    pub fn validate(&self) -> Result<(), AppError> {
        for t in MAINTENANCE_TASKS {
            if let Some(s) = self.schedule_of(t).filter(|s| s.enabled) {
                s.validate().map_err(|e| AppError::validation(format!("Maintenance schedule ({t}): {}", e.user_message())))?;
            }
        }
        if let Some(v) = &self.verify {
            v.options.validate()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holiday_calendars_from_the_registry_skip_days() {
        let s = BackupSchedule { enabled: true, calendar_ids: vec!["core-test-cal".into()], timezone: Some("UTC".into()), ..Default::default() };
        assert!(s.validate().unwrap_err().user_message().contains("core-test-cal"));
        let day = NaiveDate::from_ymd_opt(2026, 12, 25).unwrap();
        crate::calendar::registry().put(crate::calendar::HolidayCalendar { id: "core-test-cal".into(), name: "Xmas".into(), dates: vec![day], ..Default::default() });
        s.validate().unwrap();
        let at = Utc.with_ymd_and_hms(2026, 12, 25, 2, 0, 0).unwrap();
        assert_eq!(s.day_off(at).as_deref(), Some("Xmas"));
        assert_eq!(s.day_off(at + Duration::days(1)), None);
        let p = s.preview(at - Duration::hours(1), 2).unwrap();
        assert_eq!((p[0].skipped_by.as_deref(), p[1].skipped_by.as_deref()), (Some("Xmas"), None));
        crate::calendar::registry().remove("core-test-cal");
    }

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
    fn time_zone_names_parse_and_bad_ones_are_refused() {
        let off = |m: i32| ScheduleZone::Fixed(FixedOffset::east_opt(m * 60).unwrap());
        assert_eq!(ScheduleZone::parse(None).unwrap(), ScheduleZone::Local);
        assert_eq!(ScheduleZone::parse(Some(" Local ")).unwrap(), ScheduleZone::Local);
        assert_eq!(ScheduleZone::parse(Some("")).unwrap(), ScheduleZone::Local);
        for z in ["UTC", "utc", "GMT", "Z", "Etc/UTC", "+00:00", "UTC+0"] {
            assert_eq!(ScheduleZone::parse(Some(z)).unwrap(), off(0), "{z}");
        }
        assert_eq!(ScheduleZone::parse(Some("+02:00")).unwrap(), off(120));
        assert_eq!(ScheduleZone::parse(Some("UTC-05:30")).unwrap(), off(-330));
        assert_eq!(ScheduleZone::parse(Some("-0530")).unwrap(), off(-330));
        assert_eq!(ScheduleZone::parse(Some("GMT+1")).unwrap(), off(60));
        assert_eq!(ScheduleZone::parse(Some("+14:00")).unwrap(), off(840));
        for bad in ["Europe/Berlin", "+15:00", "+02:60", "2", "+", "UTC+", "+1:2:3", "+abc", "+123"] {
            let e = ScheduleZone::parse(Some(bad)).unwrap_err();
            assert!(e.user_message().contains("fixed offset"), "{bad}: {}", e.user_message());
        }
        let s = BackupSchedule { timezone: Some("Mars/Olympus".into()), ..Default::default() };
        assert!(s.validate().is_err(), "saving refuses an unknown zone");
        assert_eq!(s.zone(), ScheduleZone::Local, "a bad stored zone falls back to local time");
    }

    #[test]
    fn interval_weekday_and_cron_follow_the_schedule_zone() {
        // Daily 02:00 at UTC+2 runs at 00:00 UTC.
        let daily = BackupSchedule { enabled: true, start_time: "02:00".into(), interval_minutes: 1440, timezone: Some("+02:00".into()), ..Default::default() };
        daily.validate().unwrap();
        assert_eq!(daily.next_after(utc(2026, 3, 16, 1, 0)), Some(utc(2026, 3, 17, 0, 0)));
        assert_eq!(daily.next_after(utc(2026, 3, 15, 23, 0)), Some(utc(2026, 3, 16, 0, 0)));
        // Weekdays are the zone's weekdays: Monday 01:00 at UTC+5 is Sunday 20:00 UTC.
        let monday = BackupSchedule { enabled: true, start_time: "01:00".into(), interval_minutes: 1440, weekdays: vec![0], timezone: Some("UTC+5".into()), ..Default::default() };
        assert_eq!(monday.next_after(utc(2026, 3, 14, 0, 0)), Some(utc(2026, 3, 15, 20, 0)), "2026-03-15 is a Sunday in UTC");
        // Cron in UTC−05:00: 09:00 there is 14:00 UTC.
        let cron = BackupSchedule { timezone: Some("-05:00".into()), ..BackupSchedule::cron(&["0 9 * * *"]) };
        assert_eq!(cron.upcoming(utc(2026, 3, 16, 15, 0), 2), vec![utc(2026, 3, 17, 14, 0), utc(2026, 3, 18, 14, 0)]);
        // Catch-up after downtime and the nominal interval use the zone too.
        assert!(cron.is_due(utc(2026, 3, 16, 15, 0), utc(2026, 3, 17, 14, 1)));
        assert!(!cron.is_due(utc(2026, 3, 16, 15, 0), utc(2026, 3, 17, 13, 59)));
        assert_eq!(cron.nominal_interval_minutes(), 1440);
        // Maintenance schedules are BackupSchedules, so they follow their zone as well.
        let m = MaintenanceSchedule { compact: Some(cron.clone()), ..Default::default() };
        assert_eq!(m.schedule_of("compact").and_then(|s| s.next_after(utc(2026, 3, 16, 15, 0))), Some(utc(2026, 3, 17, 14, 0)));
    }

    #[test]
    fn preview_shows_the_zone_time_and_old_json_still_loads() {
        let s = BackupSchedule { timezone: Some("UTC".into()), ..BackupSchedule::cron(&["30 2 * * *"]) };
        let p = s.preview(utc(2026, 3, 16, 3, 0), 1).unwrap();
        assert_eq!(p[0].at, utc(2026, 3, 17, 2, 30).to_rfc3339());
        assert_eq!(p[0].zoned.as_deref(), Some("2026-03-17T02:30:00+00:00"));
        let plus = BackupSchedule { timezone: Some("+02:00".into()), ..s };
        assert_eq!(plus.preview(utc(2026, 3, 16, 3, 0), 1).unwrap()[0].zoned.as_deref(), Some("2026-03-17T02:30:00+02:00"));
        let old: BackupSchedule = serde_json::from_value(serde_json::json!({"enabled": true, "start_time": "03:00", "interval_minutes": 60})).unwrap();
        assert_eq!((old.timezone.clone(), old.zone()), (None, ScheduleZone::Local));
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

    #[test]
    fn maintenance_schedules() {
        let mut m = MaintenanceSchedule::default();
        assert!(!m.scheduled("compact"));
        m.compact = Some(BackupSchedule::cron(&["0 3 * * Sun"]));
        m.verify = Some(VerifyTask { schedule: BackupSchedule { enabled: false, ..Default::default() }, ..Default::default() });
        assert!(m.scheduled("compact") && !m.scheduled("verify") && !m.scheduled("apply_retention"));
        m.validate().unwrap();
        m.retention = Some(BackupSchedule::cron(&["* * * * *"]));
        assert!(m.validate().unwrap_err().user_message().contains("apply_retention"));
        let old: MaintenanceSchedule = serde_json::from_str("{}").unwrap();
        assert_eq!(old, MaintenanceSchedule::default());
    }
}
