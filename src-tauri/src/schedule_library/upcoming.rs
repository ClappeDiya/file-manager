//! When scheduled sync pairs and automation rules run next: the data behind
//! "Next run" in the sync and automation panels and the tray line.

use super::first_run_on_or_after;
use crate::automation_engine::{AutomationManager, RuleTrigger};
use crate::core::traits::SyncOperations;
use crate::sync_engine::SyncManager;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use ufop_core::sync_types::SyncTrigger;

/// One armed schedule and its next run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpcomingRun {
    /// `sync` or `automation`.
    pub kind: String,
    /// The pair's or rule's id.
    pub id: String,
    pub name: String,
    /// RFC 3339.
    pub next_run_at: String,
}

/// The next run as it will really happen: the armed schedule's next time,
/// moved past days off.
fn effective(expr: &str, armed_next: Option<DateTime<Utc>>) -> Option<DateTime<Utc>> {
    first_run_on_or_after(expr, armed_next?)
}

/// Enabled, armed pairs and rules with a schedule, soonest first. Watch,
/// manual and webhook triggers have no next run and are left out, as are
/// disabled ones.
pub async fn upcoming(sync: &SyncManager, automation: &AutomationManager) -> Vec<UpcomingRun> {
    let mut out = Vec::new();
    for p in sync.list_pairs().await.unwrap_or_default() {
        if let (true, SyncTrigger::Scheduled { cron_expr }) = (p.enabled, &p.trigger) {
            if let Some(t) = effective(cron_expr, sync.next_scheduled_run(p.id)) {
                out.push(UpcomingRun { kind: "sync".into(), id: p.id.to_string(), name: p.name, next_run_at: t.to_rfc3339() });
            }
        }
    }
    for r in automation.list_rules().await {
        if let (true, RuleTrigger::Schedule { cron_expr, .. }) = (r.enabled, &r.trigger) {
            if let Some(t) = effective(cron_expr, automation.next_scheduled_run(&r.id)) {
                out.push(UpcomingRun { kind: "automation".into(), id: r.id, name: r.name, next_run_at: t.to_rfc3339() });
            }
        }
    }
    out.sort_by(|a, b| a.next_run_at.cmp(&b.next_run_at));
    out
}

/// The tray's second status line: what runs next besides backups.
pub fn describe(runs: &[UpcomingRun]) -> Option<String> {
    let n = runs.first()?;
    let at = DateTime::parse_from_rfc3339(&n.next_run_at).map(|d| d.with_timezone(&chrono::Local).format("%a %H:%M").to_string()).unwrap_or_default();
    Some(format!("Next {}: {} at {at}", if n.kind == "sync" { "sync" } else { "automation" }, n.name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation_engine::{AutomationRule, RuleAction};
    use ufop_core::sync_types::SyncPair;

    #[tokio::test]
    async fn armed_schedules_are_listed_soonest_first_and_others_left_out() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let state = tempfile::tempdir().unwrap();
        let sync = SyncManager::with_state_dir(state.path().to_path_buf());
        let automation = AutomationManager::new();

        // A yearly pair (never due in the same minute as the minutely rule, so the
        // order below cannot tie), a pair on watch, a disabled scheduled pair.
        let hourly = SyncPair { name: "Hourly".into(), trigger: SyncTrigger::Scheduled { cron_expr: "0 0 0 1 1 * *".into() }, ..Default::default() };
        let watch = SyncPair { name: "Watch".into(), trigger: SyncTrigger::Watch, ..Default::default() };
        let off = SyncPair { name: "Off".into(), enabled: false, trigger: SyncTrigger::Scheduled { cron_expr: "0 0 * * * * *".into() }, ..Default::default() };
        for p in [&hourly, &watch, &off] {
            sync.create_pair(p.clone()).await.unwrap();
        }
        // A rule every minute (sooner than the pair) and a manual rule.
        let mut every_min = AutomationRule::new("Minutely".into(), RuleTrigger::Schedule { cron_expr: "0 * * * * * *".into(), cron_human: None }, RuleAction::CreateFolder { path: "/x".into() });
        every_min.enabled = true;
        let manual = AutomationRule::new("Manual".into(), RuleTrigger::Manual, RuleAction::CreateFolder { path: "/y".into() });
        for r in [&every_min, &manual] {
            automation.save_rule(&repo, r).await.unwrap();
        }
        automation.rearm(&every_min).await.unwrap();

        let runs = upcoming(&sync, &automation).await;
        assert_eq!(runs.iter().map(|r| (r.kind.as_str(), r.name.as_str())).collect::<Vec<_>>(), vec![("automation", "Minutely"), ("sync", "Hourly")], "{runs:?}");
        assert!(runs[0].next_run_at <= runs[1].next_run_at);
        let t = DateTime::parse_from_rfc3339(&runs[0].next_run_at).unwrap();
        assert!(t > Utc::now() - chrono::Duration::seconds(1) && t < Utc::now() + chrono::Duration::seconds(61), "{t}");
        assert!(describe(&runs).unwrap().starts_with("Next automation: Minutely at "));
        assert_eq!(describe(&[]), None);
    }

    #[test]
    fn days_off_are_skipped() {
        use ufop_core::calendar::HolidayCalendar;
        let reg = super::super::registry();
        // Every day at noon, but every day of the next 3 weeks is blocked by a calendar.
        let today = chrono::Local::now().date_naive();
        let days: Vec<_> = (0..21).map(|i| today + chrono::Duration::days(i)).collect();
        let cal = HolidayCalendar { id: "upcoming-test".into(), name: "Off".into(), dates: days, annual: vec![], skip_weekends: false };
        reg.insert_for_tests(
            Some(super::super::NamedSchedule { id: "upcoming-s".into(), name: "Noon".into(), description: String::new(), cron_expr: "0 0 12 * * * *".into(), calendar_ids: vec!["upcoming-test".into()], updated_at: String::new() }),
            Some(cal),
        );
        let now = Utc::now();
        let first = first_run_on_or_after("schedule:upcoming-s", now).unwrap();
        let plain = first_run_on_or_after("0 0 12 * * * *", now).unwrap();
        assert!(first > plain + chrono::Duration::days(19), "{first} vs {plain}");
        assert!(first_run_on_or_after("not cron", now).is_none());
    }
}
