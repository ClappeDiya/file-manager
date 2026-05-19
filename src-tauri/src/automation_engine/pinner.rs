//! Operation Pin & Retry — turn a single past ledger event into a saved
//! manual-trigger Quickflow (`build_pinned_rule`), or into an ephemeral
//! rule that re-attempts the operation right now (`build_retry_rule`).
//!
//! # Why this exists
//!
//! The unified [`crate::ledger::OperationLedger`] records every file
//! operation, successful or not. The automation engine already executes
//! Quickflows. The pieces missing — until now — were the bridges that
//! turn an event into a re-runnable artifact: forwards in time for the
//! Pin flow, and immediately for the Retry flow.
//!
//! # Symmetry with the rest of the ledger axis
//!
//! Three retrospective surfaces already exist over the ledger:
//!
//! - **Universal Undo** — reverses a past *successful* op (`undo_commands.rs`)
//! - **File Lineage** — shows where a file came from
//! - **Quickflow Suggester** — proposes rules from observed *patterns*
//!
//! `Operation Pin` adds the prospective leaf for *successful* events:
//! turn one specific past op into a re-runnable manual rule.
//!
//! `Operation Retry` closes the missing failure-side symmetry: every
//! success surface (Pin, Undo, Suggester, Smart Send-To, Frecency) was
//! born from `status="ok"` events; retry is the same lens applied to
//! `status="failed"` (or `cancelled`) events. The user can say "this
//! didn't work last time — try it again now" with one click, without
//! cluttering the rule list with one-shot retries.
//!
//! # DRY-by-construction
//!
//! - Source of truth: the existing `operation_ledger` table.
//! - Storage: existing `automation_rules` table (Pin) or none (Retry).
//! - Execution: the existing `executor::dispatch_action` via the
//!   `RuleAction::ReplayOp` branch — same code path for both flows.
//! - UI: one button per row on the existing activity-timeline (Pin for
//!   successful rows, Retry for failed rows — never both, since each
//!   event has exactly one status).
//!
//! No new tables, no new background daemons, no new IPC streams.

use crate::automation_engine::{AutomationRule, RuleAction, RuleTrigger};
use crate::core::error::AppError;
use crate::ledger::{LedgerEvent, LedgerQuery, OperationLedger};
use std::path::Path;

/// Ledger event kinds that can be pinned or retried today. Kept narrow
/// on purpose: only file-level copy and move have an obvious replay
/// semantic. Rename is excluded because the rename pattern (lowercase,
/// regex, date prefix) is too varied to infer from raw rows; deletes
/// are excluded because "replay a delete" is a footgun, not a feature.
const PINNABLE_KINDS: &[&str] = &["copy", "move"];

/// Statuses accepted by [`build_pinned_rule`]. Only successful ops can
/// be promoted to a future-runnable rule — pinning a failed op would
/// schedule a known-broken operation against the user.
const PIN_STATUSES: &[&str] = &["ok"];

/// Statuses accepted by [`build_retry_rule`]. Failed and cancelled
/// events both represent attempts that did not change disk state at
/// their original target, so they are safe to re-attempt now. Skipped
/// is excluded — a skip is an explicit decision, not a transient
/// failure to recover from.
const RETRY_STATUSES: &[&str] = &["failed", "cancelled"];

/// Build (but do NOT save) an [`AutomationRule`] that re-runs the file
/// operation captured by the given ledger correlation group.
///
/// Returns an `AppError` when:
/// - the correlation id has no matching rows in the ledger,
/// - none of the rows are pinnable (whitelisted, ok, has both subject and target),
/// - the rows span multiple destination directories (we'd lose fidelity), or
/// - the rows mix multiple operation kinds (suggests data corruption).
///
/// The caller is responsible for persisting the returned rule via
/// [`crate::automation_engine::AutomationManager::save_rule`]. Splitting
/// the build/save responsibilities keeps this module free of any
/// repository or manager dependency, mirroring `LedgerSuggester::build_rule`.
pub async fn build_pinned_rule(
    correlation_id: &str,
    ledger: &OperationLedger,
) -> Result<AutomationRule, AppError> {
    build_replay_rule(correlation_id, ledger, PIN_STATUSES, "Re-run").await
}

/// Build (but do NOT save) an [`AutomationRule`] that re-attempts a
/// failed (or cancelled) file operation captured by the given ledger
/// correlation group.
///
/// Semantically symmetric to [`build_pinned_rule`]: same kind allow-list,
/// same parent-dir consistency check, same DRY internal helper. The only
/// behavioural difference is the accepted status set: `failed` and
/// `cancelled` instead of `ok`, and the rule name prefix.
///
/// The caller should execute the returned rule **without persisting it**
/// (e.g. via [`crate::automation_engine::executor::execute_rule_action`])
/// — retries are one-shot operations and shouldn't pollute the saved
/// rules list. Saving is supported only for the symmetry of the API.
pub async fn build_retry_rule(
    correlation_id: &str,
    ledger: &OperationLedger,
) -> Result<AutomationRule, AppError> {
    build_replay_rule(correlation_id, ledger, RETRY_STATUSES, "Retry").await
}

/// DRY core for both [`build_pinned_rule`] and [`build_retry_rule`].
///
/// - `accept_statuses` filters which ledger rows are eligible.
/// - `name_verb` is the human-readable verb stitched into the rule name
///   (e.g. `"Re-run"` for Pin, `"Retry"` for Retry).
///
/// Kept private because every external caller wants one of the two
/// well-named flavours above, not a free-form selector.
async fn build_replay_rule(
    correlation_id: &str,
    ledger: &OperationLedger,
    accept_statuses: &[&str],
    name_verb: &str,
) -> Result<AutomationRule, AppError> {
    let events = ledger
        .query(LedgerQuery {
            engine: Some("fs".to_string()),
            kind: None,
            subject_path: None,
            correlation_id: Some(correlation_id.to_string()),
            limit: Some(500),
        })
        .await?;

    let eligible: Vec<&LedgerEvent> = events
        .iter()
        .filter(|e| {
            accept_statuses.contains(&e.status.as_str())
                && PINNABLE_KINDS.contains(&e.kind.as_str())
                && e.subject_path.is_some()
                && e.target_path.is_some()
        })
        .collect();

    if eligible.is_empty() {
        let status_label = accept_statuses.join("/");
        return Err(AppError::file_op(
            format!(
                "No replayable fs events found for correlation '{correlation_id}' (status {status_label})"
            ),
            "Only fs.copy and fs.move events with both source and target paths can be replayed.",
        ));
    }

    // All rows in a real correlation share the same kind. If they don't,
    // refuse rather than silently picking one — the user should not get
    // a surprise rule that does only half of the original operation.
    let kind = eligible[0].kind.clone();
    if eligible.iter().any(|e| e.kind != kind) {
        return Err(AppError::file_op(
            "Cannot replay: correlation contains mixed operation kinds.".to_string(),
            "This usually indicates a partial undo state. Refresh the timeline and try again.",
        ));
    }

    // Verify every target shares the same parent directory. The replay
    // action carries one dest_path, so divergent parents would silently
    // collapse files into the wrong place — refuse instead.
    let dest_path = parent_dir(eligible[0].target_path.as_deref().unwrap_or_default())
        .ok_or_else(|| {
            AppError::file_op(
                "Cannot replay: target path has no parent directory.".to_string(),
                "The original event may have targeted a root-level path; replay is not supported there.",
            )
        })?;
    let all_share_parent = eligible.iter().all(|e| {
        e.target_path
            .as_deref()
            .and_then(parent_dir)
            .as_deref()
            == Some(dest_path.as_str())
    });
    if !all_share_parent {
        return Err(AppError::file_op(
            "Cannot replay: original operation spanned multiple destination directories.".to_string(),
            "Replay individual events instead, or recreate as a custom rule.",
        ));
    }

    let source_paths: Vec<String> = eligible
        .iter()
        .filter_map(|e| e.subject_path.clone())
        .collect();

    let action = RuleAction::ReplayOp {
        kind: kind.clone(),
        source_paths: source_paths.clone(),
        dest_path: dest_path.clone(),
    };

    let count = source_paths.len();
    let kind_label = if kind == "move" { "Move" } else { "Copy" };
    let suffix = if count == 1 { "" } else { "s" };
    let dest_label = short_dir(&dest_path);
    let name = format!("{name_verb}: {kind_label} {count} file{suffix} → {dest_label}");

    Ok(AutomationRule::new(name, RuleTrigger::Manual, action))
}

fn parent_dir(path: &str) -> Option<String> {
    Path::new(path)
        .parent()
        .and_then(|p| p.to_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
}

fn short_dir(dir: &str) -> String {
    Path::new(dir)
        .file_name()
        .and_then(|n| n.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| dir.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{LedgerEngine, LedgerStatus, RecordEvent};
    use crate::storage::Repository;

    async fn fresh_ledger() -> OperationLedger {
        let repo = Repository::open_in_memory().await.unwrap();
        OperationLedger::new(repo.pool().clone())
    }

    async fn record_op(
        ledger: &OperationLedger,
        kind: &str,
        from: &str,
        to: &str,
        cid: &str,
        status: LedgerStatus,
    ) {
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, kind)
                    .status(status)
                    .subject(from)
                    .target(to)
                    .correlation(cid),
            )
            .await;
    }

    #[tokio::test]
    async fn empty_correlation_returns_error() {
        let ledger = fresh_ledger().await;
        let result = build_pinned_rule("does-not-exist", &ledger).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn single_copy_yields_replay_rule() {
        let ledger = fresh_ledger().await;
        record_op(
            &ledger,
            "copy",
            "/src/a.txt",
            "/dst/a.txt",
            "p1",
            LedgerStatus::Ok,
        )
        .await;

        let rule = build_pinned_rule("p1", &ledger).await.unwrap();
        assert!(!rule.enabled, "pinned rules must be disabled by default");
        assert!(matches!(rule.trigger, RuleTrigger::Manual));
        match rule.action {
            RuleAction::ReplayOp {
                ref kind,
                ref source_paths,
                ref dest_path,
            } => {
                assert_eq!(kind, "copy");
                assert_eq!(source_paths, &vec!["/src/a.txt".to_string()]);
                assert_eq!(dest_path, "/dst");
            }
            _ => panic!("expected ReplayOp action, got {:?}", rule.action),
        }
        assert!(rule.name.contains("Copy"));
        assert!(rule.name.contains("dst"));
    }

    #[tokio::test]
    async fn multi_file_move_groups_by_correlation() {
        let ledger = fresh_ledger().await;
        record_op(
            &ledger,
            "move",
            "/src/a.txt",
            "/dst/a.txt",
            "p2",
            LedgerStatus::Ok,
        )
        .await;
        record_op(
            &ledger,
            "move",
            "/src/b.txt",
            "/dst/b.txt",
            "p2",
            LedgerStatus::Ok,
        )
        .await;
        record_op(
            &ledger,
            "move",
            "/src/c.txt",
            "/dst/c.txt",
            "p2",
            LedgerStatus::Ok,
        )
        .await;

        let rule = build_pinned_rule("p2", &ledger).await.unwrap();
        match rule.action {
            RuleAction::ReplayOp {
                ref source_paths,
                ref dest_path,
                ..
            } => {
                assert_eq!(source_paths.len(), 3);
                assert_eq!(dest_path, "/dst");
            }
            _ => panic!("expected ReplayOp"),
        }
        assert!(rule.name.contains("Move 3 files"));
    }

    #[tokio::test]
    async fn failed_events_excluded() {
        let ledger = fresh_ledger().await;
        record_op(
            &ledger,
            "copy",
            "/src/a.txt",
            "/dst/a.txt",
            "p3",
            LedgerStatus::Failed,
        )
        .await;
        let result = build_pinned_rule("p3", &ledger).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn divergent_target_dirs_refused() {
        let ledger = fresh_ledger().await;
        record_op(
            &ledger,
            "copy",
            "/src/a.txt",
            "/dst1/a.txt",
            "p4",
            LedgerStatus::Ok,
        )
        .await;
        record_op(
            &ledger,
            "copy",
            "/src/b.txt",
            "/dst2/b.txt",
            "p4",
            LedgerStatus::Ok,
        )
        .await;
        let result = build_pinned_rule("p4", &ledger).await;
        assert!(
            result.is_err(),
            "rows with mismatched target parents must be rejected"
        );
    }

    #[tokio::test]
    async fn non_pinnable_kind_excluded() {
        let ledger = fresh_ledger().await;
        record_op(
            &ledger,
            "rename",
            "/src/a.txt",
            "/src/b.txt",
            "p5",
            LedgerStatus::Ok,
        )
        .await;
        let result = build_pinned_rule("p5", &ledger).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn other_engine_excluded() {
        let ledger = fresh_ledger().await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Sync, "copy")
                    .status(LedgerStatus::Ok)
                    .subject("/src/a.txt")
                    .target("/dst/a.txt")
                    .correlation("p6"),
            )
            .await;
        let result = build_pinned_rule("p6", &ledger).await;
        assert!(result.is_err(), "non-fs engines must not be pinnable");
    }

    // ── build_retry_rule: failure-side symmetry ────────────────────

    #[tokio::test]
    async fn retry_picks_up_failed_event() {
        let ledger = fresh_ledger().await;
        record_op(
            &ledger,
            "copy",
            "/src/a.txt",
            "/dst/a.txt",
            "r1",
            LedgerStatus::Failed,
        )
        .await;

        let rule = build_retry_rule("r1", &ledger).await.unwrap();
        assert!(!rule.enabled, "retry rules must be disabled by default");
        assert!(matches!(rule.trigger, RuleTrigger::Manual));
        assert!(rule.name.starts_with("Retry:"));
        match rule.action {
            RuleAction::ReplayOp {
                ref kind,
                ref source_paths,
                ref dest_path,
            } => {
                assert_eq!(kind, "copy");
                assert_eq!(source_paths, &vec!["/src/a.txt".to_string()]);
                assert_eq!(dest_path, "/dst");
            }
            _ => panic!("expected ReplayOp action, got {:?}", rule.action),
        }
    }

    #[tokio::test]
    async fn retry_ignores_successful_event() {
        let ledger = fresh_ledger().await;
        record_op(
            &ledger,
            "copy",
            "/src/a.txt",
            "/dst/a.txt",
            "r2",
            LedgerStatus::Ok,
        )
        .await;
        let result = build_retry_rule("r2", &ledger).await;
        assert!(
            result.is_err(),
            "successful events must not match the retry path — Pin is the right surface for them"
        );
    }

    #[tokio::test]
    async fn pin_ignores_failed_event() {
        // Counterpart to `retry_ignores_successful_event` — proves the
        // two flows are strict mirrors: each only consumes its own
        // status set.
        let ledger = fresh_ledger().await;
        record_op(
            &ledger,
            "copy",
            "/src/a.txt",
            "/dst/a.txt",
            "r3",
            LedgerStatus::Failed,
        )
        .await;
        let result = build_pinned_rule("r3", &ledger).await;
        assert!(result.is_err(), "failed events must not match the pin path");
    }

    #[tokio::test]
    async fn retry_accepts_cancelled_event() {
        let ledger = fresh_ledger().await;
        record_op(
            &ledger,
            "move",
            "/src/a.txt",
            "/dst/a.txt",
            "r4",
            LedgerStatus::Cancelled,
        )
        .await;
        let rule = build_retry_rule("r4", &ledger).await.unwrap();
        assert!(rule.name.starts_with("Retry:"));
    }

    #[tokio::test]
    async fn retry_groups_multiple_failed_sources() {
        let ledger = fresh_ledger().await;
        for name in ["a.txt", "b.txt", "c.txt"] {
            record_op(
                &ledger,
                "move",
                &format!("/src/{name}"),
                &format!("/dst/{name}"),
                "r5",
                LedgerStatus::Failed,
            )
            .await;
        }
        let rule = build_retry_rule("r5", &ledger).await.unwrap();
        match rule.action {
            RuleAction::ReplayOp {
                ref source_paths,
                ref dest_path,
                ..
            } => {
                assert_eq!(source_paths.len(), 3);
                assert_eq!(dest_path, "/dst");
            }
            _ => panic!("expected ReplayOp"),
        }
        assert!(rule.name.contains("Retry"));
        assert!(rule.name.contains("Move 3 files"));
    }
}
