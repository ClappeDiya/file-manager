use super::*;
use crate::storage::Repository;

async fn manager(dir: &Path) -> BackupManager {
    let repo = Repository::open_in_memory().await.unwrap();
    BackupManager::with_kdf(
        repo.pool().clone(),
        Arc::new(CredentialStore::new()),
        None,
        Some(OperationLedger::new(repo.pool().clone())),
        dir.join("dbs"),
        KdfParams::insecure_fast(),
    )
}

fn job(src: &Path, dst: &Path) -> BackupJob {
    BackupJob {
        name: "Docs".into(),
        sources: vec![src.to_string_lossy().into()],
        destination: BackupDestination::Local { path: dst.to_string_lossy().into(), alternate_paths: vec![], marker_file: None },
        // Host disk fullness must not turn results into warnings.
        options: BackupOptions { block_size: 4096, volume_size: 64 * 1024, quota_disable: true, ..Default::default() },
        ..Default::default()
    }
}

#[tokio::test]
async fn job_crud_validation_and_persistence() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    let m = manager(t.path()).await;

    // Encrypted job without passphrase is rejected.
    assert!(m.save_job(job(&src, &dst), None).await.is_err());
    // Destination inside source is rejected.
    assert!(m.save_job(job(&src, &src.join("b")), Some("pw".into())).await.is_err());
    let saved = m.save_job(job(&src, &dst), Some("pw".into())).await.unwrap();
    assert!(!saved.id.is_empty());
    assert_eq!(m.list_jobs().await.len(), 1);

    let m2 = BackupManager { inner: m.inner.clone() };
    m2.inner.jobs.write().await.clear();
    assert_eq!(m2.load_from_db().await.unwrap(), 1);

    m.delete_job(&saved.id, true, false).await.unwrap();
    assert!(m.list_jobs().await.is_empty());
}

#[tokio::test]
async fn backup_restore_history_and_queries() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst, out) = (t.path().join("src"), t.path().join("dst"), t.path().join("out"));
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.txt"), b"alpha").unwrap();
    std::fs::write(src.join("sub/b.txt"), b"beta").unwrap();
    let m = manager(t.path()).await;
    let j = m.save_job(job(&src, &dst), Some("pw".into())).await.unwrap();

    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert!(r.summary.contains("2 added"));
    let versions = m.list_versions(&j.id).await.unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(m.get_job(&j.id).await.unwrap().stats.version_count, 1);
    assert_eq!(m.search(&j.id, "b.txt".into(), None).await.unwrap().len(), 1);
    assert!(m.list_files(&j.id, 0, None).await.unwrap().len() >= 3);

    let restore = RestoreOptions { target: Some(out.clone()), ..Default::default() };
    let r = m.execute(&j.id, BackupOperation::Restore { options: restore }).await.unwrap();
    assert_eq!(r.report["files_restored"], 2);
    assert_eq!(std::fs::read(out.join("src/sub/b.txt")).unwrap(), b"beta");

    let r = m.execute(&j.id, BackupOperation::Verify { full: true }).await.unwrap();
    assert_eq!(r.status, "success");

    let hist = m.run_history(Some(j.id.clone()), 10).await.unwrap();
    assert_eq!(hist.len(), 3);
    assert_eq!(hist.iter().filter(|h| h.operation == "backup").count(), 1);

    // Direct restore (no job database) with only the passphrase.
    let dest = j.destination.clone();
    assert_eq!(m.browse_remote(dest.clone(), Some("pw".into()), "ufop".into()).await.unwrap().len(), 1);
    let out2 = t.path().join("out2");
    let rr = m.direct_restore(dest, Some("pw".into()), "ufop".into(), RestoreOptions { target: Some(out2.clone()), ..Default::default() }).await.unwrap();
    assert_eq!(rr.files_restored, 2);
}

#[tokio::test]
async fn background_backup_status_and_scheduler() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("f"), b"x").unwrap();
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.encrypted = false;
    jb.schedule = BackupSchedule { enabled: true, start_time: "00:00".into(), interval_minutes: 60, weekdays: vec![], skip_on_battery: false };
    let j = m.save_job(jb, None).await.unwrap();

    // Created "now": the next slot is in the future, so not due yet …
    assert!(m.due_jobs(Utc::now()).await.is_empty());
    // … but a slot two hours later is (missed-run catch-up uses the same rule).
    assert_eq!(m.due_jobs(Utc::now() + chrono::Duration::hours(2)).await, vec![j.id.clone()]);
    m.pause(None);
    assert!(m.due_jobs(Utc::now() + chrono::Duration::hours(2)).await.is_empty());
    assert!(m.status().await.paused);
    m.resume();

    let tid = m.start_backup(&j.id, false).await.unwrap();
    assert!(m.start_backup(&j.id, false).await.is_err(), "double queue rejected");
    assert!(m.status().await.tasks.iter().any(|t| t.task_id == tid));
    for _ in 0..100 {
        if !m.is_job_busy(&j.id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert!(!m.is_job_busy(&j.id));
    assert_eq!(m.get_job(&j.id).await.unwrap().stats.last_result.as_deref(), Some("success"));
    assert_eq!(m.status().await.next_runs.len(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn required_pre_script_failure_aborts() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("f"), b"x").unwrap();
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.encrypted = false;
    jb.scripts = ScriptConfig { run_before: Some("sh -c 'echo LOG:ERROR nope; exit 1'".into()), run_before_required: true, ..Default::default() };
    let j = m.save_job(jb, None).await.unwrap();
    let e = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await;
    assert!(e.is_err());
    assert!(!dst.exists());
    let h = m.run_history(Some(j.id), 5).await.unwrap();
    assert_eq!(h[0].status, "error");
    assert!(h[0].log.iter().any(|l| l.message.contains("nope")));
}

#[tokio::test]
async fn passphrase_persists_without_keychain() {
    // Regression: without an OS keychain the credential store is memory-only,
    // so a restarted server lost every job passphrase.
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    let repo = Repository::open_in_memory().await.unwrap();
    let mk = || {
        let m = BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, None, t.path().join("dbs"), KdfParams::insecure_fast());
        m.set_settings_store(SettingsStore::with_key(repo.pool().clone(), "settings-key").unwrap());
        m
    };
    let first = mk();
    let j = first.save_job(job(&src, &dst), Some("pw-123".into())).await.unwrap();
    let restarted = mk();
    restarted.load_from_db().await.unwrap();
    assert_eq!(restarted.passphrase(&j.id).await.unwrap().as_deref(), Some("pw-123"));
    // Stored encrypted, not in plain text.
    let raw: String = repo.pool().execute(|c| Ok(c.query_row("SELECT value FROM config WHERE key LIKE 'backup-passphrase:%'", [], |r| r.get(0))?)).await.unwrap();
    assert!(raw.starts_with("enc:v1:") && !raw.contains("pw-123"));
}
