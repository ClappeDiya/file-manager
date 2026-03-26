//! Security - credential management, encryption, keychain integration.
//!
//! Provides `CredentialManager` for secure credential storage using OS keychain
//! with encrypted vault fallback (T-019).
//!
//! T-047 additions:
//! - `encryption`: AES-256-GCM primitives for at-rest encryption
//! - `kdf`: Argon2id key derivation from master password
//! - `vault`: Encrypted vault management (create/lock/unlock/encrypt/decrypt)
//! - `tls`: TLS enforcement helpers for in-transit encryption

pub mod audit;
pub mod confirmation;
pub mod credential_store;
pub mod encryption;
pub mod kdf;
pub mod master_password;
pub mod tls;
pub mod vault;

pub use credential_store::CredentialStore;
pub use encryption::EncryptionKey;
pub use kdf::{KdfParams, Password, Salt};
pub use master_password::MasterPasswordManager;
pub use tls::{TransportSecurity, TransportSecurityCheck, TlsVersion};
pub use vault::{EncryptionPolicy, PolicyCheckResult, VaultInfo, VaultManager, VaultMetadata, VaultStatus};

/// Legacy credential manager (kept for backward compatibility).
pub struct CredentialManager {
    _private: (),
}

impl CredentialManager {
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for CredentialManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_credential_manager_creation() {
        let _cm = CredentialManager::new();
    }
}
