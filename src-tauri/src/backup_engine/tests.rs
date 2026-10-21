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
    let r = m.execute(&j.id, BackupOperation::Restore { options: restore, overrides: RestoreOverrides::default() }).await.unwrap();
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

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn maintenance_ops_and_source_release_script() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.bin"), vec![3u8; 20_000]).unwrap();
    let ctl = t.path().join("manifest.txt");
    std::fs::write(&ctl, b"ctl").unwrap();
    let marker = t.path().join("released");
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.encrypted = false;
    jb.options.control_files = vec![ctl.to_string_lossy().into()];
    jb.scripts.run_on_source_release = Some(format!("touch {}", marker.display()));
    let j = m.save_job(jb, None).await.unwrap();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert!(marker.exists(), "source-release script ran");

    let out = t.path().join("ctl-out");
    let r = m.execute(&j.id, BackupOperation::ExtractControlFiles { version: 0, target: out.to_string_lossy().into() }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert_eq!(std::fs::read(out.join("manifest.txt")).unwrap(), b"ctl");

    let r = m.execute(&j.id, BackupOperation::AutoTune { sample_volumes: 2 }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert!(r.summary.contains("thread"));

    for e in std::fs::read_dir(&dst).unwrap().flatten() {
        if e.file_name().to_string_lossy().contains(".dblock") {
            std::fs::remove_file(e.path()).unwrap();
        }
    }
    let r = m.execute(&j.id, BackupOperation::RebuildMissing).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert!(r.report["volumes_rebuilt"].as_u64().unwrap() >= 1);
    let r = m.execute(&j.id, BackupOperation::Verify { full: true }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
}

#[tokio::test(flavor = "multi_thread")]
async fn adopt_repository_continues_history() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), vec![9u8; 30_000]).unwrap();
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.store_config_remotely = true;
    jb.remote_config_include_secrets = true;
    jb.reports.http_urls = vec!["https://example.invalid/hook?token=x".into()];
    let j = m.save_job(jb, Some("pw".into())).await.unwrap();
    assert_eq!(m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");

    // "New computer": fresh manager; the source moved to another folder.
    let moved = t.path().join("moved");
    std::fs::rename(&src, &moved).unwrap();
    let t2 = tempfile::tempdir().unwrap();
    let m2 = manager(t2.path()).await;
    let dest = BackupDestination::Local { path: dst.to_string_lossy().into(), alternate_paths: vec![], marker_file: None };
    assert!(m2.adopt_repository(dest.clone(), Some("wrong".into()), "ufop".into(), None, vec![], vec![]).await.is_err());
    assert!(m2.list_jobs().await.is_empty(), "failed adoption leaves no job behind");
    let adopted = m2
        .adopt_repository(dest, Some("pw".into()), "ufop".into(), Some("Laptop".into()), vec![], vec![(src.to_string_lossy().into(), moved.to_string_lossy().into())])
        .await
        .unwrap();
    assert_eq!(adopted.name, "Laptop");
    assert_eq!(adopted.sources, vec![moved.to_string_lossy().to_string()]);
    assert_eq!(adopted.stats.version_count, 1, "history continues");
    assert_eq!(adopted.reports.http_urls.len(), 1, "secrets kept in encrypted config");
    let r = m2.execute(&adopted.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert_eq!(r.report["new_blocks"], 0, "data deduplicated against the old history");
    assert_eq!(m2.list_versions(&adopted.id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn global_defaults_and_destination_library() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("lib"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("f"), b"x").unwrap();
    let m = manager(t.path()).await;

    let mut d = defaults::GlobalDefaults::default();
    d.options.insert("upload_concurrency".into(), 5.into());
    m.set_global_defaults(d).await.unwrap();
    let mut bad = defaults::GlobalDefaults::default();
    bad.options.insert("block_size".into(), 4096.into());
    assert!(m.set_global_defaults(bad).await.is_err());

    let lib = m
        .save_destination(defaults::SavedDestination {
            id: String::new(),
            name: "NAS".into(),
            destination: BackupDestination::Local { path: dst.to_string_lossy().into(), alternate_paths: vec![], marker_file: None },
            created_at: String::new(),
        })
        .await
        .unwrap();
    assert_eq!(m.list_destinations().await.unwrap().len(), 1);
    let mut jb = job(&src, &dst);
    jb.encrypted = false;
    jb.destination = BackupDestination::Library { destination_id: lib.id.clone(), remote_path: "laptop".into() };
    let j = m.save_job(jb, None).await.unwrap();
    assert_eq!(m.effective_job(&j.id).await.unwrap().options.upload_concurrency, 5);
    assert_eq!(m.get_job(&j.id).await.unwrap().options.upload_concurrency, 2, "stored job unchanged");
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert!(dst.join("laptop").join("ufop-repo.json").exists(), "library sub-folder used");
    assert!(m.delete_destination(&lib.id).await.is_err(), "in use");

    // Secrets in the remote config need encryption.
    let mut jb = job(&src, &t.path().join("x"));
    jb.encrypted = false;
    jb.remote_config_include_secrets = true;
    assert!(m.save_job(jb, None).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn database_tools_recovery_and_restore_overrides() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"recover me").unwrap();
    let m = manager(t.path()).await;
    let j = m.save_job(job(&src, &dst), Some("pw".into())).await.unwrap();
    assert_eq!(m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");

    // Restore with one-off overrides (job settings untouched).
    let out = t.path().join("out");
    let ov = RestoreOverrides { download_bps: Some(10_000_000), retries: Some(0), network_timeout_secs: Some(30) };
    let r = m.execute(&j.id, BackupOperation::Restore { options: RestoreOptions { target: Some(out.clone()), ..Default::default() }, overrides: ov }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert_eq!(m.get_job(&j.id).await.unwrap().throttle.download_bps, 0);
    let ser: BackupOperation = serde_json::from_str(r#"{"type":"restore","options":{}}"#).unwrap();
    assert!(matches!(ser, BackupOperation::Restore { overrides, .. } if overrides == RestoreOverrides::default()), "overrides optional");

    // Inventory and cleanup: an orphan database is found and removed.
    std::fs::copy(m.job_db_path(&j.id), t.path().join("dbs").join("orphan.sqlite")).unwrap();
    let inv = m.database_inventory().await.unwrap();
    assert_eq!(inv.len(), 2);
    assert!(inv.iter().any(|e| e.job_id == "orphan" && e.job_name.is_none()));
    assert!(inv.iter().all(|e| e.info.as_ref().is_some_and(|i| i.integrity_ok)));
    let c = m.cleanup_databases(true, true).await.unwrap();
    assert_eq!(c.orphans_deleted, vec!["orphan".to_string()]);
    assert_eq!(c.vacuumed, 1);
    assert_eq!(m.database_inventory().await.unwrap().len(), 1);

    // Recovery tool through the manager.
    let dest = j.destination.clone();
    let folder = t.path().join("rec").to_string_lossy().to_string();
    let d = m.recovery_download(dest, Some("pw".into()), "ufop".into(), folder.clone(), Default::default()).await.unwrap();
    assert!(d.failed.is_empty() && d.downloaded > 0);
    m.recovery_index(folder.clone()).await.unwrap();
    assert_eq!(m.recovery_list(folder.clone()).await.unwrap().len(), 1);
    let rt = t.path().join("rec-out");
    let rr = m.recovery_restore(folder.clone(), 0, vec![], rt.to_string_lossy().into()).await.unwrap();
    assert_eq!(rr.files_restored, 1);
    assert_eq!(std::fs::read(rt.join("src/a.txt")).unwrap(), b"recover me");
}

#[tokio::test(flavor = "multi_thread")]
async fn replicas_mirror_after_backup_and_on_demand() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst, rep) = (t.path().join("src"), t.path().join("dst"), t.path().join("replica"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), vec![5u8; 20_000]).unwrap();
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.local_transfer = LocalTransfer { use_move: false, verify_size: true, sync_writes: true };
    jb.replicas = vec![
        Replica { name: "USB".into(), destination: BackupDestination::Local { path: rep.to_string_lossy().into(), alternate_paths: vec![], marker_file: None }, verify: true, ..Default::default() },
        Replica { name: "Hourly".into(), destination: BackupDestination::Local { path: t.path().join("hourly").to_string_lossy().into(), alternate_paths: vec![], marker_file: None }, schedule: ReplicaSchedule::Interval { minutes: 60 }, ..Default::default() },
    ];
    let mut bad = jb.clone();
    bad.replicas[0].destination = bad.destination.clone();
    assert!(m.save_job(bad, Some("pw".into())).await.is_err(), "replica == primary rejected");
    let j = m.save_job(jb, Some("pw".into())).await.unwrap();
    assert!(j.replicas.iter().all(|r| !r.id.is_empty()));

    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert_eq!(r.report["replication"].as_array().unwrap().len(), 1, "only the after-backup replica ran");
    assert!(rep.join("ufop-repo.json").exists());
    assert!(!t.path().join("hourly").exists());
    let j = m.get_job(&j.id).await.unwrap();
    assert_eq!(j.replicas[0].last_result.as_deref(), Some("success"));

    // The interval replica is due; the scheduler starts it.
    assert_eq!(m.due_replicas(Utc::now()).await.len(), 1);
    let r = m.execute(&j.id, BackupOperation::Replicate { replica_id: Some(j.replicas[1].id.clone()) }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert!(t.path().join("hourly/ufop-repo.json").exists());
    assert!(m.due_replicas(Utc::now()).await.is_empty());

    // Each replica restores independently.
    let rep_dest = BackupDestination::Local { path: rep.to_string_lossy().into(), alternate_paths: vec![], marker_file: None };
    let out = t.path().join("out");
    let rr = m.direct_restore(rep_dest.clone(), Some("pw".into()), "ufop".into(), RestoreOptions { target: Some(out.clone()), ..Default::default() }).await.unwrap();
    assert_eq!(rr.files_restored, 1);

    // Direct destination file operations, with a guard for repository files.
    let files = m.remote_list(j.destination.clone()).await.unwrap();
    assert!(files.iter().any(|f| f.name == "ufop-repo.json"));
    let lp = t.path().join("note.txt");
    std::fs::write(&lp, b"hello").unwrap();
    assert_eq!(m.remote_put(j.destination.clone(), lp.to_string_lossy().into(), "note.txt".into()).await.unwrap(), 5);
    let back = t.path().join("note2.txt");
    m.remote_get(j.destination.clone(), "note.txt".into(), back.to_string_lossy().into()).await.unwrap();
    assert_eq!(std::fs::read(back).unwrap(), b"hello");
    assert!(m.remote_delete(j.destination.clone(), "ufop-repo.json".into(), false).await.is_err());
    m.remote_delete(j.destination.clone(), "note.txt".into(), false).await.unwrap();

    // Lock admin needs a lock-capable destination.
    let r = m.execute(&j.id, BackupOperation::ApplyLocks { days: 1, mode: ufop_core::backup::backend::LockMode::Governance, all: false }).await;
    assert!(r.is_err_and(|e| e.user_message().contains("does not support object locks")));
}

#[tokio::test(flavor = "multi_thread")]
async fn cloud_destination_through_manager() {
    if std::process::Command::new("rclone").arg("version").output().is_err() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"to the cloud").unwrap();
    let m = manager(t.path()).await;
    let cloud = BackupDestination::Cloud {
        config: stores::CloudConfig::Rclone(stores::rclone::RcloneConfig { remote: ":local:".into(), path: t.path().join("remote").to_string_lossy().into(), ..Default::default() }),
        credential_id: "cred-1".into(),
    };
    m.set_destination_secret("cred-1", "unused-config-pass").await.unwrap();
    assert!(m.has_destination_secret("cred-1").await.unwrap());
    let mut jb = job(&src, &t.path().join("unused"));
    jb.destination = cloud.clone();
    let j = m.save_job(jb, Some("pw".into())).await.unwrap();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert!(t.path().join("remote/ufop-repo.json").exists());
    let out = t.path().join("out");
    let rr = m.direct_restore(cloud, Some("pw".into()), "ufop".into(), RestoreOptions { target: Some(out.clone()), ..Default::default() }).await.unwrap();
    assert_eq!(rr.files_restored, 1);
    m.set_destination_secret("cred-1", "").await.unwrap();
    assert!(!m.has_destination_secret("cred-1").await.unwrap());
    assert!(m.set_destination_secret("", "x").await.is_err());
    assert!(serde_json::to_value(stores::PROVIDERS).unwrap().as_array().unwrap().len() >= 13);
}
