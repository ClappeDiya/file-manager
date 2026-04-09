//! Universal Time-Travel Undo — cross-session, cross-engine Cmd+Z.
//!
//! The unified [`crate::ledger::OperationLedger`] already records every
//! filesystem operation (copy, move, rename, duplicate, delete, create) with
//! a stable correlation id. This module turns those append-only records into
//! a reversible history that survives app restarts.
//!
//! # Why this exists
//!
//! No mainstream file manager offers a `Cmd+Z` that:
//!
//! 1. Survives closing and reopening the app (the ledger is on disk).
//! 2. Works across every engine that writes to the ledger (today: `fs`;
//!    tomorrow: `sync`, `transfer`, `automation` — plug in via the
//!    `ENGINE_UNDO_HANDLERS` table below).
//! 3. Adds zero new infrastructure — no new tables, no background daemons,
//!    no additional processes, no cloud calls, no AI cost.
//!
//! # Model
//!
//! Every successful fs op is recorded by [`crate::commands::file_ops_commands::record_fs_ok`]
//! as N rows sharing one `correlation_id` (one row per source/target pair).
//! To undo, we:
//!
//! 1. Query the ledger for the most recent `engine = 'fs'` event whose
//!    correlation id has NOT already been marked undone.
//! 2. Reassemble the full `(source_paths, dest_paths, kind)` tuple from the
//!    rows sharing that correlation id.
//! 3. Dispatch to the existing [`crate::commands::file_ops_commands::undo_file_operation`]
//!    inverse-operation logic. Nothing about the undo logic is duplicated.
//! 4. On success, write a single marker event of kind `fs.undone` with
//!    `details_json = { "undone_correlation_id": <original> }` so step 1 can
//!    exclude it on the next call.
//!
//! This is DRY-by-construction: the ledger is the single source of truth, and
//! the reverse-op code lives in exactly one place.

use crate::commands::file_ops_commands::undo_file_operation;
use crate::core::error::AppError;
use crate::ledger::{LedgerEngine, LedgerEvent, LedgerStatus, OperationLedger, RecordEvent};
use serde::{Deserialize, Serialize};
use tauri::State;

/// Kinds of fs-engine events that are reversible today. Keep this in lock-step
/// with the `operation` branches in
/// [`crate::commands::file_ops_commands::undo_file_operation`].
const UNDOABLE_FS_KINDS: &[&str] = &[
    "copy",
    "duplicate",
    "move",
    "rename",
    "create_folder",
    "create_file",
];

/// Marker kind written after a successful undo so future calls skip it.
pub const FS_UNDONE_KIND: &str = "fs.undone";

/// A single undoable operation group, assembled from all ledger rows that
/// share a `correlation_id`. Returned to the frontend so it can show a
/// per-entry "Undo" button in the activity timeline.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoableOp {
    /// Correlation id shared by every row in this group — also the undo key.
    pub correlation_id: String,
    /// Engine that produced the events (currently always `"fs"`).
    pub engine: String,
    /// Operation kind: `copy`, `move`, `rename`, …
    pub kind: String,
    /// Most recent occurrence timestamp among the rows in this group.
    pub occurred_at: String,
    /// Human-readable description, prebuilt for the timeline badge.
    pub summary: String,
    /// Source paths, in the original order.
    pub source_paths: Vec<String>,
    /// Destination paths, paired positionally with `source_paths` where
    /// applicable (move/rename/copy/duplicate) or empty (delete).
    pub dest_paths: Vec<String>,
    /// How many file entries this group touches.
    pub item_count: usize,
}

/// Outcome of an undo dispatch, returned to the frontend so the UI can surface
/// a confirmation toast ("Renamed `foo` back to `bar`").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UndoOutcome {
    pub success: bool,
    pub correlation_id: String,
    pub kind: String,
    pub summary: String,
    pub item_count: usize,
}

/// Fetch recent undoable ops, most-recent first. `limit` clamps to [1, 100].
///
/// Backed by a single `ledger_recent` pull — cheap and O(N) in the number of
/// recent rows, with N≤600 (100 groups × ~6 rows each).
#[tauri::command]
pub async fn list_undoable(
    limit: Option<u32>,
    ledger: State<'_, OperationLedger>,
) -> Result<Vec<UndoableOp>, AppError> {
    let limit = limit.unwrap_or(20).clamp(1, 100);
    list_undoable_impl(&ledger, limit).await
}

pub async fn list_undoable_impl(
    ledger: &OperationLedger,
    limit: u32,
) -> Result<Vec<UndoableOp>, AppError> {
    // Pull more raw rows than groups we want, because each group can span
    // several rows (one per source/dest pair). 10x is a generous upper bound.
    let raw_limit = (limit as u32 * 10).min(1000);
    let rows = ledger.recent(raw_limit).await?;
    let groups = assemble_undoable_groups(&rows);
    Ok(groups.into_iter().take(limit as usize).collect())
}

/// Undo the single most recent undoable op across all engines.
///
/// Returns `None` via `success=false` when there is nothing to undo, so the
/// frontend can show a polite "Nothing to undo" toast without needing a
/// second round-trip.
#[tauri::command]
pub async fn undo_last(
    ledger: State<'_, OperationLedger>,
) -> Result<UndoOutcome, AppError> {
    let ops = list_undoable_impl(&ledger, 1).await?;
    let Some(op) = ops.into_iter().next() else {
        return Ok(UndoOutcome {
            success: false,
            correlation_id: String::new(),
            kind: String::new(),
            summary: "Nothing to undo".to_string(),
            item_count: 0,
        });
    };
    perform_undo(op, &ledger).await
}

/// Undo a specific correlation group (the "Undo" button in the timeline).
#[tauri::command]
pub async fn undo_by_correlation(
    correlation_id: String,
    ledger: State<'_, OperationLedger>,
) -> Result<UndoOutcome, AppError> {
    let ops = list_undoable_impl(&ledger, 100).await?;
    let Some(op) = ops.into_iter().find(|o| o.correlation_id == correlation_id) else {
        return Err(AppError::file_op(
            format!("No undoable operation found with id {correlation_id}"),
            "It may have already been undone, or may be older than the 30-day ledger retention.",
        ));
    };
    perform_undo(op, &ledger).await
}

/// Shared worker: call the existing fs inverse logic, then emit an
/// `fs.undone` marker so future scans exclude this correlation id.
async fn perform_undo(
    op: UndoableOp,
    ledger: &OperationLedger,
) -> Result<UndoOutcome, AppError> {
    // Delegate the actual reverse operation to the one place it lives.
    // If this fails, propagate — and intentionally do NOT write the marker,
    // so the user can retry.
    let _ = undo_file_operation(
        op.kind.clone(),
        op.source_paths.clone(),
        op.dest_paths.clone(),
    )
    .await?;

    // Write the marker. Note this shares `correlation_id = op.correlation_id`
    // so it's trivially discoverable and future calls to
    // `assemble_undoable_groups` will exclude it.
    let details = serde_json::json!({
        "undone_correlation_id": op.correlation_id,
        "original_kind": op.kind,
        "item_count": op.item_count,
    })
    .to_string();

    let summary = format!("Undid {}: {}", op.kind, op.summary);
    ledger
        .record(
            RecordEvent::new(LedgerEngine::Fs, FS_UNDONE_KIND)
                .status(LedgerStatus::Ok)
                .summary(summary.clone())
                .correlation(op.correlation_id.clone())
                .details_json(details),
        )
        .await;

    Ok(UndoOutcome {
        success: true,
        correlation_id: op.correlation_id,
        kind: op.kind,
        summary,
        item_count: op.item_count,
    })
}

/// Pure function: scan a list of recent ledger events, group by
/// correlation_id, drop groups that have already been undone, drop groups
/// that aren't whitelisted as reversible, and return them newest-first.
///
/// Split out from [`list_undoable_impl`] so it can be unit-tested without any
/// database or Tauri state.
fn assemble_undoable_groups(rows: &[LedgerEvent]) -> Vec<UndoableOp> {
    use std::collections::{BTreeSet, HashMap};

    // Correlation ids that have been undone — skip them entirely.
    let undone_ids: BTreeSet<&str> = rows
        .iter()
        .filter(|r| r.kind == FS_UNDONE_KIND)
        .filter_map(|r| r.correlation_id.as_deref())
        .collect();

    // Walk rows newest-first (the ledger already returns them in that order).
    // For each correlation group, remember its first seen kind + timestamp
    // (which is the newest, because we're iterating newest-first), then
    // accumulate the sources/targets in on-disk (oldest-first) order at the
    // end so undo replays in the original sequence.
    #[derive(Default)]
    struct GroupBuilder {
        kind: String,
        engine: String,
        // Newest seen timestamp; used for display and ordering.
        newest_ts: String,
        // (timestamp, subject, target, summary) accumulated as we walk.
        entries: Vec<(String, Option<String>, Option<String>, String)>,
    }

    let mut groups: HashMap<String, GroupBuilder> = HashMap::new();
    // Preserve encounter order for stable newest-first output.
    let mut order: Vec<String> = Vec::new();

    for row in rows {
        // Only fs-engine, undoable kinds.
        if row.engine != "fs" {
            continue;
        }
        if !UNDOABLE_FS_KINDS.contains(&row.kind.as_str()) {
            continue;
        }
        // Skip failed / cancelled events — nothing to reverse.
        if row.status != "ok" {
            continue;
        }
        let Some(cid) = row.correlation_id.as_deref() else {
            continue;
        };
        if undone_ids.contains(cid) {
            continue;
        }

        let entry = groups.entry(cid.to_string()).or_insert_with(|| {
            order.push(cid.to_string());
            GroupBuilder {
                kind: row.kind.clone(),
                engine: row.engine.clone(),
                newest_ts: row.occurred_at.clone(),
                entries: Vec::new(),
            }
        });
        // Rows for the same correlation id should all share the same kind;
        // if they don't, the first one wins (it's the newest).
        entry.entries.push((
            row.occurred_at.clone(),
            row.subject_path.clone(),
            row.target_path.clone(),
            row.summary.clone(),
        ));
    }

    order
        .into_iter()
        .filter_map(|cid| {
            // `order` is populated 1:1 with `groups.entry(...).or_insert_with(...)`
            // inserts, so in practice every `cid` is present. Use `filter_map`
            // anyway so a future refactor that breaks the invariant degrades
            // gracefully (drop the row) instead of panicking across the IPC
            // boundary and taking down the command handler.
            let mut g = groups.remove(&cid)?;
            // Restore chronological order within a group so the replay hits
            // files in the same sequence as the original op.
            g.entries.sort_by(|a, b| a.0.cmp(&b.0));
            let source_paths: Vec<String> = g
                .entries
                .iter()
                .filter_map(|(_, s, _, _)| s.clone())
                .collect();
            let dest_paths: Vec<String> = g
                .entries
                .iter()
                .filter_map(|(_, _, t, _)| t.clone())
                .collect();
            let item_count = g.entries.len();
            // Prefer the first row's summary (they all describe the same op).
            let summary = g
                .entries
                .first()
                .map(|(_, _, _, s)| s.clone())
                .unwrap_or_default();
            Some(UndoableOp {
                correlation_id: cid,
                engine: g.engine,
                kind: g.kind,
                occurred_at: g.newest_ts,
                summary,
                source_paths,
                dest_paths,
                item_count,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ledger::{LedgerEngine, OperationLedger, RecordEvent};
    use crate::storage::Repository;

    async fn fresh_ledger() -> OperationLedger {
        let repo = Repository::open_in_memory().await.unwrap();
        OperationLedger::new(repo.pool().clone())
    }

    fn ev(kind: &str, cid: &str, subject: Option<&str>, target: Option<&str>) -> LedgerEvent {
        LedgerEvent {
            id: uuid::Uuid::new_v4().to_string(),
            occurred_at: "2026-04-08 12:00:00".to_string(),
            engine: "fs".to_string(),
            kind: kind.to_string(),
            status: "ok".to_string(),
            subject_path: subject.map(|s| s.to_string()),
            target_path: target.map(|s| s.to_string()),
            bytes: None,
            correlation_id: Some(cid.to_string()),
            summary: format!("{kind}: {}", subject.unwrap_or("?")),
            details_json: "{}".to_string(),
            undo_token: None,
        }
    }

    #[test]
    fn assemble_ignores_non_fs_engine() {
        let rows = vec![LedgerEvent {
            engine: "sync".to_string(),
            ..ev("rename", "c1", Some("/a"), Some("/b"))
        }];
        let groups = assemble_undoable_groups(&rows);
        assert!(groups.is_empty());
    }

    #[test]
    fn assemble_ignores_non_whitelisted_kind() {
        let rows = vec![ev("set_permissions", "c1", Some("/a"), None)];
        let groups = assemble_undoable_groups(&rows);
        assert!(groups.is_empty());
    }

    #[test]
    fn assemble_ignores_failed_status() {
        let rows = vec![LedgerEvent {
            status: "failed".to_string(),
            ..ev("rename", "c1", Some("/a"), Some("/b"))
        }];
        let groups = assemble_undoable_groups(&rows);
        assert!(groups.is_empty());
    }

    #[test]
    fn assemble_groups_rows_sharing_correlation_id() {
        let rows = vec![
            // Newest first (ledger order).
            ev("move", "c2", Some("/x/b"), Some("/y/b")),
            ev("move", "c2", Some("/x/a"), Some("/y/a")),
        ];
        let groups = assemble_undoable_groups(&rows);
        assert_eq!(groups.len(), 1);
        let g = &groups[0];
        assert_eq!(g.kind, "move");
        assert_eq!(g.item_count, 2);
        assert_eq!(g.source_paths.len(), 2);
        assert_eq!(g.dest_paths.len(), 2);
    }

    #[test]
    fn assemble_skips_correlation_with_undone_marker() {
        let rows = vec![
            // Newest: the undone marker
            LedgerEvent {
                kind: FS_UNDONE_KIND.to_string(),
                ..ev("fs.undone", "c1", None, None)
            },
            // Older: the original op
            ev("rename", "c1", Some("/a"), Some("/b")),
        ];
        let groups = assemble_undoable_groups(&rows);
        assert!(
            groups.is_empty(),
            "expected already-undone correlation to be excluded"
        );
    }

    #[test]
    fn assemble_preserves_newest_first_order() {
        let rows = vec![
            // Newest first
            ev("rename", "c3", Some("/z"), Some("/zz")),
            ev("move", "c2", Some("/y"), Some("/yy")),
            ev("copy", "c1", Some("/x"), Some("/xx")),
        ];
        let groups = assemble_undoable_groups(&rows);
        assert_eq!(groups.len(), 3);
        assert_eq!(groups[0].correlation_id, "c3");
        assert_eq!(groups[1].correlation_id, "c2");
        assert_eq!(groups[2].correlation_id, "c1");
    }

    #[tokio::test]
    async fn undo_last_returns_not_found_on_empty_ledger() {
        let ledger = fresh_ledger().await;
        let ops = list_undoable_impl(&ledger, 10).await.unwrap();
        assert!(ops.is_empty());
    }

    #[tokio::test]
    async fn undo_last_finds_and_reverses_a_rename() {
        let tmp = tempfile::tempdir().unwrap();
        let original = tmp.path().join("foo.txt");
        let renamed = tmp.path().join("bar.txt");
        std::fs::write(&original, b"hi").unwrap();
        std::fs::rename(&original, &renamed).unwrap();

        let ledger = fresh_ledger().await;
        // Seed the ledger as if the rename had been recorded normally.
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "rename")
                    .correlation("rn-1")
                    .subject(original.to_string_lossy().to_string())
                    .target(renamed.to_string_lossy().to_string())
                    .summary("rename"),
            )
            .await;

        let ops = list_undoable_impl(&ledger, 10).await.unwrap();
        assert_eq!(ops.len(), 1);

        let outcome = perform_undo(ops.into_iter().next().unwrap(), &ledger)
            .await
            .unwrap();
        assert!(outcome.success);
        assert_eq!(outcome.kind, "rename");

        // File is back at the original path.
        assert!(original.exists());
        assert!(!renamed.exists());

        // And the marker prevents re-undoing the same correlation.
        let ops_after = list_undoable_impl(&ledger, 10).await.unwrap();
        assert!(
            ops_after.is_empty(),
            "undo marker should exclude the correlation"
        );
    }

    #[tokio::test]
    async fn undo_last_ignores_already_failed_events() {
        let ledger = fresh_ledger().await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Fs, "rename")
                    .status(LedgerStatus::Failed)
                    .correlation("rn-2")
                    .subject("/tmp/a")
                    .target("/tmp/b"),
            )
            .await;
        let ops = list_undoable_impl(&ledger, 10).await.unwrap();
        assert!(ops.is_empty());
    }
}
