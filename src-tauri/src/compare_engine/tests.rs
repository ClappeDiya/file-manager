//! Folder compare over real fixture trees (temp dirs) and a directory-backed
//! fake connection: statuses, depth, filters, errors, cancellation, and
//! acting on differences (copy both ways, delete, ledger, re-compare).

use super::apply::{self, ApplyEnv};
use super::*;
use crate::connectors::Connector;
use crate::core::types::{ConnectionProfile, FileEntry};
use crate::ledger::OperationLedger;
use crate::sync_engine::compare::{plan_action, CompareAction, CompareStatus};
use crate::core::traits::TransferOperations;
use crate::core::types::TransferStatus;
use crate::transfer_engine::TransferManager;
use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;

type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, AppError>> + Send + 'a>>;

/// A "connection" whose remote paths are files under a local folder, with
/// optional unreadable folders and a listing delay.
pub(crate) struct DirConnector {
    root: PathBuf,
    fail: Vec<String>,
    delay_ms: u64,
    pub listings: Mutex<u64>,
}

impl DirConnector {
    pub(crate) fn new(root: &Path) -> Self {
        Self { root: root.to_path_buf(), fail: Vec::new(), delay_ms: 0, listings: Mutex::new(0) }
    }

    fn local(&self, remote: &str) -> PathBuf {
        remote.trim_start_matches('/').split('/').filter(|p| !p.is_empty()).fold(self.root.clone(), |p, s| p.join(s))
    }
}

fn io(e: std::io::Error) -> AppError {
    AppError::connection(e.to_string(), "fake")
}

impl Connector for DirConnector {
    fn connect(&self, _profile: &ConnectionProfile) -> Fut<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn disconnect(&self) -> Fut<'_, ()> {
        Box::pin(async { Ok(()) })
    }
    fn is_connected(&self) -> bool {
        true
    }
    fn list_remote(&self, path: &str) -> Fut<'_, Vec<FileEntry>> {
        let path = path.to_string();
        Box::pin(async move {
            *self.listings.lock().unwrap() += 1;
            if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }
            if self.fail.iter().any(|f| f == &path) {
                return Err(AppError::connection(format!("Permission denied: {path}"), "fake"));
            }
            let mut out = Vec::new();
            for e in std::fs::read_dir(self.local(&path)).map_err(io)? {
                let e = e.map_err(io)?;
                let m = e.metadata().map_err(io)?;
                let name = e.file_name().to_string_lossy().to_string();
                out.push(FileEntry {
                    path: format!("{}/{name}", path.trim_end_matches('/')),
                    name,
                    is_dir: m.is_dir(),
                    is_symlink: false,
                    size: if m.is_dir() { 0 } else { m.len() },
                    modified: m.modified().ok().map(DateTime::<Utc>::from),
                    created: None,
                    is_hidden: false,
                    extension: None,
                    permissions: None,
                });
            }
            Ok(out)
        })
    }
    fn upload(&self, local: &str, remote: &str) -> Fut<'_, u64> {
        let (local, target) = (local.to_string(), self.local(remote));
        Box::pin(async move { std::fs::copy(local, target).map_err(io) })
    }
    fn download(&self, remote: &str, local: &str) -> Fut<'_, u64> {
        let (src, local) = (self.local(remote), local.to_string());
        Box::pin(async move {
            let n = std::fs::copy(&src, &local).map_err(io)?;
            // Like a server that reports the file's time: keep it.
            if let Ok(t) = std::fs::metadata(&src).and_then(|m| m.modified()) {
                let _ = std::fs::File::options().write(true).open(&local).and_then(|f| f.set_modified(t));
            }
            Ok(n)
        })
    }
    fn make_directory(&self, remote: &str) -> Fut<'_, ()> {
        let p = self.local(remote);
        Box::pin(async move { std::fs::create_dir(p).map_err(io) })
    }
    fn remove(&self, remote: &str, is_dir: bool) -> Fut<'_, ()> {
        let p = self.local(remote);
        Box::pin(async move { if is_dir { std::fs::remove_dir(p) } else { std::fs::remove_file(p) }.map_err(io) })
    }
}

fn write(root: &Path, rel: &str, body: &str, mtime_secs: u64) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(&p, body).unwrap();
    let t = std::time::UNIX_EPOCH + std::time::Duration::from_secs(mtime_secs);
    std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
}

/// left: docs/{same,changed,left-only}, extra/x, top; right: docs/{same,changed,right-only}, top (newer), empty/
fn fixture() -> (tempfile::TempDir, tempfile::TempDir) {
    let (l, r) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    write(l.path(), "docs/same.txt", "same", 1_000_000);
    write(r.path(), "docs/same.txt", "same", 1_000_000);
    write(l.path(), "docs/changed.txt", "left version", 1_000_000);
    write(r.path(), "docs/changed.txt", "right", 1_000_000);
    write(l.path(), "docs/left-only.txt", "L", 1_000_000);
    write(r.path(), "docs/right-only.txt", "RR", 1_000_000);
    write(l.path(), "extra/deep/x.bin", "xx", 1_000_000);
    write(l.path(), "top.md", "t", 1_000_000);
    write(r.path(), "top.md", "t", 2_000_000);
    std::fs::create_dir_all(r.path().join("empty")).unwrap();
    (l, r)
}

fn local(dir: &tempfile::TempDir) -> ResolvedSide {
    ResolvedSide::Local { root: dir.path().to_path_buf() }
}

fn remote(dir: &tempfile::TempDir) -> (ResolvedSide, Arc<DirConnector>) {
    let c = Arc::new(DirConnector::new(dir.path()));
    (ResolvedSide::remote(c.clone(), "sftp", "Test server", "/", None), c)
}

fn no_progress() -> ProgressFn {
    Arc::new(|_| {})
}

fn status(out: &[CompareEntry], path: &str) -> CompareStatus {
    out.iter().find(|e| e.path == path).unwrap_or_else(|| panic!("no entry {path}: {:?}", out.iter().map(|e| &e.path).collect::<Vec<_>>())).status
}

async fn compare(left: &ResolvedSide, right: &ResolvedSide, opts: &CompareOptions) -> CompareOutput {
    run_compare("t", left, right, opts, &CancellationToken::new(), &no_progress()).await.unwrap()
}

fn check_fixture(e: &[CompareEntry]) {
    assert_eq!(status(e, "docs/same.txt"), CompareStatus::Same);
    assert_eq!(status(e, "docs/changed.txt"), CompareStatus::Different);
    assert_eq!(status(e, "docs/left-only.txt"), CompareStatus::LeftOnly);
    assert_eq!(status(e, "docs/right-only.txt"), CompareStatus::RightOnly);
    assert_eq!(status(e, "extra"), CompareStatus::LeftOnly);
    assert_eq!(status(e, "extra/deep/x.bin"), CompareStatus::LeftOnly);
    assert_eq!(status(e, "top.md"), CompareStatus::Different);
    assert_eq!(status(e, "empty"), CompareStatus::RightOnly);
    assert_eq!(status(e, "docs"), CompareStatus::Different);
}

#[tokio::test]
async fn compares_two_local_fixture_trees() {
    let (l, r) = fixture();
    let out = compare(&local(&l), &local(&r), &CompareOptions::default()).await;
    check_fixture(&out.entries);
    assert_eq!(out.summary.same.files, 1);
    assert_eq!(out.summary.different.files, 2);
    assert_eq!(out.summary.left_only.files, 2);
    assert_eq!(out.summary.left_only.left_bytes, 3);
    assert_eq!(out.summary.right_only.files, 1);
    assert_eq!(out.summary.right_only.folders, 1);

    // Size only: top.md (same size, newer) is the same.
    let size_only = CompareOptions { compare_mode: ufop_core::sync_types::CompareMode::SizeOnly, ..Default::default() };
    let out = compare(&local(&l), &local(&r), &size_only).await;
    assert_eq!(status(&out.entries, "top.md"), CompareStatus::Same);
    // Checksum: same bytes.
    let checksum = CompareOptions { compare_mode: ufop_core::sync_types::CompareMode::Checksum, ..Default::default() };
    let out = compare(&local(&l), &local(&r), &checksum).await;
    assert_eq!(status(&out.entries, "top.md"), CompareStatus::Same);
    assert!(out.notes.is_empty(), "{:?}", out.notes);
}

#[tokio::test]
async fn compares_local_with_a_connection_the_same_way() {
    let (l, r) = fixture();
    let (right, conn) = remote(&r);
    let out = compare(&local(&l), &right, &CompareOptions::default()).await;
    check_fixture(&out.entries);
    // One listing per folder (/, docs, empty).
    assert_eq!(*conn.listings.lock().unwrap(), 3);
    let checksum = CompareOptions { compare_mode: ufop_core::sync_types::CompareMode::Checksum, ..Default::default() };
    let out = compare(&local(&l), &right, &checksum).await;
    assert!(out.notes[0].contains("local folders only"));
}

#[tokio::test]
async fn max_depth_and_filters() {
    let (l, r) = fixture();
    let depth1 = CompareOptions { max_depth: 1, ..Default::default() };
    let out = compare(&local(&l), &local(&r), &depth1).await;
    let paths: Vec<&str> = out.entries.iter().map(|e| e.path.as_str()).collect();
    assert_eq!(paths, ["docs", "empty", "extra", "top.md"]);
    let docs = out.entries.iter().find(|e| e.path == "docs").unwrap();
    assert!(docs.children_skipped);
    assert_eq!(docs.status, CompareStatus::Same);

    let only_txt = CompareOptions {
        filter: ufop_core::sync_types::SyncFilter { include_patterns: vec!["*.txt".into()], ..Default::default() },
        ..Default::default()
    };
    let out = compare(&local(&l), &local(&r), &only_txt).await;
    assert!(out.entries.iter().all(|e| e.is_dir || e.path.ends_with(".txt")));
    assert!(!out.entries.iter().any(|e| e.path.starts_with("extra")), "extra only held a .bin");
}

#[tokio::test]
async fn an_unreadable_folder_becomes_an_error_entry() {
    let (l, r) = fixture();
    let mut c = DirConnector::new(r.path());
    c.fail.push("/docs".into());
    let right = ResolvedSide::remote(Arc::new(c), "sftp", "Test", "/", None);
    let out = compare(&local(&l), &right, &CompareOptions::default()).await;
    let docs = out.entries.iter().find(|e| e.path == "docs").unwrap();
    assert_eq!(docs.status, CompareStatus::Error);
    assert!(docs.error.as_deref().unwrap().contains("Permission denied"), "{:?}", docs.error);
    assert_eq!(out.summary.error.folders, 1);
    // The rest still compares.
    assert_eq!(status(&out.entries, "top.md"), CompareStatus::Different);

    // An unreadable root fails the comparison outright.
    let mut c = DirConnector::new(r.path());
    c.fail.push("/".into());
    let right = ResolvedSide::remote(Arc::new(c), "sftp", "Test", "/", None);
    let err = run_compare("t", &local(&l), &right, &CompareOptions::default(), &CancellationToken::new(), &no_progress()).await.unwrap_err();
    assert!(err.to_string().contains("Permission denied"));
}

#[tokio::test]
async fn cancelling_stops_the_comparison_and_progress_is_reported() {
    let (l, r) = fixture();
    let mut c = DirConnector::new(r.path());
    c.delay_ms = 400;
    let right = ResolvedSide::remote(Arc::new(c), "sftp", "Test", "/", None);
    let cancel = CancellationToken::new();
    let seen = Arc::new(Mutex::new(Vec::<CompareProgress>::new()));
    let sink: ProgressFn = {
        let seen = seen.clone();
        Arc::new(move |p| seen.lock().unwrap().push(p))
    };
    let canceller = {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            cancel.cancel();
        })
    };
    let started = std::time::Instant::now();
    let err = run_compare("task-1", &local(&l), &right, &CompareOptions::default(), &cancel, &sink).await.unwrap_err();
    canceller.await.unwrap();
    assert!(err.to_string().contains("cancelled"), "{err}");
    assert!(started.elapsed() < std::time::Duration::from_millis(390), "cancel did not stop the walk");
    let seen = seen.lock().unwrap();
    assert!(seen.iter().all(|p| p.task_id == "task-1" && p.phase == ComparePhase::Listing));
    assert!(!seen.is_empty());
}

#[tokio::test]
async fn manager_cancels_and_keeps_results() {
    let m = CompareManager::new();
    let t = m.begin("a");
    assert!(m.cancel("a"));
    assert!(t.is_cancelled());
    assert!(!m.cancel("a"));
    for i in 0..10 {
        m.store(StoredCompare { task_id: format!("r{i}"), left: CompareSide::Local { path: "/".into() }, right: CompareSide::Local { path: "/".into() }, options: Default::default(), entries: vec![] }).await;
    }
    assert!(m.with("r0", |_| ()).await.is_err(), "oldest results are dropped");
    assert!(m.with("r9", |_| ()).await.is_ok());
    assert!(m.release("r9").await);
    assert!(m.with("r9", |_| ()).await.is_err());
}

#[test]
fn compare_side_wire_format() {
    let side: CompareSide = serde_json::from_str(r#"{"type":"remote","connection_id":"abc","path":"/x"}"#).unwrap();
    assert_eq!(side, CompareSide::Remote { connection_id: Some("abc".into()), protocol: None, path: "/x".into() });
    let side: CompareSide = serde_json::from_str(r#"{"type":"local","path":"/home"}"#).unwrap();
    assert_eq!(side, CompareSide::Local { path: "/home".into() });
}

// ── Acting on differences ──

async fn env_parts() -> (TransferManager, OperationLedger) {
    let pool = crate::storage::pool::DbPool::open_in_memory().unwrap();
    pool.execute(|conn| {
        crate::storage::migrations::run_migrations(conn)?;
        Ok(())
    })
    .await
    .unwrap();
    let tm = TransferManager::new();
    tm.init_journal(pool.clone()).await;
    (tm, OperationLedger::new(pool))
}

async fn act(
    left: &ResolvedSide,
    right: &ResolvedSide,
    entries: &mut Vec<CompareEntry>,
    selected: &[&str],
    action: CompareAction,
    tm: &TransferManager,
    ledger: &OperationLedger,
) -> (apply::ExecReport, compare::MergeOutcome) {
    let selected: Vec<String> = selected.iter().map(|s| s.to_string()).collect();
    let plan = plan_action(entries, &selected, action);
    let cancel = CancellationToken::new();
    let progress = no_progress();
    let env = ApplyEnv { transfer: tm, ledger, permanent: true, cancel: &cancel, progress: &progress, task_id: "t" };
    let report = apply::execute(&plan, left, right, &env).await;
    let refreshed = apply::refresh(left, right, &CompareOptions::default(), &report.touched).await.unwrap();
    let merged = compare::merge_refreshed(entries, &report.touched, refreshed);
    (report, merged)
}

#[tokio::test]
async fn copy_to_right_between_local_folders_uses_the_transfer_engine() {
    let (l, r) = fixture();
    let (left, right) = (local(&l), local(&r));
    let mut entries = compare(&left, &right, &CompareOptions::default()).await.entries;
    let (tm, ledger) = env_parts().await;
    let (report, merged) = act(&left, &right, &mut entries, &["docs", "extra", "top.md"], CompareAction::CopyToRight, &tm, &ledger).await;
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(std::fs::read_to_string(r.path().join("docs/changed.txt")).unwrap(), "left version");
    assert_eq!(std::fs::read_to_string(r.path().join("docs/left-only.txt")).unwrap(), "L");
    assert_eq!(std::fs::read_to_string(r.path().join("extra/deep/x.bin")).unwrap(), "xx");
    // Right-only files are untouched.
    assert!(r.path().join("docs/right-only.txt").exists());
    // No staging files left behind.
    assert!(!std::fs::read_dir(r.path().join("docs")).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().contains("ufop-compare")));
    // Every copy was a transfer-engine job.
    let jobs = tm.list_jobs().await.unwrap();
    assert_eq!(jobs.len(), 4, "changed, left-only, x.bin, top.md");
    assert!(jobs.iter().all(|j| j.status == TransferStatus::Completed));
    // Re-compared: the copies now match (the copy kept the source's time).
    for p in ["docs/changed.txt", "docs/left-only.txt", "extra/deep/x.bin", "top.md", "extra"] {
        assert_eq!(status(&entries, p), CompareStatus::Same, "{p}");
    }
    assert_eq!(status(&entries, "docs"), CompareStatus::Different, "right-only.txt is still only on the right");
    assert_eq!(merged.summary.left_only.files, 0);
    assert!(merged.changed.iter().any(|e| e.path == "extra/deep"));
    // New files are one undoable copy; Undo removes them.
    let undo_id = report.undo_id.clone().expect("undoable copies");
    let rows = ledger.by_correlation(undo_id).await.unwrap();
    assert_eq!(rows.len(), 2, "left-only.txt and x.bin");
    assert!(rows.iter().all(|r| r.kind == "copy" && r.engine == "fs"));
    let dests: Vec<String> = rows.iter().filter_map(|r| r.target_path.clone()).collect();
    crate::commands::file_ops_commands::undo_file_operation_inner("copy".into(), vec![], dests).await.unwrap();
    assert!(!r.path().join("docs/left-only.txt").exists());
    assert!(r.path().join("docs/changed.txt").exists(), "undoing the copies leaves replaced files alone");
}

#[tokio::test]
async fn copy_never_touches_same_entries() {
    let (l, r) = fixture();
    let (left, right) = (local(&l), local(&r));
    let mut entries = compare(&left, &right, &CompareOptions::default()).await.entries;
    let before = std::fs::metadata(r.path().join("docs/same.txt")).unwrap().modified().unwrap();
    let (tm, ledger) = env_parts().await;
    let (report, _) = act(&left, &right, &mut entries, &["docs/same.txt"], CompareAction::CopyToRight, &tm, &ledger).await;
    assert!(report.done.is_empty());
    assert!(tm.list_jobs().await.unwrap().is_empty());
    assert_eq!(std::fs::metadata(r.path().join("docs/same.txt")).unwrap().modified().unwrap(), before);
}

#[tokio::test]
async fn copy_to_left_from_a_connection_and_back() {
    let (l, r) = fixture();
    let left = local(&l);
    let (right, _) = remote(&r);
    let mut entries = compare(&left, &right, &CompareOptions::default()).await.entries;
    let (tm, ledger) = env_parts().await;

    // Download: right-only and the newer top.md come to the left.
    let (report, _) = act(&left, &right, &mut entries, &["docs/right-only.txt", "top.md", "empty"], CompareAction::CopyToLeft, &tm, &ledger).await;
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(std::fs::read_to_string(l.path().join("docs/right-only.txt")).unwrap(), "RR");
    assert!(l.path().join("empty").is_dir());
    assert_eq!(status(&entries, "docs/right-only.txt"), CompareStatus::Same);
    assert_eq!(status(&entries, "top.md"), CompareStatus::Same);
    assert_eq!(status(&entries, "empty"), CompareStatus::Same);
    // Tracked in the queue with the connection named.
    let jobs = tm.list_jobs().await.unwrap();
    assert_eq!(jobs.len(), 2);
    assert!(jobs.iter().all(|j| j.status == TransferStatus::Completed && j.source_path.starts_with("sftp://Test server/")));

    // Upload: the left-only folder goes to the connection, folders first.
    let (report, _) = act(&left, &right, &mut entries, &["extra"], CompareAction::CopyToRight, &tm, &ledger).await;
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(std::fs::read_to_string(r.path().join("extra/deep/x.bin")).unwrap(), "xx");
    assert!(entries.iter().any(|e| e.path == "extra/deep/x.bin" && e.in_right));
    let uploads = ledger.query(crate::ledger::LedgerQuery { engine: Some("connector".into()), kind: Some("copy".into()), subject_path: None, correlation_id: None, limit: None }).await.unwrap();
    assert_eq!(uploads.len(), 1);
}

#[tokio::test]
async fn delete_on_each_side_removes_files_then_empty_folders() {
    let (l, r) = fixture();
    let left = local(&l);
    let (right, _) = remote(&r);
    let mut entries = compare(&left, &right, &CompareOptions::default()).await.entries;
    let (tm, ledger) = env_parts().await;

    let (report, merged) = act(&left, &right, &mut entries, &["extra"], CompareAction::DeleteLeft, &tm, &ledger).await;
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(!l.path().join("extra").exists());
    assert!(merged.removed.contains(&"extra".to_string()) && merged.removed.contains(&"extra/deep/x.bin".to_string()));
    assert!(!entries.iter().any(|e| e.path.starts_with("extra")));
    // Recorded as the file manager's own delete.
    let deletes = ledger.query(crate::ledger::LedgerQuery { engine: Some("fs".into()), kind: Some("delete_permanent".into()), subject_path: None, correlation_id: None, limit: None }).await.unwrap();
    assert!(!deletes.is_empty());

    let (report, _) = act(&left, &right, &mut entries, &["docs/right-only.txt", "empty"], CompareAction::DeleteRight, &tm, &ledger).await;
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert!(!r.path().join("docs/right-only.txt").exists());
    assert!(!r.path().join("empty").exists());
    assert!(!entries.iter().any(|e| e.path == "empty"));

    // A folder that still holds files outside the comparison is kept.
    write(l.path(), "keep/a.txt", "a", 1);
    write(l.path(), "keep/b.log", "b", 1);
    let filtered = CompareOptions { filter: ufop_core::sync_types::SyncFilter { exclude_patterns: vec!["*.log".into()], ..Default::default() }, ..Default::default() };
    let mut entries = compare(&left, &right, &filtered).await.entries;
    let plan = plan_action(&entries, &["keep".to_string()], CompareAction::DeleteLeft);
    let cancel = CancellationToken::new();
    let progress = no_progress();
    let env = ApplyEnv { transfer: &tm, ledger: &ledger, permanent: true, cancel: &cancel, progress: &progress, task_id: "t" };
    let report = apply::execute(&plan, &left, &right, &env).await;
    assert_eq!(report.kept, ["keep"]);
    assert!(l.path().join("keep/b.log").exists());
    assert!(!l.path().join("keep/a.txt").exists());
    let refreshed = apply::refresh(&left, &right, &filtered, &report.touched).await.unwrap();
    compare::merge_refreshed(&mut entries, &report.touched, refreshed);
    assert_eq!(status(&entries, "keep"), CompareStatus::LeftOnly);
}

#[tokio::test]
async fn cancelling_an_action_stops_between_steps() {
    let (l, r) = fixture();
    let (left, right) = (local(&l), local(&r));
    let entries = compare(&left, &right, &CompareOptions::default()).await.entries;
    let (tm, ledger) = env_parts().await;
    let plan = plan_action(&entries, &["docs".to_string(), "extra".to_string()], CompareAction::CopyToRight);
    let cancel = CancellationToken::new();
    cancel.cancel();
    let progress = no_progress();
    let env = ApplyEnv { transfer: &tm, ledger: &ledger, permanent: true, cancel: &cancel, progress: &progress, task_id: "t" };
    let report = apply::execute(&plan, &left, &right, &env).await;
    assert!(report.cancelled);
    assert!(report.done.is_empty());
    assert!(!r.path().join("docs/left-only.txt").exists());
}
