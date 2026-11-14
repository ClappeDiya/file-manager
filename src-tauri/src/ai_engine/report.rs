//! Activity reports: what automation rules and file operations did over
//! the last days, as Markdown. The numbers are computed here; a local model
//! (when available and AI is on) only adds a short summary on top, from
//! those numbers — never from file contents.

use crate::automation_engine::{AutomationLog, AutomationStatus};
use crate::ledger::LedgerEvent;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActivityReport {
    pub days: u32,
    pub from: String,
    pub to: String,
    pub markdown: String,
    /// Short summary written by the local model, when one ran.
    pub summary: Option<String>,
    pub runs: u64,
    pub failed_runs: u64,
    pub operations: u64,
    pub failed_operations: u64,
}

fn parse(ts: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(ts).ok().map(|t| t.with_timezone(&Utc))
}

fn bytes(n: i64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let (mut v, mut u) = (n as f64, 0);
    while v >= 1024.0 && u < units.len() - 1 {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", units[u]) }
}

fn cell(s: &str) -> String {
    s.replace('|', "\\|").replace('\n', " ")
}

/// The report for the `days` before `now`.
pub fn activity_report(days: u32, now: DateTime<Utc>, logs: &[AutomationLog], events: &[LedgerEvent]) -> ActivityReport {
    let since = now - Duration::days(days.max(1) as i64);
    let in_range = |ts: &str| parse(ts).is_some_and(|t| t >= since && t <= now);
    let logs: Vec<&AutomationLog> = logs.iter().filter(|l| in_range(&l.triggered_at)).collect();
    let events: Vec<&LedgerEvent> = events.iter().filter(|e| in_range(&e.occurred_at)).collect();

    #[derive(Default)]
    struct RuleStats {
        runs: u64,
        failed: u64,
        files: u64,
        last_error: Option<(String, String)>,
    }
    let mut rules: BTreeMap<String, RuleStats> = BTreeMap::new();
    for l in &logs {
        let r = rules.entry(l.rule_name.clone()).or_default();
        r.runs += 1;
        r.files += l.files_affected.len() as u64;
        if l.status == AutomationStatus::Failed {
            r.failed += 1;
            if r.last_error.as_ref().map_or(true, |(t, _)| l.triggered_at > *t) {
                r.last_error = Some((l.triggered_at.clone(), l.error_message.clone().unwrap_or_default()));
            }
        }
    }
    let failed_runs: u64 = rules.values().map(|r| r.failed).sum();

    let mut kinds: BTreeMap<(String, String), (u64, u64, i64)> = BTreeMap::new();
    for e in &events {
        let k = kinds.entry((e.engine.clone(), e.kind.clone())).or_default();
        k.0 += 1;
        if e.status != crate::ledger::LedgerStatus::Ok.as_str() {
            k.1 += 1;
        }
        k.2 += e.bytes.unwrap_or(0).max(0);
    }
    let failed_ops: u64 = kinds.values().map(|k| k.1).sum();

    let mut md = format!("# Activity report — last {days} day{}\n\n{} to {}\n\n", if days == 1 { "" } else { "s" }, since.format("%Y-%m-%d %H:%M"), now.format("%Y-%m-%d %H:%M UTC"));
    md.push_str(&format!("- Automation: **{}** runs, **{}** failed\n- File operations: **{}**, **{}** not successful\n\n", logs.len(), failed_runs, events.len(), failed_ops));

    md.push_str("## Automation rules\n\n");
    if rules.is_empty() {
        md.push_str("No rule ran in this period.\n\n");
    } else {
        md.push_str("| Rule | Runs | Failed | Files | Last error |\n|---|---:|---:|---:|---|\n");
        let mut ordered: Vec<(&String, &RuleStats)> = rules.iter().collect();
        ordered.sort_by(|a, b| b.1.failed.cmp(&a.1.failed).then(b.1.runs.cmp(&a.1.runs)).then(a.0.cmp(b.0)));
        for (name, r) in ordered {
            md.push_str(&format!("| {} | {} | {} | {} | {} |\n", cell(name), r.runs, r.failed, r.files, r.last_error.as_ref().map(|(_, e)| cell(e)).unwrap_or_default()));
        }
        md.push('\n');
    }

    md.push_str("## File operations\n\n");
    if kinds.is_empty() {
        md.push_str("No file operations were recorded in this period.\n\n");
    } else {
        md.push_str("| Engine | Operation | Count | Not successful | Data |\n|---|---|---:|---:|---:|\n");
        for ((engine, kind), (n, bad, b)) in &kinds {
            md.push_str(&format!("| {} | {} | {n} | {bad} | {} |\n", cell(engine), cell(kind), if *b > 0 { bytes(*b) } else { "—".into() }));
        }
        md.push('\n');
    }

    let attention: Vec<String> = rules
        .iter()
        .filter(|(_, r)| r.failed > 0 && r.failed * 2 >= r.runs)
        .map(|(n, r)| format!("- **{n}** failed {} of {} runs{}", r.failed, r.runs, r.last_error.as_ref().map(|(_, e)| format!(": {e}")).unwrap_or_default()))
        .collect();
    if !attention.is_empty() {
        md.push_str("## Needs attention\n\n");
        md.push_str(&attention.join("\n"));
        md.push('\n');
    }

    ActivityReport {
        days,
        from: since.to_rfc3339(),
        to: now.to_rfc3339(),
        markdown: md,
        summary: None,
        runs: logs.len() as u64,
        failed_runs,
        operations: events.len() as u64,
        failed_operations: failed_ops,
    }
}

/// Prompt for the model's summary (the report's numbers only).
pub fn summary_prompt(report: &ActivityReport) -> String {
    format!(
        "Summarize this file-activity report for the person who owns these automations in 2–4 sentences. \
         Lead with anything that needs attention; mention totals briefly; do not invent facts.\n\n{}",
        report.markdown
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log(rule: &str, at: DateTime<Utc>, failed: bool) -> AutomationLog {
        AutomationLog {
            id: uuid::Uuid::new_v4().to_string(),
            rule_id: rule.into(),
            rule_name: rule.into(),
            triggered_at: at.to_rfc3339(),
            completed_at: None,
            trigger_event: "manual".into(),
            status: if failed { AutomationStatus::Failed } else { AutomationStatus::Success },
            files_affected: vec!["/a".into(), "/b".into()],
            error_message: failed.then(|| format!("disk | full at {}", at.timestamp())),
            attempts: 1,
            steps: vec![],
        }
    }

    fn event(kind: &str, at: DateTime<Utc>, status: &str, bytes: i64) -> LedgerEvent {
        LedgerEvent {
            id: uuid::Uuid::new_v4().to_string(),
            occurred_at: at.to_rfc3339(),
            engine: "fs".into(),
            kind: kind.into(),
            status: status.into(),
            subject_path: None,
            target_path: None,
            bytes: Some(bytes),
            correlation_id: None,
            summary: String::new(),
            details_json: "{}".into(),
            undo_token: None,
        }
    }

    #[test]
    fn counts_only_the_period_and_flags_failing_rules() {
        let now = Utc::now();
        let h = |n: i64| now - Duration::hours(n);
        let logs = vec![log("Tidy", h(1), false), log("Tidy", h(2), false), log("Upload", h(3), true), log("Upload", h(5), true), log("Upload", h(4), false), log("Old", h(24 * 9), true)];
        let events = vec![event("copy", h(1), "ok", 3 * 1024 * 1024), event("copy", h(2), "failed", 0), event("delete", h(1), "ok", 0), event("copy", h(24 * 30), "ok", 1)];
        let r = activity_report(7, now, &logs, &events);
        assert_eq!((r.runs, r.failed_runs, r.operations, r.failed_operations), (5, 2, 3, 1));
        assert!(!r.markdown.contains("Old"), "outside the period");
        assert!(r.markdown.contains("| Upload | 3 | 2 | 6 | disk \\| full at"), "{}", r.markdown);
        assert!(r.markdown.find("| Upload").unwrap() < r.markdown.find("| Tidy").unwrap(), "failing rules first");
        assert!(r.markdown.contains("| fs | copy | 2 | 1 | 3.0 MB |"));
        assert!(r.markdown.contains("## Needs attention\n\n- **Upload** failed 2 of 3 runs: disk | full at"), "the latest error");
        assert!(summary_prompt(&r).contains("do not invent facts"));
        let empty = activity_report(1, now, &[], &[]);
        assert!(empty.markdown.contains("last 1 day\n") && empty.markdown.contains("No rule ran") && !empty.markdown.contains("Needs attention"));
    }
}
