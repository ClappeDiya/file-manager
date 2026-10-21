//! Store tests. Emulator tests run when the emulator's address is set:
//! `UFOP_TEST_S3=http://127.0.0.1:5005` (moto), `UFOP_TEST_AZURE=http://127.0.0.1:10000`
//! (Azurite), `UFOP_TEST_GCS=http://127.0.0.1:4443` (fake-gcs-server).
//! Mock-server tests (in-process) always run.

use super::*;
use std::sync::Arc;

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

/// Create, upload (small + multi-part sized), list, download, delete.
pub(crate) async fn exercise(store: &dyn ObjectStore, big: usize) {
    store.create_folder().await.unwrap();
    store.create_folder().await.unwrap(); // idempotent
    let small = b"hello volume".to_vec();
    let large: Vec<u8> = (0..big).map(|i| (i * 31 % 251) as u8).collect();
    store.put("ufop-a.dblock", small.clone()).await.unwrap();
    store.put("ufop-b.dblock", large.clone()).await.unwrap();
    store.put("ufop-a.dblock", small.clone()).await.unwrap(); // overwrite
    let mut l = store.list().await.unwrap();
    l.sort_by(|a, b| a.name.cmp(&b.name));
    assert_eq!(l.iter().map(|f| (f.name.as_str(), f.size)).collect::<Vec<_>>(), vec![("ufop-a.dblock", small.len() as u64), ("ufop-b.dblock", large.len() as u64)]);
    assert_eq!(store.get("ufop-a.dblock").await.unwrap(), small);
    assert_eq!(store.get("ufop-b.dblock").await.unwrap(), large);
    store.delete("ufop-a.dblock").await.unwrap();
    let l = store.list().await.unwrap();
    assert_eq!(l.len(), 1);
    assert!(store.get("ufop-a.dblock").await.is_err());
    store.delete("ufop-b.dblock").await.unwrap();
}

/// Full engine backup + restore through a store.
fn backup_roundtrip(backend: &dyn ufop_core::backup::Backend) {
    use ufop_core::backup::crypto::KdfParams;
    use ufop_core::backup::db::BackupDb;
    use ufop_core::backup::*;
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 253) as u8).collect();
    std::fs::write(src.path().join("f.bin"), &data).unwrap();
    let o = BackupOptions { block_size: 16 * 1024, volume_size: 100_000, verify_samples: 1, quota_disable: true, ..Default::default() };
    let db = BackupDb::open_in_memory().unwrap();
    let r = run_backup(backend, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    let db2 = BackupDb::open_in_memory().unwrap();
    maintenance::recreate_database(backend, &db2, Some("pw"), "ufop", &OpContext::new()).unwrap();
    let repo = engine::Repo::open(backend, &db2, Some("pw"), &o, false, KdfParams::insecure_fast()).unwrap();
    let rr = run_restore(&repo, &RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
    assert!(rr.errors.is_empty(), "{:?}", rr.errors);
    assert_eq!(std::fs::read(out.path().join(src.path().file_name().unwrap()).join("f.bin")).unwrap(), data);
}

fn s3_cfg(endpoint: &str, bucket: &str) -> s3::S3Config {
    s3::S3Config { preset: "custom".into(), endpoint: endpoint.into(), bucket: bucket.into(), prefix: "backups/pc".into(), access_key_id: "test".into(), path_style: true, part_size_mb: 5, ..Default::default() }
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_against_emulator() {
    let Some(ep) = env("UFOP_TEST_S3") else { return };
    let b = format!("ufop-{}", uuid::Uuid::new_v4().simple());
    let mut cfg = s3_cfg(&ep, &b);
    cfg.storage_class = "STANDARD_IA".into();
    let store = s3::S3Store::new(cfg.clone(), "secret".into()).unwrap();
    exercise(&store, 12 << 20).await; // > part size: multipart path
    // Engine end to end through the sync adapter.
    let backend = Arc::new(StoreBackend::new(Box::new(s3::S3Store::new(cfg, "secret".into()).unwrap()), Some(std::time::Duration::from_secs(60))));
    let b2 = backend.clone();
    tokio::task::spawn_blocking(move || backup_roundtrip(b2.as_ref())).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_object_lock_against_emulator() {
    use ufop_core::backup::backend::LockMode;
    let Some(ep) = env("UFOP_TEST_S3") else { return };
    let mut cfg = s3_cfg(&ep, &format!("ufop-lock-{}", uuid::Uuid::new_v4().simple()));
    cfg.create_with_object_lock = true;
    let store = s3::S3Store::new(cfg, "secret".into()).unwrap();
    store.create_folder().await.unwrap();
    store.put("ufop-x.dblock", b"locked".to_vec()).await.unwrap();
    store.lock("ufop-x.dblock", chrono::Utc::now() + chrono::Duration::days(1), LockMode::Compliance).await.unwrap();
    let (mode, until) = store.retention("ufop-x.dblock").await.unwrap().expect("retention set");
    assert_eq!(mode, "COMPLIANCE");
    assert!(until.starts_with(&(chrono::Utc::now() + chrono::Duration::days(1)).format("%Y-%m-%d").to_string()));
    // The locked version cannot be removed permanently.
    let versions = store.versions("ufop-x.dblock").await.unwrap();
    assert_eq!(versions.len(), 1);
    assert!(store.delete_version("ufop-x.dblock", &versions[0]).await.is_err(), "compliance lock must block deletion");
    // A plain delete only hides it behind a delete marker; the data survives.
    store.delete("ufop-x.dblock").await.unwrap();
    assert_eq!(store.versions("ufop-x.dblock").await.unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn azure_against_emulator() {
    let Some(ep) = env("UFOP_TEST_AZURE") else { return };
    // Azurite's well-known development account.
    let key = "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";
    let cfg = azure::AzureConfig {
        account: "devstoreaccount1".into(),
        container: format!("ufop{}", &uuid::Uuid::new_v4().simple().to_string()[..12]),
        prefix: "pc".into(),
        endpoint: format!("{ep}/devstoreaccount1"),
        block_size_mb: 1,
        ..Default::default()
    };
    let store = azure::AzureStore::new(cfg.clone(), key.into()).unwrap();
    exercise(&store, 3 << 20).await; // > block size: block list path
    let backend = Arc::new(StoreBackend::new(Box::new(azure::AzureStore::new(cfg, key.into()).unwrap()), None));
    let b2 = backend.clone();
    tokio::task::spawn_blocking(move || backup_roundtrip(b2.as_ref())).await.unwrap();
    assert!(azure::AzureStore::new(azure::AzureConfig { account: "a".into(), container: "c".into(), ..Default::default() }, "not base64!".into()).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn gcs_against_emulator() {
    let Some(ep) = env("UFOP_TEST_GCS") else { return };
    let cfg = gcs::GcsConfig { bucket: format!("ufop-{}", uuid::Uuid::new_v4().simple()), prefix: "pc".into(), project: "test".into(), endpoint: ep.clone(), ..Default::default() };
    let store = gcs::GcsStore::new(cfg.clone(), "fake-token".into()).unwrap();
    exercise(&store, 2 << 20).await;
    let backend = Arc::new(StoreBackend::new(Box::new(gcs::GcsStore::new(cfg, "fake-token".into()).unwrap()), None));
    let b2 = backend.clone();
    tokio::task::spawn_blocking(move || backup_roundtrip(b2.as_ref())).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn rclone_local_remote() {
    if std::process::Command::new("rclone").arg("version").output().is_err() {
        return;
    }
    let d = tempfile::tempdir().unwrap();
    let cfg = rclone::RcloneConfig { remote: ":local:".into(), path: d.path().join("dest").to_string_lossy().into(), ..Default::default() };
    let store = rclone::RcloneStore::new(cfg.clone(), None);
    exercise(&store, 300_000).await;
    let backend = Arc::new(StoreBackend::new(Box::new(rclone::RcloneStore::new(cfg, None)), None));
    let b2 = backend.clone();
    tokio::task::spawn_blocking(move || backup_roundtrip(b2.as_ref())).await.unwrap();
    let bad = rclone::RcloneStore::new(rclone::RcloneConfig { remote: "nosuchremote".into(), ..Default::default() }, None);
    assert!(bad.list().await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn timeout_is_enforced() {
    struct Slow;
    impl ObjectStore for Slow {
        fn describe(&self) -> String {
            "slow".into()
        }
        fn list(&self) -> BoxFut<'_, Vec<ufop_core::backup::RemoteFile>> {
            Box::pin(async {
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                Ok(vec![])
            })
        }
        fn put<'a>(&'a self, _: &'a str, _: Vec<u8>) -> BoxFut<'a, ()> {
            Box::pin(async { Ok(()) })
        }
        fn get<'a>(&'a self, _: &'a str) -> BoxFut<'a, Vec<u8>> {
            Box::pin(async { Ok(vec![]) })
        }
        fn delete<'a>(&'a self, _: &'a str) -> BoxFut<'a, ()> {
            Box::pin(async { Ok(()) })
        }
    }
    let b = Arc::new(StoreBackend::new(Box::new(Slow), Some(std::time::Duration::from_millis(200))));
    let r = tokio::task::spawn_blocking(move || ufop_core::backup::Backend::list(b.as_ref())).await.unwrap();
    assert!(r.unwrap_err().user_message().contains("timed out"));
}

#[test]
fn keys_and_children() {
    assert_eq!(join_key("/a/b/", "x"), "a/b/x");
    assert_eq!(join_key("", "x"), "x");
    assert_eq!(direct_child("a/b", "a/b/x"), Some("x"));
    assert_eq!(direct_child("a/b", "a/b/c/x"), None);
    assert_eq!(direct_child("", "x"), Some("x"));
    assert_eq!(direct_child("a", "ab/x"), None);
}

/// `UFOP_TEST_SMB=host:port/share/user/password` (Samba with SMB 3.1.1 minimum).
#[tokio::test(flavor = "multi_thread")]
async fn smb_against_server() {
    let Some(spec) = env("UFOP_TEST_SMB") else { return };
    let p: Vec<&str> = spec.splitn(4, '/').collect();
    let (hostport, share, user, pass) = (p[0], p[1], p[2], p[3]);
    let (host, port) = hostport.split_once(':').unwrap_or((hostport, "445"));
    let cfg = smb::SmbConfig { server: host.into(), share: share.into(), path: format!("ufop/{}", &uuid::Uuid::new_v4().simple().to_string()[..8]), username: user.into(), port: port.parse().ok(), min_protocol: "SMB3_11".into(), ..Default::default() };
    let store = smb::SmbStore::new(cfg.clone(), Some(pass.into()));
    exercise(&store, 700_000).await;
    let backend = Arc::new(StoreBackend::new(Box::new(smb::SmbStore::new(cfg.clone(), Some(pass.into()))), None));
    let b2 = backend.clone();
    tokio::task::spawn_blocking(move || backup_roundtrip(b2.as_ref())).await.unwrap();
    let wrong = smb::SmbStore::new(cfg, Some("wrong".into()));
    assert!(wrong.list().await.unwrap_err().user_message().contains("NT_STATUS_LOGON_FAILURE"));
}

/// `UFOP_TEST_TAHOE=http://127.0.0.1:3456|URI:DIR2:…` (a running gateway).
#[tokio::test(flavor = "multi_thread")]
async fn tahoe_against_gateway() {
    let Some(spec) = env("UFOP_TEST_TAHOE") else { return };
    let (node, cap) = spec.split_once('|').unwrap();
    let cfg = tahoe::TahoeConfig { node_url: node.into(), dir_cap: cap.into(), path: format!("ufop/{}", &uuid::Uuid::new_v4().simple().to_string()[..8]) };
    let store = tahoe::TahoeStore::new(cfg.clone()).unwrap();
    exercise(&store, 300_000).await;
    let backend = Arc::new(StoreBackend::new(Box::new(tahoe::TahoeStore::new(cfg).unwrap()), None));
    let b2 = backend.clone();
    tokio::task::spawn_blocking(move || backup_roundtrip(b2.as_ref())).await.unwrap();
    assert!(tahoe::TahoeStore::new(tahoe::TahoeConfig { dir_cap: "URI:CHK:readonly".into(), ..Default::default() }).is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn s3_cold_storage_restore_against_emulator() {
    let Some(ep) = env("UFOP_TEST_S3") else { return };
    let mut cfg = s3_cfg(&ep, &format!("ufop-cold-{}", uuid::Uuid::new_v4().simple()));
    cfg.storage_class = "GLACIER".into();
    let store = s3::S3Store::new(cfg.clone(), "secret".into()).unwrap();
    store.create_folder().await.unwrap();
    store.put("ufop-c.dblock", b"archived".to_vec()).await.unwrap();
    // First read: archived → restore requested, clear error.
    let e = store.get("ufop-c.dblock").await.unwrap_err();
    assert!(e.user_message().contains("restore was requested"), "{}", e.user_message());
    // Once the restore is available (immediate on the emulator) reads work.
    assert_eq!(store.get("ufop-c.dblock").await.unwrap(), b"archived");
}
