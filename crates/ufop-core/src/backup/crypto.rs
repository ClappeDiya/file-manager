//! Client-side volume encryption.
//!
//! The passphrase is stretched once per repository with Argon2id (salt and
//! cost parameters live in the unencrypted `repo.json`). Every volume then
//! gets its own random 256-bit file key, which is wrapped with the master key;
//! content and wrapped key are both sealed with AES-256-GCM, so any
//! modification of a volume is detected on read.
//!
//! Wire format of a volume:
//! ```text
//! "UFOPV1" | flag u8 (0 = plain, 1 = AES-256-GCM)
//! plain:  payload
//! aes:    wrap_nonce[12] | wrapped_key[48] | nonce[12] | ciphertext+tag
//! ```
//! No creator or timestamp metadata is written into headers.

use crate::error::AppError;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use rand::RngCore;
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 6] = b"UFOPV1";
const KEY_CHECK_PLAINTEXT: &[u8] = b"ufop-backup-key-check";

/// Argon2id cost parameters, stored in the repository header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    pub memory_kib: u32,
    pub iterations: u32,
    pub parallelism: u32,
}

impl Default for KdfParams {
    fn default() -> Self {
        Self { memory_kib: 65536, iterations: 3, parallelism: 4 }
    }
}

impl KdfParams {
    /// Cheap parameters for tests only.
    pub fn insecure_fast() -> Self {
        Self { memory_kib: 1024, iterations: 1, parallelism: 1 }
    }
}

/// Encryption material for one repository. `None` key = unencrypted;
/// `external` delegates to a pluggable [`super::modules::EncryptionModule`]
/// (e.g. GPG).
#[derive(Clone)]
pub struct VolumeCipher {
    key: Option<[u8; 32]>,
    external: Option<std::sync::Arc<dyn super::modules::EncryptionModule>>,
}

impl std::fmt::Debug for VolumeCipher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VolumeCipher").field("encrypted", &self.is_encrypted()).field("module", &self.module_name()).finish()
    }
}

impl Drop for VolumeCipher {
    fn drop(&mut self) {
        if let Some(k) = self.key.as_mut() {
            k.iter_mut().for_each(|b| *b = 0);
        }
    }
}

fn enc_err(msg: &str) -> AppError {
    AppError::encryption(
        msg.to_string(),
        "Check the backup passphrase. If it is correct, the remote volume is damaged — run Verify, then Repair.",
    )
}

impl VolumeCipher {
    pub fn plain() -> Self {
        Self { key: None, external: None }
    }

    /// Use a pluggable encryption module instead of built-in AES.
    pub fn external(m: std::sync::Arc<dyn super::modules::EncryptionModule>) -> Self {
        Self { key: None, external: Some(m) }
    }

    pub fn is_encrypted(&self) -> bool {
        self.key.is_some() || self.external.is_some()
    }

    /// `aes`, `none`, or the external module's name.
    pub fn module_name(&self) -> String {
        match (&self.external, &self.key) {
            (Some(m), _) => m.name().to_string(),
            (None, Some(_)) => "aes".into(),
            (None, None) => "none".into(),
        }
    }

    /// Derive the master key from a passphrase.
    pub fn derive(passphrase: &str, salt: &[u8], params: KdfParams) -> Result<Self, AppError> {
        if passphrase.is_empty() {
            return Err(AppError::validation("The backup passphrase is empty"));
        }
        let p = argon2::Params::new(params.memory_kib, params.iterations, params.parallelism, Some(32))
            .map_err(|_| enc_err("Invalid key-derivation parameters in the repository header"))?;
        let a = argon2::Argon2::new(argon2::Algorithm::Argon2id, argon2::Version::V0x13, p);
        let mut key = [0u8; 32];
        a.hash_password_into(passphrase.as_bytes(), salt, &mut key)
            .map_err(|_| enc_err("Key derivation failed"))?;
        Ok(Self { key: Some(key), external: None })
    }

    pub fn random_salt() -> Vec<u8> {
        let mut s = vec![0u8; 16];
        OsRng.fill_bytes(&mut s);
        s
    }

    fn seal_with(key: &[u8; 32], data: &[u8]) -> Result<(Vec<u8>, Vec<u8>), AppError> {
        let c = Aes256Gcm::new_from_slice(key).map_err(|_| enc_err("Cipher init failed"))?;
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ct = c.encrypt(&nonce, data).map_err(|_| enc_err("Encryption failed"))?;
        Ok((nonce.to_vec(), ct))
    }

    fn open_with(key: &[u8; 32], nonce: &[u8], data: &[u8]) -> Result<Vec<u8>, AppError> {
        let c = Aes256Gcm::new_from_slice(key).map_err(|_| enc_err("Cipher init failed"))?;
        c.decrypt(Nonce::from_slice(nonce), data)
            .map_err(|_| enc_err("Decryption failed: wrong passphrase or modified volume"))
    }

    /// Value stored in `repo.json` that proves a passphrase is right without
    /// downloading a volume.
    pub fn key_check(&self) -> Result<Option<String>, AppError> {
        match &self.key {
            None => Ok(None),
            Some(k) => {
                let (n, ct) = Self::seal_with(k, KEY_CHECK_PLAINTEXT)?;
                Ok(Some(hex::encode([n, ct].concat())))
            }
        }
    }

    pub fn verify_key_check(&self, check: &str) -> Result<(), AppError> {
        let k = self.key.as_ref().ok_or_else(|| {
            AppError::validation("This backup is encrypted; a passphrase is required")
        })?;
        let raw = hex::decode(check).map_err(|_| enc_err("Repository key check is malformed"))?;
        if raw.len() < 12 {
            return Err(enc_err("Repository key check is malformed"));
        }
        let pt = Self::open_with(k, &raw[..12], &raw[12..]).map_err(|_| {
            AppError::encryption(
                "The backup passphrase is incorrect",
                "Enter the passphrase used when this backup was created.",
            )
        })?;
        if pt != KEY_CHECK_PLAINTEXT {
            return Err(enc_err("Repository key check mismatch"));
        }
        Ok(())
    }

    /// Wrap a payload into the volume wire format.
    pub fn seal(&self, payload: &[u8]) -> Result<Vec<u8>, AppError> {
        if let Some(m) = &self.external {
            let mut out = MAGIC.to_vec();
            out.push(2);
            out.extend(m.seal(payload)?);
            return Ok(out);
        }
        let mut out = Vec::with_capacity(payload.len() + 100);
        out.extend_from_slice(MAGIC);
        match &self.key {
            None => {
                out.push(0);
                out.extend_from_slice(payload);
            }
            Some(master) => {
                let mut file_key = [0u8; 32];
                OsRng.fill_bytes(&mut file_key);
                let (wn, wk) = Self::seal_with(master, &file_key)?;
                let (n, ct) = Self::seal_with(&file_key, payload)?;
                file_key.iter_mut().for_each(|b| *b = 0);
                out.push(1);
                out.extend_from_slice(&wn);
                out.extend_from_slice(&wk);
                out.extend_from_slice(&n);
                out.extend_from_slice(&ct);
            }
        }
        Ok(out)
    }

    /// Reverse of [`seal`](Self::seal); authenticates encrypted volumes.
    pub fn open(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        if data.len() < 7 || &data[..6] != MAGIC {
            return Err(enc_err("Not a UFOP backup volume (bad header)"));
        }
        match data[6] {
            0 => Ok(data[7..].to_vec()),
            1 => {
                let master = self.key.as_ref().ok_or_else(|| {
                    AppError::validation("This volume is encrypted; a passphrase is required")
                })?;
                let body = &data[7..];
                if body.len() < 12 + 48 + 12 + 16 {
                    return Err(enc_err("Encrypted volume is truncated"));
                }
                let fk = Self::open_with(master, &body[..12], &body[12..60])?;
                let mut file_key = [0u8; 32];
                file_key.copy_from_slice(&fk);
                let r = Self::open_with(&file_key, &body[60..72], &body[72..]);
                file_key.iter_mut().for_each(|b| *b = 0);
                r
            }
            2 => match &self.external {
                Some(m) => m.open(&data[7..]),
                None => Err(AppError::validation("This volume uses an external encryption module (e.g. GPG); configure it to read the backup")),
            },
            _ => Err(enc_err("Unknown volume encryption flag")),
        }
    }
}

/// Rough passphrase strength score 0–4 with feedback, for setup UIs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PassphraseStrength {
    pub score: u8,
    pub label: String,
    pub feedback: Vec<String>,
}

pub fn passphrase_strength(p: &str) -> PassphraseStrength {
    let len = p.chars().count();
    let classes = [
        p.chars().any(|c| c.is_ascii_lowercase()),
        p.chars().any(|c| c.is_ascii_uppercase()),
        p.chars().any(|c| c.is_ascii_digit()),
        p.chars().any(|c| !c.is_ascii_alphanumeric()),
    ]
    .iter()
    .filter(|b| **b)
    .count();
    let unique = p.chars().collect::<std::collections::HashSet<_>>().len();
    let mut feedback = Vec::new();
    let mut score: i32 = match len {
        0..=7 => 0,
        8..=11 => 1,
        12..=15 => 2,
        16..=23 => 3,
        _ => 4,
    };
    if classes >= 3 && len >= 10 {
        score += 1;
    }
    if unique * 2 < len {
        score -= 1;
        feedback.push("Avoid repeated characters.".to_string());
    }
    if len < 12 {
        feedback.push("Use at least 12 characters; a passphrase of several words is best.".to_string());
    }
    if classes < 3 {
        feedback.push("Mix upper/lower case, digits and symbols.".to_string());
    }
    let score = score.clamp(0, 4) as u8;
    let label = ["very weak", "weak", "fair", "strong", "very strong"][score as usize].to_string();
    PassphraseStrength { score, label, feedback }
}

/// Generate a random passphrase of `words` groups (≈ 5.95 bits/char, 24 chars ≈ 143 bits).
pub fn generate_passphrase(length: usize) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789-_.!@#%+=";
    let len = length.clamp(16, 128);
    let mut rng = OsRng;
    (0..len)
        .map(|_| ALPHABET[(rng.next_u32() as usize) % ALPHABET.len()] as char)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encrypted_and_plain() {
        let salt = VolumeCipher::random_salt();
        let c = VolumeCipher::derive("correct horse", &salt, KdfParams::insecure_fast()).unwrap();
        let sealed = c.seal(b"hello world").unwrap();
        assert_ne!(&sealed[7..], b"hello world");
        assert_eq!(c.open(&sealed).unwrap(), b"hello world");
        let p = VolumeCipher::plain();
        assert_eq!(p.open(&p.seal(b"x").unwrap()).unwrap(), b"x");
    }

    #[test]
    fn wrong_passphrase_and_tamper_detected() {
        let salt = VolumeCipher::random_salt();
        let c = VolumeCipher::derive("right", &salt, KdfParams::insecure_fast()).unwrap();
        let w = VolumeCipher::derive("wrong", &salt, KdfParams::insecure_fast()).unwrap();
        let check = c.key_check().unwrap().unwrap();
        assert!(c.verify_key_check(&check).is_ok());
        assert!(w.verify_key_check(&check).is_err());
        let mut sealed = c.seal(b"data").unwrap();
        assert!(w.open(&sealed).is_err());
        let last = sealed.len() - 1;
        sealed[last] ^= 1;
        assert!(c.open(&sealed).is_err());
    }

    #[test]
    fn strength_and_generation() {
        assert!(passphrase_strength("abc").score <= 1);
        let g = generate_passphrase(32);
        assert_eq!(g.len(), 32);
        assert!(passphrase_strength(&g).score >= 3);
    }
}
