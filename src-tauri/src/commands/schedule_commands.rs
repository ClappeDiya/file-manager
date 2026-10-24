//! Tauri commands for the schedule library: named schedules and holiday
//! calendars shared by sync pairs, automation rules and backup jobs.

use crate::automation_engine::{AutomationManager, RuleTrigger};
use crate::backup_engine::BackupManager;
use crate::core::error::AppError;
use crate::core::traits::SyncOperations;
use crate::schedule_library::{self, NamedSchedule, PreviewRun, ScheduleLibrary, REF_PREFIX};
use crate::sync_engine::SyncManager;
use tauri::State;
use ufop_core::calendar::HolidayCalendar;
use ufop_core::sync_types::SyncTrigger;

const MAX_ICS_BYTES: u64 = 5 * 1024 * 1024;

#[tauri::command]
pub async fn schedules_list(lib: State<'_, ScheduleLibrary>) -> Result<Vec<NamedSchedule>, AppError> {
    Ok(lib.list_schedules().await)
}

#[tauri::command]
pub async fn schedule_save(schedule: NamedSchedule, lib: State<'_, ScheduleLibrary>) -> Result<NamedSchedule, AppError> {
    lib.save_schedule(schedule).await
}

/// Sync pairs and automation rules that run on `schedule:<id>`.
async fn schedule_users(id: &str, sync: &SyncManager, automation: &AutomationManager) -> Vec<String> {
    let r = format!("{REF_PREFIX}{id}");
    let mut users: Vec<String> = sync
        .list_pairs()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|p| matches!(&p.trigger, SyncTrigger::Scheduled { cron_expr } if cron_expr.trim() == r))
        .map(|p| format!("sync pair '{}'", p.name))
        .collect();
    users.extend(
        automation
            .list_rules()
            .await
            .into_iter()
            .filter(|rule| matches!(&rule.trigger, RuleTrigger::Schedule { cron_expr, .. } if cron_expr.trim() == r))
            .map(|rule| format!("rule '{}'", rule.name)),
    );
    users
}

#[tauri::command]
pub async fn schedule_delete(id: String, lib: State<'_, ScheduleLibrary>, sync: State<'_, SyncManager>, automation: State<'_, AutomationManager>) -> Result<(), AppError> {
    let users = schedule_users(&id, &sync, &automation).await;
    lib.delete_schedule(&id, &users).await
}

/// When each enabled, scheduled sync pair and automation rule runs next,
/// soonest first (days off of their calendars skipped).
#[tauri::command]
pub async fn upcoming_runs(sync: State<'_, SyncManager>, automation: State<'_, AutomationManager>) -> Result<Vec<schedule_library::upcoming::UpcomingRun>, AppError> {
    Ok(schedule_library::upcoming::upcoming(&sync, &automation).await)
}

#[tauri::command]
pub async fn calendars_list(lib: State<'_, ScheduleLibrary>) -> Result<Vec<HolidayCalendar>, AppError> {
    Ok(lib.list_calendars().await)
}

#[tauri::command]
pub async fn calendar_save(calendar: HolidayCalendar, lib: State<'_, ScheduleLibrary>) -> Result<HolidayCalendar, AppError> {
    lib.save_calendar(calendar).await
}

#[tauri::command]
pub async fn calendar_delete(id: String, lib: State<'_, ScheduleLibrary>, backup: State<'_, BackupManager>) -> Result<(), AppError> {
    let users: Vec<String> = backup.list_jobs().await.into_iter().filter(|j| j.schedule.calendar_ids.contains(&id)).map(|j| format!("backup job '{}'", j.name)).collect();
    lib.delete_calendar(&id, &users).await
}

/// Add the all-day events of an `.ics` file to a calendar (a new one when
/// `id` is empty); returns the calendar and how many dates were added.
#[tauri::command]
pub async fn calendar_import_ics(id: Option<String>, name: String, path: String, lib: State<'_, ScheduleLibrary>) -> Result<(HolidayCalendar, usize), AppError> {
    let len = tokio::fs::metadata(&path).await?.len();
    if len > MAX_ICS_BYTES {
        return Err(AppError::validation("The calendar file is larger than 5 MB"));
    }
    let text = tokio::fs::read_to_string(&path).await.map_err(|e| AppError::validation(format!("Cannot read {path}: {e}")))?;
    lib.import_ics(id.as_deref(), &name, &text).await
}

/// Upcoming runs of a cron expression or `schedule:<id>`, marking those
/// the schedule's (and `calendar_ids`') days off skip.
#[tauri::command]
pub async fn schedule_preview(cron_expr: String, calendar_ids: Option<Vec<String>>, count: Option<usize>) -> Result<Vec<PreviewRun>, AppError> {
    let (_, cron, named) = schedule_library::resolve(&cron_expr)?;
    let mut ids = named.and_then(|id| schedule_library::registry().schedule(&id)).map(|s| s.calendar_ids).unwrap_or_default();
    ids.extend(calendar_ids.unwrap_or_default());
    schedule_library::preview(&cron, &ids, chrono::Utc::now(), count.unwrap_or(10))
}
