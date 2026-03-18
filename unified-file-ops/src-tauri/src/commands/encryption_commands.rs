//! Tauri IPC commands for encryption, vault management, and TLS enforcement (T-047).
//!
//! Provides frontend-accessible commands for:
//! - Creating, unlocking, and locking encrypted vaults
//! - Encrypting/decrypting files within vaults
//! - Zero-knowledge encrypt-before-upload
//! - Transport security checks
//! - Enterprise encryption policy management
//! - KDF parameter configuration

use crate::core::error::AppError;
use crate::core::types::ConnectionProtocol;
use crate::security::tls::{self, TransportSecurity, TransportSecurityCheck};
use crate::security::vault::{EncryptionPolicy, PolicyCheckResult, VaultInfo, VaultManager};
use tauri::State;

// ──────────────────────────────────────────────
// Vault Management
// ──────────────────────────────────────────────

/// Create a new encrypted vault at the specified path.
#[tauri::command]
pub async fn create_vault(
    path: String,
    name: String,
    password: String,
    encrypt_patterns: Option<Vec<String>>,
    exclude_patterns: Option<Vec<String>>,
    cryptomator_compat: Option<bool>,
    vault_mgr: State<'_, VaultManager>,
) -> Result<VaultInfo, AppError> {
    if password.len() < 8 {
        return Err(AppError::encryption(
            "Password too short",
            "Vault passwords must be at least 8 characters long.",
        ));
    }

    vault_mgr
        .create_vault(
            &path,
            &name,
            password,
            encrypt_patterns.unwrap_or_default(),
            exclude_patterns.unwrap_or_default(),
            cryptomator_compat.unwrap_or(false),
        )
        .await
}

/// Unlock an existing encrypted vault with a password.
#[tauri::command]
pub async fn unlock_vault(
    path: String,
    password: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<VaultInfo, AppError> {
    vault_mgr.unlock_vault(&path, password).await
}

/// Lock an unlocked vault (removes key from memory).
#[tauri::command]
pub async fn lock_vault(
    vault_id: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&vault_id, "vault")?;
    vault_mgr.lock_vault(&id).await
}

/// List vaults at the specified paths.
#[tauri::command]
pub async fn list_vaults(
    scan_paths: Vec<String>,
    vault_mgr: State<'_, VaultManager>,
) -> Result<Vec<VaultInfo>, AppError> {
    vault_mgr.list_vaults(&scan_paths).await
}

/// Check if a vault is currently unlocked.
#[tauri::command]
pub async fn is_vault_unlocked(
    vault_id: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<bool, AppError> {
    let id = parse_uuid(&vault_id, "vault")?;
    Ok(vault_mgr.is_unlocked(&id).await)
}

/// Get the number of currently unlocked vaults.
#[tauri::command]
pub async fn get_unlocked_vault_count(
    vault_mgr: State<'_, VaultManager>,
) -> Result<usize, AppError> {
    Ok(vault_mgr.unlocked_count().await)
}

// ──────────────────────────────────────────────
// File Encryption/Decryption
// ──────────────────────────────────────────────

/// Encrypt a file into a vault.
#[tauri::command]
pub async fn vault_encrypt_file(
    vault_id: String,
    source_path: String,
    relative_path: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<String, AppError> {
    let id = parse_uuid(&vault_id, "vault")?;
    vault_mgr.encrypt_file(&id, &source_path, &relative_path).await
}

/// Decrypt a file from a vault.
#[tauri::command]
pub async fn vault_decrypt_file(
    vault_id: String,
    relative_path: String,
    dest_path: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<(), AppError> {
    let id = parse_uuid(&vault_id, "vault")?;
    vault_mgr
        .decrypt_file(&id, &relative_path, &dest_path)
        .await
}

/// Encrypt data for zero-knowledge cloud upload.
///
/// The data is encrypted in memory and the encrypted bytes are returned
/// as a base64-encoded string for the frontend to upload.
#[tauri::command]
pub async fn encrypt_for_upload(
    vault_id: String,
    data_base64: String,
    file_name: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<String, AppError> {
    let id = parse_uuid(&vault_id, "vault")?;

    let plaintext = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &data_base64,
    )
    .map_err(|e| {
        AppError::encryption(
            "Invalid base64 data",
            format!("The data is not valid base64: {e}"),
        )
    })?;

    let encrypted = vault_mgr
        .encrypt_for_upload(&id, &plaintext, &file_name)
        .await?;

    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &encrypted,
    ))
}

/// Decrypt data downloaded from cloud (zero-knowledge mode).
#[tauri::command]
pub async fn decrypt_from_download(
    vault_id: String,
    data_base64: String,
    file_name: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<String, AppError> {
    let id = parse_uuid(&vault_id, "vault")?;

    let encrypted = base64::Engine::decode(
        &base64::engine::general_purpose::STANDARD,
        &data_base64,
    )
    .map_err(|e| {
        AppError::encryption(
            "Invalid base64 data",
            format!("The encrypted data is not valid base64: {e}"),
        )
    })?;

    let plaintext = vault_mgr
        .decrypt_from_download(&id, &encrypted, &file_name)
        .await?;

    Ok(base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &plaintext,
    ))
}

// ──────────────────────────────────────────────
// Password Management
// ──────────────────────────────────────────────

/// Change the password for an unlocked vault.
#[tauri::command]
pub async fn change_vault_password(
    vault_id: String,
    old_password: String,
    new_password: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<(), AppError> {
    if new_password.len() < 8 {
        return Err(AppError::encryption(
            "New password too short",
            "Vault passwords must be at least 8 characters long.",
        ));
    }

    let id = parse_uuid(&vault_id, "vault")?;
    vault_mgr
        .change_password(&id, old_password, new_password)
        .await
}

// ──────────────────────────────────────────────
// Transport Security
// ──────────────────────────────────────────────

/// Check transport security for a connection.
///
/// Returns a security assessment including whether the connection is secure,
/// any warnings, and recommended actions.
#[tauri::command]
pub async fn check_transport_security(
    protocol: String,
    host: String,
    port: Option<u16>,
) -> Result<TransportSecurityCheck, AppError> {
    let conn_protocol = ConnectionProtocol::from_str_lossy(&protocol);
    let security = tls::recommended_security(conn_protocol);
    Ok(tls::check_transport_security(
        conn_protocol,
        &host,
        port,
        &security,
    ))
}

/// Check transport security with custom security settings.
#[tauri::command]
pub async fn check_transport_security_custom(
    protocol: String,
    host: String,
    port: Option<u16>,
    security: TransportSecurity,
) -> Result<TransportSecurityCheck, AppError> {
    let conn_protocol = ConnectionProtocol::from_str_lossy(&protocol);
    Ok(tls::check_transport_security(
        conn_protocol,
        &host,
        port,
        &security,
    ))
}

/// Get recommended transport security settings for a protocol.
#[tauri::command]
pub async fn get_recommended_security(
    protocol: String,
) -> Result<TransportSecurity, AppError> {
    let conn_protocol = ConnectionProtocol::from_str_lossy(&protocol);
    Ok(tls::recommended_security(conn_protocol))
}

/// Enforce HTTPS on a URL (auto-upgrade HTTP to HTTPS).
#[tauri::command]
pub async fn enforce_https_url(url: String) -> Result<String, AppError> {
    tls::enforce_https(&url)
}

// ──────────────────────────────────────────────
// Enterprise Encryption Policies
// ──────────────────────────────────────────────

/// Add an enterprise encryption policy.
#[tauri::command]
pub async fn add_encryption_policy(
    policy: EncryptionPolicy,
    vault_mgr: State<'_, VaultManager>,
) -> Result<(), AppError> {
    vault_mgr.add_policy(policy).await
}

/// List all enterprise encryption policies.
#[tauri::command]
pub async fn list_encryption_policies(
    vault_mgr: State<'_, VaultManager>,
) -> Result<Vec<EncryptionPolicy>, AppError> {
    Ok(vault_mgr.list_policies().await)
}

/// Remove an enterprise encryption policy.
#[tauri::command]
pub async fn remove_encryption_policy(
    policy_id: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<(), AppError> {
    vault_mgr.remove_policy(&policy_id).await
}

/// Check if encryption is required for a file transfer.
#[tauri::command]
pub async fn check_encryption_policy(
    connector_type: String,
    destination: String,
    file_name: String,
    vault_mgr: State<'_, VaultManager>,
) -> Result<PolicyCheckResult, AppError> {
    Ok(vault_mgr
        .check_policy(&connector_type, &destination, &file_name)
        .await)
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

/// Parse a UUID string, returning a user-friendly error.
fn parse_uuid(s: &str, entity: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(s).map_err(|_| AppError::encryption(
        format!("Invalid {entity} ID format"),
        format!("{} IDs must be valid UUIDs.", capitalize(entity)),
    ))
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        None => String::new(),
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
    }
}
