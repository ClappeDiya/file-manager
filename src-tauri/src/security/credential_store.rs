//! Credential storage with OS keychain integration (T-019).
//!
//! - Primary: OS keychain (macOS Keychain, Windows Credential Vault, Linux Secret Service)
//! - Fallback: Encrypted vault using AES-256 when keychain is unavailable
//! - Uses the `keyring` crate for cross-platform keychain access

//! - Portable mode (`crate::paths`): an encrypted vault file inside the
//!   portable folder instead of the keychain, which cannot travel with it
//!   (see [`FileVault`]).

use crate::core::error::AppError;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::sync::RwLock;

const SERVICE_NAME: &str = "unified-file-ops";

/// Passphrase for the portable vault file (optional; see [`FileVault`]).
pub const PORTABLE_PASSPHRASE_ENV: &str = "UFOP_PORTABLE_PASSPHRASE";
const VAULT_AAD: &[u8] = b"ufop-portable-credential-vault-v1";

/// Credentials in one AES-256-GCM encrypted file (portable mode).
///
/// The key is derived with Argon2id from `UFOP_PORTABLE_PASSPHRASE` when it
/// is set (the vault then cannot be opened without it); otherwise it is a
/// random key kept in `credentials.key` next to the vault, which keeps
/// secrets out of plain text but protects them only as well as the drive
/// itself (use an encrypted drive, or set the passphrase).
pub struct FileVault {
    path: PathBuf,
    key: crate::security::encryption::EncryptionKey,
    entries: std::sync::Mutex<BTreeMap<String, String>>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct VaultFile {
    version: u32,
    /// `passphrase` (Argon2id, salt below) or `keyfile`.
    key_source: String,
    #[serde(default)]
    salt: Option<String>,
    /// base64 of nonce || ciphertext of the JSON map.
    data: String,
}

impl FileVault {
    /// Open (or create) the vault at `path`.
    pub fn open(path: &Path, passphrase: Option<&str>) -> Result<Self, AppError> {
        use crate::security::{encryption, kdf};
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD;
        let existing: Option<VaultFile> = match std::fs::read(path) {
            Ok(raw) => Some(serde_json::from_slice(&raw).map_err(|_| vault_err("The credential vault file is damaged"))?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(vault_err(&format!("Cannot read the credential vault: {e}"))),
        };
        let passphrase = passphrase.filter(|p| !p.is_empty());
        let key = match (passphrase, existing.as_ref().map(|v| v.key_source.as_str())) {
            (Some(p), None | Some("passphrase")) => {
                let salt = match existing.as_ref().and_then(|v| v.salt.clone()) {
                    Some(s) => kdf::Salt::from_bytes(b64.decode(s).map_err(|_| vault_err("The credential vault file is damaged"))?)?,
                    None => kdf::Salt::generate(),
                };
                let k = kdf::derive_key(&kdf::Password::new(p.to_string()), &salt, &kdf::KdfParams::default())?;
                if existing.is_none() {
                    Self::write_file(path, &VaultFile { version: 1, key_source: "passphrase".into(), salt: Some(b64.encode(&salt.bytes)), data: String::new() })?;
                }
                k
            }
            (None, Some("passphrase")) => {
                return Err(AppError::encryption("The portable credential vault is protected by a passphrase", format!("Set {PORTABLE_PASSPHRASE_ENV} before starting the app.")));
            }
            (Some(_), Some(_)) => {
                return Err(AppError::encryption("The portable credential vault uses a key file, not a passphrase", format!("Unset {PORTABLE_PASSPHRASE_ENV}, or move the vault aside to start a new one.")));
            }
            (None, _) => {
                let kf = path.with_file_name("credentials.key");
                match std::fs::read(&kf) {
                    Ok(b) => encryption::EncryptionKey::from_slice(&b)?,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound && existing.is_none() => {
                        let k = encryption::EncryptionKey::generate();
                        if let Some(dir) = kf.parent() {
                            std::fs::create_dir_all(dir).map_err(|e| vault_err(&e.to_string()))?;
                        }
                        let _ = std::fs::remove_file(&kf);
                        ufop_core::session::private_file(kf.parent().unwrap_or(Path::new(".")), "credentials.key", k.as_bytes(), 0o600).map_err(|e| vault_err(&format!("Cannot create the vault key: {e}")))?;
                        k
                    }
                    Err(e) => return Err(vault_err(&format!("Cannot read the vault key {}: {e}", kf.display()))),
                }
            }
        };
        let entries = match existing.filter(|v| !v.data.is_empty()) {
            Some(v) => {
                let raw = b64.decode(&v.data).map_err(|_| vault_err("The credential vault file is damaged"))?;
                let enc = encryption::EncryptedData::from_bytes(&raw)?;
                let plain = encryption::decrypt(&key, &enc, Some(VAULT_AAD))
                    .map_err(|_| AppError::encryption("Cannot open the portable credential vault", format!("Check {PORTABLE_PASSPHRASE_ENV} (or the credentials.key file) belongs to this vault.")))?;
                serde_json::from_slice(&plain).map_err(|_| vault_err("The credential vault file is damaged"))?
            }
            None => BTreeMap::new(),
        };
        let v = Self { path: path.to_path_buf(), key, entries: std::sync::Mutex::new(entries) };
        if !v.path.exists() {
            v.save()?;
        }
        Ok(v)
    }

    fn write_file(path: &Path, f: &VaultFile) -> Result<(), AppError> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| vault_err(&e.to_string()))?;
        }
        let tmp = path.with_extension("vault.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(f).map_err(|e| vault_err(&e.to_string()))?).map_err(|e| vault_err(&format!("Cannot write the credential vault: {e}")))?;
        std::fs::rename(&tmp, path).map_err(|e| vault_err(&format!("Cannot write the credential vault: {e}")))
    }

    fn save(&self) -> Result<(), AppError> {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD;
        let json = serde_json::to_vec(&*self.entries.lock().unwrap_or_else(|p| p.into_inner())).map_err(|e| vault_err(&e.to_string()))?;
        let enc = crate::security::encryption::encrypt(&self.key, &json, Some(VAULT_AAD))?;
        let current: Option<VaultFile> = std::fs::read(&self.path).ok().and_then(|r| serde_json::from_slice(&r).ok());
        let (key_source, salt) = current.map(|c| (c.key_source, c.salt)).unwrap_or_else(|| ("keyfile".into(), None));
        Self::write_file(&self.path, &VaultFile { version: 1, key_source, salt, data: b64.encode(enc.to_bytes()) })
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).get(key).cloned()
    }

    pub fn set(&self, key: &str, secret: &str) -> Result<(), AppError> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).insert(key.to_string(), secret.to_string());
        self.save()
    }

    pub fn remove(&self, key: &str) -> Result<(), AppError> {
        let removed = self.entries.lock().unwrap_or_else(|p| p.into_inner()).remove(key).is_some();
        if removed {
            self.save()?;
        }
        Ok(())
    }

    pub fn keys(&self) -> Vec<String> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner()).keys().cloned().collect()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl std::fmt::Debug for FileVault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileVault").field("path", &self.path).finish_non_exhaustive()
    }
}

fn vault_err(m: &str) -> AppError {
    AppError::encryption(m.to_string(), "Check the portable folder is writable and not damaged.")
}

/// Credential manager wrapping OS keychain with encrypted vault fallback.
pub struct CredentialStore {
    /// In-memory cache of credentials for the current session.
    pub(crate) cache: Arc<RwLock<std::collections::HashMap<String, String>>>,
    /// Whether the OS keychain is available.
    pub(crate) keychain_available: bool,
    /// Portable mode: credentials persist in this encrypted file.
    pub(crate) file_vault: Option<Arc<FileVault>>,
}

impl CredentialStore {
    /// Create a new credential store, detecting keychain availability.
    ///
    /// In debug builds, the keychain is skipped to avoid repeated macOS
    /// permission prompts caused by unsigned binary changes on each rebuild.
    ///
    /// In portable mode the keychain is never used: credentials go to the
    /// vault file `<portable>/config/credentials.vault`.
    pub fn new() -> Self {
        let dirs = crate::paths::dirs();
        if dirs.is_portable() && !cfg!(test) {
            let path = dirs.config.join("credentials.vault");
            match FileVault::open(&path, std::env::var(PORTABLE_PASSPHRASE_ENV).ok().as_deref()) {
                Ok(v) => {
                    tracing::info!("Portable mode: credentials in {}", path.display());
                    return Self::with_file_vault(Arc::new(v));
                }
                Err(e) => tracing::error!("Portable credential vault unavailable ({}); credentials last for this session only", e.user_message()),
            }
            return Self { cache: Arc::new(RwLock::new(std::collections::HashMap::new())), keychain_available: false, file_vault: None };
        }
        let keychain_available = if cfg!(debug_assertions) {
            tracing::info!(
                "Debug build detected — skipping OS keychain to avoid repeated permission prompts"
            );
            false
        } else {
            Self::test_keychain()
        };

        if keychain_available {
            tracing::info!("OS keychain available for credential storage");
        } else if !cfg!(debug_assertions) {
            tracing::warn!("OS keychain not available, using encrypted vault fallback");
        }

        Self {
            cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            keychain_available,
            file_vault: None,
        }
    }

    /// A store backed by the encrypted vault file (portable mode, tests).
    pub fn with_file_vault(vault: Arc<FileVault>) -> Self {
        let cache: std::collections::HashMap<String, String> = vault.keys().into_iter().filter_map(|k| vault.get(&k).map(|v| (k, v))).collect();
        Self { cache: Arc::new(RwLock::new(cache)), keychain_available: false, file_vault: Some(vault) }
    }

    /// `keychain`, `vault_file` (portable mode) or `memory` (this session only).
    pub fn backend_name(&self) -> &'static str {
        if self.file_vault.is_some() {
            "vault_file"
        } else if self.keychain_available {
            "keychain"
        } else {
            "memory"
        }
    }

    /// Test if the OS keychain is accessible.
    fn test_keychain() -> bool {
        let entry = keyring::Entry::new(SERVICE_NAME, "__test_probe__");
        match entry {
            Ok(entry) => {
                // Try to set and delete a test credential
                match entry.set_password("__test__") {
                    Ok(()) => {
                        let _ = entry.delete_credential();
                        true
                    }
                    Err(_) => false,
                }
            }
            Err(_) => false,
        }
    }

    /// Store a credential in the OS keychain.
    pub async fn store(&self, key: &str, secret: &str) -> Result<(), AppError> {
        let key = key.to_string();
        let secret = secret.to_string();

        // Always cache in memory
        {
            let mut cache = self.cache.write().await;
            cache.insert(key.clone(), secret.clone());
        }

        if self.keychain_available {
            let key_clone = key.clone();
            let secret_clone = secret.clone();
            tokio::task::spawn_blocking(move || {
                let entry = keyring::Entry::new(SERVICE_NAME, &key_clone)
                    .map_err(|e| AppError::Connection {
                        message: format!("Failed to create keychain entry: {e}"),
                        advice: "Check OS keychain permissions.".to_string(),
                    })?;
                entry.set_password(&secret_clone).map_err(|e| {
                    AppError::Connection {
                        message: format!("Failed to store credential in keychain: {e}"),
                        advice: "Check OS keychain permissions.".to_string(),
                    }
                })
            })
            .await
            .map_err(|e| AppError::internal(format!("Keychain task panicked: {e}")))?
        } else if let Some(v) = self.file_vault.clone() {
            tokio::task::spawn_blocking(move || v.set(&key, &secret)).await.map_err(|e| AppError::internal(format!("Vault task panicked: {e}")))?
        } else {
            // No keychain and not portable: the in-memory cache lasts for the session.
            tracing::debug!("Credential stored in memory vault: {key}");
            Ok(())
        }
    }

    /// Retrieve a credential from the OS keychain.
    pub async fn retrieve(&self, key: &str) -> Result<Option<String>, AppError> {
        // Check cache first
        {
            let cache = self.cache.read().await;
            if let Some(secret) = cache.get(key) {
                return Ok(Some(secret.clone()));
            }
        }

        if self.keychain_available {
            let key = key.to_string();
            tokio::task::spawn_blocking(move || {
                let entry = keyring::Entry::new(SERVICE_NAME, &key)
                    .map_err(|e| AppError::Connection {
                        message: format!("Failed to access keychain entry: {e}"),
                        advice: "Check OS keychain permissions.".to_string(),
                    })?;
                match entry.get_password() {
                    Ok(secret) => Ok(Some(secret)),
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(e) => Err(AppError::Connection {
                        message: format!("Failed to retrieve credential: {e}"),
                        advice: "Check OS keychain permissions.".to_string(),
                    }),
                }
            })
            .await
            .map_err(|e| AppError::internal(format!("Keychain task panicked: {e}")))?
        } else {
            Ok(self.file_vault.as_ref().and_then(|v| v.get(key)))
        }
    }

    /// Delete a credential from the OS keychain.
    pub async fn delete(&self, key: &str) -> Result<(), AppError> {
        // Remove from cache
        {
            let mut cache = self.cache.write().await;
            cache.remove(key);
        }

        if self.keychain_available {
            let key = key.to_string();
            tokio::task::spawn_blocking(move || {
                let entry = keyring::Entry::new(SERVICE_NAME, &key)
                    .map_err(|e| AppError::Connection {
                        message: format!("Failed to access keychain entry: {e}"),
                        advice: "Check OS keychain permissions.".to_string(),
                    })?;
                match entry.delete_credential() {
                    Ok(()) => Ok(()),
                    Err(keyring::Error::NoEntry) => Ok(()), // Already deleted
                    Err(e) => Err(AppError::Connection {
                        message: format!("Failed to delete credential: {e}"),
                        advice: "Check OS keychain permissions.".to_string(),
                    }),
                }
            })
            .await
            .map_err(|e| AppError::internal(format!("Keychain task panicked: {e}")))?
        } else if let Some(v) = self.file_vault.clone() {
            let key = key.to_string();
            tokio::task::spawn_blocking(move || v.remove(&key)).await.map_err(|e| AppError::internal(format!("Vault task panicked: {e}")))?
        } else {
            Ok(())
        }
    }

    /// Generate a credential reference key for a connection.
    pub fn credential_key(connection_id: &uuid::Uuid) -> String {
        format!("conn_{connection_id}")
    }

    /// Check if the OS keychain is available.
    pub fn is_keychain_available(&self) -> bool {
        self.keychain_available
    }

    /// List all cached credential keys (not the secrets themselves).
    pub async fn list_keys(&self) -> Vec<String> {
        let cache = self.cache.read().await;
        cache.keys().cloned().collect()
    }
}

impl Default for CredentialStore {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_credential_store_creation() {
        let store = CredentialStore::new();
        // Should not panic
        let _ = store.is_keychain_available();
    }

    #[tokio::test]
    async fn test_cache_operations() {
        let store = CredentialStore {
            cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            keychain_available: false, // Force vault mode for testing
            file_vault: None,
        };

        // Store
        store.store("test_key", "test_secret").await.unwrap();

        // Retrieve
        let secret = store.retrieve("test_key").await.unwrap();
        assert_eq!(secret, Some("test_secret".to_string()));

        // Delete
        store.delete("test_key").await.unwrap();
        let secret = store.retrieve("test_key").await.unwrap();
        assert_eq!(secret, None);
    }

    #[tokio::test]
    async fn test_retrieve_nonexistent() {
        let store = CredentialStore {
            cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            keychain_available: false,
            file_vault: None,
        };

        let secret = store.retrieve("nonexistent").await.unwrap();
        assert_eq!(secret, None);
    }

    #[tokio::test]
    async fn portable_vault_file_persists_encrypted_with_a_key_file() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("config/credentials.vault");
        let store = CredentialStore::with_file_vault(Arc::new(FileVault::open(&path, None).unwrap()));
        assert_eq!(store.backend_name(), "vault_file");
        store.store("conn_1", "s3cret-value").await.unwrap();
        store.store("conn_2", "other").await.unwrap();
        store.delete("conn_2").await.unwrap();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(!raw.contains("s3cret-value") && !raw.contains("conn_1"), "the vault file is encrypted");
        assert!(t.path().join("config/credentials.key").is_file());
        // A new process (same folder) reads it back.
        let again = CredentialStore::with_file_vault(Arc::new(FileVault::open(&path, None).unwrap()));
        assert_eq!(again.retrieve("conn_1").await.unwrap().as_deref(), Some("s3cret-value"));
        assert_eq!(again.retrieve("conn_2").await.unwrap(), None);
        assert_eq!(again.list_keys().await, vec!["conn_1".to_string()]);
        // A key-file vault refuses a passphrase rather than silently ignoring it.
        assert!(FileVault::open(&path, Some("pw")).is_err());
    }

    #[tokio::test]
    async fn portable_vault_with_a_passphrase() {
        let t = tempfile::tempdir().unwrap();
        let path = t.path().join("credentials.vault");
        let v = FileVault::open(&path, Some("correct horse")).unwrap();
        v.set("k", "v").unwrap();
        assert!(!t.path().join("credentials.key").exists());
        assert_eq!(FileVault::open(&path, Some("correct horse")).unwrap().get("k").as_deref(), Some("v"));
        assert!(FileVault::open(&path, Some("wrong")).is_err());
        assert!(FileVault::open(&path, None).unwrap_err().user_message().contains("passphrase"));
    }

    #[test]
    fn test_credential_key_format() {
        let id = uuid::Uuid::new_v4();
        let key = CredentialStore::credential_key(&id);
        assert!(key.starts_with("conn_"));
        assert!(key.contains(&id.to_string()));
    }

    #[tokio::test]
    async fn test_list_keys() {
        let store = CredentialStore {
            cache: Arc::new(RwLock::new(std::collections::HashMap::new())),
            keychain_available: false,
            file_vault: None,
        };

        store.store("key1", "secret1").await.unwrap();
        store.store("key2", "secret2").await.unwrap();

        let keys = store.list_keys().await;
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"key1".to_string()));
        assert!(keys.contains(&"key2".to_string()));
    }
}
