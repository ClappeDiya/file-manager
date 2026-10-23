//! Schedule library: named cron schedules and holiday calendars, defined
//! once and used by sync pairs, automation rules (both by schedule) and
//! backup jobs (by calendar).
//!
//! - A **named schedule** is a cron expression (the 7-field form the sync
//!   and automation schedulers use, in UTC) plus holiday calendars whose
//!   days it skips.
//! - A **holiday calendar** (`ufop_core::calendar`) lists days off; dates
//!   are local dates. Calendars live in `ufop_core::calendar::registry`,
//!   which the backup schedules (`ufop_core::backup::schedule`) consult.
//! - Everything lives in one process-wide [`registry`] that the schedulers
//!   consult when they check what is due, so editing a schedule or calendar
//!   applies everywhere at once, without re-registering anything. The
//!   [`ScheduleLibrary`] persists it (migration v27).

use crate::core::error::AppError;
use crate::storage::pool::DbPool;
use chrono::{DateTime, Local, NaiveDate, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::{OnceLock, RwLock};
use ufop_core::calendar::HolidayCalendar;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct NamedSchedule {
    pub id: String,
    pub name: String,
    pub description: String,
    /// `sec min hour day-of-month month day-of-week [year]`, UTC.
    pub cron_expr: String,
    /// Holiday calendars whose days this schedule skips.
    pub calendar_ids: Vec<String>,
    pub updated_at: String,
}

#[derive(Default)]
struct State {
    schedules: HashMap<String, NamedSchedule>,
}

fn calendars() -> &'static ufop_core::calendar::CalendarRegistry {
    ufop_core::calendar::registry()
}

/// The live schedules and calendars.
#[derive(Default)]
pub struct Registry(RwLock<State>);

/// The process-wide registry.
pub fn registry() -> &'static Registry {
    static R: OnceLock<Registry> = OnceLock::new();
    R.get_or_init(Registry::default)
}

/// A local date's calendar check result: the calendar that blocks it.
pub type Blocked = Option<String>;

impl Registry {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, State> {
        self.0.read().unwrap_or_else(|p| p.into_inner())
    }
    fn write(&self) -> std::sync::RwLockWriteGuard<'_, State> {
        self.0.write().unwrap_or_else(|p| p.into_inner())
    }

    pub fn schedule(&self, id: &str) -> Option<NamedSchedule> {
        self.read().schedules.get(id).cloned()
    }

    pub fn calendar(&self, id: &str) -> Option<HolidayCalendar> {
        calendars().get(id)
    }

    /// The cron expression of a named schedule.
    pub fn cron_of(&self, schedule_id: &str) -> Option<String> {
        self.read().schedules.get(schedule_id).map(|s| s.cron_expr.clone())
    }

    /// Name of the first of `calendar_ids` that blocks local date `d`.
    pub fn blocked_by(&self, calendar_ids: &[String], d: NaiveDate) -> Blocked {
        calendars().blocked_by(calendar_ids, d)
    }

    /// Whether a run of `schedule_id` (plus `extra` calendars) at `at` falls on a day off.
    pub fn run_blocked(&self, schedule_id: Option<&str>, extra: &[String], at: DateTime<Utc>) -> Blocked {
        let d = at.with_timezone(&Local).date_naive();
        let mut ids: Vec<String> = schedule_id.and_then(|id| self.schedule(id)).map(|s| s.calendar_ids).unwrap_or_default();
        ids.extend(extra.iter().cloned());
        self.blocked_by(&ids, d)
    }

    fn put_schedule(&self, s: NamedSchedule) {
        self.write().schedules.insert(s.id.clone(), s);
    }
    fn put_calendar(&self, c: HolidayCalendar) {
        calendars().put(c);
    }
    fn remove_schedule(&self, id: &str) {
        self.write().schedules.remove(id);
    }
    fn remove_calendar(&self, id: &str) {
        calendars().remove(id);
    }

    #[cfg(test)]
    pub(crate) fn insert_for_tests(&self, s: Option<NamedSchedule>, c: Option<HolidayCalendar>) {
        if let Some(s) = s {
            self.put_schedule(s);
        }
        if let Some(c) = c {
            self.put_calendar(c);
        }
    }
}

/// A schedule field (sync pair, automation rule) holds a cron expression or
/// a reference to a named schedule: `schedule:<id>`. Cron parsing and the
/// preview row are shared with the backup schedules in ufop-core.
pub use ufop_core::backup::schedule::{parse_cron_any, schedule_ref, PreviewRun, REF_PREFIX};

/// The field's current schedule: parsed cron, its text, and the named
/// schedule it came from. A named schedule is looked up each time, so its
/// edits apply at once.
pub fn resolve(expr: &str) -> Result<(cron::Schedule, String, Option<String>), AppError> {
    match schedule_ref(expr) {
        Some(id) => {
            let s = registry().schedule(id).ok_or_else(|| AppError::validation(format!("Named schedule {id} does not exist")))?;
            Ok((validate_cron(&s.cron_expr)?, s.cron_expr, Some(id.to_string())))
        }
        None => Ok((validate_cron(expr)?, expr.trim().to_string(), None)),
    }
}

/// Validate a cron expression in the schedulers' format.
pub fn validate_cron(expr: &str) -> Result<cron::Schedule, AppError> {
    cron::Schedule::from_str(expr.trim()).map_err(|e| AppError::validation(format!("Invalid cron expression '{expr}': {e}. Use `sec min hour day month weekday [year]`, e.g. `0 30 9 * * Mon-Fri`.")))
}

/// The next `n` runs of `cron_expr`, marking those `calendar_ids` skip.
pub fn preview(cron_expr: &str, calendar_ids: &[String], after: DateTime<Utc>, n: usize) -> Result<Vec<PreviewRun>, AppError> {
    let s = validate_cron(cron_expr)?;
    Ok(s.after(&after)
        .take(n.clamp(1, 100))
        .map(|t| PreviewRun { at: t.to_rfc3339(), skipped_by: registry().blocked_by(calendar_ids, t.with_timezone(&Local).date_naive()), zoned: None })
        .collect())
}

/// Persistence for the registry.
#[derive(Clone)]
pub struct ScheduleLibrary {
    pool: DbPool,
}

fn now() -> String {
    Utc::now().to_rfc3339()
}

impl ScheduleLibrary {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Load everything into the registry; returns (schedules, calendars).
    pub async fn load(&self) -> Result<(usize, usize), AppError> {
        let (scheds, cals) = self
            .pool
            .execute(|c| {
                let read = |sql: &str| -> rusqlite::Result<Vec<String>> {
                    let mut st = c.prepare(sql)?;
                    let rows = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                    Ok(rows)
                };
                Ok((read("SELECT data FROM named_schedules")?, read("SELECT data FROM holiday_calendars")?))
            })
            .await?;
        let (mut ns, mut nc) = (0, 0);
        for j in cals {
            if let Ok(c) = serde_json::from_str::<HolidayCalendar>(&j) {
                registry().put_calendar(c);
                nc += 1;
            }
        }
        for j in scheds {
            if let Ok(s) = serde_json::from_str::<NamedSchedule>(&j) {
                registry().put_schedule(s);
                ns += 1;
            }
        }
        Ok((ns, nc))
    }

    pub async fn list_schedules(&self) -> Vec<NamedSchedule> {
        let mut v: Vec<NamedSchedule> = registry().read().schedules.values().cloned().collect();
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    }

    pub async fn list_calendars(&self) -> Vec<HolidayCalendar> {
        let mut v: Vec<HolidayCalendar> = calendars().all();
        v.sort_by_key(|a| a.name.to_lowercase());
        v
    }

    /// Create (empty id) or replace a schedule.
    pub async fn save_schedule(&self, mut s: NamedSchedule) -> Result<NamedSchedule, AppError> {
        s.name = s.name.trim().to_string();
        s.cron_expr = s.cron_expr.trim().to_string();
        if s.name.is_empty() || s.name.chars().count() > 100 {
            return Err(AppError::validation("Give the schedule a name (up to 100 characters)"));
        }
        validate_cron(&s.cron_expr)?;
        if let Some(missing) = s.calendar_ids.iter().find(|id| registry().calendar(id).is_none()) {
            return Err(AppError::validation(format!("Holiday calendar {missing} does not exist")));
        }
        if registry().read().schedules.values().any(|o| o.id != s.id && o.name.eq_ignore_ascii_case(&s.name)) {
            return Err(AppError::validation(format!("A schedule named '{}' already exists", s.name)));
        }
        if s.id.is_empty() {
            s.id = uuid::Uuid::new_v4().to_string();
        }
        s.updated_at = now();
        let (id, data, at) = (s.id.clone(), serde_json::to_string(&s)?, s.updated_at.clone());
        self.pool
            .execute(move |c| {
                c.execute("INSERT INTO named_schedules (id, data, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET data = ?2, updated_at = ?3", params![id, data, at])?;
                Ok(())
            })
            .await?;
        registry().put_schedule(s.clone());
        Ok(s)
    }

    /// Delete a schedule; `used_by` names what still refers to it (refused then).
    pub async fn delete_schedule(&self, id: &str, used_by: &[String]) -> Result<(), AppError> {
        if !used_by.is_empty() {
            return Err(AppError::validation(format!("The schedule is used by {}; change those first", used_by.join(", "))));
        }
        let id2 = id.to_string();
        self.pool.execute(move |c| Ok(c.execute("DELETE FROM named_schedules WHERE id = ?1", params![id2])?)).await?;
        registry().remove_schedule(id);
        Ok(())
    }

    pub async fn save_calendar(&self, mut c: HolidayCalendar) -> Result<HolidayCalendar, AppError> {
        c.name = c.name.trim().to_string();
        c.dates.sort();
        c.dates.dedup();
        c.annual.sort();
        c.annual.dedup();
        c.validate().map_err(AppError::validation)?;
        if calendars().all().iter().any(|o| o.id != c.id && o.name.eq_ignore_ascii_case(&c.name)) {
            return Err(AppError::validation(format!("A calendar named '{}' already exists", c.name)));
        }
        if c.id.is_empty() {
            c.id = uuid::Uuid::new_v4().to_string();
        }
        let (id, data, at) = (c.id.clone(), serde_json::to_string(&c)?, now());
        self.pool
            .execute(move |conn| {
                conn.execute("INSERT INTO holiday_calendars (id, data, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET data = ?2, updated_at = ?3", params![id, data, at])?;
                Ok(())
            })
            .await?;
        registry().put_calendar(c.clone());
        Ok(c)
    }

    /// Delete a calendar; refused while a schedule or `used_by` refers to it.
    pub async fn delete_calendar(&self, id: &str, used_by: &[String]) -> Result<(), AppError> {
        let mut users: Vec<String> = registry().read().schedules.values().filter(|s| s.calendar_ids.iter().any(|c| c == id)).map(|s| format!("schedule '{}'", s.name)).collect();
        users.extend(used_by.iter().cloned());
        if !users.is_empty() {
            return Err(AppError::validation(format!("The calendar is used by {}; change those first", users.join(", "))));
        }
        let id2 = id.to_string();
        self.pool.execute(move |c| Ok(c.execute("DELETE FROM holiday_calendars WHERE id = ?1", params![id2])?)).await?;
        registry().remove_calendar(id);
        Ok(())
    }

    /// Merge an iCalendar file into a calendar (a new one when `id` is empty).
    pub async fn import_ics(&self, id: Option<&str>, name: &str, ics: &str) -> Result<(HolidayCalendar, usize), AppError> {
        let mut c = match id.filter(|i| !i.is_empty()) {
            Some(i) => registry().calendar(i).ok_or_else(|| AppError::not_found(format!("Calendar {i} not found")))?,
            None => HolidayCalendar { name: name.to_string(), ..Default::default() },
        };
        let added = c.import_ics(ics).map_err(AppError::validation)?;
        Ok((self.save_calendar(c).await?, added))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Repository;

    #[tokio::test]
    async fn crud_persistence_preview_and_references() {
        let repo = Repository::open_in_memory().await.unwrap();
        let lib = ScheduleLibrary::new(repo.pool().clone());
        assert!(lib.save_calendar(HolidayCalendar { name: " ".into(), ..Default::default() }).await.is_err());
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let cal = lib
            .save_calendar(HolidayCalendar { name: format!("Office {tag}"), annual: vec!["12-25".into(), "12-25".into()], skip_weekends: true, ..Default::default() })
            .await
            .unwrap();
        assert_eq!(cal.annual, vec!["12-25".to_string()], "deduplicated");
        assert!(lib.save_schedule(NamedSchedule { name: "x".into(), cron_expr: "nonsense".into(), ..Default::default() }).await.is_err());
        assert!(lib.save_schedule(NamedSchedule { name: "x".into(), cron_expr: "0 0 9 * * *".into(), calendar_ids: vec!["nope".into()], ..Default::default() }).await.is_err());
        let s = lib.save_schedule(NamedSchedule { name: format!("Weekday 9am {tag}"), cron_expr: "0 0 9 * * *".into(), calendar_ids: vec![cal.id.clone()], ..Default::default() }).await.unwrap();
        assert!(!s.id.is_empty());
        assert!(lib.save_schedule(NamedSchedule { name: s.name.to_uppercase(), cron_expr: "0 0 9 * * *".into(), ..Default::default() }).await.is_err(), "duplicate name");
        assert_eq!(registry().cron_of(&s.id).as_deref(), Some("0 0 9 * * *"));

        // Preview over a week: the weekend and Christmas are marked.
        let start = Local.with_ymd_and_hms(2026, 12, 21, 0, 0, 0).unwrap().with_timezone(&Utc);
        let runs = preview(&s.cron_expr, &s.calendar_ids, start, 7).unwrap();
        assert_eq!(runs.len(), 7);
        let skipped: Vec<_> = runs.iter().filter(|r| r.skipped_by.is_some()).collect();
        assert!(skipped.len() >= 3, "Christmas + a weekend: {runs:?}");
        assert!(skipped.iter().all(|r| r.skipped_by.as_deref() == Some(cal.name.as_str())));

        // Calendars in use by a schedule (or anything else) cannot be deleted.
        assert!(lib.delete_calendar(&cal.id, &[]).await.unwrap_err().to_string().contains("schedule"));
        assert!(lib.delete_schedule(&s.id, &["sync pair 'Docs'".into()]).await.is_err());

        // Reload from the database into a fresh view.
        registry().remove_schedule(&s.id);
        registry().remove_calendar(&cal.id);
        let (ns, nc) = lib.load().await.unwrap();
        assert!(ns >= 1 && nc >= 1);
        assert_eq!(registry().schedule(&s.id).unwrap().calendar_ids, vec![cal.id.clone()]);

        // ICS import into the calendar.
        let (cal2, added) = lib.import_ics(Some(&cal.id), "", "BEGIN:VCALENDAR\nBEGIN:VEVENT\nDTSTART;VALUE=DATE:20270101\nEND:VEVENT\nEND:VCALENDAR\n").await.unwrap();
        assert_eq!((added, cal2.dates.len()), (1, 1));

        lib.delete_schedule(&s.id, &[]).await.unwrap();
        lib.delete_calendar(&cal.id, &[]).await.unwrap();
        assert!(registry().schedule(&s.id).is_none() && registry().calendar(&cal.id).is_none());
    }

    use chrono::TimeZone;
}
