//! Manager tests for WP6: policy templates, effective settings, cron and
//! maintenance schedules, verification options and async hooks.

use super::*;
use crate::storage::Repository;

async fn manager(dir: &Path) -> BackupManager {
    let repo = Repository::open_in_memory().await.unwrap();
    BackupManager::with_kdf(repo.pool().clone(), Arc::new(CredentialStore::new()), None, None, dir.join("dbs"), KdfParams::insecure_fast())
}

fn job(src: &Path, dst: &Path) -> BackupJob {
    BackupJob {
        name: "Docs".into(),
        sources: vec![src.to_string_lossy().into()],
        destination: BackupDestination::Local { path: dst.to_string_lossy().into(), alternate_paths: vec![], marker_file: None },
        encrypted: false,
        options: BackupOptions { block_size: 4096, volume_size: 64 * 1024, quota_disable: true, verify_samples: 0, ..Default::default() },
        ..Default::default()
    }
}

fn setup(t: &tempfile::TempDir) -> (PathBuf, PathBuf) {
    let (src, dst) = (t.path().join("src"), t.path().join("dst"));
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.txt"), b"alpha alpha alpha").unwrap();
    std::fs::write(src.join("sub/b.txt"), vec![7u8; 20_000]).unwrap();
    (src, dst)
}

#[cfg(unix)]
#[tokio::test]
async fn async_pre_script_does_not_block_the_backup() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = setup(&t);
    let m = manager(t.path()).await;
    let mut j = job(&src, &dst);
    let marker = t.path().join("hook-ran");
    j.scripts.run_before = Some(format!("sh -c 'sleep 3; touch {}'", marker.display()));
    j.scripts.run_before_required = true; // the explicit async mode wins
    j.scripts.before_mode = Some(hooks::HookMode::Async);
    let j = m.save_job(j, None).await.unwrap();
    let started = std::time::Instant::now();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert!(started.elapsed() < std::time::Duration::from_secs(3), "the backup did not wait for the async hook");
    assert!(r.log.iter().any(|l| l.message.contains("started (async")));
    assert!(!marker.exists());
    // It still runs to the end in the background.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !marker.exists() && std::time::Instant::now() < until {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert!(marker.exists());
}

#[tokio::test]
async fn templates_crud_inheritance_and_effective_settings() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = setup(&t);
    let m = manager(t.path()).await;
    let mut patch = templates::SettingsPatch { retention: Some(RetentionPolicy::KeepCount { count: 1 }), ..Default::default() };
    patch.options.insert("compression_level".into(), 2.into());
    let tpl = m.save_template(templates::PolicyTemplate { name: "Laptops".into(), settings: patch, ..Default::default() }).await.unwrap();
    assert!(!tpl.id.is_empty());
    assert!(m.save_template(templates::PolicyTemplate { name: "laptops".into(), ..Default::default() }).await.is_err(), "duplicate name");

    // An older job (no overrides) whose compression is its own.
    let mut j = job(&src, &dst);
    j.options.compression_level = 7;
    let j = m.save_job(j, None).await.unwrap();
    assert!(j.overrides.is_none());
    let j = m.apply_template(&tpl.id, &j.id).await.unwrap();
    assert_eq!(j.template_ids, vec![tpl.id.clone()]);
    let ov = j.overrides.clone().expect("materialized on first template");
    assert!(ov.contains(&"options.compression_level".to_string()) && !ov.contains(&"retention".to_string()));

    let eff = m.effective_job(&j.id).await.unwrap();
    assert_eq!(eff.retention, RetentionPolicy::KeepCount { count: 1 });
    assert_eq!(eff.options.compression_level, 7);
    let es = m.effective_settings(Some(&j.id), None).await.unwrap();
    let ret = es.settings.iter().find(|s| s.field_path == "retention").unwrap();
    assert_eq!(ret.source, templates::SettingSource::Template { id: tpl.id.clone(), name: "Laptops".into() });
    let comp = es.settings.iter().find(|s| s.field_path == "options.compression_level").unwrap();
    assert_eq!(comp.source, templates::SettingSource::Job);
    assert_eq!(comp.inherited.as_ref().unwrap().value, serde_json::json!(2));

    // Reset to inherited (unsaved job): the template's value applies.
    let mut unsaved = j.clone();
    unsaved.overrides = Some(vec![]);
    let es = m.effective_settings(None, Some(unsaved)).await.unwrap();
    assert_eq!(es.settings.iter().find(|s| s.field_path == "options.compression_level").unwrap().value, serde_json::json!(2));

    // The run uses the inherited retention: two backups keep one version.
    m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    std::fs::write(src.join("c.txt"), b"new").unwrap();
    let r = m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert_eq!(m.list_versions(&j.id).await.unwrap().len(), 1);

    // In use: cannot delete. Unknown ids are refused on save.
    assert!(m.delete_template(&tpl.id).await.is_err());
    let mut bad = m.get_job(&j.id).await.unwrap();
    bad.template_ids.push("nope".into());
    assert!(m.save_job(bad, None).await.is_err());

    // Export and import (same name updates the existing one).
    let json = m.export_templates(&[]).await.unwrap();
    let imported = m.import_templates(&json).await.unwrap();
    assert_eq!(imported[0].id, tpl.id);
    assert_eq!(m.list_templates().await.unwrap().len(), 1);

    // Organization templates are read-only and an enforced one wins.
    let org = ufop_core::agent::ManagedBackupTemplate { id: "keep3".into(), name: "Org".into(), enforced: true, settings: serde_json::json!({"retention": {"type": "keep_count", "count": 3}}), ..Default::default() };
    assert!(m.set_managed_templates(Some(&[org])).is_empty());
    assert_eq!(m.effective_job(&j.id).await.unwrap().retention, RetentionPolicy::KeepCount { count: 3 });
    assert!(m.delete_template("org:keep3").await.is_err());
    assert!(m.save_template(templates::PolicyTemplate { id: "org:keep3".into(), name: "x".into(), ..Default::default() }).await.is_err());

    // Not in use any more: deletable.
    let mut free = m.get_job(&j.id).await.unwrap();
    free.template_ids.clear();
    m.save_job(free, None).await.unwrap();
    m.delete_template(&tpl.id).await.unwrap();
}

#[tokio::test]
async fn maintenance_runs_on_its_own_schedule() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = setup(&t);
    let m = manager(t.path()).await;
    let mut j = job(&src, &dst);
    j.created_at = (Utc::now() - chrono::Duration::days(2)).to_rfc3339();
    j.maintenance.compact = Some(schedule::BackupSchedule::cron(&["0 3 * * *"]));
    j.maintenance.verify = Some(schedule::VerifyTask {
        schedule: schedule::BackupSchedule::cron(&["0 4 * * *"]),
        options: ufop_core::backup::verify::VerifyOptions { files_percent: Some(100), ..Default::default() },
    });
    let mut j = m.save_job(j, None).await.unwrap();
    // save_job keeps created_at for new jobs only when set by the store; put it back.
    j.created_at = (Utc::now() - chrono::Duration::days(2)).to_rfc3339();
    m.inner.jobs.write().await.insert(j.id.clone(), j.clone());
    m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();

    let now = Utc::now();
    let due = m.due_maintenance(now).await;
    assert_eq!(due.iter().map(|d| d.1).collect::<Vec<_>>().len(), 2, "{due:?}");
    // Paused: nothing is due.
    m.pause(None);
    assert!(m.due_maintenance(now).await.is_empty());
    m.resume();
    // Next maintenance is listed in the status.
    let st = m.status().await;
    assert_eq!(st.next_maintenance.len(), 2);
    assert!(st.next_maintenance.iter().any(|n| n.operation == "compact"));

    // One task per job per pass; it shows in the history.
    assert_eq!(m.run_due_maintenance(now).await, 1);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    while m.is_job_busy(&j.id) && std::time::Instant::now() < until {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(m.run_due_maintenance(now).await, 1);
    while m.is_job_busy(&j.id) && std::time::Instant::now() < until {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let hist = m.run_history(Some(j.id.clone()), 10).await.unwrap();
    let ops: Vec<&str> = hist.iter().map(|h| h.operation.as_str()).collect();
    assert!(ops.contains(&"compact") && ops.contains(&"verify"), "{ops:?}");
    let v = hist.iter().find(|h| h.operation == "verify").unwrap();
    assert_eq!(v.status, "success", "{:?}", v.log);
    assert_eq!(v.report["files_checked"], 2);
    // Both slots are used up now.
    assert!(m.due_maintenance(now).await.is_empty());
    assert!(m.get_job(&j.id).await.unwrap().stats.last_maintenance.contains_key("verify"));
}

#[tokio::test]
async fn verify_operation_takes_scope_and_sampling() {
    let t = tempfile::tempdir().unwrap();
    let (src, dst) = setup(&t);
    let m = manager(t.path()).await;
    let j = m.save_job(job(&src, &dst), None).await.unwrap();
    m.execute(&j.id, BackupOperation::Backup { dry_run: false }).await.unwrap();
    let o = ufop_core::backup::verify::VerifyOptions {
        scope: ufop_core::backup::verify::VerifyScope { versions: vec!["latest".into()], paths: vec!["*b.txt".into()] },
        mode: ufop_core::backup::verify::VerifyMode::Structure,
        files_percent: Some(100),
        ..Default::default()
    };
    let r = m.execute(&j.id, BackupOperation::Verify { full: false, options: Some(o) }).await.unwrap();
    assert_eq!(r.status, "success", "{:?}", r.log);
    assert_eq!((r.report["files_in_scope"].as_u64(), r.report["files_checked"].as_u64(), r.report["checked"].as_u64()), (Some(1), Some(1), Some(0)));
    assert!(r.summary.contains("1 file(s) verified"), "{}", r.summary);
    // The older form still works.
    let r = m.execute(&j.id, BackupOperation::Verify { full: true, options: None }).await.unwrap();
    assert_eq!(r.status, "success");
}
