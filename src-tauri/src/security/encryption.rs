//! AES-256-GCM encryption primitives for at-rest vault encryption (T-047).
//!
//! Provides low-level encrypt/decrypt operations using AES-256-GCM with
//! authenticated encryption. All operations use random nonces and include
//! AAD (Additional Authenticated Data) for context binding.
//!
//! # Security properties
//! - AES-256-GCM: 256-bit key, 96-bit nonce, 128-bit auth tag
//! - Random nonces generated via OS CSPRNG
//! - Keys zeroized from memory after use via `zeroize` crate
//! - No plaintext key material in logs or error messages

use aes_gcm::{
    aead::{Aead, KeyInit, OsRng},
    Aes256Gcm, AeadCore, Nonce,
};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::core::error::AppError;

/// AES-256-GCM key size in bytes.
pub const KEY_SIZE: usize = 32;
/// AES-256-GCM nonce size in bytes.
pub const NONCE_SIZE: usize = 12;
/// AES-256-GCM authentication tag size in bytes.
pub const TAG_SIZE: usize = 16;

/// Magic bytes identifying our encrypted vault format.
pub const VAULT_MAGIC: &[u8; 8] = b"UFOPVLT\x01";

/// Vault format version.
pub const VAULT_FORMAT_VERSION: u8 = 1;

/// A 256-bit encryption key that is zeroized on drop.
///
/// This ensures key material is scrubbed from memory when no longer needed,
/// even in the face of panics or early returns.
#[derive(Clone, Zeroize, ZeroizeOnDrop)]
pub struct EncryptionKey {
    bytes: [u8; KEY_SIZE],
}

impl EncryptionKey {
    /// Create a new encryption key from raw bytes.
    ///
    /// # Panics
    /// Panics if `bytes` length is not exactly `KEY_SIZE`.
    pub fn from_bytes(bytes: [u8; KEY_SIZE]) -> Self {
        Self { bytes }
    }

    /// Create a new encryption key from a slice.
    pub fn from_slice(slice: &[u8]) -> Result<Self, AppError> {
        if slice.len() != KEY_SIZE {
            return Err(AppError::encryption(
                "Invalid key length",
                "Encryption keys must be exactly 32 bytes (256 bits).",
            ));
        }
        let mut bytes = [0u8; KEY_SIZE];
        bytes.copy_from_slice(slice);
        Ok(Self { bytes })
    }

    /// Get a reference to the raw key bytes.
    pub fn as_bytes(&self) -> &[u8; KEY_SIZE] {
        &self.bytes
    }

    /// Generate a random encryption key using OS CSPRNG.
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut bytes = [0u8; KEY_SIZE];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self { bytes }
    }
}

impl std::fmt::Debug for EncryptionKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // NEVER expose key material in debug output
        f.debug_struct("EncryptionKey")
            .field("bytes", &"[REDACTED]")
            .finish()
    }
}

/// Encrypted data container including nonce and ciphertext.
///
/// Wire format:
/// ```text
/// [nonce: 12 bytes] [ciphertext + tag: N + 16 bytes]
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedData {
    /// Random nonce used for this encryption.
    #[serde(with = "serde_bytes_base64")]
    pub nonce: Vec<u8>,
    /// Ciphertext including the 16-byte GCM authentication tag.
    #[serde(with = "serde_bytes_base64")]
    pub ciphertext: Vec<u8>,
}

impl EncryptedData {
    /// Serialize to a compact binary format: [nonce][ciphertext+tag].
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.nonce.len() + self.ciphertext.len());
        out.extend_from_slice(&self.nonce);
        out.extend_from_slice(&self.ciphertext);
        out
    }

    /// Deserialize from the compact binary format.
    pub fn from_bytes(data: &[u8]) -> Result<Self, AppError> {
        if data.len() < NONCE_SIZE + TAG_SIZE {
            return Err(AppError::encryption(
                "Encrypted data too short",
                "The encrypted data is corrupted or truncated.",
            ));
        }
        let nonce = data[..NONCE_SIZE].to_vec();
        let ciphertext = data[NONCE_SIZE..].to_vec();
        Ok(Self { nonce, ciphertext })
    }
}

/// Encrypt plaintext using AES-256-GCM with an optional AAD context.
///
/// Returns an `EncryptedData` containing a random nonce and the ciphertext
/// (with appended authentication tag).
///
/// # Arguments
/// * `key` - 256-bit encryption key
/// * `plaintext` - Data to encrypt
/// * `aad` - Additional Authenticated Data (optional, for context binding)
pub fn encrypt(
    key: &EncryptionKey,
    plaintext: &[u8],
    aad: Option<&[u8]>,
) -> Result<EncryptedData, AppError> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|e| {
        tracing::error!("Failed to initialize AES-256-GCM cipher: {e}");
        AppError::encryption(
            "Encryption initialization failed",
            "Unable to set up encryption. This may indicate a corrupted key.",
        )
    })?;

    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    // Build payload with optional AAD for context binding.
    // When AAD is provided (e.g. a file path), the ciphertext is cryptographically
    // bound to that context - decryption will fail if a different AAD is given.
    let payload = match aad {
        Some(aad_bytes) => aes_gcm::aead::Payload {
            msg: plaintext,
            aad: aad_bytes,
        },
        None => aes_gcm::aead::Payload {
            msg: plaintext,
            aad: b"",
        },
    };

    let ciphertext = cipher.encrypt(&nonce, payload).map_err(|e| {
        tracing::error!("AES-256-GCM encryption failed: {e}");
        AppError::encryption(
            "Encryption failed",
            "Unable to encrypt the data. Try again or check system resources.",
        )
    })?;

    Ok(EncryptedData {
        nonce: nonce.to_vec(),
        ciphertext,
    })
}

/// Decrypt ciphertext using AES-256-GCM.
///
/// Verifies the authentication tag and returns the plaintext on success.
///
/// # Arguments
/// * `key` - 256-bit encryption key (must match the key used for encryption)
/// * `encrypted` - The encrypted data container
/// * `aad` - Additional Authenticated Data (must match what was used during encryption)
pub fn decrypt(
    key: &EncryptionKey,
    encrypted: &EncryptedData,
    aad: Option<&[u8]>,
) -> Result<Vec<u8>, AppError> {
    let cipher = Aes256Gcm::new_from_slice(key.as_bytes()).map_err(|e| {
        tracing::error!("Failed to initialize AES-256-GCM cipher for decryption: {e}");
        AppError::encryption(
            "Decryption initialization failed",
            "Unable to set up decryption. The key may be corrupted.",
        )
    })?;

    if encrypted.nonce.len() != NONCE_SIZE {
        return Err(AppError::encryption(
            "Invalid nonce size",
            "The encrypted data is corrupted.",
        ));
    }

    let nonce = Nonce::from_slice(&encrypted.nonce);

    // Build payload with optional AAD - must match what was used during encryption.
    let payload = match aad {
        Some(aad_bytes) => aes_gcm::aead::Payload {
            msg: encrypted.ciphertext.as_ref(),
            aad: aad_bytes,
        },
        None => aes_gcm::aead::Payload {
            msg: encrypted.ciphertext.as_ref(),
            aad: b"",
        },
    };

    cipher
        .decrypt(nonce, payload)
        .map_err(|_| {
            // Deliberately vague: do not reveal whether key or data is wrong
            AppError::encryption(
                "Decryption failed",
                "Wrong password or corrupted data. Check your password and try again.",
            )
        })
}

/// Encrypt a file's contents, returning the encrypted bytes in vault wire format.
///
/// Wire format:
/// ```text
/// [VAULT_MAGIC: 8 bytes] [version: 1 byte] [nonce: 12 bytes] [ciphertext + tag]
/// ```
pub fn encrypt_file_data(
    key: &EncryptionKey,
    plaintext: &[u8],
    file_path: &str,
) -> Result<Vec<u8>, AppError> {
    // Use file path as AAD for context binding
    let encrypted = encrypt(key, plaintext, Some(file_path.as_bytes()))?;

    let mut output = Vec::with_capacity(
        VAULT_MAGIC.len() + 1 + encrypted.nonce.len() + encrypted.ciphertext.len(),
    );
    output.extend_from_slice(VAULT_MAGIC);
    output.push(VAULT_FORMAT_VERSION);
    output.extend_from_slice(&encrypted.nonce);
    output.extend_from_slice(&encrypted.ciphertext);

    Ok(output)
}

/// Decrypt file data from vault wire format.
pub fn decrypt_file_data(
    key: &EncryptionKey,
    data: &[u8],
    file_path: &str,
) -> Result<Vec<u8>, AppError> {
    let header_len = VAULT_MAGIC.len() + 1; // magic + version

    if data.len() < header_len + NONCE_SIZE + TAG_SIZE {
        return Err(AppError::encryption(
            "Invalid vault file",
            "The file does not appear to be a valid encrypted vault file.",
        ));
    }

    // Verify magic
    if &data[..VAULT_MAGIC.len()] != VAULT_MAGIC {
        return Err(AppError::encryption(
            "Not a vault file",
            "The file is not an encrypted vault file.",
        ));
    }

    let version = data[VAULT_MAGIC.len()];
    if version != VAULT_FORMAT_VERSION {
        return Err(AppError::encryption(
            "Unsupported vault version",
            format!(
                "Vault format version {version} is not supported. Expected version {VAULT_FORMAT_VERSION}."
            ),
        ));
    }

    let encrypted_data = EncryptedData::from_bytes(&data[header_len..])?;
    decrypt(key, &encrypted_data, Some(file_path.as_bytes()))
}

/// Base64 serialization module for byte vectors in serde.
mod serde_bytes_base64 {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        serializer.serialize_str(&encoded)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        base64::engine::general_purpose::STANDARD
            .decode(&s)
            .map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_generation() {
        let key1 = EncryptionKey::generate();
        let key2 = EncryptionKey::generate();
        // Two random keys should differ
        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_key_from_slice() {
        let bytes = [42u8; KEY_SIZE];
        let key = EncryptionKey::from_slice(&bytes).unwrap();
        assert_eq!(key.as_bytes(), &bytes);
    }

    #[test]
    fn test_key_from_slice_wrong_size() {
        let bytes = [42u8; 16]; // Too short
        assert!(EncryptionKey::from_slice(&bytes).is_err());
    }

    #[test]
    fn test_key_debug_redacted() {
        let key = EncryptionKey::generate();
        let debug = format!("{key:?}");
        assert!(debug.contains("REDACTED"));
        // Ensure no raw bytes leak
        assert!(!debug.contains("[42,"));
    }

    #[test]
    fn test_encrypt_decrypt_roundtrip() {
        let key = EncryptionKey::generate();
        let plaintext = b"Hello, World! This is sensitive data.";

        let encrypted = encrypt(&key, plaintext, None).unwrap();
        let decrypted = decrypt(&key, &encrypted, None).unwrap();

        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_decrypt_with_aad() {
        let key = EncryptionKey::generate();
        let plaintext = b"Secret file contents";
        let aad = b"file:///vault/secret.txt";

        let encrypted = encrypt(&key, plaintext, Some(aad)).unwrap();
        let decrypted = decrypt(&key, &encrypted, Some(aad)).unwrap();

        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_wrong_key_fails() {
        let key1 = EncryptionKey::generate();
        let key2 = EncryptionKey::generate();
        let plaintext = b"Sensitive data";

        let encrypted = encrypt(&key1, plaintext, None).unwrap();
        // Decryption with wrong key should fail
        assert!(decrypt(&key2, &encrypted, None).is_err());
    }

    #[test]
    fn test_tampered_ciphertext_fails() {
        let key = EncryptionKey::generate();
        let plaintext = b"Sensitive data";

        let mut encrypted = encrypt(&key, plaintext, None).unwrap();
        // Tamper with ciphertext
        if let Some(byte) = encrypted.ciphertext.get_mut(0) {
            *byte ^= 0xFF;
        }
        assert!(decrypt(&key, &encrypted, None).is_err());
    }

    #[test]
    fn test_encrypted_data_binary_roundtrip() {
        let key = EncryptionKey::generate();
        let plaintext = b"Binary roundtrip test";

        let encrypted = encrypt(&key, plaintext, None).unwrap();
        let bytes = encrypted.to_bytes();
        let restored = EncryptedData::from_bytes(&bytes).unwrap();

        assert_eq!(encrypted.nonce, restored.nonce);
        assert_eq!(encrypted.ciphertext, restored.ciphertext);

        let decrypted = decrypt(&key, &restored, None).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_file_data_encrypt_decrypt() {
        let key = EncryptionKey::generate();
        let plaintext = b"File contents to encrypt";
        let path = "/vault/test/secret.txt";

        let encrypted = encrypt_file_data(&key, plaintext, path).unwrap();

        // Should start with magic bytes
        assert_eq!(&encrypted[..8], VAULT_MAGIC);
        assert_eq!(encrypted[8], VAULT_FORMAT_VERSION);

        let decrypted = decrypt_file_data(&key, &encrypted, path).unwrap();
        assert_eq!(&decrypted, plaintext);
    }

    #[test]
    fn test_file_data_invalid_magic() {
        let key = EncryptionKey::generate();
        let bad_data = vec![0u8; 100];
        assert!(decrypt_file_data(&key, &bad_data, "test").is_err());
    }

    #[test]
    fn test_file_data_too_short() {
        let key = EncryptionKey::generate();
        let short_data = b"UFOPVLT\x01";
        assert!(decrypt_file_data(&key, short_data, "test").is_err());
    }

    #[test]
    fn test_empty_plaintext() {
        let key = EncryptionKey::generate();
        let plaintext = b"";

        let encrypted = encrypt(&key, plaintext, None).unwrap();
        let decrypted = decrypt(&key, &encrypted, None).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn test_large_plaintext() {
        let key = EncryptionKey::generate();
        let plaintext = vec![0xABu8; 1024 * 1024]; // 1 MB

        let encrypted = encrypt(&key, &plaintext, None).unwrap();
        let decrypted = decrypt(&key, &encrypted, None).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_unique_nonces() {
        let key = EncryptionKey::generate();
        let plaintext = b"Same data";

        let e1 = encrypt(&key, plaintext, None).unwrap();
        let e2 = encrypt(&key, plaintext, None).unwrap();

        // Nonces must differ (with overwhelming probability)
        assert_ne!(e1.nonce, e2.nonce);
        // Ciphertexts should also differ due to different nonces
        assert_ne!(e1.ciphertext, e2.ciphertext);
    }
}
