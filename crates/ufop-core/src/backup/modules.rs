//! Extension interfaces: encryption, compression and destination modules.
//!
//! Third-party code extends the engine by implementing these traits and
//! registering a factory in a [`ModuleRegistry`]:
//!
//! - [`EncryptionModule`]: seal/open whole volumes (built-ins: `aes`,
//!   `none`; `gpg` in the desktop app). Plugged in through
//!   [`super::crypto::VolumeCipher::external`]. The built-in passphrase
//!   ciphers (`aes256_gcm`, and `chacha20_poly1305` for format 2) are
//!   listed as encryption modules too; [`ModuleRegistry::cipher`] resolves
//!   them for the repository header.
//! - [`CompressionModule`]: per-block compression (built-ins: `deflate`,
//!   `stored`, `bzip2`, `zstd`).
//! - [`super::Backend`]: destinations (built-in: `file`; the desktop app
//!   registers its protocol connectors).
//!
//! See `docs/EXTENDING.md` for a worked example.

use super::backend::{Backend, LocalBackend};
use crate::error::AppError;
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::Arc;

/// Whole-volume encryption.
pub trait EncryptionModule: Send + Sync {
    fn name(&self) -> &str;
    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, AppError>;
    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, AppError>;
}

/// Per-block compression.
pub trait CompressionModule: Send + Sync {
    fn name(&self) -> &str;
    fn compress(&self, data: &[u8], level: u32) -> Result<Vec<u8>, AppError>;
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, AppError>;
}

pub struct Deflate;
pub struct Stored;
pub struct Bzip2;
pub struct Zstd;

fn cerr(e: impl std::fmt::Display) -> AppError {
    AppError::transfer(format!("Compression error: {e}"), "The volume may be damaged; run Verify.")
}

impl CompressionModule for Deflate {
    fn name(&self) -> &str {
        "deflate"
    }
    fn compress(&self, data: &[u8], level: u32) -> Result<Vec<u8>, AppError> {
        let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(level.min(9)));
        e.write_all(data).map_err(cerr)?;
        e.finish().map_err(cerr)
    }
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        let mut v = Vec::new();
        flate2::read::DeflateDecoder::new(data).read_to_end(&mut v).map_err(cerr)?;
        Ok(v)
    }
}

impl CompressionModule for Stored {
    fn name(&self) -> &str {
        "stored"
    }
    fn compress(&self, data: &[u8], _: u32) -> Result<Vec<u8>, AppError> {
        Ok(data.to_vec())
    }
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        Ok(data.to_vec())
    }
}

/// bzip2 and zstd go through single-entry ZIP streams so they share the
/// `zip` crate's codecs instead of adding separate dependencies.
fn zip_roundtrip(method: zip::CompressionMethod, level: Option<i64>, data: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let o = zip::write::SimpleFileOptions::default().compression_method(method).compression_level(level).large_file(data.len() as u64 > u32::MAX as u64);
    w.start_file("b", o).map_err(cerr)?;
    w.write_all(data).map_err(cerr)?;
    Ok(w.finish().map_err(cerr)?.into_inner())
}

fn zip_extract(data: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(data)).map_err(cerr)?;
    let mut f = a.by_index(0).map_err(cerr)?;
    let mut v = Vec::new();
    f.read_to_end(&mut v).map_err(cerr)?;
    Ok(v)
}

impl CompressionModule for Bzip2 {
    fn name(&self) -> &str {
        "bzip2"
    }
    fn compress(&self, data: &[u8], level: u32) -> Result<Vec<u8>, AppError> {
        zip_roundtrip(zip::CompressionMethod::Bzip2, Some(level.clamp(1, 9) as i64), data)
    }
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        zip_extract(data)
    }
}

impl CompressionModule for Zstd {
    fn name(&self) -> &str {
        "zstd"
    }
    fn compress(&self, data: &[u8], level: u32) -> Result<Vec<u8>, AppError> {
        zip_roundtrip(zip::CompressionMethod::Zstd, Some((level as i64 * 2).clamp(1, 21)), data)
    }
    fn decompress(&self, data: &[u8]) -> Result<Vec<u8>, AppError> {
        zip_extract(data)
    }
}

type BackendFactory = Arc<dyn Fn(&str) -> Result<Box<dyn Backend>, AppError> + Send + Sync>;

/// Named module factories. `ModuleRegistry::builtin()` contains the
/// built-in modules; applications add their own with `register_*`.
#[derive(Clone)]
pub struct ModuleRegistry {
    encryption: BTreeMap<String, Arc<dyn EncryptionModule>>,
    compression: BTreeMap<String, Arc<dyn CompressionModule>>,
    backends: BTreeMap<String, BackendFactory>,
}

impl ModuleRegistry {
    pub fn builtin() -> Self {
        let mut r = Self { encryption: BTreeMap::new(), compression: BTreeMap::new(), backends: BTreeMap::new() };
        r.register_compression(Arc::new(Deflate));
        r.register_compression(Arc::new(Stored));
        r.register_compression(Arc::new(Bzip2));
        r.register_compression(Arc::new(Zstd));
        r.register_backend("file", Arc::new(|url: &str| Ok(Box::new(LocalBackend::new(url.trim_start_matches("file://"))) as Box<dyn Backend>)));
        r
    }

    pub fn register_encryption(&mut self, m: Arc<dyn EncryptionModule>) {
        self.encryption.insert(m.name().to_string(), m);
    }
    pub fn register_compression(&mut self, m: Arc<dyn CompressionModule>) {
        self.compression.insert(m.name().to_string(), m);
    }
    pub fn register_backend(&mut self, scheme: &str, f: BackendFactory) {
        self.backends.insert(scheme.to_string(), f);
    }

    pub fn encryption(&self, name: &str) -> Option<Arc<dyn EncryptionModule>> {
        self.encryption.get(name).cloned()
    }
    /// A built-in passphrase cipher by name (`aes256_gcm`, `chacha20_poly1305`).
    pub fn cipher(&self, name: &str) -> Option<super::crypto::CipherKind> {
        super::crypto::CipherKind::parse(name)
    }
    pub fn compression(&self, name: &str) -> Option<Arc<dyn CompressionModule>> {
        self.compression.get(name).cloned()
    }
    /// Open a destination URL (`scheme://…`; bare paths are `file`).
    pub fn backend(&self, url: &str) -> Result<Box<dyn Backend>, AppError> {
        let scheme = url.split_once("://").map(|(s, _)| s).unwrap_or("file");
        let f = self.backends.get(scheme).ok_or_else(|| AppError::validation(format!("No destination module for '{scheme}://'")))?;
        f(url)
    }

    /// `(kind, name)` of every registered module.
    pub fn list(&self) -> Vec<(String, String)> {
        let mut v: Vec<(String, String)> = Vec::new();
        for c in [super::crypto::CipherKind::Aes256Gcm, super::crypto::CipherKind::ChaCha20Poly1305] {
            v.push(("encryption".to_string(), c.as_str().to_string()));
        }
        v.extend(self.encryption.keys().map(|k| ("encryption".to_string(), k.clone())));
        v.extend(self.compression.keys().map(|k| ("compression".to_string(), k.clone())));
        v.extend(self.backends.keys().map(|k| ("backend".to_string(), k.clone())));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::crypto::VolumeCipher;

    struct Rot13;
    impl EncryptionModule for Rot13 {
        fn name(&self) -> &str {
            "xor"
        }
        fn seal(&self, p: &[u8]) -> Result<Vec<u8>, AppError> {
            Ok(p.iter().map(|b| b ^ 0x5a).collect())
        }
        fn open(&self, s: &[u8]) -> Result<Vec<u8>, AppError> {
            self.seal(s)
        }
    }

    #[test]
    fn compression_modules_roundtrip() {
        let data: Vec<u8> = b"hello hello hello hello".repeat(100);
        let r = ModuleRegistry::builtin();
        for n in ["deflate", "stored", "bzip2", "zstd"] {
            let m = r.compression(n).unwrap();
            let c = m.compress(&data, 6).unwrap();
            assert_eq!(m.decompress(&c).unwrap(), data, "{n}");
            if n != "stored" {
                assert!(c.len() < data.len(), "{n} compresses");
            }
        }
    }

    #[test]
    fn external_encryption_plugs_into_cipher() {
        let c = VolumeCipher::external(Arc::new(Rot13));
        assert!(c.is_encrypted());
        assert_eq!(c.module_name(), "xor");
        let sealed = c.seal(b"secret").unwrap();
        assert_eq!(c.open(&sealed).unwrap(), b"secret");
        // A plain cipher refuses external volumes with a clear message.
        assert!(VolumeCipher::plain().open(&sealed).is_err());
        let mut r = ModuleRegistry::builtin();
        r.register_encryption(Arc::new(Rot13));
        assert!(r.list().contains(&("encryption".into(), "xor".into())));
        assert!(r.list().contains(&("encryption".into(), "chacha20_poly1305".into())));
        assert_eq!(r.cipher("chacha20_poly1305"), Some(crate::backup::crypto::CipherKind::ChaCha20Poly1305));
        let d = tempfile::tempdir().unwrap();
        let b = r.backend(&d.path().to_string_lossy()).unwrap();
        assert_eq!(b.test(false).unwrap(), 0);
        assert!(r.backend("nope://x").is_err());
    }
}
