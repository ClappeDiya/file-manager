//! Journey 3: Enterprise-governed external transfer with approval
//!
//! End-to-end test validating:
//! 1. Enterprise encryption policies enforce encryption for sensitive files
//! 2. Governance audit logging tracks all operations
//! 3. Transfer requires encryption for policy-matching files
//! 4. Zero-knowledge encryption before cloud upload
//! 5. Transport security enforcement (TLS 1.3)

use unified_file_ops_lib::core::traits::AuditOperations;
use unified_file_ops_lib::core::types::{AuditEvent, ConnectionProtocol};
use unified_file_ops_lib::governance::AuditLogger;
use unified_file_ops_lib::security::tls::{self, TransportSecurity, TlsVersion};
use unified_file_ops_lib::security::vault::{EncryptionPolicy, VaultManager};
use chrono::Utc;
use uuid::Uuid;

/// Journey 3A: Enterprise encryption policy enforcement
#[tokio::test]
async fn journey3_encryption_policy_enforcement() {
    let vault_mgr = VaultManager::new();

    // Enterprise policy: all S3 uploads to sensitive-data/* must be encrypted
    let s3_policy = EncryptionPolicy {
        id: "pol_s3_sensitive".to_string(),
        name: "S3 Sensitive Data Encryption".to_string(),
        connector_type: "s3".to_string(),
        destination_pattern: Some("sensitive-data/*".to_string()),
        required: true,
        file_patterns: vec!["*.pdf".to_string(), "*.docx".to_string(), "*.xlsx".to_string()],
        min_kdf_level: "default".to_string(),
        zero_knowledge_required: true,
        created_at: Utc::now(),
        enabled: true,
    };

    // Enterprise policy: all SFTP uploads require encryption
    let sftp_policy = EncryptionPolicy {
        id: "pol_sftp_all".to_string(),
        name: "SFTP All Files Encryption".to_string(),
        connector_type: "sftp".to_string(),
        destination_pattern: None,
        required: true,
        file_patterns: vec![],
        min_kdf_level: "default".to_string(),
        zero_knowledge_required: false,
        created_at: Utc::now(),
        enabled: true,
    };

    vault_mgr.add_policy(s3_policy).await.unwrap();
    vault_mgr.add_policy(sftp_policy).await.unwrap();

    // Check: S3 PDF upload to sensitive-data should require encryption
    let result = vault_mgr.check_policy(
        "s3", "sensitive-data/reports", "financial_report.pdf"
    ).await;
    assert!(result.encryption_required, "S3 sensitive PDF should require encryption");
    assert!(result.zero_knowledge_required, "S3 sensitive should require zero-knowledge");
    assert_eq!(result.policy_name, Some("S3 Sensitive Data Encryption".to_string()));

    // Check: S3 image upload to sensitive-data should NOT match (wrong file type)
    let result = vault_mgr.check_policy(
        "s3", "sensitive-data/images", "photo.jpg"
    ).await;
    assert!(!result.encryption_required, "S3 JPG should not require encryption by this policy");

    // Check: S3 upload to public-data should NOT match (wrong path)
    let result = vault_mgr.check_policy(
        "s3", "public-data/docs", "report.pdf"
    ).await;
    assert!(!result.encryption_required, "S3 public path should not require encryption");

    // Check: SFTP upload should always require encryption
    let result = vault_mgr.check_policy(
        "sftp", "/any/path", "any_file.txt"
    ).await;
    assert!(result.encryption_required, "SFTP should always require encryption");
    assert!(!result.zero_knowledge_required, "SFTP policy doesn't require zero-knowledge");
}

/// Journey 3B: Audit logging for all operations
#[tokio::test]
async fn journey3_audit_logging() {
    let logger = AuditLogger::new();

    // Log a series of enterprise operations
    let events = vec![
        ("file_transfer_initiated", "admin@company.com", "/data/report.pdf", "Transfer to S3 initiated"),
        ("encryption_policy_check", "system", "/data/report.pdf", "Policy pol_s3_sensitive matched, encryption required"),
        ("file_encrypted", "system", "/data/report.pdf", "AES-256-GCM encryption applied"),
        ("transfer_completed", "admin@company.com", "s3://sensitive-data/report.pdf", "Transfer completed with verification"),
        ("audit_export", "compliance@company.com", "/audit/log", "Audit log exported for Q1 review"),
    ];

    for (event_type, actor, resource, details) in &events {
        let event = AuditEvent {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            event_type: event_type.to_string(),
            actor: actor.to_string(),
            resource: resource.to_string(),
            details: details.to_string(),
        };
        logger.log_event(event).await.unwrap();
    }

    // Query all events
    let all_events = logger.query_events(10, 0).await.unwrap();
    assert_eq!(all_events.len(), 5);

    // Query with pagination
    let page1 = logger.query_events(2, 0).await.unwrap();
    assert_eq!(page1.len(), 2);

    let page2 = logger.query_events(2, 2).await.unwrap();
    assert_eq!(page2.len(), 2);

    let page3 = logger.query_events(2, 4).await.unwrap();
    assert_eq!(page3.len(), 1);
}

/// Journey 3C: Zero-knowledge encryption before cloud upload
#[tokio::test]
async fn journey3_zero_knowledge_cloud_upload() {
    let vault_mgr = VaultManager::new();
    let dir = tempfile::tempdir().unwrap();
    let vault_path = dir.path().join("enterprise_vault");

    // Create enterprise vault
    let info = vault_mgr.create_vault(
        &vault_path.to_string_lossy(),
        "Enterprise Vault",
        "enterprise_pass_123!".to_string(),
        vec![],
        vec![],
        false,
    ).await.unwrap();

    // Simulate file content that needs to be encrypted before upload
    let sensitive_data = b"CONFIDENTIAL: Q1 Financial Report - Revenue: $10M, Expenses: $8M";

    // Encrypt for upload (zero-knowledge)
    let encrypted = vault_mgr.encrypt_for_upload(
        &info.id, sensitive_data, "q1_financial.pdf"
    ).await.unwrap();

    // Encrypted data should be different from plaintext
    assert_ne!(encrypted, sensitive_data.to_vec());
    // Encrypted data should be larger (nonce + tag overhead)
    assert!(encrypted.len() > sensitive_data.len());

    // Verify it can be decrypted back
    let decrypted = vault_mgr.decrypt_from_download(
        &info.id, &encrypted, "q1_financial.pdf"
    ).await.unwrap();
    assert_eq!(decrypted, sensitive_data.to_vec());
}

/// Journey 3D: Transport security enforcement for enterprise connections
#[test]
fn journey3_transport_security_enforcement() {
    let strict_security = TransportSecurity::strict();

    // Enterprise S3 connection: must be HTTPS
    let s3_check = tls::check_transport_security(
        ConnectionProtocol::S3,
        "s3.amazonaws.com",
        Some(443),
        &strict_security,
    );
    assert!(s3_check.secure, "S3 should be secure");
    assert_eq!(strict_security.min_tls_version, TlsVersion::Tls13);

    // FTP should be auto-upgraded to FTPS
    let ftp_check = tls::check_transport_security(
        ConnectionProtocol::Ftp,
        "ftp.company.com",
        Some(21),
        &strict_security,
    );
    assert!(ftp_check.upgraded, "FTP should be auto-upgraded to FTPS");
    assert_eq!(ftp_check.effective_protocol, "ftps");

    // WebDAV over HTTP should be upgraded
    let webdav_check = tls::check_transport_security(
        ConnectionProtocol::WebDav,
        "http://dav.company.com",
        Some(80),
        &strict_security,
    );
    assert!(webdav_check.upgraded, "WebDAV HTTP should be upgraded to HTTPS");
}

/// Journey 3E: HTTPS enforcement for cloud API URLs
#[test]
fn journey3_https_enforcement() {
    // Valid HTTPS URL
    assert_eq!(
        tls::enforce_https("https://api.company.com/v2").unwrap(),
        "https://api.company.com/v2"
    );

    // HTTP auto-upgraded to HTTPS
    assert_eq!(
        tls::enforce_https("http://api.company.com/v2").unwrap(),
        "https://api.company.com/v2"
    );

    // No scheme defaults to HTTPS
    assert_eq!(
        tls::enforce_https("api.company.com/v2").unwrap(),
        "https://api.company.com/v2"
    );

    // Non-HTTP scheme rejected
    assert!(tls::enforce_https("ftp://files.company.com").is_err());
}

/// Journey 3F: Policy CRUD operations
#[tokio::test]
async fn journey3_policy_crud() {
    let vault_mgr = VaultManager::new();

    // Create
    let policy = EncryptionPolicy {
        id: "pol_test".to_string(),
        name: "Test Policy".to_string(),
        connector_type: "all".to_string(),
        destination_pattern: None,
        required: true,
        file_patterns: vec!["*.secret".to_string()],
        min_kdf_level: "default".to_string(),
        zero_knowledge_required: false,
        created_at: Utc::now(),
        enabled: true,
    };
    vault_mgr.add_policy(policy).await.unwrap();

    // List
    let policies = vault_mgr.list_policies().await;
    assert_eq!(policies.len(), 1);
    assert_eq!(policies[0].id, "pol_test");

    // Remove
    vault_mgr.remove_policy("pol_test").await.unwrap();
    let policies = vault_mgr.list_policies().await;
    assert!(policies.is_empty());

    // Remove non-existent should error
    let result = vault_mgr.remove_policy("nonexistent").await;
    assert!(result.is_err());
}

/// Journey 3G: Disabled policy is skipped
#[tokio::test]
async fn journey3_disabled_policy_skipped() {
    let vault_mgr = VaultManager::new();

    let policy = EncryptionPolicy {
        id: "pol_disabled".to_string(),
        name: "Disabled Policy".to_string(),
        connector_type: "all".to_string(),
        destination_pattern: None,
        required: true,
        file_patterns: vec![],
        min_kdf_level: "default".to_string(),
        zero_knowledge_required: true,
        created_at: Utc::now(),
        enabled: false, // Disabled!
    };
    vault_mgr.add_policy(policy).await.unwrap();

    let result = vault_mgr.check_policy("s3", "/any", "file.txt").await;
    assert!(!result.encryption_required, "Disabled policy should not enforce encryption");
}
