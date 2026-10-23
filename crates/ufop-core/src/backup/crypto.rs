//! Client-side volume encryption.
//!
//! The passphrase is stretched once per repository with Argon2id (salt and
//! cost parameters live in the unencrypted `repo.json`). Every volume then
//! gets its own random 256-bit file key, which is wrapped with the master key;
//! content and wrapped key are both sealed with AES-256-GCM, so any
//! modification of a volume is detected on read.
//!
//! Format v2 repositories add an envelope key: a random 256-bit master key
//! is wrapped by the Argon2id-derived key-encryption key (KEK) and stored in
//! the header, so changing the passphrase rewrites only the header. They may
//! also use ChaCha20-Poly1305 instead of AES-256-GCM.
//!
//! Wire format of a volume:
//! ```text
//! "UFOPV1" | flag u8 (0 = plain, 1 = AES-256-GCM, 2 = external module,
//!                     3 = ChaCha20-Poly1305)
//! plain:  payload
//! aead:   wrap_nonce[12] | wrapped_key[48] | nonce[12] | ciphertext+tag
//! ```
//! No creator or timestamp metadata is written into headers.

use crate::error::AppError;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use chacha20poly1305::ChaCha20Poly1305;
use rand::RngCore;
use serde::{Deserialize, Serialize};

const MAGIC: &[u8; 6] = b"UFOPV1";
const KEY_CHECK_PLAINTEXT: &[u8] = b"ufop-backup-key-check";

/// Authenticated cipher used for volumes and the key wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum CipherKind {
    #[default]
    #[serde(rename = "aes256_gcm")]
    Aes256Gcm,
    /// Format v2 only.
    #[serde(rename = "chacha20_poly1305")]
    ChaCha20Poly1305,
}

impl CipherKind {
    fn flag(self) -> u8 {
        match self {
            Self::Aes256Gcm => 1,
            Self::ChaCha20Poly1305 => 3,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Aes256Gcm => "aes256_gcm",
            Self::ChaCha20Poly1305 => "chacha20_poly1305",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().replace('-', "_").as_str() {
            "aes" | "aes256_gcm" | "aes_256_gcm" => Some(Self::Aes256Gcm),
            "chacha20_poly1305" | "chacha20poly1305" | "chacha" => Some(Self::ChaCha20Poly1305),
            _ => None,
        }
    }
}

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
    /// Check the parameters are usable and not dangerously weak.
    pub fn validate(&self) -> Result<(), AppError> {
        if !(8 * 1024..=4 * 1024 * 1024).contains(&self.memory_kib) {
            return Err(AppError::validation("Argon2 memory must be between 8 MiB and 4 GiB"));
        }
        if !(1..=50).contains(&self.iterations) {
            return Err(AppError::validation("Argon2 iterations must be between 1 and 50"));
        }
        if !(1..=64).contains(&self.parallelism) || self.memory_kib < 8 * self.parallelism {
            return Err(AppError::validation("Argon2 parallelism must be between 1 and 64"));
        }
        Ok(())
    }

    /// Named cost profiles.
    pub fn preset(name: &str) -> Option<Self> {
        Some(match name {
            "interactive" => Self { memory_kib: 19 * 1024, iterations: 2, parallelism: 1 },
            "default" => Self::default(),
            "sensitive" => Self { memory_kib: 256 * 1024, iterations: 4, parallelism: 4 },
            _ => return None,
        })
    }

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
    aead: CipherKind,
    external: Option<std::sync::Arc<dyn super::modules::EncryptionModule>>,
    /// Compress JSON metadata (dlist/dindex) with zstd before sealing (v2).
    compress_metadata: bool,
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
        Self { key: None, aead: CipherKind::Aes256Gcm, external: None, compress_metadata: false }
    }

    /// Use a pluggable encryption module instead of built-in AES.
    pub fn external(m: std::sync::Arc<dyn super::modules::EncryptionModule>) -> Self {
        Self { key: None, aead: CipherKind::Aes256Gcm, external: Some(m), compress_metadata: false }
    }

    /// A cipher over raw key material (the v2 master key).
    pub fn from_key(key: [u8; 32], aead: CipherKind) -> Self {
        Self { key: Some(key), aead, external: None, compress_metadata: false }
    }

    /// A fresh random 256-bit master key (new v2 repositories).
    pub fn random_master(aead: CipherKind) -> Self {
        let mut k = [0u8; 32];
        OsRng.fill_bytes(&mut k);
        Self::from_key(k, aead)
    }

    /// Same key, sealing with `aead` (volumes of every cipher stay readable).
    pub fn with_aead(mut self, aead: CipherKind) -> Self {
        self.aead = aead;
        self
    }

    pub fn aead(&self) -> CipherKind {
        self.aead
    }

    /// Turn zstd compression of sealed JSON metadata on or off.
    pub fn with_metadata_compression(mut self, on: bool) -> Self {
        self.compress_metadata = on;
        self
    }

    pub fn compresses_metadata(&self) -> bool {
        self.compress_metadata
    }

    /// Wrap `master`'s key with this key (the KEK); hex `nonce | ciphertext+tag`.
    pub fn wrap_key(&self, master: &VolumeCipher) -> Result<String, AppError> {
        let (kek, mk) = match (&self.key, &master.key) {
            (Some(a), Some(b)) => (a, b),
            _ => return Err(AppError::internal("key wrap needs two built-in keys")),
        };
        let (n, ct) = Self::seal_with(self.aead, kek, mk)?;
        Ok(hex::encode([n, ct].concat()))
    }

    /// Reverse of [`wrap_key`](Self::wrap_key): the master key as a cipher using `aead`.
    pub fn unwrap_key(&self, wrapped: &str, aead: CipherKind) -> Result<VolumeCipher, AppError> {
        let kek = self.key.as_ref().ok_or_else(|| AppError::validation("This backup is encrypted; a passphrase is required"))?;
        let raw = hex::decode(wrapped).map_err(|_| enc_err("The wrapped repository key is malformed"))?;
        if raw.len() != 12 + 48 {
            return Err(enc_err("The wrapped repository key is malformed"));
        }
        let mut pt = Self::open_with(self.aead, kek, &raw[..12], &raw[12..])?;
        let mut k = [0u8; 32];
        k.copy_from_slice(&pt);
        pt.iter_mut().for_each(|b| *b = 0);
        Ok(Self::from_key(k, aead))
    }

    /// Whether two ciphers hold the same built-in key.
    pub fn same_key(&self, other: &VolumeCipher) -> bool {
        matches!((&self.key, &other.key), (Some(a), Some(b)) if a == b)
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
        Ok(Self::from_key(key, CipherKind::Aes256Gcm))
    }

    pub fn random_salt() -> Vec<u8> {
        let mut s = vec![0u8; 16];
        OsRng.fill_bytes(&mut s);
        s
    }

    fn seal_with(aead: CipherKind, key: &[u8; 32], data: &[u8]) -> Result<(Vec<u8>, Vec<u8>), AppError> {
        let init = |_| enc_err("Cipher init failed");
        let fail = |_| enc_err("Encryption failed");
        match aead {
            CipherKind::Aes256Gcm => {
                let c = Aes256Gcm::new_from_slice(key).map_err(init)?;
                let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
                Ok((nonce.to_vec(), c.encrypt(&nonce, data).map_err(fail)?))
            }
            CipherKind::ChaCha20Poly1305 => {
                let c = ChaCha20Poly1305::new_from_slice(key).map_err(init)?;
                let nonce = ChaCha20Poly1305::generate_nonce(&mut OsRng);
                Ok((nonce.to_vec(), c.encrypt(&nonce, data).map_err(fail)?))
            }
        }
    }

    fn open_with(aead: CipherKind, key: &[u8; 32], nonce: &[u8], data: &[u8]) -> Result<Vec<u8>, AppError> {
        let init = |_| enc_err("Cipher init failed");
        let fail = |_| enc_err("Decryption failed: wrong passphrase or modified volume");
        match aead {
            CipherKind::Aes256Gcm => Aes256Gcm::new_from_slice(key).map_err(init)?.decrypt(Nonce::from_slice(nonce), data).map_err(fail),
            CipherKind::ChaCha20Poly1305 => {
                ChaCha20Poly1305::new_from_slice(key).map_err(init)?.decrypt(chacha20poly1305::Nonce::from_slice(nonce), data).map_err(fail)
            }
        }
    }

    /// Value stored in `repo.json` that proves a passphrase is right without
    /// downloading a volume.
    pub fn key_check(&self) -> Result<Option<String>, AppError> {
        if let Some(m) = &self.external {
            return Ok(Some(hex::encode(m.seal(KEY_CHECK_PLAINTEXT)?)));
        }
        match &self.key {
            None => Ok(None),
            Some(k) => {
                let (n, ct) = Self::seal_with(self.aead, k, KEY_CHECK_PLAINTEXT)?;
                Ok(Some(hex::encode([n, ct].concat())))
            }
        }
    }

    pub fn verify_key_check(&self, check: &str) -> Result<(), AppError> {
        if let Some(m) = &self.external {
            let raw = hex::decode(check).map_err(|_| enc_err("Repository key check is malformed"))?;
            let pt = m.open(&raw).map_err(|_| AppError::encryption("The backup passphrase or key is incorrect", "Enter the passphrase (or import the key) used when this backup was created."))?;
            return if pt == KEY_CHECK_PLAINTEXT { Ok(()) } else { Err(enc_err("Repository key check mismatch")) };
        }
        let k = self.key.as_ref().ok_or_else(|| {
            AppError::validation("This backup is encrypted; a passphrase is required")
        })?;
        let raw = hex::decode(check).map_err(|_| enc_err("Repository key check is malformed"))?;
        if raw.len() < 12 {
            return Err(enc_err("Repository key check is malformed"));
        }
        let pt = Self::open_with(self.aead, k, &raw[..12], &raw[12..]).map_err(|_| {
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
                let (wn, wk) = Self::seal_with(self.aead, master, &file_key)?;
                let (n, ct) = Self::seal_with(self.aead, &file_key, payload)?;
                file_key.iter_mut().for_each(|b| *b = 0);
                out.push(self.aead.flag());
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
            flag @ (1 | 3) => {
                let aead = if flag == 3 { CipherKind::ChaCha20Poly1305 } else { CipherKind::Aes256Gcm };
                let master = self.key.as_ref().ok_or_else(|| {
                    AppError::validation("This volume is encrypted; a passphrase is required")
                })?;
                let body = &data[7..];
                if body.len() < 12 + 48 + 12 + 16 {
                    return Err(enc_err("Encrypted volume is truncated"));
                }
                let fk = Self::open_with(aead, master, &body[..12], &body[12..60])?;
                let mut file_key = [0u8; 32];
                file_key.copy_from_slice(&fk);
                let r = Self::open_with(aead, &file_key, &body[60..72], &body[72..]);
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
    fn chacha_volumes_and_envelope_key() {
        let salt = VolumeCipher::random_salt();
        let kek = VolumeCipher::derive("pw", &salt, KdfParams::insecure_fast()).unwrap().with_aead(CipherKind::ChaCha20Poly1305);
        let master = VolumeCipher::random_master(CipherKind::ChaCha20Poly1305);
        let wrapped = kek.wrap_key(&master).unwrap();
        let back = kek.unwrap_key(&wrapped, CipherKind::ChaCha20Poly1305).unwrap();
        assert!(back.same_key(&master));
        let sealed = back.seal(b"payload").unwrap();
        assert_eq!(sealed[6], 3, "ChaCha20-Poly1305 flag");
        // Any cipher instance with the same key opens either AEAD.
        assert_eq!(master.clone().with_aead(CipherKind::Aes256Gcm).open(&sealed).unwrap(), b"payload");
        let aes_sealed = master.clone().with_aead(CipherKind::Aes256Gcm).seal(b"x").unwrap();
        assert_eq!(aes_sealed[6], 1);
        assert_eq!(back.open(&aes_sealed).unwrap(), b"x");
        let other = VolumeCipher::derive("other", &salt, KdfParams::insecure_fast()).unwrap().with_aead(CipherKind::ChaCha20Poly1305);
        assert!(other.unwrap_key(&wrapped, CipherKind::ChaCha20Poly1305).is_err());
        assert!(other.open(&sealed).is_err());
        assert_eq!(CipherKind::parse("chacha20-poly1305"), Some(CipherKind::ChaCha20Poly1305));
        assert_eq!(serde_json::to_string(&CipherKind::ChaCha20Poly1305).unwrap(), "\"chacha20_poly1305\"");
    }

    #[test]
    fn strength_and_generation() {
        assert!(passphrase_strength("abc").score <= 1);
        let g = generate_passphrase(32);
        assert_eq!(g.len(), 32);
        assert!(passphrase_strength(&g).score >= 3);
    }
}
