//! WS-I row 73: what started a run, a history row per replica, date ranges.

use super::*;
use crate::storage::Repository;
use ufop_core::run_trigger::RunTrigger;

async fn manager(dir: &Path) -> BackupManager {
    let repo = Repository::open_in_memory().await.unwrap();
    BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, None, dir.join("dbs"), KdfParams::insecure_fast())
}

fn local(p: &Path) -> BackupDestination {
    BackupDestination::Local { path: p.to_string_lossy().into(), alternate_paths: vec![], marker_file: None }
}

fn job(src: &Path, dst: &Path) -> BackupJob {
    BackupJob {
        name: "Docs".into(),
        sources: vec![src.to_string_lossy().into()],
        destination: local(dst),
        encrypted: false,
        options: BackupOptions { block_size: 4096, volume_size: 64 * 1024, quota_disable: true, verify_samples: 0, ..Default::default() },
        ..Default::default()
    }
}

fn setup(t: &tempfile::TempDir) -> (PathBuf, PathBuf) {
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"alpha alpha alpha").unwrap();
    (src, dst)
}

async fn wait_for_runs(m: &BackupManager, job_id: &str, n: usize) -> Vec<BackupRun> {
    for _ in 0..400 {
        let runs = m.run_history(Some(job_id.to_string()), 50).await.unwrap();
        if runs.len() >= n && !m.is_job_busy(job_id) {
            return runs;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the runs did not finish");
}

#[tokio::test(flavor = "multi_thread")]
async fn runs_record_what_started_them() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = setup(&t);
    let m = manager(t.path()).await;
    let j = m.save_job(job(&src, &dst), None).await.unwrap();

    // Nothing says otherwise: a person pressed Run.
    m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    // An API caller.
    trigger::scoped(RunTrigger::Api, m.execute(&j.id, BackupOperation::Verify { full: false, options: None })).await.unwrap();
    // A background task started by an API caller keeps the trigger.
    std::fs::write(src.join("b.txt"), b"bravo").unwrap();
    trigger::scoped(RunTrigger::Api, m.start_backup(&j.id, false)).await.unwrap();

    let runs = wait_for_runs(&m, &j.id, 3).await;
    let by = |op: &str, i: usize| runs.iter().filter(|r| r.operation == op).nth(i).map(|r| r.trigger);
    // Newest first: the queued backup, then the verify, then the manual backup.
    assert_eq!(by("backup", 0), Some(RunTrigger::Api), "the spawned task carries the trigger");
    assert_eq!(by("verify", 0), Some(RunTrigger::Api));
    assert_eq!(by("backup", 1), Some(RunTrigger::Manual));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_scheduler_records_scheduled_runs() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = setup(&t);
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.schedule = BackupSchedule { enabled: true, start_time: "00:00".into(), interval_minutes: 60, ..Default::default() };
    let j = m.save_job(jb, None).await.unwrap();
    m.inner.jobs.write().await.get_mut(&j.id).unwrap().stats.last_scheduled_at = Some((Utc::now() - chrono::Duration::hours(3)).to_rfc3339());

    assert_eq!(m.tick().await, 1);

    let runs = wait_for_runs(&m, &j.id, 1).await;
    assert_eq!(runs[0].operation, "backup");
    assert_eq!(runs[0].trigger, RunTrigger::Scheduled);
}

#[tokio::test(flavor = "multi_thread")]
async fn every_replication_leaves_a_row_per_replica() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = setup(&t);
    let m = manager(t.path()).await;
    // A replica that cannot work: its folder would have to live under a file.
    let blocker = t.path().join("blocker");
    std::fs::write(&blocker, b"not a folder").unwrap();
    let mut jb = job(&src, &dst);
    jb.replicas = vec![
        Replica { name: "USB".into(), destination: local(&t.path().join("usb")), ..Default::default() },
        Replica { name: "NAS".into(), destination: local(&t.path().join("nas")), schedule: ReplicaSchedule::Manual, ..Default::default() },
        Replica { name: "Broken".into(), destination: local(&blocker.join("inside")), schedule: ReplicaSchedule::Manual, ..Default::default() },
    ];
    let j = m.save_job(jb, None).await.unwrap();
    let id_of = |name: &str| j.replicas.iter().find(|r| r.name == name).unwrap().id.clone();

    // The backup replicates to the after-backup replica (USB).
    m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    // Then the others by hand, USB again under the API.
    m.execute(&j.id, BackupOperation::Replicate { replica_id: Some(id_of("NAS")) }).await.unwrap();
    let broken = m.execute(&j.id, BackupOperation::Replicate { replica_id: Some(id_of("Broken")) }).await.unwrap();
    assert_eq!(broken.status, "error");
    trigger::scoped(RunTrigger::Api, m.execute(&j.id, BackupOperation::Replicate { replica_id: Some(id_of("USB")) })).await.unwrap();

    let all = m.replica_history(ReplicaHistoryFilter { job_id: Some(j.id.clone()), ..Default::default() }).await.unwrap();
    let names: Vec<(&str, &str)> = all.iter().map(|r| (r.replica_name.as_str(), r.status.as_str())).collect();
    assert_eq!(names, vec![("USB", "success"), ("Broken", "error"), ("NAS", "success"), ("USB", "success")], "newest first, one row per replica run");
    let broken_row = all.iter().find(|r| r.replica_name == "Broken").unwrap();
    assert!(broken_row.error.as_deref().is_some_and(|e| !e.is_empty()), "the failure says why");
    assert_eq!((all[0].trigger, all[3].trigger), (RunTrigger::Api, RunTrigger::Manual));
    // A first copy reports what it moved; the repeat finds everything already there.
    assert!(all[2].files > 0 && all[2].bytes > 0 && all[3].files > 0, "{all:?}");
    assert_eq!((all[0].files, all[0].bytes), (0, 0), "{all:?}");

    let usb_only = m.replica_history(ReplicaHistoryFilter { replica_id: Some(id_of("USB")), ..Default::default() }).await.unwrap();
    assert_eq!(usb_only.len(), 2, "history, not only the last result");
    // Rows point at the run that made them and go when it is pruned.
    let runs = m.run_history(Some(j.id.clone()), 50).await.unwrap();
    assert!(all.iter().all(|r| runs.iter().any(|run| run.id == r.run_id)));
    assert_eq!(m.prune_history(0).await.unwrap(), runs.len());
    assert!(m.replica_history(ReplicaHistoryFilter::default()).await.unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn run_and_replica_history_can_be_limited_to_a_date_range() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = setup(&t);
    let m = manager(t.path()).await;
    let j = m.save_job(job(&src, &dst), None).await.unwrap();
    let ago = |days: i64| (Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    for (i, days) in [0i64, 3, 20].into_iter().enumerate() {
        let run = BackupRun {
            id: format!("r{i}"),
            job_id: j.id.clone(),
            operation: "backup".into(),
            status: "success".into(),
            started_at: ago(days),
            finished_at: None,
            summary: format!("run {i}"),
            report: serde_json::Value::Null,
            log: vec![],
            trigger: Default::default(),
        };
        m.insert_run(&run).await.unwrap();
    }
    let ids = |v: Vec<BackupRun>| v.into_iter().map(|r| r.id).collect::<Vec<_>>();
    assert_eq!(ids(m.run_history_between(None, 50, None, None).await.unwrap()), ["r0", "r1", "r2"]);
    assert_eq!(ids(m.run_history_between(None, 50, Some(ago(7)), None).await.unwrap()), ["r0", "r1"]);
    assert_eq!(ids(m.run_history_between(None, 50, Some(ago(30)), Some(ago(1))).await.unwrap()), ["r1", "r2"]);
    // The paged listing honours the same range and counts only matching rows.
    let page = paging::PageRequest { limit: Some(10), ..Default::default() };
    match m.run_history_listing_between(None, 50, Some(page), Some(ago(7)), None).await.unwrap() {
        paging::Listing::Page(p) => assert_eq!((p.total, p.items.len()), (2, 2)),
        _ => panic!("expected a page"),
    }
}
