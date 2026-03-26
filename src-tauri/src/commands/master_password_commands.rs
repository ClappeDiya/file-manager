//! Tauri IPC commands for Master Password management.
//!
//! Provides frontend-accessible commands for:
//! - Setting, verifying, and changing the master password
//! - Locking and unlocking the app
//! - Encrypting and decrypting saved connection credentials

use crate::core::error::AppError;
use crate::security::audit::SecurityAudit;
use crate::security::MasterPasswordManager;
use crate::storage::Repository;
use tauri::State;

/// Check if a master password has been configured.
#[tauri::command]
pub async fn is_master_password_set(
    repo: State<'_, Repository>,
    master_pw_mgr: State<'_, MasterPasswordManager>,
) -> Result<bool, AppError> {
    master_pw_mgr.is_master_password_set(repo.inner()).await
}

/// Set (create) the master password for the first time.
///
/// Requires: no master password currently set, password >= 8 chars.
/// After success the app is automatically unlocked.
#[tauri::command]
pub async fn set_master_password(
    password: String,
    repo: State<'_, Repository>,
    master_pw_mgr: State<'_, MasterPasswordManager>,
) -> Result<(), AppError> {
    let result = master_pw_mgr
        .set_master_password(repo.inner(), password)
        .await;
    if result.is_ok() {
        SecurityAudit::log(repo.inner(), "auth.master_password_set", "user", "master_password", "Initial master password configured").await;
    }
    result
}

/// Verify a master password without unlocking the app.
///
/// Returns `true` if the password matches, `false` otherwise.
#[tauri::command]
pub async fn verify_master_password(
    password: String,
    repo: State<'_, Repository>,
    master_pw_mgr: State<'_, MasterPasswordManager>,
) -> Result<bool, AppError> {
    let result = master_pw_mgr
        .verify_master_password(repo.inner(), password)
        .await?;
    SecurityAudit::log_auth_attempt(repo.inner(), result).await;
    Ok(result)
}

/// Change the master password.
///
/// Verifies the old password, derives a new key, and updates stored params.
/// Returns success; the caller (another agent) is responsible for re-encrypting
/// credentials with the new key via encrypt_credential / decrypt_credential.
#[tauri::command]
pub async fn change_master_password(
    old_password: String,
    new_password: String,
    repo: State<'_, Repository>,
    master_pw_mgr: State<'_, MasterPasswordManager>,
) -> Result<(), AppError> {
    let result = master_pw_mgr
        .change_master_password(repo.inner(), old_password, new_password)
        .await
        .map(|_| ());
    if result.is_ok() {
        SecurityAudit::log_password_change(repo.inner()).await;
    }
    result
}

/// Lock the app — clear the cached encryption key from memory.
#[tauri::command]
pub async fn master_lock_app(
    repo: State<'_, Repository>,
    master_pw_mgr: State<'_, MasterPasswordManager>,
) -> Result<(), AppError> {
    master_pw_mgr.lock_app().await;
    SecurityAudit::log_lock_event(repo.inner(), true).await;
    Ok(())
}

/// Unlock the app — verify the password and cache the derived key.
///
/// Returns `true` if the password is correct and the app is now unlocked.
#[tauri::command]
pub async fn master_unlock_app(
    password: String,
    repo: State<'_, Repository>,
    master_pw_mgr: State<'_, MasterPasswordManager>,
) -> Result<bool, AppError> {
    let result = master_pw_mgr.unlock_app(repo.inner(), password).await?;
    SecurityAudit::log_auth_attempt(repo.inner(), result).await;
    if result {
        SecurityAudit::log_lock_event(repo.inner(), false).await;
    }
    Ok(result)
}

/// Check if the app is currently unlocked.
#[tauri::command]
pub async fn is_app_unlocked(
    master_pw_mgr: State<'_, MasterPasswordManager>,
) -> Result<bool, AppError> {
    Ok(master_pw_mgr.is_unlocked().await)
}

/// Encrypt a credential string with the cached master key.
///
/// Returns a base64-encoded ciphertext. Fails if the app is locked.
#[tauri::command]
pub async fn master_encrypt_credential(
    plaintext: String,
    master_pw_mgr: State<'_, MasterPasswordManager>,
) -> Result<String, AppError> {
    master_pw_mgr.encrypt_credential(&plaintext).await
}

/// Decrypt a credential string with the cached master key.
///
/// Expects a base64-encoded ciphertext. Fails if the app is locked.
#[tauri::command]
pub async fn master_decrypt_credential(
    ciphertext: String,
    master_pw_mgr: State<'_, MasterPasswordManager>,
) -> Result<String, AppError> {
    master_pw_mgr.decrypt_credential(&ciphertext).await
}
