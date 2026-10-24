//! OS notifications for transfers and sync runs, driven by the user's
//! notification preferences (Settings → Notifications).
//!
//! The decision is a pure function ([`decide`]): preferences, what happened
//! and whether the app is in front go in, a toast (or nothing) comes out.
//! The managers only report what happened (a hook on the transfer queue, a
//! listener on the sync manager); [`show`] is the one place that reads the
//! preferences and talks to the OS.

use crate::core::types::{TransferJob, TransferStatus};
use crate::sync_engine::{SyncManager, SyncRunEvent};
use crate::transfer_engine::TransferManager;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use ufop_core::sync_types::SyncRunStatus;

/// Config key the preferences are stored under.
pub const NOTIFICATION_PREFS_KEY: &str = "notification_prefs";

/// Which events raise an OS notification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationPrefs {
    pub transfer_complete: bool,
    pub transfer_failed: bool,
    pub sync_complete: bool,
    pub sync_failed: bool,
    pub connection_lost: bool,
    /// Stay quiet while the app window is in front.
    pub only_when_backgrounded: bool,
}

impl Default for NotificationPrefs {
    fn default() -> Self {
        Self {
            transfer_complete: true,
            transfer_failed: true,
            sync_complete: false,
            sync_failed: true,
            connection_lost: false,
            only_when_backgrounded: true,
        }
    }
}

/// Preferences from their stored JSON; defaults when nothing is stored.
pub fn parse_prefs(stored: Option<&str>) -> Result<NotificationPrefs, serde_json::Error> {
    stored.map(serde_json::from_str).unwrap_or_else(|| Ok(NotificationPrefs::default()))
}

/// Something worth telling the user about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DesktopEvent {
    TransferFinished { name: String, bytes: u64 },
    TransferFailed { name: String, error: String },
    SyncFinished { pair: String, changed: u64, bytes: u64 },
    /// A sync that failed, or finished with errors.
    SyncFailed { pair: String, error: String },
}

/// What the OS notification says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Toast {
    pub title: String,
    pub body: String,
}

/// Whether `event` raises a notification under `prefs`; `app_focused` is
/// whether the app window is in front.
pub fn decide(prefs: &NotificationPrefs, event: &DesktopEvent, app_focused: bool) -> Option<Toast> {
    if prefs.only_when_backgrounded && app_focused {
        return None;
    }
    let toast = |title: &str, body: String| Some(Toast { title: title.to_string(), body });
    match event {
        DesktopEvent::TransferFinished { name, bytes } if prefs.transfer_complete => toast("Transfer complete", format!("{name} ({})", crate::backup_engine::human_bytes(*bytes))),
        DesktopEvent::TransferFailed { name, error } if prefs.transfer_failed => toast("Transfer failed", format!("{name}: {error}")),
        DesktopEvent::SyncFinished { pair, changed, bytes } if prefs.sync_complete => toast("Sync complete", format!("{pair}: {changed} file(s) changed, {}", crate::backup_engine::human_bytes(*bytes))),
        DesktopEvent::SyncFailed { pair, error } if prefs.sync_failed => toast("Sync failed", format!("{pair}: {error}")),
        _ => None,
    }
}

/// What a finished transfer job is, if it is final and worth reporting
/// (cancelled and unfinished jobs are not).
pub fn transfer_event(job: &TransferJob) -> Option<DesktopEvent> {
    let name = std::path::Path::new(&job.source_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| job.source_path.clone());
    match job.status {
        TransferStatus::Completed => Some(DesktopEvent::TransferFinished { name, bytes: job.total_bytes }),
        TransferStatus::Failed => Some(DesktopEvent::TransferFailed { name, error: job.error_message.clone().unwrap_or_else(|| "Unknown error".into()) }),
        _ => None,
    }
}

/// What a finished sync run is for the user; cancelled runs say nothing.
pub fn sync_event(run: &SyncRunEvent) -> Option<DesktopEvent> {
    let pair = run.pair.name.clone();
    match &run.outcome {
        Err(error) => Some(DesktopEvent::SyncFailed { pair, error: error.clone() }),
        Ok(r) => match r.status {
            SyncRunStatus::Success => Some(DesktopEvent::SyncFinished { pair, changed: r.files_added + r.files_modified + r.files_deleted, bytes: r.bytes_transferred }),
            SyncRunStatus::PartialSuccess | SyncRunStatus::Failed => {
                Some(DesktopEvent::SyncFailed { pair, error: r.error_messages.first().cloned().unwrap_or_else(|| format!("{} file(s) had errors", r.errors)) })
            }
            SyncRunStatus::Cancelled | SyncRunStatus::Running => None,
        },
    }
}

/// Send transfer completions and failures to `sink`.
pub fn wire_transfers(transfers: &TransferManager, sink: Arc<dyn Fn(DesktopEvent) + Send + Sync>) {
    transfers.add_terminal_hook(Arc::new(move |job| {
        if let Some(e) = transfer_event(job) {
            sink(e);
        }
    }));
}

/// Send finished sync runs to `sink`.
pub fn wire_sync(sync: &SyncManager, sink: Arc<dyn Fn(DesktopEvent) + Send + Sync>) {
    sync.add_run_listener(Arc::new(move |run| {
        if let Some(e) = sync_event(run) {
            sink(e);
        }
    }));
}

/// Read the preferences (defaults when unreadable) and show the toast the
/// event calls for. Fails open: a notification problem never affects a run.
pub async fn show(app: &tauri::AppHandle, event: DesktopEvent) {
    use crate::core::traits::StorageOperations;
    use tauri::Manager;
    use tauri_plugin_notification::NotificationExt;
    let repo = app.state::<crate::storage::Repository>();
    let stored = StorageOperations::get_config(repo.inner(), NOTIFICATION_PREFS_KEY).await.ok().flatten();
    let prefs = parse_prefs(stored.as_deref()).unwrap_or_else(|e| {
        tracing::warn!("Invalid notification preferences, using defaults: {e}");
        NotificationPrefs::default()
    });
    let focused = app.webview_windows().values().any(|w| w.is_focused().unwrap_or(false));
    if let Some(t) = decide(&prefs, &event, focused) {
        if let Err(e) = app.notification().builder().title(&t.title).body(&t.body).show() {
            tracing::warn!("Desktop notification failed: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{TransferPriority, TransferStatus};
    use std::sync::Mutex;
    use ufop_core::sync_types::{SyncHealth, SyncPair, SyncReport};

    fn job(status: TransferStatus) -> TransferJob {
        TransferJob {
            id: uuid::Uuid::new_v4(),
            source_path: "/data/photos/trip.zip".into(),
            dest_path: "/backup/trip.zip".into(),
            total_bytes: 3 * 1024 * 1024,
            transferred_bytes: 0,
            status,
            priority: TransferPriority::Normal,
            sort_order: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            error_message: None,
            retry_count: 0,
            partial_checksum: None,
            conflict_policy: None,
            connection_id: None,
            verify_checksum: false,
            checksum_algorithm: None,
            source_checksum: None,
            dest_checksum: None,
            speed_bps: 0,
            eta_seconds: None,
        }
    }

    fn report(status: SyncRunStatus, errors: u32) -> SyncReport {
        SyncReport {
            id: uuid::Uuid::new_v4(),
            pair_id: uuid::Uuid::new_v4(),
            pair_name: "Photos".into(),
            started_at: chrono::Utc::now(),
            ended_at: chrono::Utc::now(),
            duration_ms: 10,
            files_added: 2,
            files_modified: 1,
            files_deleted: 0,
            files_skipped: 0,
            conflicts_resolved: 0,
            errors,
            bytes_transferred: 2048,
            status,
            health: SyncHealth::Green,
            error_messages: if errors > 0 { vec!["locked file".into()] } else { vec![] },
            resumed: false,
            exit_code: 0,
            versions_pruned: 0,
            trigger: Default::default(),
            items_truncated: false,
            items: vec![],
        }
    }

    fn run(outcome: Result<SyncReport, String>) -> SyncRunEvent {
        SyncRunEvent { pair: SyncPair { name: "Photos".into(), ..Default::default() }, outcome, verify_mismatches: 0 }
    }

    #[test]
    fn each_preference_gates_its_own_event() {
        let all_off = NotificationPrefs { transfer_complete: false, transfer_failed: false, sync_complete: false, sync_failed: false, connection_lost: false, only_when_backgrounded: false };
        let events = [
            DesktopEvent::TransferFinished { name: "a".into(), bytes: 1 },
            DesktopEvent::TransferFailed { name: "a".into(), error: "x".into() },
            DesktopEvent::SyncFinished { pair: "p".into(), changed: 1, bytes: 1 },
            DesktopEvent::SyncFailed { pair: "p".into(), error: "x".into() },
        ];
        for e in &events {
            assert_eq!(decide(&all_off, e, false), None, "{e:?} off");
        }
        let on = |f: fn(&mut NotificationPrefs)| {
            let mut p = all_off.clone();
            f(&mut p);
            p
        };
        assert_eq!(decide(&on(|p| p.transfer_complete = true), &events[0], false).unwrap().title, "Transfer complete");
        assert_eq!(decide(&on(|p| p.transfer_failed = true), &events[1], false).unwrap().title, "Transfer failed");
        assert_eq!(decide(&on(|p| p.sync_complete = true), &events[2], false).unwrap().title, "Sync complete");
        assert_eq!(decide(&on(|p| p.sync_failed = true), &events[3], false).unwrap().title, "Sync failed");
        // Turning one on never lets another through.
        assert_eq!(decide(&on(|p| p.transfer_complete = true), &events[1], false), None);
        assert_eq!(decide(&on(|p| p.sync_failed = true), &events[2], false), None);
    }

    #[test]
    fn backgrounded_only_stays_quiet_while_the_app_is_in_front() {
        let mut p = NotificationPrefs::default();
        let done = DesktopEvent::TransferFinished { name: "a.zip".into(), bytes: 2048 };
        assert!(decide(&p, &done, true).is_none());
        assert_eq!(decide(&p, &done, false).unwrap().body, "a.zip (2.0 KB)");
        p.only_when_backgrounded = false;
        assert!(decide(&p, &done, true).is_some());
    }

    #[test]
    fn defaults_and_stored_preferences() {
        let d = parse_prefs(None).unwrap();
        assert!(d.transfer_complete && d.transfer_failed && d.sync_failed && !d.sync_complete);
        let stored = serde_json::to_string(&NotificationPrefs { sync_complete: true, ..Default::default() }).unwrap();
        assert!(parse_prefs(Some(&stored)).unwrap().sync_complete);
        assert!(parse_prefs(Some("{not json")).is_err());
    }

    #[test]
    fn jobs_and_runs_become_events() {
        assert_eq!(transfer_event(&job(TransferStatus::Completed)), Some(DesktopEvent::TransferFinished { name: "trip.zip".into(), bytes: 3 * 1024 * 1024 }));
        let mut failed = job(TransferStatus::Failed);
        failed.error_message = Some("disk full".into());
        assert_eq!(transfer_event(&failed), Some(DesktopEvent::TransferFailed { name: "trip.zip".into(), error: "disk full".into() }));
        for s in [TransferStatus::Queued, TransferStatus::Active, TransferStatus::Paused, TransferStatus::Cancelled] {
            assert_eq!(transfer_event(&job(s)), None);
        }
        assert_eq!(sync_event(&run(Ok(report(SyncRunStatus::Success, 0)))), Some(DesktopEvent::SyncFinished { pair: "Photos".into(), changed: 3, bytes: 2048 }));
        assert_eq!(sync_event(&run(Ok(report(SyncRunStatus::PartialSuccess, 1)))), Some(DesktopEvent::SyncFailed { pair: "Photos".into(), error: "locked file".into() }));
        assert_eq!(sync_event(&run(Err("source missing".into()))), Some(DesktopEvent::SyncFailed { pair: "Photos".into(), error: "source missing".into() }));
        assert_eq!(sync_event(&run(Ok(report(SyncRunStatus::Cancelled, 0)))), None);
    }

    /// The reported bug: the preferences were stored and shown in Settings,
    /// but nothing consulted them, so no transfer or sync ever raised a
    /// notification. Wired as the app wires it (hook → preferences → toast).
    #[tokio::test]
    async fn finished_transfers_raise_a_notification_when_preferences_say_so() {
        let toasts: Arc<Mutex<Vec<Toast>>> = Arc::new(Mutex::new(Vec::new()));
        let prefs = NotificationPrefs { only_when_backgrounded: false, ..Default::default() };
        let sink = {
            let toasts = toasts.clone();
            Arc::new(move |e: DesktopEvent| {
                if let Some(t) = decide(&prefs, &e, false) {
                    toasts.lock().unwrap().push(t);
                }
            })
        };
        let mgr = TransferManager::new();
        wire_transfers(&mgr, sink);
        use crate::core::traits::TransferOperations;
        let ok = mgr.enqueue("/data/a.bin", "/out/a.bin", 10).await.unwrap();
        mgr.complete_job(ok.id).await.unwrap();
        let bad = mgr.enqueue("/data/b.bin", "/out/b.bin", 10).await.unwrap();
        mgr.fail_job(bad.id, "network down").await.unwrap();
        let got = toasts.lock().unwrap().clone();
        assert_eq!(got.iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), vec!["Transfer complete", "Transfer failed"]);
        assert!(got[1].body.contains("b.bin") && got[1].body.contains("network down"));
    }

    #[tokio::test]
    async fn finished_sync_runs_raise_a_notification_when_preferences_say_so() {
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("a.txt"), "hello").unwrap();
        let state = tempfile::tempdir().unwrap();
        let mgr = SyncManager::with_state_dir(state.path().to_path_buf());
        let toasts: Arc<Mutex<Vec<Toast>>> = Arc::new(Mutex::new(Vec::new()));
        let prefs = NotificationPrefs { sync_complete: true, only_when_backgrounded: false, ..Default::default() };
        let sink = {
            let toasts = toasts.clone();
            Arc::new(move |e: DesktopEvent| {
                if let Some(t) = decide(&prefs, &e, false) {
                    toasts.lock().unwrap().push(t);
                }
            })
        };
        wire_sync(&mgr, sink);
        let pair = SyncPair { name: "Docs".into(), source_path: src.path().to_string_lossy().into(), dest_path: dst.path().to_string_lossy().into(), ..Default::default() };
        let id = pair.id;
        use crate::core::traits::SyncOperations;
        mgr.create_pair(pair).await.unwrap();
        mgr.execute_sync(id).await.unwrap();
        let got = toasts.lock().unwrap().clone();
        assert_eq!(got.len(), 1, "{got:?}");
        assert_eq!(got[0].title, "Sync complete");
        assert!(got[0].body.starts_with("Docs: 1 file(s) changed"), "{}", got[0].body);
        // A run that cannot start (the source is gone) is a failure toast.
        std::fs::remove_dir_all(src.path()).unwrap();
        let _ = mgr.execute_sync(id).await;
        let got = toasts.lock().unwrap().clone();
        assert_eq!(got.last().map(|t| t.title.as_str()), Some("Sync failed"), "{got:?}");
    }
}
