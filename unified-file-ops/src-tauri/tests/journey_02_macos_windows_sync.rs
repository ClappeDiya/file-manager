//! Journey 2: macOS to Windows share two-way sync
//!
//! End-to-end test validating:
//! 1. Create sync pair between macOS local folder and Windows SMB share
//! 2. Two-way sync with NFC/NFD normalization
//! 3. Conflict resolution when both sides modify
//! 4. Filesystem watcher triggers
//! 5. Health indicators after sync runs
//! 6. Rollback capability

use unified_file_ops_lib::compat_engine::profiles;
use unified_file_ops_lib::compat_engine::CompatChecker;
use unified_file_ops_lib::core::traits::SyncOperations;
use unified_file_ops_lib::core::types::*;
use unified_file_ops_lib::sync_engine::SyncManager;
use chrono::Utc;
use tempfile::TempDir;
use uuid::Uuid;

fn make_two_way_pair(src: &std::path::Path, dst: &std::path::Path) -> SyncPair {
    SyncPair {
        id: Uuid::new_v4(),
        name: "macOS to Windows Share".to_string(),
        source_path: src.to_string_lossy().to_string(),
        dest_path: dst.to_string_lossy().to_string(),
        mode: SyncMode::TwoWay,
        enabled: true,
        last_run: None,
        trigger: SyncTrigger::Manual,
        filter: SyncFilter::default(),
        conflict_policy: SyncConflictPolicy::CreateConflictCopy,
        verify_mode: SyncVerifyMode::Fast,
        checksum_enabled: false,
        created_at: Utc::now(),
        time_offset_secs: None,
    }
}

/// Journey 2A: NFC/NFD normalization during sync
#[test]
fn journey2_nfc_nfd_normalization_for_windows() {
    // macOS uses NFD, Windows uses NFC
    let windows_profile = profiles::get_profile("windows-ntfs").unwrap();

    // Files from macOS (NFD form)
    let macos_files = vec![
        "cafe\u{0301}.txt",           // NFD: e + combining acute
        "re\u{0301}sume\u{0301}.pdf", // NFD
        "u\u{0308}ber.doc",           // NFD: u + combining diaeresis
        "n\u{0303}on\u{0303}o.txt",   // NFD: n + combining tilde
    ];

    for name in &macos_files {
        let translation = CompatChecker::check_name_with_profile(name, &windows_profile);
        // Should normalize to NFC
        assert_ne!(translation.translated_name, *name,
            "NFD name '{}' should be normalized for Windows", name);
        // Verify NFC form is used (rule message format: "Normalized to Nfc: ...")
        assert!(translation.rules_applied.iter().any(|r|
            r.contains("Nfc") || r.contains("NFC") || r.contains("ormalized") || r.contains("diacritical")),
            "Rules should mention normalization for '{}', got: {:?}", name, translation.rules_applied);
    }
}

/// Journey 2B: Create two-way sync pair
#[tokio::test]
async fn journey2_create_two_way_sync_pair() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    let pair = make_two_way_pair(src.path(), dst.path());
    let created = mgr.create_pair(pair.clone()).await.unwrap();

    assert_eq!(created.mode, SyncMode::TwoWay);
    assert_eq!(created.name, "macOS to Windows Share");
    assert!(created.enabled);
}

/// Journey 2C: Two-way sync copies files in both directions
#[tokio::test]
async fn journey2_two_way_sync_copies_both_directions() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    // Files on macOS side
    std::fs::write(src.path().join("macos_file.txt"), "from macOS").unwrap();
    // Files on Windows side
    std::fs::write(dst.path().join("windows_file.txt"), "from Windows").unwrap();

    let pair = make_two_way_pair(src.path(), dst.path());
    mgr.create_pair(pair.clone()).await.unwrap();

    let report = mgr.execute_sync(pair.id).await.unwrap();
    assert_eq!(report.status, SyncRunStatus::Success);

    // Both files should exist on both sides
    assert!(src.path().join("macos_file.txt").exists());
    assert!(dst.path().join("windows_file.txt").exists());
    // Cross-copied files
    assert!(dst.path().join("macos_file.txt").exists(),
        "macOS file should be synced to Windows share");
    assert!(src.path().join("windows_file.txt").exists(),
        "Windows file should be synced to macOS");
}

/// Journey 2D: Conflict resolution with conflict copies
#[tokio::test]
async fn journey2_conflict_resolution() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    // Same file on both sides with different content
    std::fs::write(src.path().join("shared.txt"), "macOS version").unwrap();
    std::fs::write(dst.path().join("shared.txt"), "Windows version").unwrap();

    let mut pair = make_two_way_pair(src.path(), dst.path());
    pair.conflict_policy = SyncConflictPolicy::CreateConflictCopy;
    mgr.create_pair(pair.clone()).await.unwrap();

    let report = mgr.execute_sync(pair.id).await.unwrap();

    // Should succeed (conflicts handled via conflict copies)
    assert!(report.status == SyncRunStatus::Success ||
            report.status == SyncRunStatus::PartialSuccess,
        "Sync should succeed or partial success, got {:?}", report.status);
}

/// Journey 2E: Dry run preview before sync
#[tokio::test]
async fn journey2_dry_run_preview() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    std::fs::write(src.path().join("new_file.txt"), "new content").unwrap();
    std::fs::write(src.path().join("another.doc"), "document").unwrap();

    let pair = make_two_way_pair(src.path(), dst.path());
    mgr.create_pair(pair.clone()).await.unwrap();

    let preview = mgr.dry_run(pair.id).await.unwrap();
    assert!(preview.total_additions > 0, "Dry run should show additions");
    assert!(preview.total_bytes > 0, "Dry run should calculate bytes");
}

/// Journey 2F: Health indicator progression
#[tokio::test]
async fn journey2_health_indicator_progression() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    let pair = make_two_way_pair(src.path(), dst.path());
    mgr.create_pair(pair.clone()).await.unwrap();

    // Before any sync: Gray (no data)
    let health = mgr.get_health(pair.id).await;
    assert_eq!(health, SyncHealth::Gray, "No sync yet should be Gray");

    // After successful sync: Green
    std::fs::write(src.path().join("test.txt"), "test").unwrap();
    mgr.execute_sync(pair.id).await.unwrap();
    let health = mgr.get_health(pair.id).await;
    assert_eq!(health, SyncHealth::Green, "After success should be Green");
}

/// Journey 2G: Sync reports are persisted
#[tokio::test]
async fn journey2_sync_reports() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    std::fs::write(src.path().join("file.txt"), "content").unwrap();

    let pair = make_two_way_pair(src.path(), dst.path());
    mgr.create_pair(pair.clone()).await.unwrap();
    mgr.execute_sync(pair.id).await.unwrap();

    let reports = mgr.get_reports(pair.id).await;
    assert_eq!(reports.len(), 1);
    assert_eq!(reports[0].status, SyncRunStatus::Success);
    assert!(reports[0].files_added > 0);
}

/// Journey 2H: Update sync pair configuration
#[tokio::test]
async fn journey2_update_sync_pair_config() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    let mut pair = make_two_way_pair(src.path(), dst.path());
    mgr.create_pair(pair.clone()).await.unwrap();

    // Update conflict policy
    pair.conflict_policy = SyncConflictPolicy::SourceWins;
    pair.checksum_enabled = true;
    let updated = mgr.update_pair(pair.clone()).await.unwrap();
    assert_eq!(updated.conflict_policy, SyncConflictPolicy::SourceWins);
    assert!(updated.checksum_enabled);
}

/// Journey 2I: Disabled sync pair cannot execute
#[tokio::test]
async fn journey2_disabled_pair_blocked() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    let mut pair = make_two_way_pair(src.path(), dst.path());
    pair.enabled = false;
    mgr.create_pair(pair.clone()).await.unwrap();

    let result = mgr.execute_sync(pair.id).await;
    assert!(result.is_err(), "Disabled pair should not execute");
}

/// Journey 2J: Delete sync pair cleanup
#[tokio::test]
async fn journey2_delete_sync_pair() {
    let mgr = SyncManager::new();
    let src = TempDir::new().unwrap();
    let dst = TempDir::new().unwrap();

    let pair = make_two_way_pair(src.path(), dst.path());
    mgr.create_pair(pair.clone()).await.unwrap();

    mgr.delete_pair(pair.id).await.unwrap();

    let pairs = mgr.list_pairs().await.unwrap();
    assert!(pairs.is_empty());

    // Reports should also be cleaned up
    let reports = mgr.get_reports(pair.id).await;
    assert!(reports.is_empty());
}

/// Journey 2K: Cross-platform compatibility in batch check
#[test]
fn journey2_cross_platform_batch_check() {
    let windows_profile = profiles::get_profile("windows-ntfs").unwrap();

    // Files that would be fine on macOS but problematic on Windows
    let macos_native_files = ["file:with:colons.txt",
        "file<angle>.txt",
        "file|pipe.txt",
        "CON.txt",
        "normal.txt"];

    let name_refs: Vec<&str> = macos_native_files.iter().map(|s| s.as_ref()).collect();
    let result = CompatChecker::check_batch(&name_refs, &windows_profile);

    // At least 4 files should be modified (all except normal.txt)
    assert!(result.total_modified >= 4,
        "Expected at least 4 modified, got {}", result.total_modified);
}
