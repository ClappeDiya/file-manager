//! One view of everything the app is doing: running and queued transfers,
//! syncs, backups and search indexing, plus recently finished operations
//! from the ledger (file operations, archives, automation…), with cancel.

use crate::backup_engine::{BackupManager, TaskInfo};
use crate::core::error::AppError;
use crate::core::traits::TransferOperations;
use crate::core::types::{TransferJob, TransferStatus};
use crate::ledger::{LedgerEvent, OperationLedger};
use crate::search_engine::{IndexJob, SearchManager};
use crate::sync_engine::{executor::SyncProgress, SyncManager};
use crate::transfer_engine::TransferManager;
use serde::Serialize;
use std::collections::HashMap;
use tauri::State;

/// An operation in progress (or waiting to start).
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct OpItem {
    /// `transfer`, `sync`, `backup` or `index`; with `id`, what cancel takes.
    pub engine: String,
    pub id: String,
    pub title: String,
    pub detail: Option<String>,
    /// `running`, `queued` or `paused`.
    pub state: String,
    /// 0..=1 when known.
    pub fraction: Option<f64>,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub items_done: u64,
    pub items_total: u64,
    pub started_at: Option<String>,
    pub bytes_per_sec: u64,
    pub eta_secs: Option<u64>,
    pub cancellable: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct OpsSnapshot {
    pub active: Vec<OpItem>,
    pub recent: Vec<LedgerEvent>,
}

fn ratio(done: u64, total: u64) -> Option<f64> {
    (total > 0).then(|| (done as f64 / total as f64).clamp(0.0, 1.0))
}

fn base_name(p: &str) -> String {
    p.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or(p).to_string()
}

pub fn from_transfer(j: &TransferJob) -> Option<OpItem> {
    let state = match j.status {
        TransferStatus::Active => "running",
        TransferStatus::Queued => "queued",
        TransferStatus::Paused => "paused",
        _ => return None,
    };
    Some(OpItem {
        engine: "transfer".into(),
        id: j.id.to_string(),
        title: format!("Copy {}", base_name(&j.source_path)),
        detail: Some(format!("→ {}", j.dest_path)),
        state: state.into(),
        fraction: ratio(j.transferred_bytes, j.total_bytes),
        bytes_done: j.transferred_bytes,
        bytes_total: j.total_bytes,
        items_done: 0,
        items_total: 1,
        started_at: Some(j.created_at.to_rfc3339()),
        bytes_per_sec: j.speed_bps,
        eta_secs: j.eta_seconds,
        cancellable: true,
    })
}

pub fn from_sync(p: &SyncProgress, names: &HashMap<uuid::Uuid, String>) -> OpItem {
    OpItem {
        engine: "sync".into(),
        id: p.pair_id.to_string(),
        title: format!("Sync {}", names.get(&p.pair_id).cloned().unwrap_or_else(|| "pair".into())),
        detail: Some(match (&p.current_file, p.cancel_requested) {
            (_, true) => "Stopping after the current file…".into(),
            (Some(f), _) => format!("{} · {f}", p.phase),
            (None, _) => p.phase.clone(),
        }),
        state: "running".into(),
        fraction: ratio(p.bytes_done, p.bytes_total).or_else(|| ratio(p.files_done, p.files_total)),
        bytes_done: p.bytes_done,
        bytes_total: p.bytes_total,
        items_done: p.files_done,
        items_total: p.files_total,
        started_at: Some(p.started_at.to_rfc3339()),
        bytes_per_sec: 0,
        eta_secs: None,
        cancellable: !p.cancel_requested,
    }
}

pub fn from_backup(t: &TaskInfo) -> OpItem {
    let p = t.progress.as_ref();
    OpItem {
        engine: "backup".into(),
        id: t.task_id.clone(),
        title: format!("{} {}", capitalize(&t.operation), t.job_name),
        detail: p.and_then(|p| p.current_path.clone()),
        state: t.state.clone(),
        fraction: p.and_then(|p| ratio(p.files_processed, p.files_total)),
        bytes_done: p.map(|p| p.bytes_processed).unwrap_or(0),
        bytes_total: 0,
        items_done: p.map(|p| p.files_processed).unwrap_or(0),
        items_total: p.map(|p| p.files_total).unwrap_or(0),
        started_at: t.started_at.clone(),
        bytes_per_sec: t.bytes_per_sec,
        eta_secs: t.eta_secs,
        cancellable: true,
    }
}

pub fn from_index(j: &IndexJob) -> Option<OpItem> {
    j.running.then(|| OpItem {
        engine: "index".into(),
        id: j.root.clone(),
        title: format!("Index {}", base_name(&j.root)),
        detail: Some(format!("{} items scanned", j.scanned)),
        state: "running".into(),
        fraction: None,
        bytes_done: 0,
        bytes_total: 0,
        items_done: j.scanned,
        items_total: 0,
        started_at: None,
        bytes_per_sec: 0,
        eta_secs: None,
        cancellable: true,
    })
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
}

/// Running first, then queued/paused; newest first within each.
pub fn order(items: &mut [OpItem]) {
    let rank = |s: &str| match s {
        "running" => 0,
        "paused" => 1,
        _ => 2,
    };
    items.sort_by(|a, b| rank(&a.state).cmp(&rank(&b.state)).then_with(|| b.started_at.cmp(&a.started_at)));
}

#[tauri::command]
pub async fn operations_snapshot(
    recent_limit: Option<u32>,
    transfers: State<'_, TransferManager>,
    sync: State<'_, SyncManager>,
    backup: State<'_, BackupManager>,
    search: State<'_, SearchManager>,
    ledger: State<'_, OperationLedger>,
) -> Result<OpsSnapshot, AppError> {
    use crate::core::traits::SyncOperations;
    let mut active: Vec<OpItem> = transfers.list_jobs().await?.iter().filter_map(from_transfer).collect();
    let progress = sync.all_progress();
    if !progress.is_empty() {
        let names: HashMap<uuid::Uuid, String> = sync.list_pairs().await?.into_iter().map(|p| (p.id, p.name)).collect();
        active.extend(progress.iter().map(|p| from_sync(p, &names)));
    }
    active.extend(backup.status().await.tasks.iter().map(from_backup));
    active.extend(search.jobs().iter().filter_map(from_index));
    order(&mut active);
    let recent = ledger.recent(recent_limit.unwrap_or(30).min(200)).await?;
    Ok(OpsSnapshot { active, recent })
}

/// Cancel an operation from the snapshot. Returns whether it was running.
#[tauri::command]
pub async fn operations_cancel(
    engine: String,
    id: String,
    transfers: State<'_, TransferManager>,
    sync: State<'_, SyncManager>,
    backup: State<'_, BackupManager>,
    search: State<'_, SearchManager>,
) -> Result<bool, AppError> {
    let uuid = || uuid::Uuid::parse_str(&id).map_err(|_| AppError::validation(format!("Invalid id {id}")));
    match engine.as_str() {
        "transfer" => transfers.cancel(uuid()?).await.map(|_| true),
        "sync" => Ok(sync.cancel_sync(uuid()?).await),
        "backup" => Ok(backup.cancel(&id)),
        "index" => {
            search.cancel(&id);
            Ok(true)
        }
        other => Err(AppError::validation(format!("Unknown operation type '{other}'"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job(status: TransferStatus, done: u64) -> TransferJob {
        let now = chrono::Utc::now();
        serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::new_v4(), "source_path": "/a/report.pdf", "dest_path": "/b", "total_bytes": 200, "transferred_bytes": done,
            "status": status, "priority": "normal", "sort_order": 0, "created_at": now, "updated_at": now, "error_message": null,
            "retry_count": 0, "partial_checksum": null, "conflict_policy": null, "connection_id": null, "verify_checksum": false, "checksum_algorithm": null,
            "source_checksum": null, "dest_checksum": null, "speed_bps": 1000, "eta_seconds": 5
        }))
        .unwrap_or_else(|e| panic!("TransferJob shape changed: {e}"))
    }

    #[test]
    fn transfers_map_and_finished_ones_drop_out() {
        let t = from_transfer(&job(TransferStatus::Active, 50)).unwrap();
        assert_eq!((t.title.as_str(), t.state.as_str(), t.fraction), ("Copy report.pdf", "running", Some(0.25)));
        assert_eq!((t.bytes_per_sec, t.eta_secs), (1000, Some(5)));
        assert_eq!(from_transfer(&job(TransferStatus::Paused, 0)).unwrap().state, "paused");
        assert!(from_transfer(&job(TransferStatus::Completed, 200)).is_none());
        assert!(from_transfer(&job(TransferStatus::Failed, 0)).is_none());
    }

    #[test]
    fn sync_backup_and_index_map() {
        let pair = uuid::Uuid::new_v4();
        let p = SyncProgress { pair_id: pair, run_id: uuid::Uuid::new_v4(), phase: "copying".into(), files_done: 1, files_total: 4, bytes_done: 0, bytes_total: 0, current_file: Some("x.txt".into()), started_at: chrono::Utc::now(), cancel_requested: false };
        let s = from_sync(&p, &HashMap::from([(pair, "Photos".to_string())]));
        assert_eq!((s.title.as_str(), s.fraction, s.detail.as_deref()), ("Sync Photos", Some(0.25), Some("copying · x.txt")));
        let stopping = from_sync(&SyncProgress { cancel_requested: true, ..p }, &HashMap::new());
        assert!(!stopping.cancellable && stopping.title == "Sync pair");

        let idx = IndexJob { root: "/home/me/Docs".into(), scanned: 42, running: true, last: None, error: None };
        assert_eq!(from_index(&idx).unwrap().title, "Index Docs");
        assert!(from_index(&IndexJob { running: false, ..idx }).is_none());
    }

    #[test]
    fn running_first_then_newest() {
        let mk = |state: &str, at: &str| OpItem { engine: "x".into(), id: at.into(), title: String::new(), detail: None, state: state.into(), fraction: None, bytes_done: 0, bytes_total: 0, items_done: 0, items_total: 0, started_at: Some(at.into()), bytes_per_sec: 0, eta_secs: None, cancellable: true };
        let mut v = vec![mk("queued", "3"), mk("running", "1"), mk("running", "2"), mk("paused", "4")];
        order(&mut v);
        assert_eq!(v.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), vec!["2", "1", "4", "3"]);
    }
}
