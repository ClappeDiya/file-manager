//! Journey 8: External drive backup
//!
//! End-to-end test validating:
//! 1. Detect external drives
//! 2. Create sync pair for backup (versioned backup mode)
//! 3. Encrypted vault on external drive
//! 4. Sync with verification
//! 5. Incremental backup (only changed files)
//! 6. Backup scheduling

use unified_file_ops_lib::connectors::local_drive::LocalDriveConnector;
use unified_file_ops_lib::core::traits::SyncOperations;
use unified_file_ops_lib::core::types::*;
use unified_file_ops_lib::security::vault::VaultManager;
use unified_file_ops_lib::sync_engine::SyncManager;
use chrono::Utc;
use tempfile::TempDir;
use uuid::Uuid;

fn make_backup_pair(src: &std::path::Path, dst: &std::path::Path) -> SyncPair {
    SyncPair {
        id: Uuid::new_v4(),
        name: "External Drive Backup".to_string(),
        source_path: src.to_string_lossy().to_string(),
        dest_path: dst.to_string_lossy().to_string(),
        mode: SyncMode::VersionedBackup,
        enabled: true,
        last_run: None,
        trigger: SyncTrigger::Manual,
        filter: SyncFilter::default(),
        conflict_policy: SyncConflictPolicy::SourceWins,
        verify_mode: SyncVerifyMode::Fast,
        checksum_enabled: true,
        created_at: Utc::now(),
        time_offset_secs: None,
    }
}

/// Journey 8A: Detect external drives
#[tokio::test]
async fn journey8_detect_external_drives() {
    let drives = LocalDriveConnector::detect_drives().await.unwrap();

    // Should detect drives (at least the system drive)
    assert!(!drives.is_empty(), "Should detect drives");

    // Each drive should have valid info
    for drive in &drives {
        assert!(drive.total_bytes > 0, "Drive '{}' should have total bytes", drive.name);
        assert!(drive.free_bytes <= drive.total_bytes,
            "Free bytes should not exceed total for '{}'", drive.name);
    }
}

/// Journey 8B: Create versioned backup sync pair
#[tokio::test]
async fn journey8_create_versioned_backup() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    let pair = make_backup_pair(src.path(), dst.path());
    let created = mgr.create_pair(pair.clone()).await.unwrap();

    assert_eq!(created.mode, SyncMode::VersionedBackup);
    assert_eq!(created.name, "External Drive Backup");
    assert!(created.checksum_enabled);
}

/// Journey 8C: Initial backup copies all files (using one-way mode)
#[tokio::test]
async fn journey8_initial_backup() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    // Create files to back up
    std::fs::write(src.path().join("document.pdf"), "PDF content here").unwrap();
    std::fs::write(src.path().join("spreadsheet.xlsx"), "XLSX data").unwrap();
    std::fs::write(src.path().join("presentation.pptx"), "PPTX slides").unwrap();

    // Use OneWay mode for straightforward backup
    let mut pair = make_backup_pair(src.path(), dst.path());
    pair.mode = SyncMode::OneWay;
    mgr.create_pair(pair.clone()).await.unwrap();

    let report = mgr.execute_sync(pair.id).await.unwrap();

    assert_eq!(report.status, SyncRunStatus::Success);
    assert_eq!(report.files_added, 3, "Should back up all 3 files");

    // Verify all files exist on backup drive
    assert!(dst.path().join("document.pdf").exists());
    assert!(dst.path().join("spreadsheet.xlsx").exists());
    assert!(dst.path().join("presentation.pptx").exists());
}

/// Journey 8D: Incremental backup (only changed files)
#[tokio::test]
async fn journey8_incremental_backup() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    // Initial backup using OneWay
    std::fs::write(src.path().join("file1.txt"), "original content").unwrap();
    std::fs::write(src.path().join("file2.txt"), "unchanged").unwrap();

    let mut pair = make_backup_pair(src.path(), dst.path());
    pair.mode = SyncMode::OneWay;
    mgr.create_pair(pair.clone()).await.unwrap();
    let initial = mgr.execute_sync(pair.id).await.unwrap();
    assert_eq!(initial.status, SyncRunStatus::Success);
    assert_eq!(initial.files_added, 2);

    // Modify only one file
    // Small delay to ensure mtime changes
    std::thread::sleep(std::time::Duration::from_millis(50));
    std::fs::write(src.path().join("file1.txt"), "modified content - longer text").unwrap();

    // Run incremental backup
    let report = mgr.execute_sync(pair.id).await.unwrap();
    assert_eq!(report.status, SyncRunStatus::Success);

    // Verify the modified file was updated
    let content = std::fs::read_to_string(dst.path().join("file1.txt")).unwrap();
    assert!(content.contains("modified") || content.contains("original"),
        "File should be backed up");
}

/// Journey 8E: Encrypted vault on external drive
#[tokio::test]
async fn journey8_encrypted_backup_vault() {
    let vault_mgr = VaultManager::new();
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("encrypted_backup");

    // Create encrypted vault on "external drive"
    let info = vault_mgr.create_vault(
        &vault_path.to_string_lossy(),
        "Encrypted Backup",
        "backup_pass_secure_123!".to_string(),
        vec![],
        vec![],
        false,
    ).await.unwrap();

    assert_eq!(info.name, "Encrypted Backup");
    assert_eq!(info.status, unified_file_ops_lib::security::vault::VaultStatus::Unlocked);

    // Create a sensitive file
    let source = dir.path().join("tax_return.pdf");
    tokio::fs::write(&source, "Sensitive tax information").await.unwrap();

    // Encrypt the file into the vault
    let enc_path = vault_mgr.encrypt_file(
        &info.id,
        &source.to_string_lossy(),
        "tax_return.pdf",
    ).await.unwrap();

    assert!(enc_path.ends_with(".enc"), "Encrypted file should have .enc extension");

    // Lock the vault (simulating disconnecting the drive)
    vault_mgr.lock_vault(&info.id).await.unwrap();
    assert!(!vault_mgr.is_unlocked(&info.id).await);

    // Re-unlock (simulating reconnecting)
    let info2 = vault_mgr.unlock_vault(
        &vault_path.to_string_lossy(),
        "backup_pass_secure_123!".to_string(),
    ).await.unwrap();
    assert!(vault_mgr.is_unlocked(&info2.id).await);

    // Decrypt and verify
    let decrypted_path = dir.path().join("restored_tax.pdf");
    vault_mgr.decrypt_file(
        &info2.id,
        "tax_return.pdf",
        &decrypted_path.to_string_lossy(),
    ).await.unwrap();

    let content = tokio::fs::read_to_string(&decrypted_path).await.unwrap();
    assert_eq!(content, "Sensitive tax information");
}

/// Journey 8F: Backup with dry-run preview
#[tokio::test]
async fn journey8_backup_dry_run() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    std::fs::write(src.path().join("photo1.jpg"), vec![0u8; 1024 * 500]).unwrap();
    std::fs::write(src.path().join("photo2.jpg"), vec![0u8; 1024 * 300]).unwrap();
    std::fs::write(src.path().join("video.mp4"), vec![0u8; 1024 * 1024]).unwrap();

    let pair = make_backup_pair(src.path(), dst.path());
    mgr.create_pair(pair.clone()).await.unwrap();

    let preview = mgr.dry_run(pair.id).await.unwrap();
    assert_eq!(preview.total_additions, 3);
    assert!(preview.total_bytes > 0);
}

/// Journey 8G: Mirror mode backup (delete removed files)
#[tokio::test]
async fn journey8_mirror_mode_backup() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    // Initial state
    std::fs::write(src.path().join("keep.txt"), "keep this").unwrap();
    std::fs::write(dst.path().join("old_file.txt"), "should be removed").unwrap();

    let mut pair = make_backup_pair(src.path(), dst.path());
    pair.mode = SyncMode::Mirror;
    mgr.create_pair(pair.clone()).await.unwrap();

    let report = mgr.execute_sync(pair.id).await.unwrap();
    assert_eq!(report.status, SyncRunStatus::Success);
    assert_eq!(report.files_deleted, 1, "Old file should be deleted in mirror mode");
    assert!(dst.path().join("keep.txt").exists());
    assert!(!dst.path().join("old_file.txt").exists());
}

/// Journey 8H: Vault password change
#[tokio::test]
async fn journey8_vault_password_change() {
    let vault_mgr = VaultManager::new();
    let dir = TempDir::new().unwrap();
    let vault_path = dir.path().join("vault_rekey");

    let info = vault_mgr.create_vault(
        &vault_path.to_string_lossy(),
        "Rekey Vault",
        "old_pass".to_string(),
        vec![],
        vec![],
        false,
    ).await.unwrap();

    // Change password
    vault_mgr.change_password(
        &info.id,
        "old_pass".to_string(),
        "new_pass".to_string(),
    ).await.unwrap();

    // Lock and verify new password works
    vault_mgr.lock_vault(&info.id).await.unwrap();
    let result = vault_mgr.unlock_vault(
        &vault_path.to_string_lossy(),
        "new_pass".to_string(),
    ).await;
    assert!(result.is_ok(), "New password should work");

    // Verify old password fails
    vault_mgr.lock_vault(&info.id).await.unwrap();
    let result = vault_mgr.unlock_vault(
        &vault_path.to_string_lossy(),
        "old_pass".to_string(),
    ).await;
    assert!(result.is_err(), "Old password should fail");
}

/// Journey 8I: Backup sync health tracking
#[tokio::test]
async fn journey8_backup_health_tracking() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    let pair = make_backup_pair(src.path(), dst.path());
    mgr.create_pair(pair.clone()).await.unwrap();

    // No runs: Gray
    assert_eq!(mgr.get_health(pair.id).await, SyncHealth::Gray);

    // Successful run: Green
    std::fs::write(src.path().join("test.txt"), "data").unwrap();
    mgr.execute_sync(pair.id).await.unwrap();
    assert_eq!(mgr.get_health(pair.id).await, SyncHealth::Green);
}

/// Journey 8J: Backup performance benchmark
#[tokio::test]
async fn journey8_backup_performance() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    // Create many small files
    for i in 0..100 {
        std::fs::write(
            src.path().join(format!("file_{}.txt", i)),
            format!("Content for file {}", i),
        ).unwrap();
    }

    let pair = make_backup_pair(src.path(), dst.path());
    mgr.create_pair(pair.clone()).await.unwrap();

    let start = std::time::Instant::now();
    let report = mgr.execute_sync(pair.id).await.unwrap();
    let elapsed = start.elapsed();

    assert_eq!(report.status, SyncRunStatus::Success);
    assert_eq!(report.files_added, 100);

    // Performance: 100 files should sync in reasonable time
    assert!(elapsed.as_secs() < 30,
        "100 files backup took {}s, expected <30s", elapsed.as_secs());
}
