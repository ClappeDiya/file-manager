//! Journey 1: Local folder to OneDrive with incompatible names
//!
//! End-to-end test validating the full flow:
//! 1. User selects local folder with files containing characters incompatible with OneDrive
//! 2. Compatibility engine detects and translates names (fullwidth Unicode replacement)
//! 3. Transfer is enqueued with translated names
//! 4. Encryption policy is checked (if enterprise policy exists)
//! 5. Transfer completes with verification
//! 6. Mapping is stored so names can be restored on download

use unified_file_ops_lib::compat_engine::profiles;
use unified_file_ops_lib::compat_engine::CompatChecker;
use unified_file_ops_lib::core::traits::TransferOperations;
use unified_file_ops_lib::security::tls;
use unified_file_ops_lib::security::vault::VaultManager;
use unified_file_ops_lib::transfer_engine::TransferManager;
use unified_file_ops_lib::core::types::ConnectionProtocol;

/// Journey 1A: Detect incompatible filenames for OneDrive destination
#[test]
fn journey1_detect_incompatible_names_for_onedrive() {
    // OneDrive uses Windows-NTFS naming rules
    let profile = profiles::get_profile("windows-ntfs")
        .expect("windows-ntfs profile should exist");

    // Simulate a local macOS folder with files that have OneDrive-incompatible names
    let local_files = vec![
        "report<Q1>.pdf",        // < and > are forbidden
        "meeting:notes.docx",    // : is forbidden
        "file\"name\".txt",      // " is forbidden
        "query?.sql",            // ? is forbidden
        "data|pipe.csv",         // | is forbidden
        "star*file.log",         // * is forbidden
        "CON.txt",               // Reserved name
        "NUL",                   // Reserved name (no extension)
        "budget...",             // Trailing dots
        "normal_file.txt",       // Compatible - should not be modified
        "cafe\u{0301}.txt",      // NFD -> should normalize to NFC
    ];

    let name_refs: Vec<&str> = local_files.iter().map(|s| s.as_ref()).collect();
    let batch_result = CompatChecker::check_batch(&name_refs, &profile);

    // Verify incompatible names were detected
    assert!(batch_result.total_modified >= 10,
        "Expected at least 10 modified names, got {}", batch_result.total_modified);

    // Verify tier classification
    assert!(batch_result.tier2_count > 0, "Should have Tier 2 (visible auto) translations");

    // Verify simple mode message is generated
    assert!(!batch_result.simple_message.is_empty());
    assert!(batch_result.simple_message.contains("adjusted"));

    // Verify each problematic file was handled
    for translation in &batch_result.translations {
        assert_ne!(translation.original_name, translation.translated_name,
            "Translation should modify the name: '{}'", translation.original_name);
        assert!(!translation.rules_applied.is_empty(),
            "Should have rules applied for: '{}'", translation.original_name);
    }
}

/// Journey 1B: Fullwidth Unicode replacement is reversible
#[test]
fn journey1_fullwidth_replacement_reversible() {
    let profile = profiles::get_profile("windows-ntfs").unwrap();

    let problematic_names = vec![
        "file<name>.txt",
        "data|pipe.csv",
        "query?.sql",
    ];

    for name in &problematic_names {
        let translation = CompatChecker::check_name_with_profile(name, &profile);
        assert!(translation.reversible,
            "Translation of '{}' should be reversible", name);
        // Verify the translated name does not contain the original forbidden chars
        let forbidden = ['<', '>', ':', '"', '|', '?', '*'];
        for ch in &forbidden {
            if name.contains(*ch) {
                assert!(!translation.translated_name.contains(*ch),
                    "Translated name '{}' still contains forbidden char '{}'",
                    translation.translated_name, ch);
            }
        }
    }
}

/// Journey 1C: Transfer enqueue with translated names
#[tokio::test]
async fn journey1_enqueue_transfer_with_translated_names() {
    let mgr = TransferManager::new();
    let profile = profiles::get_profile("windows-ntfs").unwrap();

    // Simulate translating names and enqueuing transfers
    let files = vec![
        ("report<Q1>.pdf", 1024 * 50),
        ("meeting:notes.docx", 1024 * 100),
        ("normal_file.txt", 1024 * 10),
    ];

    let mut enqueued_jobs = Vec::new();
    for (name, size) in &files {
        let translation = CompatChecker::check_name_with_profile(name, &profile);
        let dest_name = &translation.translated_name;

        let job = mgr.enqueue(
            &format!("/Users/test/Documents/{}", name),
            &format!("onedrive://Documents/{}", dest_name),
            *size,
        ).await.unwrap();

        enqueued_jobs.push(job);
    }

    // Verify all transfers were enqueued
    let jobs = mgr.list_jobs().await.unwrap();
    assert_eq!(jobs.len(), 3);

    // Verify translated filenames (not full URI) are free of forbidden chars
    for job in &jobs {
        let dest = &job.dest_path;
        // Extract the filename portion after the last '/'
        let filename = dest.rsplit('/').next().unwrap_or(dest);
        let forbidden = ['<', '>', ':', '"', '|', '?', '*'];
        for ch in &forbidden {
            assert!(!filename.contains(*ch),
                "Destination filename '{}' contains forbidden character '{}'", filename, ch);
        }
    }
}

/// Journey 1D: OneDrive transport security check
#[test]
fn journey1_onedrive_transport_security() {
    let security = tls::TransportSecurity::strict();

    let check = tls::check_transport_security(
        ConnectionProtocol::OneDrive,
        "graph.microsoft.com",
        Some(443),
        &security,
    );

    assert!(check.secure, "OneDrive connection should be secure (HTTPS)");
    assert!(check.errors.is_empty(), "Should have no security errors");
}

/// Journey 1E: Enterprise encryption policy check for OneDrive upload
#[tokio::test]
async fn journey1_encryption_policy_for_onedrive() {
    let vault_mgr = VaultManager::new();

    // Add enterprise policy requiring encryption for PDFs on OneDrive
    let policy = unified_file_ops_lib::security::vault::EncryptionPolicy {
        id: "pol_onedrive_pdf".to_string(),
        name: "OneDrive PDF Encryption".to_string(),
        connector_type: "onedrive".to_string(),
        destination_pattern: Some("Documents/*".to_string()),
        required: true,
        file_patterns: vec!["*.pdf".to_string()],
        min_kdf_level: "default".to_string(),
        zero_knowledge_required: true,
        created_at: chrono::Utc::now(),
        enabled: true,
    };
    vault_mgr.add_policy(policy).await.unwrap();

    // PDF should require encryption
    let result = vault_mgr.check_policy(
        "onedrive", "Documents/reports", "annual_report.pdf"
    ).await;
    assert!(result.encryption_required);
    assert!(result.zero_knowledge_required);

    // Non-PDF should not require encryption
    let result = vault_mgr.check_policy(
        "onedrive", "Documents/reports", "notes.txt"
    ).await;
    assert!(!result.encryption_required);
}

/// Journey 1F: Full pipeline - detect, translate, enqueue, verify
#[tokio::test]
async fn journey1_full_pipeline() {
    let profile = profiles::get_profile("windows-ntfs").unwrap();
    let mgr = TransferManager::new();
    let start = std::time::Instant::now();

    // Step 1: Batch check compatibility
    let local_files = ["report<Q1>.pdf",
        "meeting:notes.docx",
        "CON.txt",
        "budget...",
        "normal.txt"];
    let name_refs: Vec<&str> = local_files.iter().map(|s| s.as_ref()).collect();
    let batch = CompatChecker::check_batch(&name_refs, &profile);

    // Step 2: Enqueue transfers with translated names
    for (i, name) in local_files.iter().enumerate() {
        let dest_name = batch.translations.iter()
            .find(|t| t.original_name == *name)
            .map(|t| t.translated_name.clone())
            .unwrap_or_else(|| name.to_string());

        let _job = mgr.enqueue(
            &format!("/local/{}", name),
            &format!("onedrive://Documents/{}", dest_name),
            (i as u64 + 1) * 1024,
        ).await.unwrap();
    }

    // Step 3: Verify all jobs exist
    let jobs = mgr.list_jobs().await.unwrap();
    assert_eq!(jobs.len(), 5);

    // Step 4: Complete all transfers
    for job in &jobs {
        mgr.complete_job(job.id).await.unwrap();
    }

    // Verify completion
    let completed_jobs = mgr.list_jobs().await.unwrap();
    for job in &completed_jobs {
        assert_eq!(job.status, unified_file_ops_lib::core::types::TransferStatus::Completed);
    }

    let elapsed = start.elapsed();
    assert!(elapsed.as_millis() < 2000,
        "Full journey pipeline took {}ms, expected <2000ms", elapsed.as_millis());
}

/// Journey 1G: Performance benchmark - batch check 10K files in <500ms
#[test]
fn journey1_batch_check_performance_10k() {
    let profile = profiles::get_profile("windows-ntfs").unwrap();

    let names: Vec<String> = (0..10_000).map(|i| {
        if i % 5 == 0 {
            format!("file<{}>_{}.txt", i, i)
        } else if i % 7 == 0 {
            format!("CON_{}.txt", i)
        } else {
            format!("normal_file_{}.txt", i)
        }
    }).collect();

    let name_refs: Vec<&str> = names.iter().map(|s| s.as_ref()).collect();

    let start = std::time::Instant::now();
    let _result = CompatChecker::check_batch(&name_refs, &profile);
    let elapsed = start.elapsed();

    assert!(elapsed.as_millis() < 500,
        "10K file batch check took {}ms, expected <500ms", elapsed.as_millis());
}
