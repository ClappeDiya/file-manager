//! Repository format versions, header creation, and the header-only
//! operations: upgrade v1 → v2, rollback, and password change.
//!
//! **Format 1**: fixed-size blocks, per-block deflate, JSON metadata sealed
//! as is, AES-256-GCM volumes keyed directly by the Argon2id-derived key,
//! flat layout.
//!
//! **Format 2** adds (each recorded in the header, see [`RepoHeader`]):
//! - an envelope key: volumes are keyed by a master key stored in the header
//!   wrapped by the Argon2id key-encryption key (KEK), so a password change
//!   rewrites only the header ([`change_password`]);
//! - zstd block codec (pack flag 2) and zstd-compressed dlist/dindex;
//! - content-defined chunking (FastCDC) recorded as the header's chunker;
//! - ChaCha20-Poly1305 as an alternative to AES-256-GCM;
//! - an optional sharded layout ([`super::layout`]).
//!
//! Readers dispatch on the header's version and `features`; flags they do
//! not know make them refuse the repository instead of misreading it.
//!
//! [`upgrade_repository`] converts a v1 repository in place by rewriting
//! only its header: the v2 master key *is* the old passphrase-derived key,
//! so every existing volume stays readable. Until the first write after the
//! upgrade, [`rollback_upgrade`] restores the saved v1 header.

use super::backend::Backend;
use super::chunker::ChunkerSpec;
use super::crypto::{CipherKind, KdfParams, VolumeCipher};
use super::layout::RepoLayout;
use super::volume::{self, RepoHeader};
use super::{BackupOptions, BlockCodec, ChunkerKind, MetadataCompression, MIN_REPO_FORMAT_VERSION, REPO_FORMAT_VERSION};
use crate::error::AppError;
use serde::{Deserialize, Serialize};

pub const FEATURE_ENVELOPE_KEY: &str = "envelope_key";
pub const FEATURE_ZSTD_BLOCKS: &str = "zstd_blocks";
pub const FEATURE_ZSTD_METADATA: &str = "zstd_metadata";
pub const FEATURE_CDC: &str = "content_defined_chunking";
pub const FEATURE_CHACHA: &str = "chacha20_poly1305";
pub const FEATURE_SHARDED: &str = "sharded_layout";
/// Set by [`upgrade_repository`]; cleared by the first write in format 2.
pub const FEATURE_V1_ROLLBACK: &str = "v1_rollback_available";

/// Every feature flag this version understands.
pub const KNOWN_FEATURES: &[&str] =
    &[FEATURE_ENVELOPE_KEY, FEATURE_ZSTD_BLOCKS, FEATURE_ZSTD_METADATA, FEATURE_CDC, FEATURE_CHACHA, FEATURE_SHARDED, FEATURE_V1_ROLLBACK];

pub fn v1_backup_name(prefix: &str) -> String {
    format!("{prefix}-repo.v1.bak.json")
}

fn previous_header_name(prefix: &str) -> String {
    format!("{prefix}-repo.bak.json")
}

fn new_header_name(prefix: &str) -> String {
    format!("{prefix}-repo.new.json")
}

fn newer() -> String {
    "Update the app to a version that supports this backup.".into()
}

/// Refuse headers this version cannot read correctly.
pub fn check_readable(h: &RepoHeader) -> Result<(), AppError> {
    if h.format_version > REPO_FORMAT_VERSION {
        return Err(AppError::validation(format!("This backup was written by a newer version (format {}); update the app", h.format_version)));
    }
    if h.format_version < MIN_REPO_FORMAT_VERSION {
        return Err(AppError::validation(format!("Unknown backup format {}", h.format_version)));
    }
    if let Some(f) = h.features.iter().find(|f| !KNOWN_FEATURES.contains(&f.as_str())) {
        return Err(AppError::state(format!("This backup uses a feature this version does not know ({f})"), newer()));
    }
    let v2_only = h.chunker.is_some_and(|c| c.is_content_defined())
        || h.block_codec != BlockCodec::Deflate
        || h.metadata_compression != MetadataCompression::None
        || h.cipher != CipherKind::Aes256Gcm
        || h.wrapped_master_key.is_some()
        || h.layout.is_sharded();
    if h.format_version < 2 && v2_only {
        return Err(AppError::state("The repository header is inconsistent: format 1 with format 2 settings", "Restore the header from its .bak copy or recreate it."));
    }
    h.layout.validate()
}

/// Recompute the feature flags implied by the header's settings (keeps
/// [`FEATURE_V1_ROLLBACK`] when present).
pub fn sync_features(h: &mut RepoHeader) {
    let mut f: Vec<String> = Vec::new();
    let mut add = |on: bool, name: &str| {
        if on {
            f.push(name.to_string());
        }
    };
    add(h.wrapped_master_key.is_some(), FEATURE_ENVELOPE_KEY);
    add(h.block_codec == BlockCodec::Zstd, FEATURE_ZSTD_BLOCKS);
    add(h.metadata_compression == MetadataCompression::Zstd, FEATURE_ZSTD_METADATA);
    add(h.chunker.is_some_and(|c| c.is_content_defined()), FEATURE_CDC);
    add(h.encrypted && h.wrapped_master_key.is_some() && h.cipher == CipherKind::ChaCha20Poly1305, FEATURE_CHACHA);
    add(h.layout.is_sharded(), FEATURE_SHARDED);
    add(h.has_feature(FEATURE_V1_ROLLBACK), FEATURE_V1_ROLLBACK);
    h.features = f;
}

/// Apply the header's sealing settings (AEAD, metadata compression) to a
/// cipher holding the repository's volume key.
pub fn configure_cipher(h: &RepoHeader, c: VolumeCipher) -> VolumeCipher {
    c.with_aead(h.cipher).with_metadata_compression(h.metadata_compression == MetadataCompression::Zstd)
}

/// Built-in key material of a new repository: (volume cipher, salt hex,
/// wrapped master key, key check).
type NewKeys = (VolumeCipher, Option<String>, Option<String>, Option<String>);

/// Derive the KEK and, for v2, create and wrap a random master key.
fn new_keys(version: u32, passphrase: &str, kdf: KdfParams, aead: CipherKind) -> Result<NewKeys, AppError> {
    let salt = VolumeCipher::random_salt();
    let kek = VolumeCipher::derive(passphrase, &salt, kdf)?.with_aead(aead);
    let check = kek.key_check()?;
    if version < 2 {
        return Ok((kek, Some(hex::encode(&salt)), None, check));
    }
    let master = VolumeCipher::random_master(aead);
    let wrapped = kek.wrap_key(&master)?;
    Ok((master, Some(hex::encode(&salt)), Some(wrapped), check))
}

/// Build the header (and volume cipher) of a new repository from `options`.
/// `external` is an external encryption module (GPG); otherwise a non-empty
/// `passphrase` selects built-in encryption.
pub fn create_header(options: &BackupOptions, passphrase: Option<&str>, external: Option<VolumeCipher>, kdf: KdfParams, backend: &dyn Backend) -> Result<(RepoHeader, VolumeCipher), AppError> {
    let version = options.format_version.unwrap_or(REPO_FORMAT_VERSION);
    if !(MIN_REPO_FORMAT_VERSION..=REPO_FORMAT_VERSION).contains(&version) {
        return Err(AppError::validation(format!("Repository format {version} is not supported (use 1 or 2)")));
    }
    let v2 = version >= 2;
    let needs_v2 = |what: &str| AppError::state(format!("{what} needs repository format 2"), "Leave the format at 2 (the default) or turn this option off.");
    if !v2 {
        if options.chunker == Some(ChunkerKind::FastCdc) {
            return Err(needs_v2("Content-defined chunking"));
        }
        if options.block_codec == Some(BlockCodec::Zstd) {
            return Err(needs_v2("The zstd block codec"));
        }
        if options.metadata_compression == Some(MetadataCompression::Zstd) {
            return Err(needs_v2("Metadata compression"));
        }
        if options.cipher != CipherKind::Aes256Gcm {
            return Err(needs_v2("ChaCha20-Poly1305"));
        }
        if options.layout.is_sharded() {
            return Err(needs_v2("The sharded layout"));
        }
    }
    options.layout.validate()?;
    if options.layout.is_sharded() && !backend.supports_subfolders() {
        return Err(AppError::state(format!("{} does not support subfolders", backend.describe()), "Use the flat layout on this destination."));
    }
    let block_size = options.block_size.max(1024);
    let (cipher, salt, wrapped, key_check, aes) = match (external, passphrase.filter(|p| !p.is_empty())) {
        (Some(ext), _) => {
            let kc = ext.key_check()?;
            (ext, None, None, kc, false)
        }
        (None, Some(p)) => {
            let (c, salt, wrapped, kc) = new_keys(version, p, kdf, if v2 { options.cipher } else { CipherKind::Aes256Gcm })?;
            (c, salt, wrapped, kc, true)
        }
        (None, None) => (VolumeCipher::plain(), None, None, None, false),
    };
    let mut header = RepoHeader {
        format_version: version,
        encrypted: cipher.is_encrypted(),
        salt,
        kdf: aes.then_some(kdf),
        key_check: if cipher.is_encrypted() { key_check } else { None },
        block_size,
        block_hash: options.block_hash,
        file_hash: options.file_hash,
        volume_format: options.volume_format,
        encryption_module: if aes { "aes".into() } else if cipher.is_encrypted() { cipher.module_name() } else { String::new() },
        gpg_recipients: if cipher.module_name() == "gpg" { options.gpg.recipients.clone() } else { Vec::new() },
        chunker: v2.then(|| match options.chunker.unwrap_or_default() {
            ChunkerKind::Fixed => ChunkerSpec::Fixed { size: block_size },
            ChunkerKind::FastCdc => ChunkerSpec::fastcdc(block_size),
        }),
        block_codec: if v2 { options.block_codec.unwrap_or(BlockCodec::Zstd) } else { BlockCodec::Deflate },
        metadata_compression: if v2 { options.metadata_compression.unwrap_or(MetadataCompression::Zstd) } else { MetadataCompression::None },
        cipher: if v2 && aes { options.cipher } else { CipherKind::Aes256Gcm },
        wrapped_master_key: wrapped,
        layout: if v2 { options.layout } else { RepoLayout::Flat },
        features: Vec::new(),
    };
    sync_features(&mut header);
    let cipher = configure_cipher(&header, cipher);
    Ok((header, cipher))
}

/// Read and parse the header of `prefix`.
pub fn read_header(backend: &dyn Backend, prefix: &str) -> Result<(RepoHeader, Vec<u8>), AppError> {
    let raw = backend.get(&volume::repo_header_name(prefix)).map_err(|_| {
        AppError::file_op(format!("No backup with prefix '{prefix}' at {}", backend.describe()), "Check the destination and file-name prefix.")
    })?;
    Ok((serde_json::from_slice(&raw)?, raw))
}

/// Replace the header safely: the current header is kept as
/// `{prefix}-repo.bak.json`, the new one is written under a temporary name
/// and moved into place, read back and compared; only then is the backup
/// copy removed.
pub fn write_header_raw(backend: &dyn Backend, prefix: &str, data: &[u8]) -> Result<(), AppError> {
    let hname = volume::repo_header_name(prefix);
    let bak = previous_header_name(prefix);
    let tmp = new_header_name(prefix);
    let had_old = match backend.get(&hname) {
        Ok(old) => {
            backend.put(&bak, &old)?;
            true
        }
        Err(_) => false,
    };
    backend.put(&tmp, data)?;
    backend.rename(&tmp, &hname)?;
    if backend.get(&hname)? != data {
        return Err(AppError::transfer(
            "The new repository header did not read back correctly",
            format!("The previous header is kept as {bak}; rename it back to {hname} if the backup cannot be opened."),
        ));
    }
    if had_old {
        let _ = backend.delete(&bak);
    }
    Ok(())
}

pub fn write_header(backend: &dyn Backend, prefix: &str, h: &RepoHeader) -> Result<(), AppError> {
    write_header_raw(backend, prefix, &serde_json::to_vec_pretty(h)?)
}

/// Summary of a format upgrade / rollback / password change.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct FormatReport {
    pub from_version: u32,
    pub to_version: u32,
    /// Nothing to do (already in the target format).
    pub unchanged: bool,
    pub features: Vec<String>,
    pub message: String,
}

/// Upgrade the repository at `backend` to `target` (only 2 exists). Only
/// the header is rewritten; the original is saved as
/// `{prefix}-repo.v1.bak.json` for [`rollback_upgrade`].
///
/// Built-in encryption: the v2 master key is the existing derived key,
/// wrapped by itself as KEK, so the same passphrase opens it and every v1
/// volume stays readable. New data is written with zstd blocks and
/// compressed metadata; the chunker stays fixed-size so deduplication
/// against existing blocks continues.
pub fn upgrade_repository(backend: &dyn Backend, prefix: &str, passphrase: Option<&str>, target: u32) -> Result<FormatReport, AppError> {
    if target != 2 {
        return Err(AppError::validation(format!("Cannot upgrade to format {target}; the newest format is {REPO_FORMAT_VERSION}")));
    }
    let (old, raw) = read_header(backend, prefix)?;
    check_readable(&old)?;
    let mut r = FormatReport { from_version: old.format_version, to_version: target, ..Default::default() };
    if old.format_version >= target {
        r.unchanged = true;
        r.features = old.features.clone();
        r.message = format!("The repository already uses format {}", old.format_version);
        return Ok(r);
    }
    let mut h = old.clone();
    if old.encrypted && old.encryption_module != "gpg" && old.salt.is_some() {
        // Prove the passphrase, then wrap the current key with itself.
        let kek = super::engine::header_cipher(&old, passphrase, None)?;
        h.wrapped_master_key = Some(kek.wrap_key(&kek)?);
    }
    h.format_version = target;
    h.chunker = Some(ChunkerSpec::Fixed { size: old.block_size });
    h.block_codec = BlockCodec::Zstd;
    h.metadata_compression = MetadataCompression::Zstd;
    h.cipher = CipherKind::Aes256Gcm;
    h.layout = RepoLayout::Flat;
    h.features = vec![FEATURE_V1_ROLLBACK.to_string()];
    sync_features(&mut h);
    backend.put(&v1_backup_name(prefix), &raw)?;
    write_header(backend, prefix, &h)?;
    // Prove the new header opens with the same passphrase before reporting success.
    if h.wrapped_master_key.is_some() {
        super::engine::header_cipher(&h, passphrase, None)?;
    }
    r.features = h.features.clone();
    r.message = format!("Upgraded to format {target}. Until the next backup or maintenance write you can roll back to format 1.");
    Ok(r)
}

/// Restore the v1 header saved by [`upgrade_repository`]. Refused once
/// anything was written in format 2 (or the password changed), because v1
/// cannot read that data.
pub fn rollback_upgrade(backend: &dyn Backend, prefix: &str) -> Result<FormatReport, AppError> {
    let (h, _) = read_header(backend, prefix)?;
    let refuse = |why: &str| {
        AppError::state(
            format!("Cannot roll back to format 1: {why}"),
            "Keep using format 2; it reads all existing data. Restore or Convert can produce a format 1 copy if one is needed.",
        )
    };
    if h.format_version < 2 {
        return Err(refuse("the repository already uses format 1"));
    }
    if !h.has_feature(FEATURE_V1_ROLLBACK) {
        return Err(refuse("data was written in format 2 since the upgrade, the password changed, or the repository was created as format 2"));
    }
    let bak = backend.get(&v1_backup_name(prefix)).map_err(|_| refuse("the saved format 1 header is missing"))?;
    let v1: RepoHeader = serde_json::from_slice(&bak).map_err(|_| refuse("the saved format 1 header is damaged"))?;
    if v1.format_version != 1 {
        return Err(refuse("the saved header is not format 1"));
    }
    write_header_raw(backend, prefix, &bak)?;
    let _ = backend.delete(&v1_backup_name(prefix));
    Ok(FormatReport { from_version: h.format_version, to_version: 1, unchanged: false, features: Vec::new(), message: "Rolled back to format 1.".into() })
}

/// Called before the first format-2 write to an upgraded repository: the
/// v1 header can no longer describe the data, so drop the rollback option.
pub fn clear_rollback(backend: &dyn Backend, prefix: &str) -> Result<(), AppError> {
    let (mut h, _) = read_header(backend, prefix)?;
    if !h.has_feature(FEATURE_V1_ROLLBACK) {
        return Ok(());
    }
    h.features.retain(|f| f != FEATURE_V1_ROLLBACK);
    write_header(backend, prefix, &h)?;
    let _ = backend.delete(&v1_backup_name(prefix));
    Ok(())
}

/// Change the passphrase of a format 2 repository with built-in encryption:
/// the master key is unwrapped with the old KEK and rewrapped with a new
/// one (new salt, optional new Argon2id cost). Only the header changes.
pub fn change_password(backend: &dyn Backend, prefix: &str, old: &str, new: &str, kdf: Option<KdfParams>) -> Result<FormatReport, AppError> {
    if new.is_empty() {
        return Err(AppError::validation("The new passphrase is empty"));
    }
    let (mut h, _) = read_header(backend, prefix)?;
    check_readable(&h)?;
    if !h.encrypted || h.encryption_module == "gpg" {
        return Err(AppError::state("This backup does not use passphrase encryption", "Only backups encrypted with the built-in cipher have a passphrase to change."));
    }
    let Some(wrapped) = h.wrapped_master_key.clone() else {
        return Err(AppError::state(
            "Changing the passphrase in place needs repository format 2",
            "Upgrade the repository to format 2 first (it keeps all data), or use Recovery → Convert to re-encrypt a copy.",
        ));
    };
    let salt = hex::decode(h.salt.as_deref().unwrap_or_default()).map_err(|_| AppError::validation("Repository header salt is malformed"))?;
    let kek_old = VolumeCipher::derive(old, &salt, h.kdf.unwrap_or_default())?.with_aead(h.cipher);
    kek_old.verify_key_check(h.key_check.as_deref().unwrap_or_default())?;
    let master = kek_old.unwrap_key(&wrapped, h.cipher)?;
    let kdf = kdf.or(h.kdf).unwrap_or_default();
    kdf.validate().or_else(|e| if kdf == KdfParams::insecure_fast() { Ok(()) } else { Err(e) })?;
    let new_salt = VolumeCipher::random_salt();
    let kek_new = VolumeCipher::derive(new, &new_salt, kdf)?.with_aead(h.cipher);
    h.salt = Some(hex::encode(&new_salt));
    h.kdf = Some(kdf);
    h.key_check = kek_new.key_check()?;
    h.wrapped_master_key = Some(kek_new.wrap_key(&master)?);
    // The saved v1 header would need the old passphrase: no rollback.
    let had_rollback = h.has_feature(FEATURE_V1_ROLLBACK);
    h.features.retain(|f| f != FEATURE_V1_ROLLBACK);
    sync_features(&mut h);
    write_header(backend, prefix, &h)?;
    let reopened = super::engine::header_cipher(&h, Some(new), None)?;
    if !reopened.same_key(&master) {
        return Err(AppError::internal("password change verification failed"));
    }
    if had_rollback {
        let _ = backend.delete(&v1_backup_name(prefix));
    }
    Ok(FormatReport { from_version: h.format_version, to_version: h.format_version, unchanged: false, features: h.features.clone(), message: "Passphrase changed.".into() })
}

/// Settings of a repository, for the UI and `ufop backup info`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FormatInfo {
    pub format_version: u32,
    pub chunker: ChunkerSpec,
    pub block_codec: BlockCodec,
    pub metadata_compression: MetadataCompression,
    pub cipher: Option<CipherKind>,
    pub envelope_key: bool,
    pub layout: RepoLayout,
    pub features: Vec<String>,
    pub rollback_available: bool,
}

impl From<&RepoHeader> for FormatInfo {
    fn from(h: &RepoHeader) -> Self {
        Self {
            format_version: h.format_version,
            chunker: h.chunker_spec(),
            block_codec: h.block_codec,
            metadata_compression: h.metadata_compression,
            cipher: (h.encrypted && h.encryption_module != "gpg").then_some(h.cipher),
            envelope_key: h.wrapped_master_key.is_some(),
            layout: h.layout,
            features: h.features.clone(),
            rollback_available: h.has_feature(FEATURE_V1_ROLLBACK),
        }
    }
}

/// Format settings of the repository at `backend` (no passphrase needed).
pub fn format_info(backend: &dyn Backend, prefix: &str) -> Result<FormatInfo, AppError> {
    let (h, _) = read_header(backend, prefix)?;
    Ok(FormatInfo::from(&h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::db::BackupDb;
    use crate::backup::engine::Repo;
    use crate::backup::test_util::write;
    use crate::backup::{maintenance, run_backup, run_restore, OpContext, RestoreOptions, RetentionPolicy, SourceFilter};
    use std::path::Path;

    fn opts(version: Option<u32>) -> BackupOptions {
        BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 0, format_version: version, ..Default::default() }
    }

    fn backup(src: &Path, be: &dyn Backend, db: &BackupDb, o: &BackupOptions, pass: Option<&str>) -> crate::backup::BackupReport {
        let r = run_backup(be, db, &[src.into()], &SourceFilter::default(), o, &RetentionPolicy::KeepAll, pass, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        r
    }

    fn restore_all(be: &dyn Backend, pass: Option<&str>, out: &Path) {
        let db = BackupDb::open_in_memory().unwrap();
        maintenance::recreate_database(be, &db, pass, "ufop", &OpContext::new()).unwrap();
        let repo = Repo::open(be, &db, pass, &BackupOptions::default(), false, KdfParams::insecure_fast()).unwrap();
        let r = run_restore(&repo, &RestoreOptions { target: Some(out.into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let v = maintenance::verify(&repo, None, true).unwrap();
        assert!(v.errors.is_empty(), "{:?}", v.errors);
    }

    fn header(dir: &Path) -> RepoHeader {
        serde_json::from_slice(&std::fs::read(dir.join("ufop-repo.json")).unwrap()).unwrap()
    }

    fn data(len: usize, seed: u64) -> Vec<u8> {
        (0..len as u64).map(|i| ((i ^ seed).wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407) >> 37) as u8).collect()
    }

    #[test]
    fn new_repositories_default_to_v2_and_v1_stays_writable() {
        let (src, d1, d2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "a.bin", &data(20_000, 1));
        let (b1, b2) = (LocalBackend::new(d1.path()), LocalBackend::new(d2.path()));
        backup(src.path(), &b2, &BackupDb::open_in_memory().unwrap(), &opts(None), Some("pw"));
        let h = header(d2.path());
        assert_eq!(h.format_version, 2);
        assert_eq!(h.chunker, Some(ChunkerSpec::fastcdc(1024)));
        assert_eq!((h.block_codec, h.metadata_compression), (BlockCodec::Zstd, MetadataCompression::Zstd));
        assert!(h.wrapped_master_key.is_some());
        for f in [FEATURE_ENVELOPE_KEY, FEATURE_ZSTD_BLOCKS, FEATURE_ZSTD_METADATA, FEATURE_CDC] {
            assert!(h.has_feature(f), "{f}");
        }
        // Explicit format 1: the exact v1 header (no v2 fields written).
        let db1 = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &b1, &db1, &opts(Some(1)), Some("pw"));
        let raw = std::fs::read_to_string(d1.path().join("ufop-repo.json")).unwrap();
        assert!(!raw.contains("chunker") && !raw.contains("features") && !raw.contains("wrapped"), "{raw}");
        assert_eq!(header(d1.path()).format_version, 1);
        // A second run on a v1 repository keeps writing v1.
        write(src.path(), "b.bin", &data(5000, 2));
        backup(src.path(), &b1, &db1, &opts(None), Some("pw"));
        assert_eq!(std::fs::read_to_string(d1.path().join("ufop-repo.json")).unwrap(), raw);
        for (b, d) in [(&b1, d1.path()), (&b2, d2.path())] {
            let out = tempfile::tempdir().unwrap();
            restore_all(b, Some("pw"), out.path());
            let _ = d;
        }
        // v2-only options are refused for format 1.
        let o = BackupOptions { cipher: crate::backup::crypto::CipherKind::ChaCha20Poly1305, ..opts(Some(1)) };
        assert!(create_header(&o, Some("pw"), None, KdfParams::insecure_fast(), &b1).is_err());
        assert!(create_header(&opts(Some(3)), None, None, KdfParams::insecure_fast(), &b1).is_err());
    }

    #[test]
    fn unknown_features_and_newer_versions_are_refused() {
        let d = tempfile::tempdir().unwrap();
        let be = LocalBackend::new(d.path());
        let (mut h, _) = create_header(&opts(None), None, None, KdfParams::insecure_fast(), &be).unwrap();
        assert!(check_readable(&h).is_ok());
        h.features.push("from_the_future".into());
        assert!(check_readable(&h).unwrap_err().user_message().contains("from_the_future"));
        h.features.pop();
        h.format_version = 3;
        assert!(check_readable(&h).is_err());
        h.format_version = 1;
        assert!(check_readable(&h).is_err(), "v1 header with v2 settings");
    }

    #[test]
    fn upgrade_then_rollback_before_any_write() {
        let (src, dst, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "a.bin", &data(9000, 3));
        let be = LocalBackend::new(dst.path());
        backup(src.path(), &be, &BackupDb::open_in_memory().unwrap(), &opts(Some(1)), Some("pw"));
        let v1_raw = std::fs::read(dst.path().join("ufop-repo.json")).unwrap();
        assert!(rollback_upgrade(&be, "ufop").is_err(), "nothing to roll back");
        assert!(upgrade_repository(&be, "ufop", Some("wrong"), 2).is_err());
        assert_eq!(std::fs::read(dst.path().join("ufop-repo.json")).unwrap(), v1_raw, "failed upgrade changes nothing");
        let r = upgrade_repository(&be, "ufop", Some("pw"), 2).unwrap();
        assert_eq!((r.from_version, r.to_version, r.unchanged), (1, 2, false));
        let h = header(dst.path());
        assert_eq!(h.chunker, Some(ChunkerSpec::Fixed { size: 1024 }), "fixed chunks keep deduplicating");
        assert!(h.has_feature(FEATURE_V1_ROLLBACK) && h.wrapped_master_key.is_some());
        assert!(dst.path().join("ufop-repo.v1.bak.json").is_file());
        assert!(!dst.path().join("ufop-repo.bak.json").exists() && !dst.path().join("ufop-repo.new.json").exists());
        assert!(upgrade_repository(&be, "ufop", Some("pw"), 2).unwrap().unchanged);
        // Old volumes stay readable through the envelope key.
        restore_all(&be, Some("pw"), out.path());
        let r = rollback_upgrade(&be, "ufop").unwrap();
        assert_eq!(r.to_version, 1);
        assert_eq!(std::fs::read(dst.path().join("ufop-repo.json")).unwrap(), v1_raw);
        assert!(!dst.path().join("ufop-repo.v1.bak.json").exists());
    }

    #[test]
    fn write_after_upgrade_removes_rollback_and_mixes_formats() {
        let (src, dst, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "a.bin", &data(9000, 4));
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db, &opts(Some(1)), Some("pw"));
        upgrade_repository(&be, "ufop", Some("pw"), 2).unwrap();
        write(src.path(), "b.bin", &data(9000, 5));
        let r = backup(src.path(), &be, &db, &opts(None), Some("pw"));
        assert!(r.new_blocks > 0);
        let h = header(dst.path());
        assert!(!h.has_feature(FEATURE_V1_ROLLBACK));
        assert!(!dst.path().join("ufop-repo.v1.bak.json").exists());
        let e = rollback_upgrade(&be, "ufop").unwrap_err();
        assert!(e.user_message().contains("Cannot roll back"), "{}", e.user_message());
        // v1 deflate volumes and v2 zstd volumes restore together.
        restore_all(&be, Some("pw"), out.path());
        let base = out.path().join(src.path().file_name().unwrap());
        assert_eq!(std::fs::read(base.join("a.bin")).unwrap(), data(9000, 4));
        assert_eq!(std::fs::read(base.join("b.bin")).unwrap(), data(9000, 5));
    }

    #[test]
    fn unencrypted_upgrade_and_rollback() {
        let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "a.bin", b"hello");
        let be = LocalBackend::new(dst.path());
        backup(src.path(), &be, &BackupDb::open_in_memory().unwrap(), &opts(Some(1)), None);
        let r = upgrade_repository(&be, "ufop", None, 2).unwrap();
        assert!(!r.features.contains(&FEATURE_ENVELOPE_KEY.to_string()));
        assert!(header(dst.path()).wrapped_master_key.is_none());
        rollback_upgrade(&be, "ufop").unwrap();
        assert_eq!(header(dst.path()).format_version, 1);
    }

    #[test]
    fn change_password_rewraps_header_only() {
        let (src, dst, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "a.bin", &data(9000, 6));
        let be = LocalBackend::new(dst.path());
        backup(src.path(), &be, &BackupDb::open_in_memory().unwrap(), &opts(None), Some("old-pw"));
        let volumes = |d: &Path| -> Vec<(String, Vec<u8>)> {
            let mut v: Vec<(String, Vec<u8>)> = std::fs::read_dir(d)
                .unwrap()
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n != "ufop-repo.json")
                .map(|n| (n.clone(), std::fs::read(d.join(&n)).unwrap()))
                .collect();
            v.sort();
            v
        };
        let before = volumes(dst.path());
        assert!(change_password(&be, "ufop", "wrong", "new-pw", None).is_err());
        change_password(&be, "ufop", "old-pw", "new-pw", Some(KdfParams::insecure_fast())).unwrap();
        assert_eq!(volumes(dst.path()), before, "no volume rewritten, no temp/backup header left");
        let h = header(dst.path());
        assert!(crate::backup::engine::header_cipher(&h, Some("old-pw"), None).is_err());
        restore_all(&be, Some("new-pw"), out.path());
        // v1 repositories need an upgrade first.
        let d1 = tempfile::tempdir().unwrap();
        let b1 = LocalBackend::new(d1.path());
        backup(src.path(), &b1, &BackupDb::open_in_memory().unwrap(), &opts(Some(1)), Some("pw"));
        assert!(change_password(&b1, "ufop", "pw", "x", None).unwrap_err().user_message().contains("format 2"));
        upgrade_repository(&b1, "ufop", Some("pw"), 2).unwrap();
        change_password(&b1, "ufop", "pw", "x", Some(KdfParams::insecure_fast())).unwrap();
        assert!(rollback_upgrade(&b1, "ufop").is_err(), "rollback would need the old passphrase");
        restore_all(&b1, Some("x"), tempfile::tempdir().unwrap().path());
    }

    #[test]
    fn header_write_keeps_previous_until_success() {
        struct NoRename(LocalBackend);
        impl Backend for NoRename {
            fn describe(&self) -> String { self.0.describe() }
            fn list(&self) -> Result<Vec<crate::backup::RemoteFile>, AppError> { self.0.list() }
            fn put(&self, n: &str, d: &[u8]) -> Result<(), AppError> { self.0.put(n, d) }
            fn get(&self, n: &str) -> Result<Vec<u8>, AppError> { self.0.get(n) }
            fn delete(&self, n: &str) -> Result<(), AppError> { self.0.delete(n) }
            fn create_folder(&self) -> Result<(), AppError> { self.0.create_folder() }
            fn rename(&self, _: &str, _: &str) -> Result<(), AppError> { Err(AppError::connection("offline", "retry")) }
        }
        let d = tempfile::tempdir().unwrap();
        let be = NoRename(LocalBackend::new(d.path()));
        be.put("ufop-repo.json", b"{\"old\":1}").unwrap();
        assert!(write_header_raw(&be, "ufop", b"{\"new\":1}").is_err());
        assert_eq!(std::fs::read(d.path().join("ufop-repo.json")).unwrap(), b"{\"old\":1}");
        assert_eq!(std::fs::read(d.path().join("ufop-repo.bak.json")).unwrap(), b"{\"old\":1}");
    }

    #[test]
    fn chacha_cdc_sharded_repository_full_cycle() {
        let (src, dst, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "a.bin", &data(50_000, 7));
        write(src.path(), "b.txt", b"small file");
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        let o = BackupOptions {
            cipher: crate::backup::crypto::CipherKind::ChaCha20Poly1305,
            layout: RepoLayout::Sharded { prefix_len: 1 },
            ..opts(None)
        };
        backup(src.path(), &be, &db, &o, Some("pw"));
        let h = header(dst.path());
        assert!(h.has_feature(FEATURE_CHACHA) && h.has_feature(FEATURE_SHARDED));
        // No block or index volume at the top level.
        let top: Vec<String> = std::fs::read_dir(dst.path()).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(top.iter().all(|n| !n.contains(".dblock") && !n.contains(".dindex")), "{top:?}");
        let shards = top.iter().filter(|n| dst.path().join(n).is_dir()).count();
        assert!(shards >= 1);
        // Sealed with ChaCha20-Poly1305 (flag 3).
        let dl = top.iter().find(|n| n.contains(".dlist")).unwrap();
        assert_eq!(std::fs::read(dst.path().join(dl)).unwrap()[6], 3);
        let repo = Repo::open(&be, &db, Some("pw"), &o, false, KdfParams::insecure_fast()).unwrap();
        assert!(repo.consistency_check().unwrap().is_clean());
        // Second version (insertion) + compaction after deleting the first.
        let mut changed = data(50_000, 7);
        changed.splice(100..100, b"INSERTED".iter().copied());
        write(src.path(), "a.bin", &changed);
        let r = backup(src.path(), &be, &db, &o, Some("pw"));
        assert!(r.new_blocks <= 6, "CDC re-uses most blocks: {}", r.new_blocks);
        restore_all(&be, Some("pw"), out.path());
        assert_eq!(std::fs::read(out.path().join(src.path().file_name().unwrap()).join("a.bin")).unwrap(), changed);
        assert_eq!(maintenance::delete_versions(&repo, &[1], false).unwrap(), 1);
        maintenance::compact(&repo, &BackupOptions { compact_threshold_percent: 1, ..o.clone() }, &OpContext::new()).unwrap();
        assert!(repo.consistency_check().unwrap().is_clean());
        let out2 = tempfile::tempdir().unwrap();
        restore_all(&be, Some("pw"), out2.path());
        assert_eq!(std::fs::read(out2.path().join(src.path().file_name().unwrap()).join("a.bin")).unwrap(), changed);
    }

    #[test]
    fn reshard_moves_volumes_both_ways() {
        let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "a.bin", &data(30_000, 8));
        let be = LocalBackend::new(dst.path());
        let db = BackupDb::open_in_memory().unwrap();
        backup(src.path(), &be, &db, &opts(None), Some("pw"));
        let to = RepoLayout::Sharded { prefix_len: 2 };
        let dry = crate::backup::layout::reshard(&be, "ufop", to, true, &OpContext::new()).unwrap();
        assert!(dry.moved >= 2 && header(dst.path()).layout == RepoLayout::Flat);
        let r = crate::backup::layout::reshard(&be, "ufop", to, false, &OpContext::new()).unwrap();
        assert_eq!(r.moved, dry.moved);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(header(dst.path()).layout, to);
        assert!(header(dst.path()).has_feature(FEATURE_SHARDED));
        let out = tempfile::tempdir().unwrap();
        restore_all(&be, Some("pw"), out.path());
        // The existing local database keeps working, and new backups shard.
        write(src.path(), "c.bin", &data(3000, 9));
        backup(src.path(), &be, &db, &opts(None), Some("pw"));
        let again = crate::backup::layout::reshard(&be, "ufop", to, false, &OpContext::new()).unwrap();
        assert_eq!(again.moved, 0);
        let back = crate::backup::layout::reshard(&be, "ufop", RepoLayout::Flat, false, &OpContext::new()).unwrap();
        assert!(back.moved > r.moved);
        assert!(std::fs::read_dir(dst.path()).unwrap().flatten().all(|e| e.path().is_file()), "shard folders removed");
        restore_all(&be, Some("pw"), tempfile::tempdir().unwrap().path());
        // Format 1 cannot shard.
        let d1 = tempfile::tempdir().unwrap();
        let b1 = LocalBackend::new(d1.path());
        backup(src.path(), &b1, &BackupDb::open_in_memory().unwrap(), &opts(Some(1)), None);
        assert!(crate::backup::layout::reshard(&b1, "ufop", to, false, &OpContext::new()).is_err());
    }

    #[test]
    fn mirror_and_recovery_handle_sharded_repositories() {
        let (src, dst, copy, rec, out) =
            (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "a.bin", &data(20_000, 10));
        let be = LocalBackend::new(dst.path());
        let o = BackupOptions { layout: RepoLayout::Sharded { prefix_len: 1 }, ..opts(None) };
        backup(src.path(), &be, &BackupDb::open_in_memory().unwrap(), &o, Some("pw"));
        let cb = LocalBackend::new(copy.path());
        let m = crate::backup::mirror::mirror(&be, &cb, "ufop", &Default::default(), &OpContext::new()).unwrap();
        assert!(m.errors.is_empty(), "{:?}", m.errors);
        restore_all(&cb, Some("pw"), out.path());
        use crate::backup::recovery;
        let d = recovery::download(&be, Some("pw"), "ufop", rec.path(), &Default::default(), &OpContext::new()).unwrap();
        assert!(d.failed.is_empty() && d.downloaded >= 3, "{d:?}");
        recovery::build_index(rec.path(), &OpContext::new()).unwrap();
        let out2 = tempfile::tempdir().unwrap();
        let r = recovery::restore(rec.path(), 0, &[], out2.path(), &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let restored: Vec<_> = walk(out2.path());
        assert!(restored.iter().any(|(n, d)| n.ends_with("a.bin") && *d == data(20_000, 10)), "CDC offsets reassemble the file");
        // Convert keeps format 2 with a new key, flat layout.
        let conv = tempfile::tempdir().unwrap();
        let cv = LocalBackend::new(conv.path());
        recovery::convert(rec.path(), &cv, &recovery::ConvertOptions { passphrase: Some("new".into()), kdf: KdfParams::insecure_fast(), ..Default::default() }, &OpContext::new()).unwrap();
        let ch = header(conv.path());
        assert_eq!((ch.format_version, ch.layout), (2, RepoLayout::Flat));
        assert_eq!(ch.chunker, header(dst.path()).chunker);
        restore_all(&cv, Some("new"), tempfile::tempdir().unwrap().path());
    }

    fn walk(p: &Path) -> Vec<(String, Vec<u8>)> {
        let mut v = Vec::new();
        for e in std::fs::read_dir(p).unwrap().flatten() {
            if e.path().is_dir() {
                v.extend(walk(&e.path()));
            } else {
                v.push((e.path().to_string_lossy().into_owned(), std::fs::read(e.path()).unwrap()));
            }
        }
        v
    }
}
