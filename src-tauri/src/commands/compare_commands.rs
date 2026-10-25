//! IPC commands for the standalone folder compare (local folders and saved
//! connections) and for acting on its differences.
//!
//! - `compare_folders` lists and compares two sides as a cancellable task
//!   (`compare_cancel`), emitting [`COMPARE_PROGRESS_EVENT`] as it goes, and
//!   keeps the results for the actions below (`compare_release` frees them).
//! - `compare_plan_action` says what an action over a selection would do,
//!   with the operation intent the Safety Interlock assesses.
//! - `compare_apply` runs it (cancellable with `compare_cancel(apply_id)`)
//!   and returns the re-compared entries.

use crate::compare_engine::apply::{self, ApplyEnv, ApplyOutcome};
use crate::compare_engine::{run_compare, CompareManager, CompareProgress, CompareResult, CompareSide, ProgressFn, ResolvedSide, StoredCompare, COMPARE_PROGRESS_EVENT};
use crate::connectors::{ConnectionManager, ConnectorRegistry};
use crate::core::error::AppError;
use crate::ledger::OperationLedger;
use crate::safety::OperationIntent;
use crate::sync_engine::compare::{self, ActionPlan, CompareAction, CompareOptions, CompareSideId};
use crate::transfer_engine::TransferManager;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};

fn emitter(app: AppHandle) -> ProgressFn {
    std::sync::Arc::new(move |p: CompareProgress| {
        let _ = app.emit(COMPARE_PROGRESS_EVENT, p);
    })
}

/// Compare two folders — each a local path or a folder on a connection —
/// recursively. `task_id` (optional, generated otherwise) names the task
/// for `compare_cancel` and for the actions on its results.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn compare_folders(
    left: CompareSide,
    right: CompareSide,
    options: Option<CompareOptions>,
    task_id: Option<String>,
    app: AppHandle,
    manager: State<'_, CompareManager>,
    registry: State<'_, ConnectorRegistry>,
    connections: State<'_, ConnectionManager>,
) -> Result<CompareResult, AppError> {
    let task_id = task_id.filter(|t| !t.is_empty()).unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let options = options.unwrap_or_default();
    let cancel = manager.begin(&task_id);
    let progress = emitter(app);
    let outcome = async {
        let l = ResolvedSide::resolve(&left, &registry, &connections).await?;
        let r = ResolvedSide::resolve(&right, &registry, &connections).await?;
        if cancel.is_cancelled() {
            return Err(compare::cancelled());
        }
        let out = run_compare(&task_id, &l, &r, &options, &cancel, &progress).await?;
        Ok((l, r, out))
    }
    .await;
    manager.end(&task_id, &cancel);
    let (l, r, out) = outcome?;
    manager
        .store(StoredCompare {
            task_id: task_id.clone(),
            left: left.clone(),
            right: right.clone(),
            options: options.clone(),
            entries: out.entries.clone(),
            left_paths: l.listed_paths(),
            right_paths: r.listed_paths(),
        })
        .await;
    Ok(CompareResult {
        task_id,
        left_label: l.label(""),
        right_label: r.label(""),
        left,
        right,
        options,
        entries: out.entries,
        summary: out.summary,
        notes: out.notes,
        computed_at: chrono::Utc::now(),
    })
}

/// Stop a running comparison or action; `false` when it already ended.
#[tauri::command]
pub async fn compare_cancel(task_id: String, manager: State<'_, CompareManager>) -> Result<bool, AppError> {
    Ok(manager.cancel(&task_id))
}

/// Forget a comparison's results (the compare window closed).
#[tauri::command]
pub async fn compare_release(task_id: String, manager: State<'_, CompareManager>) -> Result<bool, AppError> {
    Ok(manager.release(&task_id).await)
}

/// An action plan and the Safety Interlock intent for it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompareActionPlan {
    pub plan: ActionPlan,
    pub intent: OperationIntent,
}

/// The operation intent of `plan` (engine `fs` for a local folder,
/// `connector` for a connection; the kinds the actions record).
pub fn intent_for(plan: &ActionPlan, target: &CompareSide, target_label: &str, permanent: bool) -> OperationIntent {
    let local = matches!(target, CompareSide::Local { .. });
    let deleting = matches!(plan.action, CompareAction::DeleteLeft | CompareAction::DeleteRight);
    let kind = match (deleting, local) {
        (true, true) if permanent => "delete_permanent",
        (true, true) => "delete_trash",
        (true, false) => "delete",
        (false, _) if plan.replaced > 0 => "overwrite",
        (false, _) => "copy",
    };
    let side = if plan.target() == CompareSideId::Left { "left" } else { "right" };
    let summary = if deleting {
        format!("Delete {} file(s) and {} folder(s) on the {side} ({target_label})", plan.files, plan.folders)
    } else {
        format!("Copy {} file(s) to the {side} ({target_label}), replacing {}", plan.files, plan.replaced)
    };
    OperationIntent {
        engine: if local { "fs" } else { "connector" }.to_string(),
        kind: kind.to_string(),
        affected_files: plan.files,
        total_bytes: plan.bytes,
        subject_path: Some(target_label.to_string()),
        summary: Some(summary),
    }
}

fn side_label(side: &CompareSide) -> String {
    match side {
        CompareSide::Local { path } => path.clone(),
        CompareSide::Remote { path, connection_id, protocol } => {
            format!("{}://{}{path}", protocol.as_deref().unwrap_or("remote"), connection_id.as_deref().unwrap_or(""))
        }
    }
}

/// What `action` would do with the `paths` selected in comparison
/// `task_id` (a selected folder stands for everything in it).
#[tauri::command]
pub async fn compare_plan_action(
    task_id: String,
    paths: Vec<String>,
    action: CompareAction,
    permanent: Option<bool>,
    target_label: Option<String>,
    manager: State<'_, CompareManager>,
) -> Result<CompareActionPlan, AppError> {
    let (plan, target) = manager
        .with(&task_id, |s| {
            let plan = compare::plan_action(&s.entries, &paths, action);
            let target = if plan.target() == CompareSideId::Left { s.left.clone() } else { s.right.clone() };
            (plan, target)
        })
        .await?;
    let label = target_label.unwrap_or_else(|| side_label(&target));
    let intent = intent_for(&plan, &target, &label, permanent.unwrap_or(false));
    Ok(CompareActionPlan { plan, intent })
}

/// Run `action` over the `paths` selected in comparison `task_id`, then
/// re-compare the entries it touched. `permanent` deletes (and replaces)
/// without the Trash. `apply_id` lets `compare_cancel` stop it between files.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn compare_apply(
    task_id: String,
    paths: Vec<String>,
    action: CompareAction,
    permanent: Option<bool>,
    apply_id: Option<String>,
    app: AppHandle,
    manager: State<'_, CompareManager>,
    registry: State<'_, ConnectorRegistry>,
    connections: State<'_, ConnectionManager>,
    transfer: State<'_, TransferManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<ApplyOutcome, AppError> {
    let (plan, left, right, options, left_paths, right_paths) = manager
        .with(&task_id, |s| {
            (compare::plan_action(&s.entries, &paths, action), s.left.clone(), s.right.clone(), s.options.clone(), s.left_paths.clone(), s.right_paths.clone())
        })
        .await?;
    // Remote entries are addressed where their connection listed them.
    let l = ResolvedSide::resolve(&left, &registry, &connections).await?.with_listed(left_paths);
    let r = ResolvedSide::resolve(&right, &registry, &connections).await?.with_listed(right_paths);
    let run_id = apply_id.filter(|a| !a.is_empty()).unwrap_or_else(|| format!("{task_id}:apply"));
    let cancel = manager.begin(&run_id);
    let progress = emitter(app);
    let env = ApplyEnv { transfer: &transfer, ledger: &ledger, permanent: permanent.unwrap_or(false), cancel: &cancel, progress: &progress, task_id: &task_id };
    let report = apply::execute(&plan, &l, &r, &env).await;
    manager.end(&run_id, &cancel);
    // Keep where the folders made and files sent are, even if the
    // re-compare below fails.
    let save_paths = |s: &mut StoredCompare| (s.left_paths, s.right_paths) = (l.listed_paths(), r.listed_paths());
    let _ = manager.with(&task_id, save_paths).await;

    progress(CompareProgress { task_id: task_id.clone(), phase: crate::compare_engine::ComparePhase::Refreshing, folders_listed: 0, entries_found: 0, done: 0, total: report.touched.len() as u64, current: None });
    let refreshed = apply::refresh(&l, &r, &options, &report.touched).await?;
    let merged = manager
        .with(&task_id, |s| {
            // The re-compare saw where each touched entry is now.
            save_paths(s);
            compare::merge_refreshed(&mut s.entries, &report.touched, refreshed)
        })
        .await?;
    Ok(ApplyOutcome {
        task_id,
        action,
        done: report.done,
        failed: report.failed,
        skipped: plan.skipped,
        kept: report.kept,
        notes: report.notes,
        cancelled: report.cancelled,
        undo_id: report.undo_id,
        undo_ids: report.undo_ids,
        changed: merged.changed,
        removed: merged.removed,
        summary: merged.summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_engine::compare::ActionStep;

    fn plan(action: CompareAction, replaced: u64) -> ActionPlan {
        ActionPlan { action, steps: vec![ActionStep::MakeDir { side: CompareSideId::Right, path: "a".into() }], skipped: vec![], files: 3, folders: 1, bytes: 300, replaced }
    }

    #[test]
    fn intents_use_the_kinds_the_actions_record() {
        let local = CompareSide::Local { path: "/home/me/b".into() };
        let remote = CompareSide::Remote { connection_id: Some("c".into()), protocol: None, path: "/x".into() };
        let i = intent_for(&plan(CompareAction::CopyToRight, 0), &local, "/home/me/b", false);
        assert_eq!((i.engine.as_str(), i.kind.as_str(), i.affected_files, i.total_bytes), ("fs", "copy", 3, 300));
        assert_eq!(intent_for(&plan(CompareAction::CopyToRight, 2), &local, "b", false).kind, "overwrite");
        assert_eq!(intent_for(&plan(CompareAction::DeleteRight, 0), &local, "b", false).kind, "delete_trash");
        assert_eq!(intent_for(&plan(CompareAction::DeleteRight, 0), &local, "b", true).kind, "delete_permanent");
        let r = intent_for(&plan(CompareAction::DeleteLeft, 0), &remote, "sftp://s/x", false);
        assert_eq!((r.engine.as_str(), r.kind.as_str()), ("connector", "delete"));
        assert!(r.summary.unwrap().contains("on the left"));
    }
}
