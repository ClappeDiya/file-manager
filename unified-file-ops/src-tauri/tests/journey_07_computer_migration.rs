//! Journey 7: Computer migration workflow
//!
//! End-to-end test validating:
//! 1. Export connection profiles for transfer
//! 2. Transfer with checkpoint/resume capability
//! 3. Verify transferred data integrity
//! 4. Workspace state export/import
//! 5. Connection grouping and search

use unified_file_ops_lib::connectors::ConnectionManager;
use unified_file_ops_lib::core::traits::{StorageOperations, TransferOperations};
use unified_file_ops_lib::core::types::*;
use unified_file_ops_lib::security::CredentialStore;
use unified_file_ops_lib::storage::Repository;
use unified_file_ops_lib::transfer_engine::TransferManager;
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

fn make_connection(name: &str, protocol: ConnectionProtocol, host: &str) -> ConnectionProfile {
    ConnectionProfile {
        id: Uuid::new_v4(),
        name: name.to_string(),
        protocol,
        host: host.to_string(),
        port: None,
        username: Some("user".to_string()),
        credential_ref: None,
        remote_path: "/".to_string(),
        created_at: Utc::now(),
        last_used: None,
        group_id: None,
        bandwidth_limit_bps: 0,
        conflict_policy: None,
        retry_policy: None,
        verify_checksums: true,
        checksum_algorithm: None,
        proxy_type: None,
        proxy_host: None,
        proxy_port: None,
        proxy_username: None,
        proxy_password: None,
        default_local_dir: None,
        default_remote_dir: None,
        charset: None,
        default_file_mode: None,
        default_dir_mode: None,
        ftp_use_mlsd: None,
        ftp_force_passive_ip: None,
        ftp_post_login_commands: None,
        symlink_policy: None,
    }
}

/// Journey 7A: Export and import connection profiles
#[tokio::test]
async fn journey7_export_import_connections() {
    let repo = Repository::open_in_memory().await.unwrap();
    let credential_store = Arc::new(CredentialStore::new());
    let conn_mgr = ConnectionManager::new(repo, credential_store);

    let sftp_conn = make_connection("Work SFTP", ConnectionProtocol::Sftp, "sftp.company.com");
    let s3_conn = make_connection("AWS S3 Backup", ConnectionProtocol::S3, "s3.amazonaws.com");

    conn_mgr.save_connection(sftp_conn.clone(), None).await.unwrap();
    conn_mgr.save_connection(s3_conn.clone(), None).await.unwrap();

    // Export connections
    let exported = conn_mgr.export_connections().await.unwrap();
    assert!(!exported.is_empty(), "Export should produce JSON data");

    // Simulate import on new computer
    let repo2 = Repository::open_in_memory().await.unwrap();
    let credential_store2 = Arc::new(CredentialStore::new());
    let conn_mgr2 = ConnectionManager::new(repo2, credential_store2);

    let import_result = conn_mgr2.import_connections(&exported).await.unwrap();
    assert!(import_result.imported_connections > 0, "Should import connections");
}

/// Journey 7B: Workspace state transfer
#[tokio::test]
async fn journey7_workspace_state_transfer() {
    let repo1 = Repository::open_in_memory().await.unwrap();
    let state = WorkspaceState::default();
    StorageOperations::save_workspace_state(&repo1, &state).await.unwrap();

    let loaded = StorageOperations::load_workspace_state(&repo1).await.unwrap();
    assert_eq!(loaded.version, 1);
}

/// Journey 7C: Migration transfer with checkpoint/resume
#[tokio::test]
async fn journey7_migration_transfer_checkpoint() {
    let mgr = TransferManager::new();

    let migration_files = vec![
        ("/Users/old/Documents/report.pdf", "/Users/new/Documents/report.pdf", 1024u64 * 100),
        ("/Users/old/Pictures/photo.jpg", "/Users/new/Pictures/photo.jpg", 1024 * 500),
        ("/Users/old/Music/song.mp3", "/Users/new/Music/song.mp3", 1024 * 1000),
        ("/Users/old/Desktop/notes.txt", "/Users/new/Desktop/notes.txt", 512),
        ("/Users/old/Projects/code.zip", "/Users/new/Projects/code.zip", 1024 * 2000),
    ];

    let mut job_ids = Vec::new();
    for (src, dst, size) in &migration_files {
        let job = mgr.enqueue(src, dst, *size).await.unwrap();
        job_ids.push(job.id);
    }

    // Simulate progress on first two files
    mgr.update_progress(job_ids[0], 1024 * 100, 1024 * 50).await.unwrap();
    mgr.complete_job(job_ids[0]).await.unwrap();
    mgr.update_progress(job_ids[1], 1024 * 250, 1024 * 50).await.unwrap();

    // "Crash" - snapshot jobs for persistence
    let snapshot = mgr.snapshot_jobs().await;
    assert_eq!(snapshot.len(), 5);

    // Simulate restore on new session
    let mgr2 = TransferManager::new();
    mgr2.restore_jobs(snapshot).await;

    let restored = mgr2.list_jobs().await.unwrap();
    assert_eq!(restored.len(), 5);

    let in_progress = restored.iter().find(|j| j.id == job_ids[1]).unwrap();
    assert_eq!(in_progress.transferred_bytes, 1024 * 250);
}

/// Journey 7D: Data integrity verification
#[tokio::test]
async fn journey7_data_integrity_verification() {
    let mgr = TransferManager::new();

    let job = mgr.enqueue("/old/file.dat", "/new/file.dat", 1024).await.unwrap();

    mgr.set_job_checksums(
        job.id,
        Some("abc123def456".to_string()),
        Some("abc123def456".to_string()),
    ).await.unwrap();

    let updated = mgr.get_job(job.id).await.unwrap().unwrap();
    assert_eq!(updated.source_checksum, updated.dest_checksum);
}

/// Journey 7E: Connection search and grouping
#[tokio::test]
async fn journey7_connection_organization() {
    let repo = Repository::open_in_memory().await.unwrap();
    let credential_store = Arc::new(CredentialStore::new());
    let conn_mgr = ConnectionManager::new(repo, credential_store);

    // Create connection group
    let group = conn_mgr.create_group("Work Servers".to_string(), None).await.unwrap();
    assert!(!group.id.is_nil());

    let mut conn = make_connection("Production SFTP", ConnectionProtocol::Sftp, "prod.company.com");
    conn.group_id = Some(group.id);
    conn_mgr.save_connection(conn, None).await.unwrap();

    // Search connections
    let results = conn_mgr.search_connections("production").await.unwrap();
    assert!(!results.is_empty(), "Should find connection by name search");

    // List groups
    let groups = conn_mgr.list_groups().await.unwrap();
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "Work Servers");
}

/// Journey 7F: Credential migration
#[tokio::test]
async fn journey7_credential_migration() {
    let store = CredentialStore::new();

    store.store("migration_test_key", "secret_password").await.unwrap();
    let secret = store.retrieve("migration_test_key").await.unwrap();
    assert_eq!(secret, Some("secret_password".to_string()));

    let keys = store.list_keys().await;
    assert!(keys.contains(&"migration_test_key".to_string()));

    store.delete("migration_test_key").await.unwrap();
    let secret = store.retrieve("migration_test_key").await.unwrap();
    assert_eq!(secret, None);
}

/// Journey 7G: Migration pipeline performance
#[tokio::test]
async fn journey7_migration_pipeline_performance() {
    let start = std::time::Instant::now();

    let mgr = TransferManager::new();
    let repo = Repository::open_in_memory().await.unwrap();

    let mut handles = Vec::new();
    for i in 0..1000 {
        let mgr_clone = mgr.clone();
        handles.push(tokio::spawn(async move {
            mgr_clone.enqueue(
                &format!("/old/path/file_{}.dat", i),
                &format!("/new/path/file_{}.dat", i),
                1024 * (i as u64 + 1),
            ).await.unwrap()
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let jobs = mgr.snapshot_jobs().await;
    StorageOperations::save_transfer_queue(&repo, &jobs).await.unwrap();

    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 2000,
        "Migration of 1000 files took {}ms, expected <2000ms", elapsed.as_millis());
}
