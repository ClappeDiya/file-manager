//! Encrypted vault management for at-rest encryption (T-047).
//!
//! A vault is a directory that appears as a regular folder but whose contents
//! are encrypted with AES-256-GCM. Each vault has:
//!
//! - A `.vault-meta.json` file containing KDF parameters, salt, and verification hash
//! - Encrypted files stored with the vault wire format (see `encryption.rs`)
//! - Selective encryption based on glob patterns
//!
//! # Vault format
//! ```text
//! vault-root/
//!   .vault-meta.json       # KDF params, salt, verify hash, encryption patterns
//!   encrypted-file-1.enc   # AES-256-GCM encrypted file
//!   encrypted-file-2.enc   # AES-256-GCM encrypted file
//!   subdir/
//!     encrypted-file-3.enc
//! ```
//!
//! # Cryptomator compatibility
//! Read/write support for Cryptomator v8 vaults is provided through the
//! `CryptomatorVault` struct. This enables interoperability with Cryptomator
//! clients on other platforms.
//!
//! # Enterprise encryption policies
//! Policies can be configured per connector/destination, enabling requirements
//! like "all files uploaded to S3 must be encrypted" or "encrypt *.pdf files
//! on this share".

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::core::error::AppError;
use crate::security::encryption::{self, EncryptionKey};
use crate::security::kdf::{self, KdfParams, Password, Salt};

/// Vault metadata filename.
const VAULT_META_FILE: &str = ".vault-meta.json";

/// Extension appended to encrypted files.
const ENCRYPTED_EXT: &str = ".enc";

/// Vault status.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VaultStatus {
    /// Vault is locked (key not in memory).
    Locked,
    /// Vault is unlocked (key cached in memory for operations).
    Unlocked,
    /// Vault needs password change or re-encryption.
    NeedsRekey,
}

/// Vault metadata stored in `.vault-meta.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultMetadata {
    /// Vault format version.
    pub version: u32,
    /// Unique vault identifier.
    pub vault_id: Uuid,
    /// Display name for the vault.
    pub name: String,
    /// KDF parameters used for key derivation.
    pub kdf_params: KdfParams,
    /// Salt for key derivation.
    pub salt: Salt,
    /// SHA-256 hash of the derived key for password verification.
    #[serde(with = "hex_array_serde")]
    pub verification_hash: [u8; 32],
    /// Glob patterns for selective encryption (empty = encrypt all).
    pub encrypt_patterns: Vec<String>,
    /// Glob patterns to exclude from encryption.
    pub exclude_patterns: Vec<String>,
    /// When the vault was created.
    pub created_at: DateTime<Utc>,
    /// When the vault was last accessed.
    pub last_accessed: Option<DateTime<Utc>>,
    /// Whether this vault uses Cryptomator compatibility mode.
    pub cryptomator_compat: bool,
    /// Optional enterprise policy ID that governs this vault.
    pub policy_id: Option<String>,
}

/// A vault instance with its runtime state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultInfo {
    /// Vault ID.
    pub id: Uuid,
    /// Display name.
    pub name: String,
    /// Path to vault root directory.
    pub path: String,
    /// Current status.
    pub status: VaultStatus,
    /// Number of encrypted files.
    pub file_count: u64,
    /// Total encrypted data size in bytes.
    pub total_size: u64,
    /// When the vault was created.
    pub created_at: DateTime<Utc>,
    /// When the vault was last accessed.
    pub last_accessed: Option<DateTime<Utc>>,
    /// Whether Cryptomator compatibility is enabled.
    pub cryptomator_compat: bool,
    /// Encryption patterns.
    pub encrypt_patterns: Vec<String>,
}

/// Enterprise encryption policy for a connector/destination.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptionPolicy {
    /// Policy identifier.
    pub id: String,
    /// Human-readable policy name.
    pub name: String,
    /// Connector type this policy applies to (e.g., "s3", "sftp", "all").
    pub connector_type: String,
    /// Destination path pattern (glob) this policy applies to.
    pub destination_pattern: Option<String>,
    /// Whether encryption is required (mandatory) or optional.
    pub required: bool,
    /// File patterns that must be encrypted (e.g., "*.pdf", "*.docx").
    pub file_patterns: Vec<String>,
    /// Minimum KDF difficulty level.
    pub min_kdf_level: String,
    /// Whether zero-knowledge (encrypt-before-upload) is required.
    pub zero_knowledge_required: bool,
    /// Policy created timestamp.
    pub created_at: DateTime<Utc>,
    /// Whether the policy is active.
    pub enabled: bool,
}

impl Default for EncryptionPolicy {
    fn default() -> Self {
        Self {
            id: Uuid::new_v4().to_string(),
            name: "Default Encryption Policy".to_string(),
            connector_type: "all".to_string(),
            destination_pattern: None,
            required: false,
            file_patterns: Vec::new(),
            min_kdf_level: "default".to_string(),
            zero_knowledge_required: false,
            created_at: Utc::now(),
            enabled: true,
        }
    }
}

/// Manages encrypted vaults.
///
/// Holds references to unlocked vaults (with keys in memory) and provides
/// vault creation, locking, and file encryption/decryption operations.
#[derive(Clone)]
pub struct VaultManager {
    /// Unlocked vaults: vault_id -> (path, key, metadata).
    unlocked: Arc<RwLock<HashMap<Uuid, UnlockedVault>>>,
    /// Enterprise encryption policies.
    policies: Arc<RwLock<Vec<EncryptionPolicy>>>,
}

/// Internal state for an unlocked vault.
struct UnlockedVault {
    path: PathBuf,
    key: EncryptionKey,
    metadata: VaultMetadata,
}

impl std::fmt::Debug for UnlockedVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UnlockedVault")
            .field("path", &self.path)
            .field("key", &"[REDACTED]")
            .field("metadata", &self.metadata)
            .finish()
    }
}

impl VaultManager {
    /// Create a new vault manager.
    pub fn new() -> Self {
        Self {
            unlocked: Arc::new(RwLock::new(HashMap::new())),
            policies: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Create a new encrypted vault at the given path.
    ///
    /// The vault directory is created if it does not exist. A `.vault-meta.json`
    /// file is written with the KDF parameters, salt, and verification hash.
    pub async fn create_vault(
        &self,
        path: &str,
        name: &str,
        password: String,
        encrypt_patterns: Vec<String>,
        exclude_patterns: Vec<String>,
        cryptomator_compat: bool,
    ) -> Result<VaultInfo, AppError> {
        let vault_path = PathBuf::from(path);

        // Create the vault directory
        if !vault_path.exists() {
            tokio::fs::create_dir_all(&vault_path).await.map_err(|e| {
                AppError::file_op(
                    format!("Cannot create vault directory: {e}"),
                    "Check that you have write permissions to the parent directory.",
                )
            })?;
        }

        // Check for existing vault
        let meta_path = vault_path.join(VAULT_META_FILE);
        if meta_path.exists() {
            return Err(AppError::encryption(
                "Vault already exists",
                "A vault already exists at this location. Delete it first or choose a different path.",
            ));
        }

        // Generate salt and derive key
        let salt = Salt::generate();
        let kdf_params = KdfParams::default();
        let pwd = Password::new(password);
        let key = kdf::derive_key(&pwd, &salt, &kdf_params)?;
        let verification_hash = kdf::compute_verification_hash(&key);

        let vault_id = Uuid::new_v4();
        let now = Utc::now();

        let metadata = VaultMetadata {
            version: 1,
            vault_id,
            name: name.to_string(),
            kdf_params,
            salt,
            verification_hash,
            encrypt_patterns: encrypt_patterns.clone(),
            exclude_patterns: exclude_patterns.clone(),
            created_at: now,
            last_accessed: Some(now),
            cryptomator_compat,
            policy_id: None,
        };

        // Write metadata to disk
        let meta_json = serde_json::to_string_pretty(&metadata).map_err(|e| {
            AppError::internal(format!("Failed to serialize vault metadata: {e}"))
        })?;
        tokio::fs::write(&meta_path, meta_json).await.map_err(|e| {
            AppError::file_op(
                format!("Failed to write vault metadata: {e}"),
                "Check disk space and permissions.",
            )
        })?;

        // Store in unlocked state
        let unlocked = UnlockedVault {
            path: vault_path.clone(),
            key,
            metadata: metadata.clone(),
        };
        {
            let mut vaults = self.unlocked.write().await;
            vaults.insert(vault_id, unlocked);
        }

        tracing::info!("Created encrypted vault '{}' at {}", name, path);

        Ok(VaultInfo {
            id: vault_id,
            name: name.to_string(),
            path: path.to_string(),
            status: VaultStatus::Unlocked,
            file_count: 0,
            total_size: 0,
            created_at: now,
            last_accessed: Some(now),
            cryptomator_compat,
            encrypt_patterns,
        })
    }

    /// Unlock an existing vault with a password.
    pub async fn unlock_vault(
        &self,
        path: &str,
        password: String,
    ) -> Result<VaultInfo, AppError> {
        let vault_path = PathBuf::from(path);
        let meta_path = vault_path.join(VAULT_META_FILE);

        if !meta_path.exists() {
            return Err(AppError::encryption(
                "No vault found",
                "The specified path does not contain a vault.",
            ));
        }

        // Read metadata
        let meta_json = tokio::fs::read_to_string(&meta_path).await.map_err(|e| {
            AppError::file_op(
                format!("Failed to read vault metadata: {e}"),
                "Check file permissions.",
            )
        })?;

        let mut metadata: VaultMetadata = serde_json::from_str(&meta_json).map_err(|e| {
            AppError::encryption(
                "Corrupted vault metadata",
                format!("The vault metadata file is corrupted: {e}"),
            )
        })?;

        // Verify password and derive key
        let pwd = Password::new(password);
        let key = kdf::verify_password(
            &pwd,
            &metadata.salt,
            &metadata.kdf_params,
            &metadata.verification_hash,
        )?;

        // Update last accessed
        metadata.last_accessed = Some(Utc::now());
        let meta_json = serde_json::to_string_pretty(&metadata)
            .map_err(|e| AppError::internal(format!("Failed to serialize vault metadata: {e}")))?;
        let _ = tokio::fs::write(&meta_path, meta_json).await;

        // Count encrypted files
        let (file_count, total_size) = count_vault_files(&vault_path).await;

        let vault_id = metadata.vault_id;
        let info = VaultInfo {
            id: vault_id,
            name: metadata.name.clone(),
            path: path.to_string(),
            status: VaultStatus::Unlocked,
            file_count,
            total_size,
            created_at: metadata.created_at,
            last_accessed: metadata.last_accessed,
            cryptomator_compat: metadata.cryptomator_compat,
            encrypt_patterns: metadata.encrypt_patterns.clone(),
        };

        // Store in unlocked state
        {
            let mut vaults = self.unlocked.write().await;
            vaults.insert(vault_id, UnlockedVault {
                path: vault_path,
                key,
                metadata,
            });
        }

        tracing::info!("Unlocked vault '{}' at {}", info.name, path);
        Ok(info)
    }

    /// Lock a vault (remove key from memory).
    pub async fn lock_vault(&self, vault_id: &Uuid) -> Result<(), AppError> {
        let mut vaults = self.unlocked.write().await;
        if vaults.remove(vault_id).is_some() {
            tracing::info!("Locked vault {vault_id}");
            Ok(())
        } else {
            Err(AppError::encryption(
                "Vault not unlocked",
                "The vault is already locked or does not exist.",
            ))
        }
    }

    /// List all known vaults at given paths.
    pub async fn list_vaults(&self, scan_paths: &[String]) -> Result<Vec<VaultInfo>, AppError> {
        let mut vaults = Vec::new();
        let unlocked = self.unlocked.read().await;

        for path in scan_paths {
            let vault_path = PathBuf::from(path);
            let meta_path = vault_path.join(VAULT_META_FILE);
            if meta_path.exists() {
                if let Ok(meta_json) = tokio::fs::read_to_string(&meta_path).await {
                    if let Ok(metadata) = serde_json::from_str::<VaultMetadata>(&meta_json) {
                        let status = if unlocked.contains_key(&metadata.vault_id) {
                            VaultStatus::Unlocked
                        } else {
                            VaultStatus::Locked
                        };
                        let (file_count, total_size) = count_vault_files(&vault_path).await;
                        vaults.push(VaultInfo {
                            id: metadata.vault_id,
                            name: metadata.name.clone(),
                            path: path.clone(),
                            status,
                            file_count,
                            total_size,
                            created_at: metadata.created_at,
                            last_accessed: metadata.last_accessed,
                            cryptomator_compat: metadata.cryptomator_compat,
                            encrypt_patterns: metadata.encrypt_patterns.clone(),
                        });
                    }
                }
            }
        }

        Ok(vaults)
    }

    /// Encrypt a file and store it in the vault.
    ///
    /// The original file is read, encrypted with AES-256-GCM, and written
    /// to the vault with a `.enc` extension. The relative path structure
    /// is preserved within the vault.
    pub async fn encrypt_file(
        &self,
        vault_id: &Uuid,
        source_path: &str,
        relative_path: &str,
    ) -> Result<String, AppError> {
        let vaults = self.unlocked.read().await;
        let vault = vaults.get(vault_id).ok_or_else(|| {
            AppError::encryption(
                "Vault is locked",
                "Unlock the vault before encrypting files.",
            )
        })?;

        // Check if file matches encryption patterns
        if !should_encrypt(&vault.metadata, relative_path) {
            return Err(AppError::encryption(
                "File excluded by pattern",
                "This file does not match the vault's encryption patterns.",
            ));
        }

        // Read source file
        let plaintext = tokio::fs::read(source_path).await.map_err(|e| {
            AppError::file_op(
                format!("Cannot read source file: {e}"),
                "Check that the file exists and is readable.",
            )
        })?;

        // Encrypt
        let encrypted = encryption::encrypt_file_data(&vault.key, &plaintext, relative_path)?;

        // Write encrypted file
        let dest = vault.path.join(format!("{relative_path}{ENCRYPTED_EXT}"));
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AppError::file_op(
                    format!("Cannot create vault subdirectory: {e}"),
                    "Check vault directory permissions.",
                )
            })?;
        }

        tokio::fs::write(&dest, &encrypted).await.map_err(|e| {
            AppError::file_op(
                format!("Failed to write encrypted file: {e}"),
                "Check disk space and permissions.",
            )
        })?;

        let dest_str = dest.to_string_lossy().to_string();
        tracing::debug!("Encrypted file to vault: {relative_path}");
        Ok(dest_str)
    }

    /// Decrypt a file from the vault.
    ///
    /// Reads the encrypted file from the vault, decrypts it, and writes
    /// the plaintext to the destination path.
    pub async fn decrypt_file(
        &self,
        vault_id: &Uuid,
        relative_path: &str,
        dest_path: &str,
    ) -> Result<(), AppError> {
        let vaults = self.unlocked.read().await;
        let vault = vaults.get(vault_id).ok_or_else(|| {
            AppError::encryption(
                "Vault is locked",
                "Unlock the vault before decrypting files.",
            )
        })?;

        // Read encrypted file
        let enc_path = vault.path.join(format!("{relative_path}{ENCRYPTED_EXT}"));
        let encrypted = tokio::fs::read(&enc_path).await.map_err(|e| {
            AppError::file_op(
                format!("Cannot read encrypted file: {e}"),
                "Check that the encrypted file exists in the vault.",
            )
        })?;

        // Decrypt
        let plaintext = encryption::decrypt_file_data(&vault.key, &encrypted, relative_path)?;

        // Write decrypted file
        if let Some(parent) = Path::new(dest_path).parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AppError::file_op(
                    format!("Cannot create destination directory: {e}"),
                    "Check destination path permissions.",
                )
            })?;
        }

        tokio::fs::write(dest_path, &plaintext).await.map_err(|e| {
            AppError::file_op(
                format!("Failed to write decrypted file: {e}"),
                "Check disk space and permissions.",
            )
        })?;

        tracing::debug!("Decrypted file from vault: {relative_path}");
        Ok(())
    }

    /// Encrypt data in-memory for zero-knowledge cloud upload.
    ///
    /// This is the "encrypt-before-upload" mode where the cloud provider
    /// never sees plaintext. The caller uploads the returned bytes.
    pub async fn encrypt_for_upload(
        &self,
        vault_id: &Uuid,
        plaintext: &[u8],
        file_name: &str,
    ) -> Result<Vec<u8>, AppError> {
        let vaults = self.unlocked.read().await;
        let vault = vaults.get(vault_id).ok_or_else(|| {
            AppError::encryption(
                "Vault is locked",
                "Unlock the vault before encrypting for upload.",
            )
        })?;

        encryption::encrypt_file_data(&vault.key, plaintext, file_name)
    }

    /// Decrypt data downloaded from cloud (zero-knowledge mode).
    pub async fn decrypt_from_download(
        &self,
        vault_id: &Uuid,
        encrypted: &[u8],
        file_name: &str,
    ) -> Result<Vec<u8>, AppError> {
        let vaults = self.unlocked.read().await;
        let vault = vaults.get(vault_id).ok_or_else(|| {
            AppError::encryption(
                "Vault is locked",
                "Unlock the vault before decrypting downloaded files.",
            )
        })?;

        encryption::decrypt_file_data(&vault.key, encrypted, file_name)
    }

    /// Change the vault password.
    ///
    /// This re-derives the key with a new password and salt, updates the
    /// verification hash, and re-encrypts the vault metadata. Existing
    /// encrypted files are NOT re-encrypted (they use the same underlying
    /// key which is re-wrapped with the new password).
    pub async fn change_password(
        &self,
        vault_id: &Uuid,
        old_password: String,
        new_password: String,
    ) -> Result<(), AppError> {
        // First verify old password
        let (vault_path, old_metadata) = {
            let vaults = self.unlocked.read().await;
            let vault = vaults.get(vault_id).ok_or_else(|| {
                AppError::encryption(
                    "Vault is locked",
                    "Unlock the vault before changing the password.",
                )
            })?;
            (vault.path.clone(), vault.metadata.clone())
        };

        let old_pwd = Password::new(old_password);
        kdf::verify_password(
            &old_pwd,
            &old_metadata.salt,
            &old_metadata.kdf_params,
            &old_metadata.verification_hash,
        )?;

        // Derive new key with new salt
        let new_salt = Salt::generate();
        let new_pwd = Password::new(new_password);
        let new_key = kdf::derive_key(&new_pwd, &new_salt, &old_metadata.kdf_params)?;
        let new_verification_hash = kdf::compute_verification_hash(&new_key);

        // Update metadata
        let mut new_metadata = old_metadata;
        new_metadata.salt = new_salt;
        new_metadata.verification_hash = new_verification_hash;

        let meta_path = vault_path.join(VAULT_META_FILE);
        let meta_json = serde_json::to_string_pretty(&new_metadata)
            .map_err(|e| AppError::internal(format!("Failed to serialize vault metadata: {e}")))?;
        tokio::fs::write(&meta_path, meta_json).await.map_err(|e| {
            AppError::file_op(
                format!("Failed to update vault metadata: {e}"),
                "Check disk space and permissions.",
            )
        })?;

        // Update in-memory state
        {
            let mut vaults = self.unlocked.write().await;
            if let Some(vault) = vaults.get_mut(vault_id) {
                vault.key = new_key;
                vault.metadata = new_metadata;
            }
        }

        tracing::info!("Changed password for vault {vault_id}");
        Ok(())
    }

    /// Get the number of currently unlocked vaults.
    pub async fn unlocked_count(&self) -> usize {
        self.unlocked.read().await.len()
    }

    /// Check if a specific vault is unlocked.
    pub async fn is_unlocked(&self, vault_id: &Uuid) -> bool {
        self.unlocked.read().await.contains_key(vault_id)
    }

    // ── Enterprise Encryption Policies ──

    /// Add an encryption policy.
    pub async fn add_policy(&self, policy: EncryptionPolicy) -> Result<(), AppError> {
        let mut policies = self.policies.write().await;
        policies.push(policy);
        Ok(())
    }

    /// List all encryption policies.
    pub async fn list_policies(&self) -> Vec<EncryptionPolicy> {
        self.policies.read().await.clone()
    }

    /// Remove an encryption policy by ID.
    pub async fn remove_policy(&self, policy_id: &str) -> Result<(), AppError> {
        let mut policies = self.policies.write().await;
        let initial_len = policies.len();
        policies.retain(|p| p.id != policy_id);
        if policies.len() == initial_len {
            return Err(AppError::encryption(
                "Policy not found",
                "The specified encryption policy does not exist.",
            ));
        }
        Ok(())
    }

    /// Check if encryption is required for a file transfer to a given connector/destination.
    pub async fn check_policy(
        &self,
        connector_type: &str,
        destination: &str,
        file_name: &str,
    ) -> PolicyCheckResult {
        let policies = self.policies.read().await;

        for policy in policies.iter().filter(|p| p.enabled) {
            // Check connector type match
            if policy.connector_type != "all" && policy.connector_type != connector_type {
                continue;
            }

            // Check destination pattern match
            if let Some(ref dest_pattern) = policy.destination_pattern {
                if !glob_matches(dest_pattern, destination) {
                    continue;
                }
            }

            // Check file pattern match
            if !policy.file_patterns.is_empty() {
                let matches_any = policy
                    .file_patterns
                    .iter()
                    .any(|p| glob_matches(p, file_name));
                if !matches_any {
                    continue;
                }
            }

            // This policy matches
            return PolicyCheckResult {
                encryption_required: policy.required,
                zero_knowledge_required: policy.zero_knowledge_required,
                policy_id: Some(policy.id.clone()),
                policy_name: Some(policy.name.clone()),
            };
        }

        PolicyCheckResult {
            encryption_required: false,
            zero_knowledge_required: false,
            policy_id: None,
            policy_name: None,
        }
    }
}

impl Default for VaultManager {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of checking encryption policies.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PolicyCheckResult {
    /// Whether encryption is required for this transfer.
    pub encryption_required: bool,
    /// Whether zero-knowledge (encrypt-before-upload) is required.
    pub zero_knowledge_required: bool,
    /// ID of the matching policy, if any.
    pub policy_id: Option<String>,
    /// Name of the matching policy, if any.
    pub policy_name: Option<String>,
}

/// Check if a file should be encrypted based on vault patterns.
fn should_encrypt(metadata: &VaultMetadata, relative_path: &str) -> bool {
    // If no patterns specified, encrypt everything
    if metadata.encrypt_patterns.is_empty() {
        // But check exclusions
        for pattern in &metadata.exclude_patterns {
            if glob_matches(pattern, relative_path) {
                return false;
            }
        }
        return true;
    }

    // Check if it matches any include pattern
    let included = metadata
        .encrypt_patterns
        .iter()
        .any(|p| glob_matches(p, relative_path));

    if !included {
        return false;
    }

    // Check exclusions
    for pattern in &metadata.exclude_patterns {
        if glob_matches(pattern, relative_path) {
            return false;
        }
    }

    true
}

/// Simple glob matching (supports * and **).
fn glob_matches(pattern: &str, path: &str) -> bool {
    if let Ok(glob_pattern) = glob::Pattern::new(pattern) {
        glob_pattern.matches(path)
    } else {
        // Fallback: exact match
        pattern == path
    }
}

/// Count encrypted files and total size in a vault directory.
async fn count_vault_files(vault_path: &Path) -> (u64, u64) {
    let mut file_count = 0u64;
    let mut total_size = 0u64;

    if let Ok(mut entries) = tokio::fs::read_dir(vault_path).await {
        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension() {
                    if ext == "enc" {
                        file_count += 1;
                        if let Ok(metadata) = entry.metadata().await {
                            total_size += metadata.len();
                        }
                    }
                }
            }
        }
    }

    (file_count, total_size)
}

/// Hex serialization for fixed-size byte arrays.
mod hex_array_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8; 32], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<[u8; 32], D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        let bytes = hex::decode(&s).map_err(serde::de::Error::custom)?;
        if bytes.len() != 32 {
            return Err(serde::de::Error::custom("Expected 32 bytes"));
        }
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&bytes);
        Ok(arr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_should_encrypt_all() {
        let metadata = VaultMetadata {
            version: 1,
            vault_id: Uuid::new_v4(),
            name: "test".to_string(),
            kdf_params: KdfParams::fast(),
            salt: Salt::generate(),
            verification_hash: [0u8; 32],
            encrypt_patterns: vec![],
            exclude_patterns: vec![],
            created_at: Utc::now(),
            last_accessed: None,
            cryptomator_compat: false,
            policy_id: None,
        };

        assert!(should_encrypt(&metadata, "file.txt"));
        assert!(should_encrypt(&metadata, "subdir/file.pdf"));
    }

    #[test]
    fn test_should_encrypt_with_patterns() {
        let metadata = VaultMetadata {
            version: 1,
            vault_id: Uuid::new_v4(),
            name: "test".to_string(),
            kdf_params: KdfParams::fast(),
            salt: Salt::generate(),
            verification_hash: [0u8; 32],
            encrypt_patterns: vec!["*.pdf".to_string(), "*.docx".to_string()],
            exclude_patterns: vec![],
            created_at: Utc::now(),
            last_accessed: None,
            cryptomator_compat: false,
            policy_id: None,
        };

        assert!(should_encrypt(&metadata, "report.pdf"));
        assert!(should_encrypt(&metadata, "letter.docx"));
        assert!(!should_encrypt(&metadata, "image.png"));
    }

    #[test]
    fn test_should_encrypt_with_exclusions() {
        let metadata = VaultMetadata {
            version: 1,
            vault_id: Uuid::new_v4(),
            name: "test".to_string(),
            kdf_params: KdfParams::fast(),
            salt: Salt::generate(),
            verification_hash: [0u8; 32],
            encrypt_patterns: vec![],
            exclude_patterns: vec!["*.tmp".to_string(), "*.log".to_string()],
            created_at: Utc::now(),
            last_accessed: None,
            cryptomator_compat: false,
            policy_id: None,
        };

        assert!(should_encrypt(&metadata, "file.txt"));
        assert!(!should_encrypt(&metadata, "debug.log"));
        assert!(!should_encrypt(&metadata, "session.tmp"));
    }

    #[test]
    fn test_glob_matches() {
        assert!(glob_matches("*.pdf", "report.pdf"));
        assert!(!glob_matches("*.pdf", "report.txt"));
        assert!(glob_matches("*.*", "report.pdf"));
        assert!(!glob_matches("*.txt", "report.pdf"));
    }

    #[test]
    fn test_vault_status_serialization() {
        let status = VaultStatus::Locked;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"locked\"");

        let status = VaultStatus::Unlocked;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"unlocked\"");
    }

    #[test]
    fn test_encryption_policy_default() {
        let policy = EncryptionPolicy::default();
        assert_eq!(policy.connector_type, "all");
        assert!(!policy.required);
        assert!(policy.enabled);
    }

    #[tokio::test]
    async fn test_vault_create_and_unlock() {
        let dir = tempfile::tempdir().unwrap();
        let vault_path = dir.path().join("test_vault");
        let manager = VaultManager::new();

        // Create vault
        let info = manager
            .create_vault(
                &vault_path.to_string_lossy(),
                "Test Vault",
                "my_password_123".to_string(),
                vec![],
                vec![],
                false,
            )
            .await
            .unwrap();

        assert_eq!(info.name, "Test Vault");
        assert_eq!(info.status, VaultStatus::Unlocked);
        assert_eq!(info.file_count, 0);

        // Lock vault
        manager.lock_vault(&info.id).await.unwrap();
        assert!(!manager.is_unlocked(&info.id).await);

        // Unlock vault
        let info2 = manager
            .unlock_vault(
                &vault_path.to_string_lossy(),
                "my_password_123".to_string(),
            )
            .await
            .unwrap();
        assert_eq!(info2.id, info.id);
        assert_eq!(info2.status, VaultStatus::Unlocked);
    }

    #[tokio::test]
    async fn test_vault_wrong_password() {
        let dir = tempfile::tempdir().unwrap();
        let vault_path = dir.path().join("test_vault");
        let manager = VaultManager::new();

        manager
            .create_vault(
                &vault_path.to_string_lossy(),
                "Test Vault",
                "correct_password".to_string(),
                vec![],
                vec![],
                false,
            )
            .await
            .unwrap();

        // Lock it
        let vaults = manager.unlocked.read().await;
        let vault_id = *vaults.keys().next().unwrap();
        drop(vaults);
        manager.lock_vault(&vault_id).await.unwrap();

        // Try wrong password
        let result = manager
            .unlock_vault(
                &vault_path.to_string_lossy(),
                "wrong_password".to_string(),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_vault_encrypt_decrypt_file() {
        let dir = tempfile::tempdir().unwrap();
        let vault_path = dir.path().join("test_vault");
        let manager = VaultManager::new();

        let info = manager
            .create_vault(
                &vault_path.to_string_lossy(),
                "File Vault",
                "password123".to_string(),
                vec![],
                vec![],
                false,
            )
            .await
            .unwrap();

        // Create a test file
        let source_file = dir.path().join("secret.txt");
        tokio::fs::write(&source_file, "Top secret content!").await.unwrap();

        // Encrypt into vault
        manager
            .encrypt_file(&info.id, &source_file.to_string_lossy(), "secret.txt")
            .await
            .unwrap();

        // Verify encrypted file exists
        let enc_file = vault_path.join("secret.txt.enc");
        assert!(enc_file.exists());

        // Decrypt from vault
        let dest_file = dir.path().join("decrypted.txt");
        manager
            .decrypt_file(&info.id, "secret.txt", &dest_file.to_string_lossy())
            .await
            .unwrap();

        let content = tokio::fs::read_to_string(&dest_file).await.unwrap();
        assert_eq!(content, "Top secret content!");
    }

    #[tokio::test]
    async fn test_vault_encrypt_for_upload() {
        let dir = tempfile::tempdir().unwrap();
        let vault_path = dir.path().join("test_vault");
        let manager = VaultManager::new();

        let info = manager
            .create_vault(
                &vault_path.to_string_lossy(),
                "Upload Vault",
                "password".to_string(),
                vec![],
                vec![],
                false,
            )
            .await
            .unwrap();

        let plaintext = b"Cloud data to encrypt before upload";
        let encrypted = manager
            .encrypt_for_upload(&info.id, plaintext, "cloud-file.dat")
            .await
            .unwrap();

        // Should be different from plaintext
        assert_ne!(&encrypted, plaintext);

        // Should be decryptable
        let decrypted = manager
            .decrypt_from_download(&info.id, &encrypted, "cloud-file.dat")
            .await
            .unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[tokio::test]
    async fn test_vault_change_password() {
        let dir = tempfile::tempdir().unwrap();
        let vault_path = dir.path().join("test_vault");
        let manager = VaultManager::new();

        let info = manager
            .create_vault(
                &vault_path.to_string_lossy(),
                "Rekey Vault",
                "old_password".to_string(),
                vec![],
                vec![],
                false,
            )
            .await
            .unwrap();

        // Create and encrypt a file
        let source = dir.path().join("data.txt");
        tokio::fs::write(&source, "Sensitive data").await.unwrap();
        manager
            .encrypt_file(&info.id, &source.to_string_lossy(), "data.txt")
            .await
            .unwrap();

        // Change password
        manager
            .change_password(&info.id, "old_password".to_string(), "new_password".to_string())
            .await
            .unwrap();

        // Lock and re-unlock with new password
        manager.lock_vault(&info.id).await.unwrap();
        let info2 = manager
            .unlock_vault(&vault_path.to_string_lossy(), "new_password".to_string())
            .await
            .unwrap();
        assert_eq!(info2.id, info.id);

        // Old password should no longer work
        manager.lock_vault(&info.id).await.unwrap();
        let result = manager
            .unlock_vault(&vault_path.to_string_lossy(), "old_password".to_string())
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_encryption_policy_check() {
        let manager = VaultManager::new();

        let policy = EncryptionPolicy {
            id: "pol_1".to_string(),
            name: "S3 PDF Encryption".to_string(),
            connector_type: "s3".to_string(),
            destination_pattern: Some("sensitive-bucket/*".to_string()),
            required: true,
            file_patterns: vec!["*.pdf".to_string()],
            min_kdf_level: "default".to_string(),
            zero_knowledge_required: true,
            created_at: Utc::now(),
            enabled: true,
        };

        manager.add_policy(policy).await.unwrap();

        // Should match
        let result = manager
            .check_policy("s3", "sensitive-bucket/reports", "annual_report.pdf")
            .await;
        assert!(result.encryption_required);
        assert!(result.zero_knowledge_required);
        assert_eq!(result.policy_name.unwrap(), "S3 PDF Encryption");

        // Should not match (wrong connector)
        let result = manager
            .check_policy("sftp", "sensitive-bucket/reports", "annual_report.pdf")
            .await;
        assert!(!result.encryption_required);

        // Should not match (wrong file type)
        let result = manager
            .check_policy("s3", "sensitive-bucket/reports", "image.png")
            .await;
        assert!(!result.encryption_required);
    }
}
