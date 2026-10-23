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
    jb.schedule = BackupSchedule { enabled: true, start_time: "00:00".into(), interval_minutes: 60, weekdays: vec![], skip_on_battery: false, calendar_ids: vec![] };
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
    assert!(m2.adopt_repository(dest.clone(), Some("wrong".into()), "ufop".into(), None, vec![], vec![], false).await.is_err());
    assert!(m2.list_jobs().await.is_empty(), "failed adoption leaves no job behind");
    let adopted = m2
        .adopt_repository(dest, Some("pw".into()), "ufop".into(), Some("Laptop".into()), vec![], vec![(src.to_string_lossy().into(), moved.to_string_lossy().into())], false)
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

#[tokio::test(flavor = "multi_thread")]
async fn run_logs_notifications_and_redaction() {
    use axum::{routing::post, Json, Router};
    let seen = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
    let s2 = seen.clone();
    let app = Router::new()
        .route("/botTOK/sendMessage", post({ let s = s2.clone(); move |Json(v): Json<serde_json::Value>| { let s = s.clone(); async move { s.lock().unwrap().push(v); Json(serde_json::json!({ "ok": true })) } } }))
        .route("/hook", post(move |Json(v): Json<serde_json::Value>| { let s = s2.clone(); async move { s.lock().unwrap().push(v); Json(serde_json::json!({})) } }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("secret-src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"x").unwrap();
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.encrypted = false;
    jb.sources.push(src.join("missing").to_string_lossy().into()); // → warning mentioning a path
    jb.options.missing_source = ufop_core::backup::MissingSourcePolicy::Warn;
    let log_path = t.path().join("logs/run.log");
    jb.log_outputs = logs::LogOutputs { file: Some(log_path.to_string_lossy().into()), ..Default::default() };
    jb.reports.http_urls = vec![format!("{base}/hook")];
    jb.reports.send_on = vec!["warning".into()];
    jb.reports.redact_paths = true;
    jb.notifications = vec![notify::Notification {
        channel: notify::Channel::Telegram { chat_id: "42".into(), api_base: base.clone() },
        send_on: vec!["warning".into()],
        credential_id: "tg".into(),
        template: String::new(),
        cert: Default::default(),
    }];
    m.set_destination_secret("tg", "TOK").await.unwrap();
    let j = m.save_job(jb, None).await.unwrap();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "warning", "{:?}", r.log);
    let file = std::fs::read_to_string(&log_path).unwrap();
    assert!(file.contains("backup warning") && file.contains("missing"), "{file}");
    let got = seen.lock().unwrap().clone();
    assert_eq!(got.len(), 2, "webhook + telegram");
    let all = serde_json::to_string(&got).unwrap();
    assert!(!all.contains("secret-src"), "paths redacted: {all}");
    assert!(all.contains("[source:"));
}

#[test]
fn progress_rate_and_eta() {
    let p = Progress { phase: ufop_core::backup::Phase::Uploading, files_processed: 25, files_total: 100, bytes_processed: 10_000_000, current_path: None };
    let (rate, eta) = progress_rate(&p, 10.0);
    assert_eq!(rate, 1_000_000);
    assert_eq!(eta, Some(30));
    assert_eq!(progress_rate(&p, 0.1), (0, None), "too early to estimate");
    let done = Progress { files_processed: 100, ..p };
    assert_eq!(progress_rate(&done, 10.0).1, None);
}

#[tokio::test(flavor = "multi_thread")]
async fn bug_report_is_sanitized_and_diagnostics_detect_problems() {
    use std::io::Read;
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("alicesecret"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("budgetzz.txt"), b"numbers").unwrap();
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.reports.http_urls = vec!["https://hooks.example.com/notify?token=tok3n".into()];
    jb.reports.extra_fields.insert("api_key".into(), "k3y".into());
    jb.notifications = vec![notify::Notification {
        channel: notify::Channel::Email { host: "smtp.example.com".into(), port: None, tls: Default::default(), username: "alice@example.com".into(), from: "alice@example.com".into(), to: vec!["bob@example.com".into()], subject: "s".into() },
        send_on: vec!["error".into()],
        credential_id: "c".into(),
        template: String::new(),
        cert: Default::default(),
    }];
    jb.log_outputs.file = Some(t.path().join("job.log").to_string_lossy().into());
    let j = m.save_job(jb, Some("pw".into())).await.unwrap();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);

    let out = t.path().join("reports/bug.zip");
    let rep = m.bug_report(&j.id, out.to_string_lossy().into()).await.unwrap();
    assert!(rep.size > 0 && rep.paths_obfuscated >= 2 && rep.runs_included == 1);
    for f in ["job.json", "runs.json", "database.sqlite", "system.json", "job.log"] {
        assert!(rep.files.iter().any(|x| x == f), "{f} missing from {:?}", rep.files);
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
    let mut all = String::new();
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).unwrap();
        let mut b = Vec::new();
        f.read_to_end(&mut b).unwrap();
        let text = String::from_utf8_lossy(&b).into_owned();
        for leak in ["alicesecret", "budgetzz", "tok3n", "k3y", "alice@example.com", "bob@example.com", t.path().to_str().unwrap()] {
            let at = text.find(leak).map(|p| &text[p.saturating_sub(60)..(p + 60).min(text.len())]);
            assert!(at.is_none(), "{leak} leaked into {}: {at:?}", f.name());
        }
        all.push_str(&text);
    }
    let text = all;
    assert!(text.contains("hooks.example.com") && text.contains("app_version"));
    assert!(m.bug_report(&j.id, " ".into()).await.is_err());

    // Diagnostics: healthy, then a missing volume and an untracked file.
    let d = m.diagnose(&j.id, true, 64).await.unwrap();
    assert!(!d.snapshot_providers.is_empty(), "diagnostics list the snapshot methods");
    let db = d.database.clone().unwrap();
    assert!(db.ok(), "{db:?}");
    assert!(db.tracked_volumes >= 2 && d.network.is_none(), "local destinations have no network endpoint");
    let thr = d.throughput.unwrap();
    assert!(thr.verified && thr.error.is_none() && thr.list_ms.len() == 3 && thr.upload_bps > 0);
    assert!(!std::fs::read_dir(&dst).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("diagnostic")), "test file removed");
    let vol = std::fs::read_dir(&dst).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).find(|n| n.contains("dblock")).unwrap();
    std::fs::remove_file(dst.join(&vol)).unwrap();
    std::fs::write(dst.join("ufop-stray.bin"), b"x").unwrap();
    let db = m.diagnose(&j.id, false, 0).await.unwrap().database.unwrap();
    assert_eq!(db.missing_remote, vec![vol]);
    assert_eq!(db.untracked_remote, vec!["ufop-stray.bin".to_string()]);
    assert!(!db.ok());
}

#[tokio::test]
async fn network_probe_times_dns_tcp_and_tls() {
    // Plain TCP against a local listener.
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    tokio::spawn(async move { while let Ok((_s, _)) = l.accept().await {} });
    let n = diagnostics::probe("localhost", port, false, &notify::CertPolicy::default(), std::time::Duration::from_secs(5)).await;
    assert!(n.error.is_none() && n.dns_ms.is_some() && n.tcp_ms.is_some() && !n.addresses.is_empty(), "{n:?}");
    // TLS against a non-TLS listener fails the handshake but keeps the timings.
    let n = diagnostics::probe("localhost", port, true, &notify::CertPolicy::default(), std::time::Duration::from_secs(2)).await;
    assert!(n.tcp_ms.is_some() && n.tls_ms.is_none() && n.error.is_some());
    let n = diagnostics::probe("no-such-host.invalid", 443, true, &notify::CertPolicy::default(), std::time::Duration::from_secs(5)).await;
    assert!(n.error.unwrap().contains("DNS"));
    // Endpoint mapping.
    let s3 = stores::CloudConfig::S3(stores::s3::S3Config { region: "eu-west-1".into(), ..Default::default() });
    assert_eq!(s3.network_endpoint(), Some(("s3.eu-west-1.amazonaws.com".into(), 443, true)));
    let tahoe = stores::CloudConfig::TahoeLafs(stores::tahoe::TahoeConfig { node_url: "http://127.0.0.1:3456".into(), ..Default::default() });
    assert_eq!(tahoe.network_endpoint(), Some(("127.0.0.1".into(), 3456, false)));
}

#[tokio::test]
async fn test_notification_reports_channel_errors() {
    let t = tempfile::tempdir().unwrap();
    let m = manager(t.path()).await;
    let n = notify::Notification {
        channel: notify::Channel::Telegram { chat_id: "1".into(), api_base: String::new() },
        send_on: vec![],
        credential_id: "tg-missing".into(),
        template: String::new(),
        cert: Default::default(),
    };
    // Sent regardless of `send_on`; a missing bot token is reported, not swallowed.
    let e = m.test_notification(n).await.unwrap_err();
    assert!(e.user_message().contains("bot token is missing"), "{}", e.user_message());
}

fn provider(id: &str, kind: secrets::ProviderKind) -> secrets::SecretProvider {
    secrets::SecretProvider { id: id.into(), name: id.into(), kind, credential_id: format!("cred-{id}"), enabled: true, global: true }
}

#[tokio::test(flavor = "multi_thread")]
async fn placeholders_resolve_from_file_provider_and_are_masked() {
    let t = tempfile::tempdir().unwrap();
    let src = t.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"x").unwrap();
    let sfile = t.path().join("secrets.env");
    std::fs::write(&sfile, "BACKUP_PASS=correct horse\nHOOK_TOKEN=tok-9f8e7d\n").unwrap();
    let m = manager(t.path()).await;
    m.set_secret_settings(secrets::SecretSettings { providers: vec![provider("file", secrets::ProviderKind::File { path: sfile.to_string_lossy().into() })], cache: secrets::CacheMode::Persistent, cache_ttl_secs: 3600 }).await.unwrap();

    let mut jb = job(&src, &t.path().join("dst"));
    // The script echoes the token; the run log must not show it.
    jb.scripts.run_before = Some(if cfg!(windows) { "echo LOG:INFO token ${HOOK_TOKEN}".into() } else { "echo 'LOG:INFO token ${HOOK_TOKEN}'".into() });
    let j = m.save_job(jb, Some("$BACKUP_PASS".into())).await.unwrap();
    assert_eq!(m.passphrase(&j.id).await.unwrap().as_deref(), Some("correct horse"), "stored passphrase placeholder resolves");
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    let log = serde_json::to_string(&r.log).unwrap();
    assert!(log.contains("token ***") && !log.contains("tok-9f8e7d"), "{log}");
    assert_eq!(m.get_job(&j.id).await.unwrap().scripts.run_before.unwrap().matches("${HOOK_TOKEN}").count(), 1, "stored job keeps the placeholder");

    let rep = m.job_placeholders(&j.id).await.unwrap();
    assert_eq!(rep.placeholders.len(), 1);
    assert_eq!(rep.placeholders[0].provider.as_deref(), Some("file"));

    // Provider unreachable: the persistent cache keeps runs working.
    std::fs::remove_file(&sfile).unwrap();
    m.inner.secret_cache.clear();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);

    // Unknown placeholder: the run fails with a clear error, nothing leaks.
    let mut j2 = m.get_job(&j.id).await.unwrap();
    j2.reports.http_urls = vec!["https://example.invalid/${MISSING_ONE}".into()];
    let j2 = m.save_job(j2, None).await.unwrap();
    let e = m.execute(&j2.id, BackupOperation::Backup { dry_run: false }).await.unwrap_err();
    assert!(e.user_message().contains("MISSING_ONE"), "{}", e.user_message());
    assert_eq!(m.run_history(Some(j2.id.clone()), 1).await.unwrap()[0].status, "error");

    let tests = m.test_secret_providers(None, vec!["HOOK_TOKEN".into()]).await.unwrap();
    assert!(!tests[0].ok && !tests[0].errors.is_empty(), "file is gone: {tests:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn gcp_and_azure_secret_providers_against_api_mocks() {
    use axum::{extract::Path as P, routing::{get, post}, Json, Router};
    let app = Router::new()
        .route(
            "/v1/projects/{proj}/secrets/{id}/versions/{ver}",
            get(|P((proj, id, ver)): P<(String, String, String)>| async move {
                use base64::Engine;
                assert_eq!(ver, "latest:access");
                if proj != "demo" || id != "db" {
                    return Err(axum::http::StatusCode::NOT_FOUND);
                }
                Ok(Json(serde_json::json!({ "payload": { "data": base64::engine::general_purpose::STANDARD.encode(r#"{"user":"u1","pass":"gcp-pass"}"#) } })))
            }),
        )
        .route("/google-token", post(|body: String| async move {
            assert!(body.contains("jwt-bearer") && body.contains("assertion="));
            Json(serde_json::json!({ "access_token": "ya29.from-jwt", "expires_in": 3600 }))
        }))
        .route("/tenant-1/oauth2/v2.0/token", post(|body: String| async move {
            assert!(body.contains("client_secret=azsecret") && body.contains("vault.azure.net"));
            Json(serde_json::json!({ "access_token": "az-token" }))
        }))
        .route(
            "/secrets/{name}",
            get(|P(name): P<String>, h: axum::http::HeaderMap| async move {
                assert_eq!(h["authorization"], "Bearer az-token");
                if name == "smtp-pass" { Ok(Json(serde_json::json!({ "value": "az-value" }))) } else { Err(axum::http::StatusCode::NOT_FOUND) }
            }),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

    let t = tempfile::tempdir().unwrap();
    let m = manager(t.path()).await;
    let s = m
        .set_secret_settings(secrets::SecretSettings {
            providers: vec![
                provider("gcp", secrets::ProviderKind::GcpSecretManager { project: "demo".into(), endpoint: base.clone(), tls: Default::default() }),
                provider("az", secrets::ProviderKind::AzureKeyVault { vault_url: base.clone(), tenant_id: "tenant-1".into(), client_id: "app".into(), authority: base.clone(), tls: Default::default() }),
            ],
            cache: secrets::CacheMode::None,
            cache_ttl_secs: 60,
        })
        .await
        .unwrap();
    m.set_destination_secret("cred-gcp", "ya29.access-token").await.unwrap();
    m.set_destination_secret("cred-az", "azsecret").await.unwrap();
    let ps = s.for_job(&[]);
    assert_eq!(m.resolve_secret(&s, &ps, "db#pass").await.unwrap().0, "gcp-pass");
    let (v, by, _) = m.resolve_secret(&s, &ps, "smtp_pass").await.unwrap();
    assert_eq!((v.as_str(), by.as_str()), ("az-value", "az"), "falls through to Azure, _ maps to -");
    assert!(m.resolve_secret(&s, &ps, "nothing").await.unwrap_err().user_message().contains("not found in: gcp, az"));
    let tests = m.test_secret_providers(None, vec![]).await.unwrap();
    assert!(tests.iter().all(|t| t.ok && t.has_credential), "{tests:?}");

    // Service-account key: a signed JWT is exchanged for a token first.
    let key = t.path().join("sa.pem");
    if std::process::Command::new("openssl").args(["genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048", "-out"]).arg(&key).output().is_ok_and(|o| o.status.success()) {
        let sa = serde_json::json!({ "client_email": "ufop@demo.iam", "private_key": std::fs::read_to_string(&key).unwrap(), "token_uri": format!("{base}/google-token") });
        m.set_destination_secret("cred-gcp", &sa.to_string()).await.unwrap();
        assert_eq!(m.resolve_secret(&s, &ps, "db#user").await.unwrap().0, "u1");
    }
}

/// Real services: Vault dev server, moto Secrets Manager, and `pass`.
#[tokio::test(flavor = "multi_thread")]
async fn vault_aws_and_pass_secret_providers_against_real_servers() {
    let t = tempfile::tempdir().unwrap();
    let m = manager(t.path()).await;
    let mut providers = Vec::new();
    let mut checks: Vec<(&str, &str)> = Vec::new();
    if let Ok(v) = std::env::var("UFOP_TEST_VAULT") {
        let (addr, token) = v.split_once('|').unwrap();
        providers.push(provider("vault", secrets::ProviderKind::Vault { address: addr.into(), mount: "secret".into(), path: "ufop".into(), namespace: String::new(), kv_version: 2, tls: Default::default() }));
        providers.push(provider("vault1", secrets::ProviderKind::Vault { address: addr.into(), mount: "kv1".into(), path: "app".into(), namespace: String::new(), kv_version: 1, tls: Default::default() }));
        m.set_destination_secret("cred-vault", token).await.unwrap();
        m.set_destination_secret("cred-vault1", token).await.unwrap();
        checks.extend([("db_pass", "hunter2"), ("token", "t1-legacy")]);
    }
    if let Ok(ep) = std::env::var("UFOP_TEST_S3") {
        providers.push(provider("aws", secrets::ProviderKind::AwsSecretsManager { region: "us-east-1".into(), access_key_id: "AKIDUFOP".into(), endpoint: ep, secret_id: String::new(), tls: Default::default() }));
        m.set_destination_secret("cred-aws", "ufopsecret").await.unwrap();
        checks.extend([("ufop/smtp", "sm-plain-value"), ("ufop/bundle#api_key", "sm-key")]);
    }
    if let Ok(w) = std::env::var("UFOP_TEST_PASS") {
        providers.push(provider("pass", secrets::ProviderKind::Pass { executable: w, prefix: "ufop".into() }));
        checks.push(("smtp", "pass-secret-1"));
    }
    if providers.is_empty() {
        return;
    }
    let s = m.set_secret_settings(secrets::SecretSettings { providers, cache: secrets::CacheMode::Memory, cache_ttl_secs: 60 }).await.unwrap();
    let ps = s.for_job(&[]);
    for (name, want) in checks {
        assert_eq!(m.resolve_secret(&s, &ps, name).await.unwrap().0, want, "{name}");
    }
    assert!(m.resolve_secret(&s, &ps, "definitely-missing").await.is_err());
    let tests = m.test_secret_providers(None, vec![]).await.unwrap();
    assert!(tests.iter().all(|t| t.ok), "{tests:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn gpg_job_kdf_validation_and_file_crypt() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst, out) = (t.path().join("src"), t.path().join("dst"), t.path().join("out"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"through gpg").unwrap();
    let m = manager(t.path()).await;

    let mut weak = job(&src, &dst);
    weak.options.kdf = Some(KdfParams { memory_kib: 64, iterations: 1, parallelism: 1 });
    assert!(m.save_job(weak, Some("pw".into())).await.is_err(), "weak KDF refused");

    if std::process::Command::new("gpg").arg("--version").output().is_ok() {
        let home = t.path().join("gnupg");
        std::fs::create_dir_all(&home).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&home, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let mut jb = job(&src, &dst);
        jb.options.encryption_module = ufop_core::backup::EncryptionModuleKind::Gpg;
        jb.options.gpg.homedir = home.to_string_lossy().into();
        let j = m.save_job(jb, Some("gpg-pw".into())).await.unwrap();
        let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
        assert_eq!(r.status, "success", "{:?}", r.log);
        assert!(std::fs::read_dir(&dst).unwrap().flatten().any(|e| e.file_name().to_string_lossy().ends_with(".dblock.gpg")));
        let rr = m.execute(&j.id, BackupOperation::Restore { options: RestoreOptions { target: Some(out.clone()), ..Default::default() }, overrides: RestoreOverrides::default() }).await.unwrap();
        assert_eq!(rr.report["files_restored"], 1, "{:?}", rr.log);
        // Key mode needs no stored passphrase.
        let mut keyed = job(&src, &t.path().join("dst2"));
        keyed.options.encryption_module = ufop_core::backup::EncryptionModuleKind::Gpg;
        keyed.options.gpg.recipients = vec!["someone@example.invalid".into()];
        assert!(m.save_job(keyed, None).await.is_ok());
    }

    let (plain, enc, dec) = (t.path().join("p.bin"), t.path().join("p.ufopenc"), t.path().join("p.out"));
    std::fs::write(&plain, vec![9u8; 100_000]).unwrap();
    let s = |p: &std::path::Path| p.to_string_lossy().into_owned();
    m.crypt_file(false, s(&plain), s(&enc), "file-pw".into(), Some("interactive".into())).await.unwrap();
    assert!(m.crypt_file(true, s(&enc), s(&dec), "nope".into(), None).await.is_err());
    m.crypt_file(true, s(&enc), s(&dec), "file-pw".into(), None).await.unwrap();
    assert_eq!(std::fs::read(&dec).unwrap(), vec![9u8; 100_000]);
    assert!(m.crypt_file(false, s(&plain), s(&enc), String::new(), None).await.is_err());
}

/// Minimal Photos library (modern layout) built with the real table names.
fn photos_library(dir: &Path) -> PathBuf {
    let lib = dir.join("Photos Library.photoslibrary");
    std::fs::create_dir_all(lib.join("database")).unwrap();
    std::fs::create_dir_all(lib.join("originals/A")).unwrap();
    std::fs::write(lib.join("originals/A/U1.jpeg"), b"beach").unwrap();
    std::fs::write(lib.join("originals/A/U2.jpeg"), b"hidden").unwrap();
    let c = rusqlite::Connection::open(lib.join("database/Photos.sqlite")).unwrap();
    // 2024-06-01T12:00:00Z in Core Data seconds.
    let created = 1_717_243_200 - 978_307_200;
    c.execute_batch(&format!(
        "CREATE TABLE ZASSET (Z_PK INTEGER PRIMARY KEY, ZDIRECTORY TEXT, ZFILENAME TEXT, ZUUID TEXT, ZDATECREATED REAL, ZTRASHEDSTATE INTEGER, ZHIDDEN INTEGER, ZFAVORITE INTEGER, ZKIND INTEGER, ZUNIFORMTYPEIDENTIFIER TEXT);
         CREATE TABLE ZADDITIONALASSETATTRIBUTES (Z_PK INTEGER PRIMARY KEY, ZASSET INTEGER, ZORIGINALFILENAME TEXT, ZTIMEZONEOFFSET INTEGER);
         CREATE TABLE ZGENERICALBUM (Z_PK INTEGER PRIMARY KEY, ZKIND INTEGER, ZTITLE TEXT, ZUUID TEXT, ZTRASHEDSTATE INTEGER);
         CREATE TABLE Z_27ASSETS (Z_27ALBUMS INTEGER, Z_3ASSETS INTEGER, Z_FOK_3ASSETS INTEGER);
         INSERT INTO ZASSET VALUES (1, 'A', 'U1.jpeg', 'U1', {created}, 0, 0, 1, 0, 'public.jpeg'), (2, 'A', 'U2.jpeg', 'U2', {created}, 0, 1, 0, 0, 'public.jpeg');
         INSERT INTO ZADDITIONALASSETATTRIBUTES (ZASSET, ZORIGINALFILENAME, ZTIMEZONEOFFSET) VALUES (1, 'IMG_1000.JPG', 0), (2, 'IMG_1001.JPG', 0);
         INSERT INTO ZGENERICALBUM VALUES (9, 2, 'Holiday', 'ALB9', 0);
         INSERT INTO Z_27ASSETS VALUES (9, 1, 1024);"
    ))
    .unwrap();
    lib
}

#[tokio::test]
async fn photos_library_job_backup_restore_and_commands() {
    let t = tempfile::tempdir().unwrap();
    let (dst, out) = (t.path().join("dst"), t.path().join("out"));
    let lib = photos_library(t.path());
    let m = manager(t.path()).await;
    let source = ufop_core::backup::photos::source_uri(&lib);

    // A destination inside the library and a bare scheme are rejected.
    let mut bad = job(&lib, &lib.join("backups"));
    bad.sources = vec![source.clone()];
    assert!(m.save_job(bad, Some("pw".into())).await.unwrap_err().user_message().contains("inside the source"));
    let mut bare = job(&lib, &dst);
    bare.sources = vec!["photos://".into()];
    assert!(m.save_job(bare, Some("pw".into())).await.unwrap_err().user_message().contains(".photoslibrary"));

    let mut j = job(&lib, &dst);
    j.sources = vec![source.clone()];
    let j = m.save_job(j, Some("pw".into())).await.unwrap();

    let p = api::preview_sources(j.clone()).await.unwrap();
    assert_eq!(p.files, 2, "one visible photo + albums.json: {:?}", p.sample);
    assert!(p.sample.iter().any(|s| s.ends_with("IMG_1000.JPG")));

    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert!(r.summary.contains("2 added"), "{}", r.summary);

    let restore = RestoreOptions { target: Some(out.clone()), ..Default::default() };
    let r = m.execute(&j.id, BackupOperation::Restore { options: restore, overrides: RestoreOverrides::default() }).await.unwrap();
    assert_eq!(r.report["files_restored"], 2);
    let ex = out.join("Photos Library (Photos export)");
    assert_eq!(std::fs::read(ex.join("Photos/2024/06/IMG_1000.JPG")).unwrap(), b"beach");
    let manifest: serde_json::Value = serde_json::from_slice(&std::fs::read(ex.join("albums.json")).unwrap()).unwrap();
    assert_eq!(manifest["albums"][0]["title"], "Holiday");
    assert_eq!(manifest["albums"][0]["assets"][0], "Photos/2024/06/IMG_1000.JPG");
    assert_eq!(manifest["assets"][0]["favorite"], true);

    // Hidden assets on request.
    let mut j2 = m.get_job(&j.id).await.unwrap();
    j2.options.photos.include_hidden = true;
    let j2 = m.save_job(j2, None).await.unwrap();
    let r = m.execute(&j2.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert!(r.summary.contains("1 added"), "{}", r.summary);

    // Library commands through the shared dispatcher.
    let info = api::dispatch(&m, "backup_photos_library_info", &serde_json::json!({ "path": source })).await.unwrap();
    assert_eq!(info["assets"], 2);
    assert_eq!(info["hidden"], 1);
    assert_eq!(info["albums"], serde_json::json!(["Holiday"]));
    let libs = api::dispatch(&m, "backup_photos_libraries", &serde_json::json!({})).await.unwrap();
    if !cfg!(target_os = "macos") {
        assert_eq!(libs, serde_json::json!([]));
    }
    assert!(api::dispatch(&m, "backup_photos_library_info", &serde_json::json!({ "path": t.path().join("none.photoslibrary") })).await.is_err());
}

#[cfg(unix)]
#[tokio::test]
async fn concurrency_cap_runs_jobs_in_parallel_but_never_one_job_twice() {
    let t = tempfile::tempdir().unwrap();
    let m = manager(t.path()).await;
    let mk = |n: &str| {
        let (src, dst) = (t.path().join(format!("src{n}")), t.path().join(format!("dst{n}")));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("f"), n).unwrap();
        let mut jb = job(&src, &dst);
        jb.name = format!("job {n}");
        jb.encrypted = false;
        jb.scripts.run_before = Some("sleep 0.6".into());
        jb
    };
    let a = m.save_job(mk("a"), None).await.unwrap();
    let b = m.save_job(mk("b"), None).await.unwrap();
    let both = |x: &str, y: &str| {
        let (m1, m2, x, y) = (m.clone(), m.clone(), x.to_string(), y.to_string());
        async move {
            let t0 = std::time::Instant::now();
            let (r1, r2) = tokio::join!(m1.execute(&x, BackupOperation::Backup { dry_run: false }), m2.execute(&y, BackupOperation::Backup { dry_run: false }));
            r1.unwrap();
            r2.unwrap();
            t0.elapsed().as_secs_f64()
        }
    };

    // Default: one operation at a time.
    assert_eq!(m.effective_caps().await.concurrency(), 1);
    assert!(both(&a.id, &b.id).await >= 1.15);

    let mut d = m.global_defaults().await.unwrap();
    d.caps = defaults::TransferCaps { upload_bps: 0, download_bps: 0, max_concurrent_runs: 2 };
    m.set_global_defaults(d.clone()).await.unwrap();
    assert!(both(&a.id, &b.id).await < 1.15, "two jobs run side by side");
    assert!(both(&a.id, &a.id).await >= 1.15, "the same job never runs twice at once");

    // The console's cap wins over the machine's; clearing it restores the machine's.
    m.set_managed_caps(Some(ufop_core::agent::ManagedTransfers { max_concurrent_runs: Some(1), max_upload_kbps: Some(64), max_download_kbps: None })).await;
    let caps = m.effective_caps().await;
    assert_eq!((caps.concurrency(), caps.upload_bps), (1, 64 * 1024));
    assert_eq!(ufop_core::backup::backend::GLOBAL_UPLOAD.get(), 64 * 1024);
    m.set_managed_caps(None).await;
    assert_eq!(m.effective_caps().await.concurrency(), 2);
    assert_eq!(ufop_core::backup::backend::GLOBAL_UPLOAD.get(), 0);

    d.caps.max_concurrent_runs = 9;
    assert!(m.set_global_defaults(d).await.is_err());
}

#[tokio::test]
async fn legal_hold_suspends_retention_blocks_removal_and_feeds_ediscovery() {
    use crate::governance::ediscovery::{export, EDiscoveryRequest, ManifestEntry};
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(src.join("legal")).unwrap();
    std::fs::write(src.join("legal/contract.txt"), b"v1").unwrap();
    std::fs::write(src.join("other.txt"), b"x").unwrap();
    let repo = Repository::open_in_memory().await.unwrap();
    let ledger = OperationLedger::new(repo.pool().clone());
    let m = BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, Some(ledger.clone()), t.path().join("dbs"), KdfParams::insecure_fast());
    let audit = crate::security::audit::AuditTrail::new(repo.pool().clone(), None, None);
    let mut j = job(&src, &dst);
    j.retention = RetentionPolicy::KeepCount { count: 1 };
    let j = m.save_job(j, Some("pw".into())).await.unwrap();

    assert!(m.place_legal_hold(&j.id, "  ", "", "user").await.is_err());
    let held = m.place_legal_hold(&j.id, "Case 2026-17", "Litigation", "user").await.unwrap();
    assert_eq!(held.legal_hold.as_ref().unwrap().matter, "Case 2026-17");

    // Retention (keep 1) is suspended: both versions stay.
    for body in [b"v22".as_slice(), b"v333".as_slice()] {
        std::fs::write(src.join("legal/contract.txt"), body).unwrap();
        let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
        assert_eq!(r.status, "success", "{:?}", r.log);
    }
    assert_eq!(m.list_versions(&j.id).await.unwrap().len(), 2);

    // Removal is refused; a dry run is not.
    assert!(m.execute(&j.id, BackupOperation::DeleteVersions { versions: vec![1], allow_full_removal: false, force: false }).await.is_err());
    assert!(m.execute(&j.id, BackupOperation::Purge { paths: vec!["/x".into()], versions: None, dry_run: false }).await.is_err());
    assert!(m.execute(&j.id, BackupOperation::Purge { paths: vec!["/x".into()], versions: None, dry_run: true }).await.is_ok());
    assert!(m.delete_job(&j.id, true, false).await.is_err());

    // The job editor keeps the hold (it cannot be cleared by saving).
    let mut edited = m.get_job(&j.id).await.unwrap();
    edited.legal_hold = None;
    edited.name = "Docs renamed".into();
    assert!(m.save_job(edited, None).await.unwrap().legal_hold.is_some());
    // …and it survives a reload from the database.
    m.inner.jobs.write().await.clear();
    m.load_from_db().await.unwrap();
    assert!(m.get_job(&j.id).await.unwrap().legal_hold.is_some());

    // eDiscovery export of the held job, scoped to the legal folder, with files.
    let out = t.path().join("matter.zip");
    let legal = src.join("legal").to_string_lossy().into_owned();
    let req = EDiscoveryRequest { matter: "Case 2026-17".into(), output: out.to_string_lossy().into(), paths: vec![legal.clone()], include_files: true, ..Default::default() };
    assert!(export(&EDiscoveryRequest { output: t.path().join("x.tar").to_string_lossy().into(), ..req.clone() }, &m, &ledger, &audit, "user").await.is_err());
    let rep = export(&req, &m, &ledger, &audit, "user").await.unwrap();
    assert_eq!(rep.jobs, vec!["Docs renamed".to_string()]);
    assert_eq!(rep.restored_files, 1);
    assert!(rep.audit_ok && !rep.files_truncated);
    assert!(!t.path().join("matter.zip.part").exists());
    assert_eq!(rep.sha256, hex::encode(Sha256::digest(std::fs::read(&out).unwrap())));
    // Exporting over an existing file is refused.
    assert!(export(&req, &m, &ledger, &audit, "user").await.is_err());

    let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
    assert_eq!(zip.len(), rep.entries);
    let mut manifest = String::new();
    zip.by_name("manifest.json").unwrap().read_to_string(&mut manifest).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    assert_eq!(manifest["format"], "ufop-ediscovery/1");
    assert_eq!(manifest["matter"], "Case 2026-17");
    let entries: Vec<ManifestEntry> = serde_json::from_value(manifest["entries"].clone()).unwrap();
    assert_eq!(entries.len() + 1, zip.len());
    for e in &entries {
        let mut data = Vec::new();
        zip.by_name(&e.path).unwrap().read_to_end(&mut data).unwrap();
        assert_eq!((data.len() as u64, hex::encode(Sha256::digest(&data))), (e.bytes, e.sha256.clone()), "{}", e.path);
    }
    let names: Vec<&str> = entries.iter().map(|e| e.path.as_str()).collect();
    for want in ["audit/audit-log.jsonl", "audit/verification.json", "ledger/events.jsonl", "README.txt"] {
        assert!(names.contains(&want), "{want} missing from {names:?}");
    }
    let restored: Vec<&&str> = names.iter().filter(|n| n.starts_with("files/")).collect();
    assert_eq!(restored.len(), 1);
    assert!(restored[0].ends_with("legal/contract.txt"), "{restored:?}");
    let mut body = String::new();
    zip.by_name(restored[0]).unwrap().read_to_string(&mut body).unwrap();
    assert_eq!(body, "v333");
    // Only files under the requested paths are listed.
    let mut listed = String::new();
    zip.by_name(names.iter().find(|n| n.ends_with("files-v0.jsonl")).unwrap()).unwrap().read_to_string(&mut listed).unwrap();
    assert!(listed.contains("contract.txt") && !listed.contains("other.txt"));
    let hold: serde_json::Value = {
        let mut s = String::new();
        zip.by_name(names.iter().find(|n| n.ends_with("legal-hold.json")).unwrap()).unwrap().read_to_string(&mut s).unwrap();
        serde_json::from_str(&s).unwrap()
    };
    assert_eq!(hold["reason"], "Litigation");
    drop(zip);

    // Release: retention applies again and the job can be deleted.
    let (released, hold) = m.release_legal_hold(&j.id).await.unwrap();
    assert!(released.legal_hold.is_none() && hold.matter == "Case 2026-17");
    assert!(m.release_legal_hold(&j.id).await.is_err());
    std::fs::write(src.join("legal/contract.txt"), b"v4444").unwrap();
    m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(m.list_versions(&j.id).await.unwrap().len(), 1);
    m.delete_job(&j.id, true, false).await.unwrap();
}

#[tokio::test]
async fn holiday_calendar_skips_a_scheduled_backup_slot() {
    use crate::schedule_library::registry;
    use ufop_core::calendar::HolidayCalendar;
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    let m = manager(t.path()).await;
    let cal = HolidayCalendar { id: format!("bk-{}", uuid::Uuid::new_v4().simple()), name: "Closed today".into(), dates: vec![chrono::Local::now().date_naive()], ..Default::default() };
    let mut jb = job(&src, &dst);
    jb.encrypted = false;
    jb.schedule = BackupSchedule { enabled: true, start_time: "00:00".into(), interval_minutes: 60, calendar_ids: vec![cal.id.clone()], ..Default::default() };
    assert!(m.save_job(jb.clone(), None).await.unwrap_err().to_string().contains("does not exist"), "unknown calendar");
    registry().insert_for_tests(None, Some(cal));
    let j = m.save_job(jb, None).await.unwrap();
    // Make a slot overdue.
    m.inner.jobs.write().await.get_mut(&j.id).unwrap().stats.last_scheduled_at = Some((Utc::now() - chrono::Duration::hours(3)).to_rfc3339());
    assert_eq!(m.due_jobs(Utc::now()).await, vec![j.id.clone()]);
    m.tick().await;
    // The slot was used up without running anything.
    assert!(!m.is_job_busy(&j.id));
    assert!(m.run_history(Some(j.id.clone()), 10).await.unwrap().is_empty());
    assert!(m.due_jobs(Utc::now()).await.is_empty());
}

#[tokio::test]
async fn format_upgrade_rollback_reshard_and_passphrase_change() {
    use ufop_core::backup::layout::RepoLayout;
    let t = tempfile::tempdir().unwrap();
    let (src, dst, out) = (t.path().join("src"), t.path().join("dst"), t.path().join("out"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"alpha").unwrap();
    let m = manager(t.path()).await;
    // A format 1 job (as created by older versions).
    let mut j = job(&src, &dst);
    j.options.format_version = Some(1);
    let j = m.save_job(j, Some("pw".into())).await.unwrap();
    assert_eq!(m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");
    let info = m.execute(&j.id, BackupOperation::FormatInfo).await.unwrap();
    assert_eq!(info.report["format_version"], 1);
    // v1 cannot change the passphrase in place.
    assert!(m.change_passphrase(&j.id, None, "new-pw".into(), None).await.unwrap_err().user_message().contains("format 2"));

    let up = m.execute(&j.id, BackupOperation::UpgradeFormat { rollback: false }).await.unwrap();
    assert_eq!(up.status, "success", "{:?}", up.log);
    assert_eq!(up.report["to_version"], 2);
    assert_eq!(m.execute(&j.id, BackupOperation::FormatInfo).await.unwrap().report["rollback_available"], true);
    let rb = m.execute(&j.id, BackupOperation::UpgradeFormat { rollback: true }).await.unwrap();
    assert_eq!(rb.status, "success", "{:?}", rb.log);
    assert_eq!(m.execute(&j.id, BackupOperation::FormatInfo).await.unwrap().report["format_version"], 1);
    m.execute(&j.id, BackupOperation::UpgradeFormat { rollback: false }).await.unwrap();

    // Reshard, then a new backup and restore through the sharded layout.
    let rs = m.execute(&j.id, BackupOperation::Reshard { layout: RepoLayout::Sharded { prefix_len: 1 }, dry_run: false }).await.unwrap();
    assert_eq!(rs.status, "success", "{:?}", rs.log);
    assert!(rs.summary.contains("moved"), "{}", rs.summary);
    std::fs::write(src.join("b.txt"), b"beta").unwrap();
    assert_eq!(m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");
    let e = m.execute(&j.id, BackupOperation::UpgradeFormat { rollback: true }).await.unwrap_err();
    assert!(e.user_message().contains("Cannot roll back"), "rollback refused after a format 2 write");

    // Password change: header only, stored passphrase updated, old one refused.
    assert!(m.change_passphrase(&j.id, Some("wrong".into()), "new-pw".into(), None).await.is_err());
    let r = m.change_passphrase(&j.id, None, "new-pw".into(), None).await.unwrap();
    assert_eq!(r.to_version, 2);
    assert_eq!(m.passphrase(&j.id).await.unwrap().as_deref(), Some("new-pw"));
    let restore = RestoreOptions { target: Some(out.clone()), ..Default::default() };
    let r = m.execute(&j.id, BackupOperation::Restore { options: restore, overrides: RestoreOverrides::default() }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert_eq!(std::fs::read(out.join("src/b.txt")).unwrap(), b"beta");
    assert!(m.browse_remote(j.destination.clone(), Some("pw".into()), "ufop".into()).await.is_err());
    assert_eq!(m.browse_remote(j.destination.clone(), Some("new-pw".into()), "ufop".into()).await.unwrap().len(), 2);
    // Operation JSON shape used by the frontend.
    let op: BackupOperation = serde_json::from_value(serde_json::json!({"type": "reshard", "layout": {"type": "flat"}, "dry_run": true})).unwrap();
    assert_eq!(op.name(), "reshard");
    let op: BackupOperation = serde_json::from_value(serde_json::json!({"type": "upgrade_format", "rollback": true})).unwrap();
    assert_eq!(op.name(), "rollback_format");
}

#[tokio::test(flavor = "multi_thread")]
async fn repository_lock_blocks_other_writers_and_can_be_broken() {
    use ufop_core::backup::lock::{LockKind, LockOptions, RepoLock};
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"alpha").unwrap();
    let m = manager(t.path()).await;
    let j = m.save_job(job(&src, &dst), Some("pw".into())).await.unwrap();
    assert_eq!(m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");
    assert!(m.lock_status(&j.id).await.unwrap().locks.is_empty(), "released after the run");

    // Another computer is compacting the same repository.
    let other = LocalBackend::new(&dst);
    let held = RepoLock::acquire(&other, "ufop", LockKind::Exclusive, "compact", &LockOptions::default()).unwrap();
    let st = m.lock_status(&j.id).await.unwrap();
    assert!(st.exclusive && st.summary().unwrap().contains("(compact)"));
    let e = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap_err();
    assert!(e.user_message().contains("Repository in use by vm (compact)") || e.user_message().contains("Repository in use by"), "{e}");
    // Readers wait for writers too.
    let restore = RestoreOptions { target: Some(t.path().join("out")), ..Default::default() };
    assert!(m.execute(&j.id, BackupOperation::Restore { options: restore.clone(), overrides: RestoreOverrides::default() }).await.is_err());
    // Format information needs no lock.
    assert_eq!(m.execute(&j.id, BackupOperation::FormatInfo).await.unwrap().status, "success");

    // Breaking it needs confirmation and the lock that was shown.
    assert!(m.break_lock(&j.id, false, None).await.is_err());
    assert!(m.break_lock(&j.id, true, Some("not-the-owner".into())).await.is_err());
    let b = m.break_lock(&j.id, true, Some(held.info().owner_id)).await.unwrap();
    assert_eq!(b.removed, ["ufop-lock.json"]);
    drop(held);
    assert_eq!(m.execute(&j.id, BackupOperation::Restore { options: restore, overrides: RestoreOverrides::default() }).await.unwrap().status, "success");
}

#[tokio::test(flavor = "multi_thread")]
async fn read_only_jobs_restore_and_browse_but_never_write() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"alpha").unwrap();
    let m = manager(t.path()).await;
    let j = m.save_job(job(&src, &dst), Some("pw".into())).await.unwrap();
    assert_eq!(m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");
    let listing = |d: &Path| -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        v.sort();
        v
    };
    let before = listing(&dst);

    // Adopted on another computer as a read-only connection.
    let t2 = tempfile::tempdir().unwrap();
    let m2 = manager(t2.path()).await;
    let ro = m2.adopt_repository(j.destination.clone(), Some("pw".into()), "ufop".into(), Some("Viewer".into()), vec![src.to_string_lossy().into()], vec![], true).await.unwrap();
    assert!(ro.read_only);
    assert_eq!(m2.list_versions(&ro.id).await.unwrap().len(), 1);
    for op in [
        BackupOperation::Backup { dry_run: false },
        BackupOperation::Compact,
        BackupOperation::Repair,
        BackupOperation::DeleteVersions { versions: vec![0], allow_full_removal: true, force: false },
        BackupOperation::Purge { paths: vec!["*".into()], versions: None, dry_run: false },
        BackupOperation::UpgradeFormat { rollback: false },
        BackupOperation::Reshard { layout: ufop_core::backup::layout::RepoLayout::Sharded { prefix_len: 1 }, dry_run: false },
    ] {
        let name = op.name();
        let e = m2.execute(&ro.id, op).await.unwrap_err();
        assert!(e.user_message().contains("read-only"), "{name}: {e}");
    }
    assert!(m2.change_passphrase(&ro.id, Some("pw".into()), "new".into(), None).await.unwrap_err().user_message().contains("read-only"));
    assert!(m2.break_lock(&ro.id, true, None).await.is_err());
    assert!(m2.delete_job(&ro.id, false, true).await.is_err());
    // Reading works, and leaves no trace at the destination.
    let out = t.path().join("out");
    let r = m2.execute(&ro.id, BackupOperation::Restore { options: RestoreOptions { target: Some(out.clone()), ..Default::default() }, overrides: RestoreOverrides::default() }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.summary);
    assert_eq!(std::fs::read(out.join("src/a.txt")).unwrap(), b"alpha");
    assert_eq!(m2.execute(&ro.id, BackupOperation::Verify { full: true }).await.unwrap().status, "success");
    assert_eq!(m2.execute(&ro.id, BackupOperation::Backup { dry_run: true }).await.unwrap().status, "success", "a dry run writes nothing");
    assert!(m2.lock_status(&ro.id).await.unwrap().locks.is_empty());
    assert_eq!(listing(&dst), before, "nothing written, not even a lock");
    // Never scheduled.
    let mut sched = m2.get_job(&ro.id).await.unwrap();
    sched.schedule = BackupSchedule { enabled: true, start_time: "00:00".into(), interval_minutes: 60, weekdays: vec![], skip_on_battery: false, calendar_ids: vec![] };
    m2.save_job(sched, None).await.unwrap();
    assert!(m2.due_jobs(Utc::now() + chrono::Duration::hours(3)).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn local_database_encryption_default_migration_and_lost_key() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("zzPrivateName.txt"), b"alpha").unwrap();
    let m = manager(t.path()).await;
    // New encrypted job: encrypted database by default.
    let j = m.save_job(job(&src, &dst), Some("pw".into())).await.unwrap();
    assert_eq!(j.encrypt_local_database, Some(true));
    assert_eq!(m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");
    let path = m.job_db_path(&j.id);
    assert!(BackupDb::inspect(&path).unwrap().encrypted);
    assert!(!String::from_utf8_lossy(&std::fs::read(&path).unwrap()).contains("zzPrivateName"));
    assert!(m.search(&j.id, "zzPrivate".into(), None).await.unwrap().len() == 1);

    // One-click switch off and on again keeps the history.
    assert!(!m.set_database_encryption(&j.id, false).await.unwrap().encrypted);
    assert!(!BackupDb::inspect(&path).unwrap().encrypted);
    assert_eq!(m.list_versions(&j.id).await.unwrap().len(), 1);
    assert!(m.set_database_encryption(&j.id, true).await.unwrap().encrypted);
    assert!(BackupDb::inspect(&path).unwrap().encrypted);

    // The key disappears from the keychain: the database is rebuilt from the destination.
    let key_name = format!("{DB_KEY_PREFIX}{}", j.id);
    m.inner.creds.delete(&key_name).await.unwrap();
    assert_eq!(m.list_versions(&j.id).await.unwrap().len(), 1, "rebuilt automatically");
    assert!(BackupDb::inspect(&path).unwrap().encrypted, "with a new key");
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_ne!(r.status, "error", "{:?}", r.summary);

    // Jobs from before the option keep a plain database until asked.
    let mut old = job(&src, &t.path().join("dst2"));
    old.encrypted = false;
    let old = m.save_job(old, None).await.unwrap();
    assert_eq!(old.encrypt_local_database, Some(false));
    let mut legacy = m.get_job(&old.id).await.unwrap();
    legacy.encrypt_local_database = None;
    m.inner.jobs.write().await.insert(legacy.id.clone(), legacy.clone());
    assert_eq!(m.execute(&old.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");
    assert!(!BackupDb::inspect(&m.job_db_path(&old.id)).unwrap().encrypted);
}

#[tokio::test(flavor = "multi_thread")]
async fn version_notes_pins_retention_preview_and_delete() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(&src).unwrap();
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.options.default_note = Some("scheduled".into());
    jb.options.default_tags.insert("env".into(), "prod".into());
    let j = m.save_job(jb, Some("pw".into())).await.unwrap();
    for i in 0..3u8 {
        std::fs::write(src.join("f.txt"), [i; 20]).unwrap();
        assert_eq!(m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");
        std::thread::sleep(std::time::Duration::from_millis(1100));
    }
    let v = m.list_versions(&j.id).await.unwrap();
    assert_eq!(v.len(), 3);
    assert_eq!(v[0].note.as_deref(), Some("scheduled"));
    assert_eq!(v[0].identity.as_ref().map(|i| i.job_id.clone()), Some(j.id.clone()), "the job id is the identity");
    assert!(v[0].stats.unwrap().bytes_added > 0);
    assert_eq!(m.list_versions_in(&j.id, None, vec!["env=prod".into()]).await.unwrap().len(), 3);
    assert!(m.list_versions_in(&j.id, None, vec!["env=dev".into()]).await.unwrap().is_empty());

    // Pin the oldest with a note; retention keeps it.
    let patch = ufop_core::backup::versions::MetadataPatch { pinned: Some(true), note: Some("year end".into()), ..Default::default() };
    let r = m.execute(&j.id, BackupOperation::SetVersionMetadata { version: 2, source: None, patch }).await.unwrap();
    assert_eq!(r.report["pinned"], true, "{:?}", r.log);
    let p = m.preview_retention(&j.id, Some(RetentionPolicy::KeepCount { count: 1 }), None).await.unwrap();
    assert_eq!((p.keep, p.delete), (2, 1));
    assert_eq!(p.versions[2].summary, "kept: pinned");
    assert_eq!(m.list_versions(&j.id).await.unwrap().len(), 3, "a preview changes nothing");
    let r = m.execute(&j.id, BackupOperation::ApplyRetention { policy: Some(RetentionPolicy::KeepCount { count: 1 }), source: None }).await.unwrap();
    assert_eq!(r.report["versions_deleted"], 1);
    let v = m.list_versions(&j.id).await.unwrap();
    assert!(v[1].pinned && v[1].note.as_deref() == Some("year end"));
    // Deleting the pinned version needs force.
    assert!(m.execute(&j.id, BackupOperation::DeleteVersions { versions: vec![1], allow_full_removal: false, force: false }).await.is_err());
    let r = m.execute(&j.id, BackupOperation::DeleteVersions { versions: vec![1], allow_full_removal: false, force: true }).await.unwrap();
    assert_eq!(r.report["versions_deleted"], 1);
    // A legal hold keeps everything in the preview and refuses retention.
    m.place_legal_hold(&j.id, "Case 1", "", "test").await.unwrap();
    assert!(m.preview_retention(&j.id, None, None).await.unwrap().legal_hold);
    assert!(m.execute(&j.id, BackupOperation::ApplyRetention { policy: None, source: None }).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn copy_versions_to_another_job_and_a_new_destination() {
    let t = tempfile::tempdir().unwrap();
    let (src, d1, d2, d3, out) = (t.path().join("src"), t.path().join("d1"), t.path().join("d2"), t.path().join("d3"), t.path().join("out"));
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.bin"), (0..50_000u32).map(|i| (i % 249) as u8).collect::<Vec<_>>()).unwrap();
    let m = manager(t.path()).await;
    let a = m.save_job(job(&src, &d1), Some("pw".into())).await.unwrap();
    let mut jb = job(&src, &d2);
    jb.name = "Archive".into();
    jb.options.format_version = Some(1);
    let b = m.save_job(jb, Some("pw2".into())).await.unwrap();
    assert_eq!(m.execute(&a.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");
    let patch = ufop_core::backup::versions::MetadataPatch { pinned: Some(true), ..Default::default() };
    m.execute(&a.id, BackupOperation::SetVersionMetadata { version: 0, source: None, patch }).await.unwrap();

    let op = |target: versions::CopyTarget, dry_run| BackupOperation::CopyVersions { versions: vec![0], source: None, target, dry_run };
    let dry = m.execute(&a.id, op(versions::CopyTarget::Job { job_id: b.id.clone() }, true)).await.unwrap();
    assert_eq!(dry.report["versions_copied"], 1, "{:?}", dry.log);
    assert!(m.list_versions(&b.id).await.unwrap().is_empty());
    let r = m.execute(&a.id, op(versions::CopyTarget::Job { job_id: b.id.clone() }, false)).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert_eq!(r.report["rechunked"], true, "format 2 CDC into format 1 fixed blocks");
    let copied = m.list_versions(&b.id).await.unwrap();
    assert_eq!(copied.len(), 1);
    assert!(copied[0].pinned, "pin copied");
    assert_eq!(m.get_job(&b.id).await.unwrap().stats.version_count, 1);
    let restore = RestoreOptions { target: Some(out.clone()), ..Default::default() };
    let r = m.execute(&b.id, BackupOperation::Restore { options: restore, overrides: RestoreOverrides::default() }).await.unwrap();
    assert_eq!(r.report["files_restored"], 1, "{:?}", r.log);
    assert!(m.execute(&a.id, op(versions::CopyTarget::Job { job_id: a.id.clone() }, false)).await.is_err(), "not onto itself");

    // A destination without a job, unencrypted.
    let dest = BackupDestination::Local { path: d3.to_string_lossy().into(), alternate_paths: vec![], marker_file: None };
    let r = m.execute(&a.id, op(versions::CopyTarget::Destination { destination: dest.clone(), passphrase: None, prefix: None, options: None }, false)).await.unwrap();
    assert_eq!(r.report["versions_copied"], 1, "{:?}", r.log);
    let again = m.execute(&a.id, op(versions::CopyTarget::Destination { destination: dest, passphrase: None, prefix: None, options: None }, false)).await.unwrap();
    assert_eq!((again.report["versions_copied"].as_u64(), again.report["versions_skipped"].as_u64()), (Some(0), Some(1)));
}

#[tokio::test(flavor = "multi_thread")]
async fn shared_repository_between_jobs_and_adoption_as_a_new_source() {
    let t = tempfile::tempdir().unwrap();
    let (sa, sb, dst) = (t.path().join("a"), t.path().join("b"), t.path().join("dst"));
    std::fs::create_dir_all(&sa).unwrap();
    std::fs::create_dir_all(&sb).unwrap();
    let data: Vec<u8> = (0..60_000u32).map(|i| (i.wrapping_mul(2654435761) >> 11) as u8).collect();
    std::fs::write(sa.join("same.bin"), &data).unwrap();
    std::fs::write(sb.join("same.bin"), &data).unwrap();
    let m = manager(t.path()).await;
    let a = m.save_job(job(&sa, &dst), Some("pw".into())).await.unwrap();
    assert_eq!(m.execute(&a.id, BackupOperation::Backup { dry_run: false }).await.unwrap().status, "success");
    let r = m.execute(&a.id, BackupOperation::EnableSharing { claim_all: true }).await.unwrap();
    assert_eq!(r.report["versions_assigned"], 0, "the job's versions already carry its identity");
    let mut jb = job(&sb, &dst);
    jb.name = "Other".into();
    let b = m.save_job(jb, Some("pw".into())).await.unwrap();
    let r = m.execute(&b.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert_eq!(r.report["new_blocks"], 0, "deduplicated against job A");
    assert_eq!(m.list_versions(&b.id).await.unwrap().len(), 1);
    assert_eq!(m.list_versions_in(&b.id, Some("*".into()), vec![]).await.unwrap().len(), 2);
    let r = m.execute(&a.id, BackupOperation::SyncSources).await.unwrap();
    assert_eq!(r.report["shared"], true);
    assert_eq!(r.report["sources"].as_array().unwrap().len(), 2);
    let srcs = m.list_sources(&a.id).await.unwrap();
    assert!(srcs[0].own && srcs[0].label.ends_with("(Docs)"));
    // Retention on A never touches B's version.
    let r = m.execute(&a.id, BackupOperation::ApplyRetention { policy: Some(RetentionPolicy::KeepCount { count: 1 }), source: None }).await.unwrap();
    assert_eq!(r.report["versions_deleted"], 0);
    assert_eq!(m.list_versions(&b.id).await.unwrap().len(), 1);

    // Another computer adopting the shared repository becomes a new source.
    let t2 = tempfile::tempdir().unwrap();
    let m2 = manager(t2.path()).await;
    let dest = BackupDestination::Local { path: dst.to_string_lossy().into(), alternate_paths: vec![], marker_file: None };
    let adopted = m2.adopt_repository(dest, Some("pw".into()), "ufop".into(), Some("Laptop".into()), vec![sa.to_string_lossy().into()], vec![], false).await.unwrap();
    assert_eq!(adopted.stats.version_count, 0, "no other job's history taken over");
    assert_eq!(m2.list_sources(&adopted.id).await.unwrap().len(), 2);
}

async fn wait_idle(m: &BackupManager, ids: &[&str]) {
    for _ in 0..600 {
        if ids.iter().all(|id| !m.is_job_busy(id)) {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("jobs did not finish");
}

#[tokio::test]
async fn run_all_queues_writable_jobs_and_estimate_reports_new_data() {
    let t = tempfile::tempdir().unwrap();
    let m = manager(t.path()).await;
    let mk = |n: &str| {
        let (src, dst) = (t.path().join(format!("src{n}")), t.path().join(format!("dst{n}")));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("f.txt"), format!("file of {n}")).unwrap();
        let mut jb = job(&src, &dst);
        jb.name = format!("job {n}");
        jb.encrypted = false;
        jb
    };
    let a = m.save_job(mk("a"), None).await.unwrap();
    let b = m.save_job(mk("b"), None).await.unwrap();
    let mut ro = mk("c");
    ro.read_only = true;
    let c = m.save_job(ro, None).await.unwrap();

    let r = m.run_all(runs::RunAllFilter::default()).await.unwrap();
    let started: Vec<&str> = r.started.iter().map(|s| s.job_id.as_str()).collect();
    assert_eq!(started.len(), 2, "{r:?}");
    assert!(started.contains(&a.id.as_str()) && started.contains(&b.id.as_str()));
    assert_eq!(r.skipped.len(), 1);
    assert_eq!(r.skipped[0].job_id, c.id);
    assert!(r.skipped[0].reason.contains("read-only"));
    assert!(crate::tray::run_all_summary(&r).contains("2 backup(s) queued"));
    wait_idle(&m, &[&a.id, &b.id]).await;
    for id in [&a.id, &b.id] {
        let h = m.run_history(Some(id.clone()), 5).await.unwrap();
        assert_eq!(h.first().map(|r| r.status.as_str()), Some("success"), "{h:?}");
    }
    // A filter: only scheduled jobs (none are), or named ones.
    let r = m.run_all(runs::RunAllFilter { scheduled_only: true, ..Default::default() }).await.unwrap();
    assert!(r.started.is_empty() && r.skipped.iter().filter(|s| s.reason == "no schedule").count() == 2, "{r:?}");
    let r = m.run_all(runs::RunAllFilter { job_ids: vec![b.id.clone(), "nope".into()], dry_run: true, ..Default::default() }).await.unwrap();
    assert_eq!(r.started.len(), 1);
    assert!(r.skipped.iter().any(|s| s.job_id == "nope"));
    wait_idle(&m, &[&b.id]).await;

    // Estimate: one new file since the last backup.
    std::fs::write(t.path().join("srca/new.bin"), vec![7u8; 10_000]).unwrap();
    let e = m.estimate(&a.id, Some(1000)).await.unwrap();
    assert!(e.has_previous_version);
    assert_eq!((e.new_files, e.unchanged_files, e.new_bytes_at_most), (1, 1, 10_000), "{e:?}");
    assert!(e.largest[0].path.ends_with("new.bin"));
    assert!(e.estimated_duration_secs.is_some());
    assert!(e.speed_source == "history" || e.speed_source == "assumed");
}

#[cfg(unix)]
#[tokio::test]
async fn stdin_command_output_is_backed_up_and_a_failing_command_fails_the_run() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst, out) = (t.path().join("src"), t.path().join("dst"), t.path().join("out"));
    std::fs::create_dir_all(&src).unwrap();
    let m = manager(t.path()).await;
    let mut jb = job(&src, &dst);
    jb.encrypted = false;
    jb.sources.clear();
    jb.stdin_command = Some(hooks::StdinCommand { command: "sh -c 'printf \"dump of %s\" \"$UFOP_BACKUP_NAME\"'".into(), path: "db.sql".into(), timeout_secs: 30 });
    let j = m.save_job(jb, None).await.unwrap();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    let files = m.list_files(&j.id, 0, None).await.unwrap();
    let f = files.iter().find(|f| f.path == "/stdin/db.sql").unwrap();
    assert_eq!(f.size, "dump of Docs".len() as u64);
    let restore = RestoreOptions { target: Some(out.clone()), ..Default::default() };
    let r = m.execute(&j.id, BackupOperation::Restore { options: restore, overrides: RestoreOverrides::default() }).await.unwrap();
    assert_eq!(r.report["files_restored"], 1, "{:?}", r.log);
    let restored = std::fs::read_dir(&out).unwrap().flatten().map(|e| e.path()).find(|p| p.ends_with("db.sql")).unwrap_or_else(|| out.join("stdin/db.sql"));
    assert_eq!(std::fs::read_to_string(restored).unwrap(), "dump of Docs");

    let mut jb = m.get_job(&j.id).await.unwrap();
    jb.stdin_command = Some(hooks::StdinCommand { command: "sh -c 'printf partial; echo boom >&2; exit 3'".into(), path: "db.sql".into(), timeout_secs: 0 });
    m.save_job(jb.clone(), None).await.unwrap();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "error", "{:?}", r.log);
    assert!(r.log.iter().any(|l| l.message.contains("status 3") && l.message.contains("boom")), "{:?}", r.log);

    // A command that outlives its timeout is killed.
    jb.stdin_command = Some(hooks::StdinCommand { command: "sleep 30".into(), path: "slow".into(), timeout_secs: 1 });
    m.save_job(jb.clone(), None).await.unwrap();
    let t0 = std::time::Instant::now();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert!(t0.elapsed().as_secs() < 20);
    assert!(r.log.iter().any(|l| l.message.contains("timed out")), "{:?}", r.log);
    // Without sources the job needs the command; a broken command line is refused.
    jb.stdin_command = Some(hooks::StdinCommand { command: "sh -c 'unclosed".into(), ..Default::default() });
    assert!(m.save_job(jb.clone(), None).await.is_err());
    jb.stdin_command = None;
    assert!(m.save_job(jb, None).await.is_err());
}
