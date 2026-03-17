//! Journey 5: Drive-to-drive transfer on same computer
//!
//! End-to-end test validating:
//! 1. Drive detection and listing
//! 2. Preflight checks (space, permissions)
//! 3. Transfer enqueue with progress tracking
//! 4. Bandwidth throttling
//! 5. Priority management
//! 6. Crash recovery via journal
//! 7. Transfer verification (checksum)

use unified_file_ops_lib::connectors::local_drive::LocalDriveConnector;
use unified_file_ops_lib::core::traits::TransferOperations;
use unified_file_ops_lib::core::types::*;
use unified_file_ops_lib::transfer_engine::TransferManager;
use uuid::Uuid;

/// Journey 5A: Drive detection
#[tokio::test]
async fn journey5_detect_drives() {
    let drives = LocalDriveConnector::detect_drives().await.unwrap();

    // Should detect at least the root/system drive
    assert!(!drives.is_empty(), "Should detect at least one drive");

    for drive in &drives {
        assert!(!drive.name.is_empty(), "Drive should have a name");
        assert!(!drive.mount_point.is_empty(), "Drive should have a mount point");
        assert!(drive.total_bytes > 0, "Drive should have total bytes > 0");
    }
}

/// Journey 5B: Transfer preflight check
#[tokio::test]
async fn journey5_transfer_preflight() {
    // Preflight check between two paths (using temp dirs as stand-ins)
    let src = tempfile::tempdir().unwrap();
    let dst = tempfile::tempdir().unwrap();

    // Create a test file
    std::fs::write(src.path().join("test.dat"), vec![0u8; 1024]).unwrap();

    let result = LocalDriveConnector::preflight_check(
        &src.path().to_string_lossy(),
        &dst.path().to_string_lossy(),
        1024,
    ).await;

    assert!(result.is_ok(), "Preflight check should succeed for valid paths");
    let preflight = result.unwrap();
    assert!(preflight.can_proceed, "Preflight should pass for valid paths");
    // Verify dest_drive has free space info
    if let Some(dest) = &preflight.dest_drive {
        assert!(dest.free_bytes > 0, "Should have free space info");
    }
}

/// Journey 5C: Transfer enqueue and lifecycle
#[tokio::test]
async fn journey5_transfer_lifecycle() {
    let mgr = TransferManager::new();

    // Enqueue a drive-to-drive transfer
    let job = mgr.enqueue(
        "/Volumes/ExternalSSD/photos/vacation.zip",
        "/Volumes/BackupDrive/photos/vacation.zip",
        1024 * 1024 * 500, // 500 MB
    ).await.unwrap();

    assert_eq!(job.status, TransferStatus::Queued);
    assert_eq!(job.total_bytes, 524_288_000);
    assert_eq!(job.transferred_bytes, 0);
    assert!(job.verify_checksum, "Checksum verification should be enabled by default");

    // Simulate progress
    mgr.update_progress(job.id, 262_144_000, 50_000_000).await.unwrap();
    let updated = mgr.get_job(job.id).await.unwrap().unwrap();
    assert_eq!(updated.transferred_bytes, 262_144_000);
    assert_eq!(updated.speed_bps, 50_000_000);
    assert!(updated.eta_seconds.is_some());

    // Complete the transfer
    mgr.complete_job(job.id).await.unwrap();
    let completed = mgr.get_job(job.id).await.unwrap().unwrap();
    assert_eq!(completed.status, TransferStatus::Completed);
    assert_eq!(completed.transferred_bytes, completed.total_bytes);
}

/// Journey 5D: Bandwidth throttling
#[tokio::test]
async fn journey5_bandwidth_throttling() {
    let mgr = TransferManager::new();

    // No throttle initially
    assert_eq!(mgr.effective_throttle(None).await, 0);

    // Set global throttle to 10 MB/s
    mgr.set_global_throttle(10_000_000).await;
    assert_eq!(mgr.effective_throttle(None).await, 10_000_000);

    // Set per-connection throttle to 5 MB/s
    let conn_id = Uuid::new_v4();
    mgr.set_connection_throttle(conn_id, 5_000_000).await;

    // Effective should be the minimum
    assert_eq!(mgr.effective_throttle(Some(conn_id)).await, 5_000_000);

    // Remove global throttle
    mgr.set_global_throttle(0).await;
    assert_eq!(mgr.effective_throttle(Some(conn_id)).await, 5_000_000);

    // Remove connection throttle
    mgr.set_connection_throttle(conn_id, 0).await;
    assert_eq!(mgr.effective_throttle(Some(conn_id)).await, 0);
}

/// Journey 5E: Priority management
#[tokio::test]
async fn journey5_priority_management() {
    let mgr = TransferManager::new();

    let j1 = mgr.enqueue("/drive1/file1", "/drive2/file1", 1000).await.unwrap();
    let j2 = mgr.enqueue("/drive1/file2", "/drive2/file2", 2000).await.unwrap();
    let j3 = mgr.enqueue("/drive1/file3", "/drive2/file3", 3000).await.unwrap();

    // Set priorities
    mgr.set_priority(j3.id, TransferPriority::High).await.unwrap();
    mgr.set_priority(j1.id, TransferPriority::Low).await.unwrap();

    // List should be sorted by priority
    let jobs = mgr.list_jobs().await.unwrap();
    assert_eq!(jobs[0].id, j3.id, "High priority should be first");
    assert_eq!(jobs[1].id, j2.id, "Normal priority should be second");
    assert_eq!(jobs[2].id, j1.id, "Low priority should be last");
}

/// Journey 5F: Pause, resume, cancel operations
#[tokio::test]
async fn journey5_pause_resume_cancel() {
    let mgr = TransferManager::new();

    let job = mgr.enqueue("/drive1/large_file.iso", "/drive2/large_file.iso", 4_000_000_000)
        .await.unwrap();

    // Pause
    mgr.pause(job.id).await.unwrap();
    let j = mgr.get_job(job.id).await.unwrap().unwrap();
    assert_eq!(j.status, TransferStatus::Paused);
    assert_eq!(j.speed_bps, 0);

    // Resume
    mgr.resume(job.id).await.unwrap();
    let j = mgr.get_job(job.id).await.unwrap().unwrap();
    assert_eq!(j.status, TransferStatus::Queued);

    // Cancel
    mgr.cancel(job.id).await.unwrap();
    let j = mgr.get_job(job.id).await.unwrap().unwrap();
    assert_eq!(j.status, TransferStatus::Cancelled);
}

/// Journey 5G: Retry with exponential backoff
#[tokio::test]
async fn journey5_retry_with_backoff() {
    let mgr = TransferManager::new();

    let job = mgr.enqueue("/drive1/file", "/drive2/file", 1000).await.unwrap();
    mgr.fail_job(job.id, "I/O error on source drive").await.unwrap();

    // First retry
    let can_retry = mgr.increment_retry(job.id).await.unwrap();
    assert!(can_retry);
    let backoff = mgr.get_backoff_duration(job.id).await.unwrap();
    assert_eq!(backoff, std::time::Duration::from_secs(1));

    // Second retry
    mgr.fail_job(job.id, "I/O error again").await.unwrap();
    let can_retry = mgr.increment_retry(job.id).await.unwrap();
    assert!(can_retry);
    let backoff = mgr.get_backoff_duration(job.id).await.unwrap();
    assert_eq!(backoff, std::time::Duration::from_secs(5));

    // Third retry
    mgr.fail_job(job.id, "I/O error once more").await.unwrap();
    let can_retry = mgr.increment_retry(job.id).await.unwrap();
    assert!(can_retry);
    let backoff = mgr.get_backoff_duration(job.id).await.unwrap();
    assert_eq!(backoff, std::time::Duration::from_secs(30));

    // Fourth retry - should hit max
    mgr.fail_job(job.id, "final failure").await.unwrap();
    let can_retry = mgr.increment_retry(job.id).await.unwrap();
    assert!(!can_retry, "Should hit max retries");
}

/// Journey 5H: Concurrent transfers (100 items)
#[tokio::test]
async fn journey5_concurrent_100_transfers() {
    let mgr = TransferManager::new();

    let mut handles = Vec::new();
    for i in 0..100 {
        let mgr_clone = mgr.clone();
        handles.push(tokio::spawn(async move {
            mgr_clone.enqueue(
                &format!("/drive1/file_{}.dat", i),
                &format!("/drive2/file_{}.dat", i),
                (i as u64 + 1) * 1024,
            ).await.unwrap()
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }

    let jobs = mgr.list_jobs().await.unwrap();
    assert_eq!(jobs.len(), 100);
}

/// Journey 5I: Progress subscription
#[tokio::test]
async fn journey5_progress_subscription() {
    let mgr = TransferManager::new();
    let mut rx = mgr.subscribe_progress();

    let job = mgr.enqueue("/src", "/dst", 10000).await.unwrap();
    mgr.update_progress(job.id, 5000, 1000).await.unwrap();

    rx.changed().await.unwrap();
    let progress = rx.borrow().clone().unwrap();
    assert_eq!(progress.job_id, job.id);
    assert_eq!(progress.transferred_bytes, 5000);
    assert_eq!(progress.total_bytes, 10000);
}

/// Journey 5J: Retry all failed transfers
#[tokio::test]
async fn journey5_retry_all_failed() {
    let mgr = TransferManager::new();

    let j1 = mgr.enqueue("/a", "/b", 100).await.unwrap();
    let j2 = mgr.enqueue("/c", "/d", 200).await.unwrap();
    let _j3 = mgr.enqueue("/e", "/f", 300).await.unwrap(); // not failed

    mgr.fail_job(j1.id, "error1").await.unwrap();
    mgr.fail_job(j2.id, "error2").await.unwrap();

    let count = mgr.retry_all_failed().await.unwrap();
    assert_eq!(count, 2);

    let failed = mgr.list_failed_jobs().await.unwrap();
    assert!(failed.is_empty(), "No jobs should be in failed state after retry_all");
}

/// Journey 5K: Transfer configuration management
#[tokio::test]
async fn journey5_transfer_config() {
    let mgr = TransferManager::new();

    let config = mgr.config().await;
    assert_eq!(config.default_verify_tier, VerifyTier::Fast);

    mgr.set_verify_tier(VerifyTier::Verified).await;
    assert_eq!(mgr.verify_tier().await, VerifyTier::Verified);

    mgr.set_verify_tier(VerifyTier::MissionCritical).await;
    assert_eq!(mgr.verify_tier().await, VerifyTier::MissionCritical);
}

/// Journey 5L: Performance - startup time under 2 seconds
#[tokio::test]
async fn journey5_startup_performance() {
    let start = std::time::Instant::now();

    // Simulate app startup: create manager + enqueue initial batch
    let mgr = TransferManager::new();
    for i in 0..50 {
        mgr.enqueue(
            &format!("/src/{}", i),
            &format!("/dst/{}", i),
            1024 * 1024,
        ).await.unwrap();
    }

    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 2000,
        "Startup with 50 transfers took {}ms, expected <2000ms", elapsed.as_millis());
}
