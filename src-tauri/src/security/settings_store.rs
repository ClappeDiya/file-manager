//! Server settings in the app database (`config` table) with optional
//! field encryption for secret-bearing values.
//!
//! The settings key comes from `UFOP_SETTINGS_ENCRYPTION_KEY`, or is
//! created in the OS secret store when one is available. Encrypted values
//! are stored as `enc:v1:<hex>`. Policies:
//! - `require_key`: refuse to start without a usable key (unless
//!   encryption is explicitly disabled);
//! - `disabled`: store secrets in plain text (explicit opt-out).
//!
//! Losing the key makes encrypted values unreadable;
//! [`SettingsStore::clear_encrypted`] removes them (after backing up the
//! database file) so the server can start and secrets can be re-entered.

use crate::core::error::AppError;
use crate::security::CredentialStore;
use crate::storage::pool::DbPool;
use rusqlite::{params, OptionalExtension};
use std::sync::Arc;
use ufop_core::backup::crypto::{KdfParams, VolumeCipher};

const ENC_PREFIX: &str = "enc:v1:";
const KEYCHAIN_KEY: &str = "ufop-settings-encryption-key";
pub const ENV_KEY: &str = "UFOP_SETTINGS_ENCRYPTION_KEY";
/// Fixed salt: the key itself is high-entropy (or user-chosen and held
/// outside the database), so the salt only domain-separates it.
const SALT: &[u8] = b"ufop-settings-v1";

#[derive(Debug, Clone, Copy, Default)]
pub struct SettingsKeyPolicy {
    pub require_key: bool,
    pub disabled: bool,
}

#[derive(Clone)]
pub struct SettingsStore {
    pool: DbPool,
    cipher: Option<Arc<VolumeCipher>>,
}

impl SettingsStore {
    /// Resolve the settings key per policy.
    pub async fn open(pool: DbPool, creds: &CredentialStore, policy: SettingsKeyPolicy) -> Result<Self, AppError> {
        if policy.disabled {
            return Ok(Self { pool, cipher: None });
        }
        let key = match std::env::var(ENV_KEY).ok().filter(|k| !k.is_empty()) {
            Some(k) => Some(k),
            None if creds.is_keychain_available() => match creds.retrieve(KEYCHAIN_KEY).await? {
                Some(k) => Some(k),
                None => {
                    let k = ufop_core::backup::crypto::generate_passphrase(48);
                    creds.store(KEYCHAIN_KEY, &k).await?;
                    Some(k)
                }
            },
            None => None,
        };
        let cipher = match key {
            Some(k) => Some(Arc::new(VolumeCipher::derive(&k, SALT, KdfParams { memory_kib: 19456, iterations: 2, parallelism: 1 })?)),
            None if policy.require_key => {
                return Err(AppError::Configuration {
                    message: "No settings encryption key is available".into(),
                    advice: format!("Set {ENV_KEY}, make an OS secret store available, or start with --disable-settings-encryption."),
                })
            }
            None => {
                tracing::warn!("No settings encryption key; server secrets are stored unencrypted");
                None
            }
        };
        Ok(Self { pool, cipher })
    }

    /// Plain store for tests.
    pub fn plain(pool: DbPool) -> Self {
        Self { pool, cipher: None }
    }

    pub fn with_key(pool: DbPool, key: &str) -> Result<Self, AppError> {
        Ok(Self { pool, cipher: Some(Arc::new(VolumeCipher::derive(key, SALT, KdfParams::insecure_fast())?)) })
    }

    pub fn pool(&self) -> &DbPool {
        &self.pool
    }

    pub fn is_encrypted(&self) -> bool {
        self.cipher.is_some()
    }

    pub async fn get(&self, key: &str) -> Result<Option<String>, AppError> {
        let k = key.to_string();
        self.pool
            .execute(move |c| Ok(c.query_row("SELECT value FROM config WHERE key = ?1", [k], |r| r.get(0)).optional()?))
            .await
    }

    pub async fn set(&self, key: &str, value: &str) -> Result<(), AppError> {
        let (k, v) = (key.to_string(), value.to_string());
        self.pool
            .execute(move |c| {
                c.execute(
                    "INSERT INTO config(key, value, updated_at) VALUES (?1, ?2, datetime('now'))
                     ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                    params![k, v],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn delete(&self, key: &str) -> Result<(), AppError> {
        let k = key.to_string();
        self.pool.execute(move |c| Ok(c.execute("DELETE FROM config WHERE key = ?1", [k]).map(|_| ())?)).await
    }

    /// Store a secret, encrypted when a key is available.
    pub async fn set_secret(&self, key: &str, value: &str) -> Result<(), AppError> {
        let stored = match &self.cipher {
            Some(c) => format!("{ENC_PREFIX}{}", hex::encode(c.seal(value.as_bytes())?)),
            None => value.to_string(),
        };
        self.set(key, &stored).await
    }

    pub async fn get_secret(&self, key: &str) -> Result<Option<String>, AppError> {
        let Some(v) = self.get(key).await? else { return Ok(None) };
        match (v.strip_prefix(ENC_PREFIX), &self.cipher) {
            (None, _) => Ok(Some(v)),
            (Some(hexed), Some(c)) => {
                let raw = hex::decode(hexed).map_err(|_| AppError::internal("corrupt encrypted setting"))?;
                let pt = c.open(&raw).map_err(|_| AppError::Configuration {
                    message: format!("Setting '{key}' cannot be decrypted with the current settings key"),
                    advice: "Restore the original key, or run `ufop-server --clear-encrypted-settings` and re-enter the secrets.".into(),
                })?;
                Ok(Some(String::from_utf8_lossy(&pt).into_owned()))
            }
            (Some(_), None) => Err(AppError::Configuration {
                message: format!("Setting '{key}' is encrypted but no settings key is available"),
                advice: format!("Set {ENV_KEY}, or run `ufop-server --clear-encrypted-settings` and re-enter the secrets."),
            }),
        }
    }

    /// Lost-key recovery: back up the DB file (when given) and delete
    /// every encrypted setting. Returns the removed keys.
    pub async fn clear_encrypted(&self, db_file: Option<&std::path::Path>) -> Result<Vec<String>, AppError> {
        if let Some(f) = db_file {
            if f.exists() {
                let bak = f.with_extension(format!("sqlite.bak-{}", chrono::Utc::now().format("%Y%m%d%H%M%S")));
                std::fs::copy(f, &bak)?;
                tracing::warn!("Database backed up to {}", bak.display());
            }
        }
        self.pool
            .execute(|c| {
                let mut st = c.prepare("SELECT key FROM config WHERE value LIKE 'enc:v1:%'")?;
                let keys = st.query_map([], |r| r.get::<_, String>(0))?.collect::<Result<Vec<_>, _>>()?;
                c.execute("DELETE FROM config WHERE value LIKE 'enc:v1:%'", [])?;
                Ok(keys)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Repository;

    #[tokio::test]
    async fn secrets_encrypt_and_lost_key_recovery() {
        let repo = Repository::open_in_memory().await.unwrap();
        let s = SettingsStore::with_key(repo.pool().clone(), "k1").unwrap();
        s.set_secret("a", "hunter2").await.unwrap();
        assert!(s.get("a").await.unwrap().unwrap().starts_with(ENC_PREFIX));
        assert_eq!(s.get_secret("a").await.unwrap().as_deref(), Some("hunter2"));
        let wrong = SettingsStore::with_key(repo.pool().clone(), "k2").unwrap();
        assert!(wrong.get_secret("a").await.is_err());
        let none = SettingsStore::plain(repo.pool().clone());
        assert!(none.get_secret("a").await.is_err());
        assert_eq!(none.clear_encrypted(None).await.unwrap(), vec!["a".to_string()]);
        assert!(none.get_secret("a").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn require_key_policy() {
        let repo = Repository::open_in_memory().await.unwrap();
        std::env::remove_var(ENV_KEY);
        let creds = CredentialStore::new(); // debug build: no keychain
        let r = SettingsStore::open(repo.pool().clone(), &creds, SettingsKeyPolicy { require_key: true, disabled: false }).await;
        assert!(r.is_err());
        let r = SettingsStore::open(repo.pool().clone(), &creds, SettingsKeyPolicy { require_key: true, disabled: true }).await;
        assert!(r.is_ok() && !r.unwrap().is_encrypted());
    }
}
