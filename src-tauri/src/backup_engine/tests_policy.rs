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
