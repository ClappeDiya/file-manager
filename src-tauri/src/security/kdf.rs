//! Argon2id Key Derivation Function for deriving encryption keys from passwords (T-047).
//!
//! Uses Argon2id (hybrid of Argon2i and Argon2d) which is the recommended variant
//! for password hashing and key derivation. Parameters follow OWASP recommendations.
//!
//! # Security properties
//! - Argon2id: memory-hard, resistant to GPU/ASIC attacks
//! - 256-bit derived keys suitable for AES-256-GCM
//! - Unique random salt per derivation (stored with vault metadata)
//! - Password zeroized from memory after derivation
//! - Tunable parameters for security/performance tradeoff

use argon2::{Algorithm, Argon2, Params, Version};
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::core::error::AppError;
use crate::security::encryption::{EncryptionKey, KEY_SIZE};

/// Salt size in bytes (128-bit, recommended minimum by RFC 9106).
pub const SALT_SIZE: usize = 16;

/// Default Argon2id parameters (OWASP "first choice" recommendation).
///
/// These parameters target ~1 second on modern hardware:
/// - Memory: 64 MiB (65536 KiB)
/// - Iterations: 3
/// - Parallelism: 4 threads
///
/// For YubiKey/FIDO2 scenarios, faster parameters can be used since
/// the hardware token provides the additional entropy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    /// Memory cost in KiB (default: 65536 = 64 MiB).
    pub memory_kib: u32,
    /// Number of iterations / passes (default: 3).
    pub iterations: u32,
    /// Degree of parallelism (default: 4).
    pub parallelism: u32,
    /// Output key length in bytes (default: 32 = 256 bits).
    pub output_len: usize,
    /// Algorithm identifier for forward compatibility.
    pub algorithm: String,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self {
            memory_kib: 65536,   // 64 MiB
            iterations: 3,
            parallelism: 4,
            output_len: KEY_SIZE, // 32 bytes = 256 bits
            algorithm: "argon2id".to_string(),
        }
    }
}

impl KdfParams {
    /// "Fast" parameters for hardware-token-backed scenarios.
    /// Lower memory/iterations since the token provides high entropy.
    pub fn fast() -> Self {
        Self {
            memory_kib: 16384,  // 16 MiB
            iterations: 2,
            parallelism: 4,
            output_len: KEY_SIZE,
            algorithm: "argon2id".to_string(),
        }
    }

    /// "High security" parameters for maximum resistance.
    /// ~3-5 seconds on modern hardware.
    pub fn high_security() -> Self {
        Self {
            memory_kib: 262144,  // 256 MiB
            iterations: 4,
            parallelism: 4,
            output_len: KEY_SIZE,
            algorithm: "argon2id".to_string(),
        }
    }

    /// Validate that parameters are within acceptable bounds.
    pub fn validate(&self) -> Result<(), AppError> {
        if self.algorithm != "argon2id" {
            return Err(AppError::encryption(
                "Unsupported KDF algorithm",
                format!("Only argon2id is supported, got '{}'.", self.algorithm),
            ));
        }
        if self.memory_kib < 8192 {
            return Err(AppError::encryption(
                "Memory cost too low",
                "Minimum memory cost is 8192 KiB (8 MiB) for security.",
            ));
        }
        if self.iterations < 1 {
            return Err(AppError::encryption(
                "Iteration count too low",
                "At least 1 iteration is required.",
            ));
        }
        if self.parallelism < 1 || self.parallelism > 255 {
            return Err(AppError::encryption(
                "Invalid parallelism",
                "Parallelism must be between 1 and 255.",
            ));
        }
        if self.output_len != KEY_SIZE {
            return Err(AppError::encryption(
                "Invalid output length",
                format!("Output length must be {KEY_SIZE} bytes for AES-256."),
            ));
        }
        Ok(())
    }
}

/// A password wrapper that zeroizes on drop.
#[derive(Zeroize, ZeroizeOnDrop)]
pub struct Password {
    inner: String,
}

impl Password {
    pub fn new(password: String) -> Self {
        Self { inner: password }
    }

    pub fn as_bytes(&self) -> &[u8] {
        self.inner.as_bytes()
    }
}

impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Password")
            .field("inner", &"[REDACTED]")
            .finish()
    }
}

/// Salt used for key derivation, stored alongside the vault.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Salt {
    #[serde(with = "hex_serde")]
    pub bytes: Vec<u8>,
}

impl Salt {
    /// Generate a new random salt.
    pub fn generate() -> Self {
        use rand::RngCore;
        let mut bytes = vec![0u8; SALT_SIZE];
        rand::rngs::OsRng.fill_bytes(&mut bytes);
        Self { bytes }
    }

    /// Create from existing bytes.
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Self, AppError> {
        if bytes.len() < SALT_SIZE {
            return Err(AppError::encryption(
                "Salt too short",
                format!("Salt must be at least {SALT_SIZE} bytes."),
            ));
        }
        Ok(Self { bytes })
    }
}

/// Derive an encryption key from a password using Argon2id.
///
/// # Arguments
/// * `password` - The user's master password (zeroized after use)
/// * `salt` - Random salt (stored with vault metadata)
/// * `params` - KDF parameters controlling memory/time cost
///
/// # Returns
/// A 256-bit `EncryptionKey` suitable for AES-256-GCM.
pub fn derive_key(
    password: &Password,
    salt: &Salt,
    params: &KdfParams,
) -> Result<EncryptionKey, AppError> {
    params.validate()?;

    let argon2_params = Params::new(
        params.memory_kib,
        params.iterations,
        params.parallelism,
        Some(params.output_len),
    )
    .map_err(|e| {
        tracing::error!("Invalid Argon2id parameters: {e}");
        AppError::encryption(
            "Invalid KDF parameters",
            "The key derivation parameters are invalid.",
        )
    })?;

    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon2_params);

    let mut key_bytes = vec![0u8; params.output_len];
    argon2
        .hash_password_into(password.as_bytes(), &salt.bytes, &mut key_bytes)
        .map_err(|e| {
            tracing::error!("Argon2id key derivation failed: {e}");
            AppError::encryption(
                "Key derivation failed",
                "Unable to derive encryption key from password.",
            )
        })?;

    let mut key_array = [0u8; KEY_SIZE];
    key_array.copy_from_slice(&key_bytes);
    key_bytes.zeroize();

    Ok(EncryptionKey::from_bytes(key_array))
}

/// Verify a password against stored KDF output (for vault unlock).
///
/// Derives the key and checks it matches the expected verification hash.
/// The verification hash is SHA-256 of the derived key, stored in vault metadata.
pub fn verify_password(
    password: &Password,
    salt: &Salt,
    params: &KdfParams,
    expected_hash: &[u8; 32],
) -> Result<EncryptionKey, AppError> {
    let key = derive_key(password, salt, params)?;

    // Compute SHA-256 of derived key for verification
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let hash = hasher.finalize();

    // Constant-time comparison to prevent timing attacks.
    // XOR all bytes and accumulate into a single value; any bit difference
    // sets a bit in `diff`. This avoids early-exit branches that could
    // leak information through timing side-channels.
    let mut diff = 0u8;
    for (a, b) in hash.iter().zip(expected_hash.iter()) {
        diff |= a ^ b;
    }

    if diff != 0 {
        return Err(AppError::encryption(
            "Incorrect password",
            "The password is incorrect. Please try again.",
        ));
    }

    Ok(key)
}

/// Compute the verification hash for a derived key.
///
/// This hash is stored in vault metadata to verify the password on unlock
/// without storing the key itself.
pub fn compute_verification_hash(key: &EncryptionKey) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    let hash = hasher.finalize();
    let mut result = [0u8; 32];
    result.copy_from_slice(&hash);
    result
}

/// Hex serialization for salt bytes.
mod hex_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &Vec<u8>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&hex::encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        hex::decode(&s).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_params() {
        let params = KdfParams::default();
        assert_eq!(params.memory_kib, 65536);
        assert_eq!(params.iterations, 3);
        assert_eq!(params.parallelism, 4);
        assert_eq!(params.output_len, 32);
        assert_eq!(params.algorithm, "argon2id");
        params.validate().unwrap();
    }

    #[test]
    fn test_fast_params() {
        let params = KdfParams::fast();
        params.validate().unwrap();
        assert!(params.memory_kib < KdfParams::default().memory_kib);
    }

    #[test]
    fn test_high_security_params() {
        let params = KdfParams::high_security();
        params.validate().unwrap();
        assert!(params.memory_kib > KdfParams::default().memory_kib);
    }

    #[test]
    fn test_invalid_algorithm() {
        let params = KdfParams {
            algorithm: "bcrypt".to_string(),
            ..Default::default()
        };
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_memory_too_low() {
        let params = KdfParams {
            memory_kib: 1024, // Below 8 MiB minimum
            ..Default::default()
        };
        assert!(params.validate().is_err());
    }

    #[test]
    fn test_salt_generation() {
        let salt1 = Salt::generate();
        let salt2 = Salt::generate();
        assert_eq!(salt1.bytes.len(), SALT_SIZE);
        assert_ne!(salt1.bytes, salt2.bytes);
    }

    #[test]
    fn test_salt_too_short() {
        let short_salt = vec![0u8; 8];
        assert!(Salt::from_bytes(short_salt).is_err());
    }

    #[test]
    fn test_derive_key_deterministic() {
        // Use fast params for test speed
        let params = KdfParams::fast();
        let password = Password::new("test_password_123".to_string());
        let salt = Salt::from_bytes(vec![1u8; SALT_SIZE]).unwrap();

        let key1 = derive_key(&password, &salt, &params).unwrap();
        let key2 = derive_key(&password, &salt, &params).unwrap();

        // Same password + salt + params = same key
        assert_eq!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_derive_key_different_passwords() {
        let params = KdfParams::fast();
        let salt = Salt::from_bytes(vec![1u8; SALT_SIZE]).unwrap();

        let key1 = derive_key(
            &Password::new("password_a".to_string()),
            &salt,
            &params,
        )
        .unwrap();
        let key2 = derive_key(
            &Password::new("password_b".to_string()),
            &salt,
            &params,
        )
        .unwrap();

        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_derive_key_different_salts() {
        let params = KdfParams::fast();
        let password = Password::new("same_password".to_string());

        let salt1 = Salt::from_bytes(vec![1u8; SALT_SIZE]).unwrap();
        let salt2 = Salt::from_bytes(vec![2u8; SALT_SIZE]).unwrap();

        let key1 = derive_key(&password, &salt1, &params).unwrap();
        let key2 = derive_key(&password, &salt2, &params).unwrap();

        assert_ne!(key1.as_bytes(), key2.as_bytes());
    }

    #[test]
    fn test_verify_password_correct() {
        let params = KdfParams::fast();
        let password = Password::new("correct_password".to_string());
        let salt = Salt::generate();

        let key = derive_key(&password, &salt, &params).unwrap();
        let hash = compute_verification_hash(&key);

        // Verify with same password succeeds
        let password2 = Password::new("correct_password".to_string());
        let verified_key = verify_password(&password2, &salt, &params, &hash).unwrap();
        assert_eq!(key.as_bytes(), verified_key.as_bytes());
    }

    #[test]
    fn test_verify_password_wrong() {
        let params = KdfParams::fast();
        let salt = Salt::generate();

        let key = derive_key(
            &Password::new("correct_password".to_string()),
            &salt,
            &params,
        )
        .unwrap();
        let hash = compute_verification_hash(&key);

        // Verify with wrong password fails
        let wrong = Password::new("wrong_password".to_string());
        assert!(verify_password(&wrong, &salt, &params, &hash).is_err());
    }

    #[test]
    fn test_password_debug_redacted() {
        let password = Password::new("super_secret".to_string());
        let debug = format!("{password:?}");
        assert!(debug.contains("REDACTED"));
        assert!(!debug.contains("super_secret"));
    }

    #[test]
    fn test_kdf_params_serialization() {
        let params = KdfParams::default();
        let json = serde_json::to_string(&params).unwrap();
        let restored: KdfParams = serde_json::from_str(&json).unwrap();
        assert_eq!(params.memory_kib, restored.memory_kib);
        assert_eq!(params.iterations, restored.iterations);
        assert_eq!(params.parallelism, restored.parallelism);
    }

    #[test]
    fn test_salt_serialization() {
        let salt = Salt::generate();
        let json = serde_json::to_string(&salt).unwrap();
        let restored: Salt = serde_json::from_str(&json).unwrap();
        assert_eq!(salt.bytes, restored.bytes);
    }
}
