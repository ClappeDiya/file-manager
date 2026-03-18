//! Master Password module for session-based app-wide encryption of saved credentials.
//!
//! Provides:
//! - Argon2id-based key derivation from user's master password
//! - AES-256-GCM encryption/decryption of connection credentials
//! - Verification hash stored in SQLite config (never the password itself)
//! - Session-based unlock: derived key cached in memory until lock
//!
//! # Security properties
//! - Password never stored; only a SHA-256 hash of the derived key is persisted
//! - Derived key zeroized from memory on lock
//! - Uses existing KDF (Argon2id) and encryption (AES-256-GCM) primitives
//! - Constant-time password verification to prevent timing attacks

use std::sync::Arc;
use tokio::sync::RwLock;
use zeroize::Zeroize;

use crate::core::error::AppError;
use crate::security::encryption::{self, EncryptedData, EncryptionKey};
use crate::security::kdf::{self, KdfParams, Password, Salt};

/// Config keys used in the SQLite `config` table.
const CONFIG_KEY_VERIFICATION_HASH: &str = "master_password_verification_hash";
const CONFIG_KEY_SALT: &str = "master_password_salt";
const CONFIG_KEY_KDF_PARAMS: &str = "master_password_kdf_params";

/// AAD context for credential encryption, binding ciphertexts to our domain.
const CREDENTIAL_AAD: &[u8] = b"ufop-master-password-credential-v1";

/// In-memory state for the master password session.
pub struct MasterPasswordManager {
    /// The derived encryption key, present only while the app is unlocked.
    cached_key: Arc<RwLock<Option<EncryptionKey>>>,
}

impl MasterPasswordManager {
    pub fn new() -> Self {
        Self {
            cached_key: Arc::new(RwLock::new(None)),
        }
    }

    // ──────────────────────────────────────────────
    // Setup & Verification
    // ──────────────────────────────────────────────

    /// Check if a master password has been configured (verification hash exists in config).
    pub async fn is_master_password_set(
        &self,
        repo: &crate::storage::Repository,
    ) -> Result<bool, AppError> {
        use crate::core::traits::StorageOperations;
        let hash = StorageOperations::get_config(repo, CONFIG_KEY_VERIFICATION_HASH).await?;
        Ok(hash.is_some())
    }

    /// Set (create) the master password for the first time.
    ///
    /// Derives a key via Argon2id, stores the verification hash and salt in config,
    /// and caches the derived key in memory (app is now unlocked).
    pub async fn set_master_password(
        &self,
        repo: &crate::storage::Repository,
        password: String,
    ) -> Result<(), AppError> {
        use crate::core::traits::StorageOperations;

        // Reject if already set
        if self.is_master_password_set(repo).await? {
            return Err(AppError::encryption(
                "Master password already set",
                "A master password is already configured. Use change_master_password to update it.",
            ));
        }

        if password.len() < 8 {
            return Err(AppError::encryption(
                "Password too short",
                "Master password must be at least 8 characters.",
            ));
        }

        let salt = Salt::generate();
        let params = KdfParams::default();
        let pwd = Password::new(password);

        let key = kdf::derive_key(&pwd, &salt, &params)?;
        let verification_hash = kdf::compute_verification_hash(&key);

        // Persist salt, params, and verification hash
        StorageOperations::set_config(
            repo,
            CONFIG_KEY_SALT,
            &serde_json::to_string(&salt)?,
        )
        .await?;
        StorageOperations::set_config(
            repo,
            CONFIG_KEY_KDF_PARAMS,
            &serde_json::to_string(&params)?,
        )
        .await?;
        StorageOperations::set_config(
            repo,
            CONFIG_KEY_VERIFICATION_HASH,
            &hex::encode(verification_hash),
        )
        .await?;

        // Cache key in memory — app is unlocked
        {
            let mut cached = self.cached_key.write().await;
            *cached = Some(key);
        }

        tracing::info!("Master password set successfully");
        Ok(())
    }

    /// Verify a master password against the stored verification hash.
    /// Does NOT cache the key — use `unlock_app` for that.
    pub async fn verify_master_password(
        &self,
        repo: &crate::storage::Repository,
        password: String,
    ) -> Result<bool, AppError> {
        let (salt, params, expected_hash) = self.load_stored_params(repo).await?;
        let pwd = Password::new(password);
        match kdf::verify_password(&pwd, &salt, &params, &expected_hash) {
            Ok(_) => Ok(true),
            Err(_) => Ok(false),
        }
    }

    /// Change the master password. Re-encrypts nothing here (credentials are
    /// re-encrypted by the caller if needed, since we don't own the credential store).
    ///
    /// Steps:
    /// 1. Verify old password
    /// 2. Derive new key from new password with fresh salt
    /// 3. Store new verification hash, salt
    /// 4. Cache new key
    ///
    /// Returns the old key and new key so callers can re-encrypt credentials.
    pub async fn change_master_password(
        &self,
        repo: &crate::storage::Repository,
        old_password: String,
        new_password: String,
    ) -> Result<(EncryptionKey, EncryptionKey), AppError> {
        use crate::core::traits::StorageOperations;

        if new_password.len() < 8 {
            return Err(AppError::encryption(
                "New password too short",
                "Master password must be at least 8 characters.",
            ));
        }

        // Verify old password and get old key
        let (old_salt, old_params, old_hash) = self.load_stored_params(repo).await?;
        let old_pwd = Password::new(old_password);
        let old_key = kdf::verify_password(&old_pwd, &old_salt, &old_params, &old_hash)?;

        // Derive new key
        let new_salt = Salt::generate();
        let new_params = KdfParams::default();
        let new_pwd = Password::new(new_password);
        let new_key = kdf::derive_key(&new_pwd, &new_salt, &new_params)?;
        let new_hash = kdf::compute_verification_hash(&new_key);

        // Persist new salt, params, hash
        StorageOperations::set_config(
            repo,
            CONFIG_KEY_SALT,
            &serde_json::to_string(&new_salt)?,
        )
        .await?;
        StorageOperations::set_config(
            repo,
            CONFIG_KEY_KDF_PARAMS,
            &serde_json::to_string(&new_params)?,
        )
        .await?;
        StorageOperations::set_config(
            repo,
            CONFIG_KEY_VERIFICATION_HASH,
            &hex::encode(new_hash),
        )
        .await?;

        // Cache new key
        {
            let mut cached = self.cached_key.write().await;
            *cached = Some(new_key.clone());
        }

        tracing::info!("Master password changed successfully");
        Ok((old_key, new_key))
    }

    // ──────────────────────────────────────────────
    // Session Lock / Unlock
    // ──────────────────────────────────────────────

    /// Lock the app — clear the cached key from memory.
    pub async fn lock_app(&self) {
        let mut cached = self.cached_key.write().await;
        if let Some(ref mut key) = *cached {
            // EncryptionKey implements ZeroizeOnDrop, but we also drop explicitly
            let _ = key;
        }
        *cached = None;
        tracing::info!("App locked — master key cleared from memory");
    }

    /// Unlock the app — verify the password and cache the derived key.
    pub async fn unlock_app(
        &self,
        repo: &crate::storage::Repository,
        password: String,
    ) -> Result<bool, AppError> {
        let (salt, params, expected_hash) = self.load_stored_params(repo).await?;
        let pwd = Password::new(password);

        match kdf::verify_password(&pwd, &salt, &params, &expected_hash) {
            Ok(key) => {
                let mut cached = self.cached_key.write().await;
                *cached = Some(key);
                tracing::info!("App unlocked");
                Ok(true)
            }
            Err(_) => Ok(false),
        }
    }

    /// Check if the app is currently unlocked (key is cached).
    pub async fn is_unlocked(&self) -> bool {
        let cached = self.cached_key.read().await;
        cached.is_some()
    }

    // ──────────────────────────────────────────────
    // Credential Encryption / Decryption
    // ──────────────────────────────────────────────

    /// Encrypt a credential string using the cached master key.
    /// Returns a base64-encoded string of [nonce || ciphertext || tag].
    pub async fn encrypt_credential(&self, plaintext: &str) -> Result<String, AppError> {
        let cached = self.cached_key.read().await;
        let key = cached.as_ref().ok_or_else(|| {
            AppError::encryption(
                "App is locked",
                "Unlock the app with your master password before encrypting credentials.",
            )
        })?;

        let encrypted = encryption::encrypt(key, plaintext.as_bytes(), Some(CREDENTIAL_AAD))?;
        let bytes = encrypted.to_bytes();
        Ok(base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            &bytes,
        ))
    }

    /// Decrypt a credential string using the cached master key.
    /// Expects a base64-encoded string of [nonce || ciphertext || tag].
    pub async fn decrypt_credential(&self, ciphertext: &str) -> Result<String, AppError> {
        let cached = self.cached_key.read().await;
        let key = cached.as_ref().ok_or_else(|| {
            AppError::encryption(
                "App is locked",
                "Unlock the app with your master password before decrypting credentials.",
            )
        })?;

        let bytes = base64::Engine::decode(
            &base64::engine::general_purpose::STANDARD,
            ciphertext,
        )
        .map_err(|e| {
            AppError::encryption(
                "Invalid credential data",
                format!("The encrypted credential is not valid base64: {e}"),
            )
        })?;

        let encrypted = EncryptedData::from_bytes(&bytes)?;
        let mut plaintext_bytes = encryption::decrypt(key, &encrypted, Some(CREDENTIAL_AAD))?;
        let result = String::from_utf8(plaintext_bytes.clone()).map_err(|_| {
            AppError::encryption(
                "Decryption produced invalid data",
                "The decrypted credential is not valid UTF-8. It may be corrupted.",
            )
        })?;
        plaintext_bytes.zeroize();
        Ok(result)
    }

    // ──────────────────────────────────────────────
    // Internal helpers
    // ──────────────────────────────────────────────

    /// Load salt, KDF params, and verification hash from the config store.
    async fn load_stored_params(
        &self,
        repo: &crate::storage::Repository,
    ) -> Result<(Salt, KdfParams, [u8; 32]), AppError> {
        use crate::core::traits::StorageOperations;

        let salt_json = StorageOperations::get_config(repo, CONFIG_KEY_SALT)
            .await?
            .ok_or_else(|| {
                AppError::encryption(
                    "No master password configured",
                    "Set a master password first before trying to verify or unlock.",
                )
            })?;

        let params_json = StorageOperations::get_config(repo, CONFIG_KEY_KDF_PARAMS)
            .await?
            .ok_or_else(|| {
                AppError::encryption(
                    "Missing KDF parameters",
                    "Master password configuration is corrupt. You may need to reset.",
                )
            })?;

        let hash_hex = StorageOperations::get_config(repo, CONFIG_KEY_VERIFICATION_HASH)
            .await?
            .ok_or_else(|| {
                AppError::encryption(
                    "Missing verification hash",
                    "Master password configuration is corrupt. You may need to reset.",
                )
            })?;

        let salt: Salt = serde_json::from_str(&salt_json)?;
        let params: KdfParams = serde_json::from_str(&params_json)?;

        let hash_bytes = hex::decode(&hash_hex).map_err(|e| {
            AppError::encryption(
                "Corrupt verification hash",
                format!("Failed to decode stored hash: {e}"),
            )
        })?;

        if hash_bytes.len() != 32 {
            return Err(AppError::encryption(
                "Invalid verification hash length",
                "The stored verification hash is corrupt. You may need to reset.",
            ));
        }

        let mut expected_hash = [0u8; 32];
        expected_hash.copy_from_slice(&hash_bytes);

        Ok((salt, params, expected_hash))
    }
}

impl Default for MasterPasswordManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Repository;

    async fn test_repo() -> Repository {
        Repository::open_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn test_initial_state_no_password() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;
        assert!(!mgr.is_master_password_set(&repo).await.unwrap());
        assert!(!mgr.is_unlocked().await);
    }

    #[tokio::test]
    async fn test_set_master_password() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        mgr.set_master_password(&repo, "my_secure_password_123".to_string())
            .await
            .unwrap();

        assert!(mgr.is_master_password_set(&repo).await.unwrap());
        assert!(mgr.is_unlocked().await);
    }

    #[tokio::test]
    async fn test_set_password_too_short() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        let result = mgr.set_master_password(&repo, "short".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_set_password_twice_fails() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        mgr.set_master_password(&repo, "first_password_123".to_string())
            .await
            .unwrap();

        let result = mgr
            .set_master_password(&repo, "second_password_123".to_string())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_verify_correct_password() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        mgr.set_master_password(&repo, "test_password_456".to_string())
            .await
            .unwrap();

        let ok = mgr
            .verify_master_password(&repo, "test_password_456".to_string())
            .await
            .unwrap();
        assert!(ok);
    }

    #[tokio::test]
    async fn test_verify_wrong_password() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        mgr.set_master_password(&repo, "correct_password_789".to_string())
            .await
            .unwrap();

        let ok = mgr
            .verify_master_password(&repo, "wrong_password".to_string())
            .await
            .unwrap();
        assert!(!ok);
    }

    #[tokio::test]
    async fn test_lock_unlock() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        mgr.set_master_password(&repo, "locktest_password_123".to_string())
            .await
            .unwrap();
        assert!(mgr.is_unlocked().await);

        mgr.lock_app().await;
        assert!(!mgr.is_unlocked().await);

        let ok = mgr
            .unlock_app(&repo, "locktest_password_123".to_string())
            .await
            .unwrap();
        assert!(ok);
        assert!(mgr.is_unlocked().await);
    }

    #[tokio::test]
    async fn test_unlock_wrong_password() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        mgr.set_master_password(&repo, "real_password_xyz".to_string())
            .await
            .unwrap();
        mgr.lock_app().await;

        let ok = mgr
            .unlock_app(&repo, "wrong_password".to_string())
            .await
            .unwrap();
        assert!(!ok);
        assert!(!mgr.is_unlocked().await);
    }

    #[tokio::test]
    async fn test_encrypt_decrypt_credential() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        mgr.set_master_password(&repo, "encrypt_test_password".to_string())
            .await
            .unwrap();

        let secret = "my_ssh_private_key_content";
        let encrypted = mgr.encrypt_credential(secret).await.unwrap();

        // Encrypted should be different from plaintext
        assert_ne!(encrypted, secret);

        let decrypted = mgr.decrypt_credential(&encrypted).await.unwrap();
        assert_eq!(decrypted, secret);
    }

    #[tokio::test]
    async fn test_encrypt_while_locked_fails() {
        let mgr = MasterPasswordManager::new();
        assert!(!mgr.is_unlocked().await);

        let result = mgr.encrypt_credential("secret").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_decrypt_while_locked_fails() {
        let mgr = MasterPasswordManager::new();
        assert!(!mgr.is_unlocked().await);

        let result = mgr.decrypt_credential("dGVzdA==").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_change_master_password() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        mgr.set_master_password(&repo, "original_password_123".to_string())
            .await
            .unwrap();

        // Encrypt something with old key
        let encrypted = mgr.encrypt_credential("my_secret_data").await.unwrap();

        // Change password
        let (_old_key, _new_key) = mgr
            .change_master_password(
                &repo,
                "original_password_123".to_string(),
                "new_password_456".to_string(),
            )
            .await
            .unwrap();

        // The old ciphertext should still be decryptable only if we re-encrypted
        // with the new key. Since we didn't re-encrypt here, it should fail
        // because the cached key is now the new one.
        let result = mgr.decrypt_credential(&encrypted).await;
        assert!(result.is_err());

        // Verify new password works for unlock
        mgr.lock_app().await;
        let ok = mgr
            .unlock_app(&repo, "new_password_456".to_string())
            .await
            .unwrap();
        assert!(ok);
    }

    #[tokio::test]
    async fn test_change_password_wrong_old() {
        let mgr = MasterPasswordManager::new();
        let repo = test_repo().await;

        mgr.set_master_password(&repo, "the_real_password_000".to_string())
            .await
            .unwrap();

        let result = mgr
            .change_master_password(
                &repo,
                "wrong_old_password".to_string(),
                "new_password_111".to_string(),
            )
            .await;
        assert!(result.is_err());
    }
}
