//! Remote volume formats and naming.
//!
//! dblock payload (before sealing):
//! ```text
//! count u32 LE, then per block:
//!   hash_len u8 | hash (ASCII hex) | flag u8 (0 stored, 1 deflate, 2 zstd) | len u32 LE | data
//! ```
//! Flag 2 (zstd) is only written to format v2 repositories; readers accept
//! every flag in any repository.
//!
//! dindex and dlist payloads are JSON; format v2 compresses that JSON with
//! zstd before sealing (detected on read by the zstd frame magic).

use super::crypto::VolumeCipher;
use super::{BlockCodec, FileEntry, HashAlgorithm, VolumeFormat, ZipMethod};
use crate::error::AppError;
use chrono::{DateTime, NaiveDateTime, Utc};
use flate2::read::DeflateDecoder;
use flate2::write::DeflateEncoder;
use flate2::Compression;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::{Read, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VolumeKind {
    Block,
    Index,
    FileList,
}

impl VolumeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Block => "dblock",
            Self::Index => "dindex",
            Self::FileList => "dlist",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dblock" => Some(Self::Block),
            "dindex" => Some(Self::Index),
            "dlist" => Some(Self::FileList),
            _ => None,
        }
    }
}

const TS_FMT: &str = "%Y%m%dT%H%M%SZ";

fn ext(cipher: &VolumeCipher) -> &'static str {
    match (cipher.is_encrypted(), cipher.module_name().as_str()) {
        (false, _) => "",
        (true, "gpg") => ".gpg",
        (true, _) => ".aes",
    }
}

pub fn block_volume_name(prefix: &str, cipher: &VolumeCipher) -> String {
    format!("{prefix}-b{}.dblock{}", uuid::Uuid::new_v4().simple(), ext(cipher))
}

pub fn index_volume_name(prefix: &str, cipher: &VolumeCipher) -> String {
    format!("{prefix}-i{}.dindex{}", uuid::Uuid::new_v4().simple(), ext(cipher))
}

pub fn filelist_volume_name(prefix: &str, ts: DateTime<Utc>, cipher: &VolumeCipher) -> String {
    format!("{prefix}-{}.dlist{}", ts.format(TS_FMT), ext(cipher))
}

pub fn repo_header_name(prefix: &str) -> String {
    format!("{prefix}-repo.json")
}

pub fn verification_file_name(prefix: &str) -> String {
    format!("{prefix}-verification.json")
}

/// Classify a remote file belonging to `prefix`.
pub fn classify(prefix: &str, name: &str) -> Option<VolumeKind> {
    let rest = name.strip_prefix(prefix)?.strip_prefix('-')?;
    let rest = rest.strip_suffix(".aes").or_else(|| rest.strip_suffix(".gpg")).unwrap_or(rest);
    let (_, kind) = rest.rsplit_once('.')?;
    VolumeKind::parse(kind)
}

/// Parse the timestamp of a dlist name.
pub fn filelist_timestamp(prefix: &str, name: &str) -> Option<DateTime<Utc>> {
    let rest = name.strip_prefix(prefix)?.strip_prefix('-')?;
    let ts = rest.split('.').next()?;
    NaiveDateTime::parse_from_str(ts, TS_FMT).ok().map(|n| n.and_utc())
}

/// Unencrypted repository header.
///
/// Format v1 headers have none of the fields marked "v2"; their defaults
/// describe exactly how v1 repositories are written (fixed-size blocks,
/// deflate, uncompressed metadata, AES-256-GCM with the passphrase-derived
/// key, flat layout).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoHeader {
    pub format_version: u32,
    pub encrypted: bool,
    pub salt: Option<String>,
    pub kdf: Option<super::crypto::KdfParams>,
    pub key_check: Option<String>,
    pub block_size: usize,
    pub block_hash: HashAlgorithm,
    pub file_hash: HashAlgorithm,
    /// Container format of block volumes (older repositories: UFOP).
    #[serde(default)]
    pub volume_format: VolumeFormat,
    /// `aes` (or empty, older repositories) or `gpg`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub encryption_module: String,
    /// GPG key encryption: the recipients (informational; empty = symmetric).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gpg_recipients: Vec<String>,
    /// v2: how files are split into blocks (`None` = fixed `block_size`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chunker: Option<super::chunker::ChunkerSpec>,
    /// v2: codec for new blocks in UFOP pack volumes.
    #[serde(default, skip_serializing_if = "is_default")]
    pub block_codec: BlockCodec,
    /// v2: compression of sealed dlist/dindex JSON.
    #[serde(default, skip_serializing_if = "is_default")]
    pub metadata_compression: super::MetadataCompression,
    /// v2: AEAD for built-in encryption.
    #[serde(default, skip_serializing_if = "is_default")]
    pub cipher: super::crypto::CipherKind,
    /// v2: random master key wrapped by the passphrase-derived KEK (hex).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapped_master_key: Option<String>,
    /// v2: where volumes live on the destination.
    #[serde(default, skip_serializing_if = "is_default")]
    pub layout: super::layout::RepoLayout,
    /// v2: feature flags; readers refuse flags they do not know.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub features: Vec<String>,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

impl RepoHeader {
    /// The chunker new blocks are cut with.
    pub fn chunker_spec(&self) -> super::chunker::ChunkerSpec {
        self.chunker.unwrap_or(super::chunker::ChunkerSpec::Fixed { size: self.block_size })
    }

    pub fn has_feature(&self, f: &str) -> bool {
        self.features.iter().any(|x| x == f)
    }
}

/// Accumulates blocks for one dblock volume.
#[derive(Default)]
pub struct BlockVolumeWriter {
    entries: Vec<(String, u8, Vec<u8>)>,
    size: usize,
    format: VolumeFormat,
    zip_method: ZipMethod,
    codec: BlockCodec,
}

impl BlockVolumeWriter {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_format(format: VolumeFormat, zip_method: ZipMethod) -> Self {
        Self { format, zip_method, ..Default::default() }
    }

    /// Add a block encoded by [`encode_block`] (UFOP) or raw with flag
    /// 1 = compress / 0 = store (ZIP). Lets worker threads compress in parallel.
    pub fn add_encoded(&mut self, hash: String, flag: u8, data: Vec<u8>) {
        self.size += data.len() + hash.len() + if self.format == VolumeFormat::Zip { 30 } else { 6 };
        self.entries.push((hash, flag, data));
    }

    /// Codec used by [`add`](Self::add) for UFOP volumes (zstd: v2 only).
    pub fn with_codec(mut self, codec: BlockCodec) -> Self {
        self.codec = codec;
        self
    }

    pub fn format(&self) -> VolumeFormat {
        self.format
    }

    pub fn codec(&self) -> BlockCodec {
        self.codec
    }

    /// Add a block; compresses it unless `level == 0` or compression does not help.
    pub fn add(&mut self, hash: String, data: &[u8], level: u32) -> Result<(), AppError> {
        if self.format == VolumeFormat::Zip {
            // Compressed by the ZIP writer in `finish`; flag 0 = store.
            self.size += data.len() + hash.len() + 30;
            self.entries.push((hash, (level > 0) as u8, data.to_vec()));
            return Ok(());
        }
        let (flag, stored) = encode_block(self.format, self.codec, data, level)?;
        self.size += stored.len() + hash.len() + 6;
        self.entries.push((hash, flag, stored));
        Ok(())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
    pub fn byte_size(&self) -> usize {
        self.size
    }
    pub fn hashes(&self) -> Vec<String> {
        self.entries.iter().map(|(h, _, _)| h.clone()).collect()
    }

    pub fn finish(self) -> Vec<u8> {
        if self.format == VolumeFormat::Zip {
            return self.finish_zip().unwrap_or_default();
        }
        let mut out = Vec::with_capacity(self.size + 4);
        out.extend_from_slice(&(self.entries.len() as u32).to_le_bytes());
        for (h, flag, d) in self.entries {
            out.push(h.len() as u8);
            out.extend_from_slice(h.as_bytes());
            out.push(flag);
            out.extend_from_slice(&(d.len() as u32).to_le_bytes());
            out.extend_from_slice(&d);
        }
        out
    }
}

/// Encode one block for a volume of `format`: UFOP compresses here with
/// `codec` (returns flag 1 = deflate, 2 = zstd, or 0 = stored when
/// compression does not help); ZIP keeps raw data and only records whether
/// to compress.
pub fn encode_block(format: VolumeFormat, codec: BlockCodec, data: &[u8], level: u32) -> Result<(u8, Vec<u8>), AppError> {
    if format == VolumeFormat::Zip {
        return Ok(((level > 0) as u8, data.to_vec()));
    }
    if level == 0 {
        return Ok((0, data.to_vec()));
    }
    let (flag, c) = match codec {
        BlockCodec::Deflate => {
            let mut e = DeflateEncoder::new(Vec::new(), Compression::new(level.min(9)));
            e.write_all(data)?;
            (1, e.finish()?)
        }
        BlockCodec::Zstd => (2, zstd::bulk::compress(data, zstd_level(level)).map_err(|e| AppError::internal(format!("zstd: {e}")))?),
    };
    Ok(if c.len() < data.len() { (flag, c) } else { (0, data.to_vec()) })
}

/// Map the 0–9 (deflate-style) compression level onto zstd levels.
pub fn zstd_level(level: u32) -> i32 {
    match level {
        0..=2 => 1,
        3..=4 => 2,
        5..=6 => 3,
        7 => 6,
        8 => 12,
        _ => 19,
    }
}

/// Zstd frame magic number (little endian `0xFD2FB528`).
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xb5, 0x2f, 0xfd];

/// Decompress a zstd stream, refusing output beyond `limit` bytes.
fn zstd_decode(data: &[u8], limit: usize) -> Result<Vec<u8>, AppError> {
    let mut out = Vec::new();
    zstd::stream::read::Decoder::new(data)
        .and_then(|d| d.take(limit as u64 + 1).read_to_end(&mut out))
        .map_err(|_| AppError::transfer("Compressed data is corrupt", "Run Verify and Repair on this backup."))?;
    if out.len() > limit {
        return Err(AppError::transfer("Compressed data is larger than allowed", "Run Verify and Repair on this backup."));
    }
    Ok(out)
}

/// Largest decompressed block accepted (blocks are at most a few MiB).
const MAX_BLOCK: usize = 256 << 20;
/// Largest decompressed metadata volume accepted.
const MAX_META: usize = 4 << 30;

impl BlockVolumeWriter {
    fn finish_zip(self) -> Result<Vec<u8>, AppError> {
        let method = match self.zip_method {
            ZipMethod::Deflate => zip::CompressionMethod::Deflated,
            ZipMethod::Stored => zip::CompressionMethod::Stored,
            ZipMethod::Bzip2 => zip::CompressionMethod::Bzip2,
            ZipMethod::Zstd => zip::CompressionMethod::Zstd,
        };
        let zerr = |e: zip::result::ZipError| AppError::internal(format!("zip write: {e}"));
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::with_capacity(self.size)));
        for (h, compress, data) in self.entries {
            let o = zip::write::SimpleFileOptions::default()
                .compression_method(if compress == 1 { method } else { zip::CompressionMethod::Stored })
                // ZIP64 is chosen automatically for large entries.
                .large_file(data.len() as u64 >= u32::MAX as u64);
            w.start_file(h, o).map_err(zerr)?;
            w.write_all(&data)?;
        }
        Ok(w.finish().map_err(zerr)?.into_inner())
    }
}

/// Decode a dblock payload into hash → raw (decompressed) block data.
/// Accepts both the UFOP pack format and ZIP volumes.
pub fn read_block_volume(payload: &[u8]) -> Result<HashMap<String, Vec<u8>>, AppError> {
    if payload.starts_with(b"PK\x03\x04") || payload.starts_with(b"PK\x05\x06") {
        let corrupt = |_| AppError::transfer("ZIP block volume is corrupt", "Run Verify and Repair on this backup.");
        let mut a = zip::ZipArchive::new(std::io::Cursor::new(payload)).map_err(corrupt)?;
        let mut out = HashMap::with_capacity(a.len());
        for i in 0..a.len() {
            let mut f = a.by_index(i).map_err(corrupt)?;
            // Sizes come from the (possibly damaged) file: never trust them for allocation.
            let mut v = Vec::with_capacity((f.size() as usize).min(payload.len().saturating_mul(4)).min(64 << 20));
            f.read_to_end(&mut v).map_err(|_| AppError::transfer("ZIP block volume is corrupt", "Run Verify and Repair on this backup."))?;
            out.insert(f.name().to_string(), v);
        }
        return Ok(out);
    }
    let corrupt = || AppError::transfer("Block volume is corrupt", "Run Verify and Repair on this backup.");
    let mut p = 0usize;
    let take = |p: &mut usize, n: usize| -> Result<&[u8], AppError> {
        let s = payload.get(*p..*p + n).ok_or_else(corrupt)?;
        *p += n;
        Ok(s)
    };
    let count = u32::from_le_bytes(take(&mut p, 4)?.try_into().unwrap()) as usize;
    // Each entry needs at least 6 bytes; a damaged count must not drive allocation.
    if count > payload.len() / 6 {
        return Err(corrupt());
    }
    let mut out = HashMap::with_capacity(count);
    for _ in 0..count {
        let hl = take(&mut p, 1)?[0] as usize;
        let h = String::from_utf8(take(&mut p, hl)?.to_vec()).map_err(|_| corrupt())?;
        let flag = take(&mut p, 1)?[0];
        let len = u32::from_le_bytes(take(&mut p, 4)?.try_into().unwrap()) as usize;
        let d = take(&mut p, len)?;
        let raw = match flag {
            0 => d.to_vec(),
            1 => {
                let mut v = Vec::new();
                DeflateDecoder::new(d).read_to_end(&mut v).map_err(|_| corrupt())?;
                v
            }
            2 => zstd_decode(d, MAX_BLOCK).map_err(|_| corrupt())?,
            _ => return Err(corrupt()),
        };
        out.insert(h, raw);
    }
    Ok(out)
}

/// dindex payload: which blocks a dblock contains.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexVolume {
    pub block_volume: String,
    pub blocks: Vec<IndexedBlock>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IndexedBlock {
    pub hash: String,
    pub size: u64,
}

/// dlist payload: one backup version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileListVolume {
    pub timestamp: String,
    pub partial: bool,
    pub files: Vec<FileEntry>,
    /// Embedded job configuration (secrets stripped), when enabled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_config: Option<serde_json::Value>,
    /// Extra files attached to this version (`control_files` option).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub control_files: Vec<ControlFile>,
}

/// An auxiliary file stored inside a file-list volume.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlFile {
    pub name: String,
    /// Base64 content.
    pub data: String,
}

/// Serialize and seal metadata; compressed with zstd first when the cipher
/// belongs to a v2 repository with metadata compression.
pub fn seal_json<T: Serialize>(cipher: &VolumeCipher, v: &T) -> Result<Vec<u8>, AppError> {
    let json = serde_json::to_vec(v)?;
    if cipher.compresses_metadata() {
        let z = zstd::bulk::compress(&json, 3).map_err(|e| AppError::internal(format!("zstd: {e}")))?;
        return cipher.seal(&z);
    }
    cipher.seal(&json)
}

/// Undo optional metadata compression of an opened payload (JSON never
/// starts with the zstd magic, so v1 payloads pass through unchanged).
pub fn decode_meta(payload: Vec<u8>) -> Result<Vec<u8>, AppError> {
    if payload.starts_with(&ZSTD_MAGIC) {
        return zstd_decode(&payload, MAX_META);
    }
    Ok(payload)
}

pub fn open_json<T: for<'de> Deserialize<'de>>(cipher: &VolumeCipher, data: &[u8]) -> Result<T, AppError> {
    Ok(serde_json::from_slice(&decode_meta(cipher.open(data)?)?)?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_classify_and_parse() {
        let c = VolumeCipher::plain();
        let b = block_volume_name("ufop", &c);
        assert_eq!(classify("ufop", &b), Some(VolumeKind::Block));
        assert_eq!(classify("ufop", &index_volume_name("ufop", &c)), Some(VolumeKind::Index));
        let ts = DateTime::parse_from_rfc3339("2026-01-02T03:04:05Z").unwrap().with_timezone(&Utc);
        let l = filelist_volume_name("ufop", ts, &c);
        assert_eq!(l, "ufop-20260102T030405Z.dlist");
        assert_eq!(filelist_timestamp("ufop", &l), Some(ts));
        assert_eq!(classify("ufop", "ufop-repo.json"), None);
        assert_eq!(classify("other", &b), None);
        assert_eq!(classify("ufop", "ufop-x.dlist.aes"), Some(VolumeKind::FileList));
        assert_eq!(classify("ufop", "ufop-x.dblock.gpg"), Some(VolumeKind::Block));
    }

    #[test]
    fn block_volume_roundtrip_with_and_without_compression() {
        let mut w = BlockVolumeWriter::new();
        w.add("aa".into(), &[7u8; 1000], 6).unwrap();
        w.add("bb".into(), b"xyz", 0).unwrap();
        assert!(w.byte_size() < 1000);
        let m = read_block_volume(&w.finish()).unwrap();
        assert_eq!(m["aa"], vec![7u8; 1000]);
        assert_eq!(m["bb"], b"xyz");
        assert!(read_block_volume(&[9, 0, 0, 0, 1]).is_err());
    }

    #[test]
    fn corrupt_volume_is_rejected_without_huge_allocation() {
        // Regression: a damaged count field made the reader reserve ~200 GB.
        assert!(read_block_volume(b"corrupt").is_err());
        assert!(read_block_volume(&[0xff, 0xff, 0xff, 0x7f, 1, 2, 3]).is_err());
        let mut w = BlockVolumeWriter::new();
        w.add("h".into(), b"data", 6).unwrap();
        let mut v = w.finish();
        v.truncate(v.len() - 2);
        assert!(read_block_volume(&v).is_err());
    }

    #[test]
    fn zstd_blocks_and_mixed_flags() {
        let mut w = BlockVolumeWriter::new().with_codec(BlockCodec::Zstd);
        w.add("z".into(), &[3u8; 4000], 6).unwrap();
        w.add("s".into(), b"tiny", 6).unwrap();
        let mut d = BlockVolumeWriter::new();
        d.add("d".into(), &[5u8; 4000], 6).unwrap();
        let bytes = w.finish();
        // Flag after count(4) + hash_len(1) + hash(1).
        assert_eq!(bytes[6], 2, "zstd flag");
        let m = read_block_volume(&bytes).unwrap();
        assert_eq!(m["z"], vec![3u8; 4000]);
        assert_eq!(m["s"], b"tiny", "incompressible block stored");
        let dm = read_block_volume(&d.finish()).unwrap();
        assert_eq!(dm["d"], vec![5u8; 4000]);
        assert_eq!(encode_block(VolumeFormat::Ufop, BlockCodec::Zstd, b"ab", 6).unwrap().0, 0);
        assert_eq!(encode_block(VolumeFormat::Ufop, BlockCodec::Zstd, &[1u8; 100], 0).unwrap().0, 0);
        assert!(zstd_level(9) > zstd_level(6) && zstd_level(1) == 1);
    }

    #[test]
    fn metadata_compression_roundtrip_and_v1_compat() {
        let idx = IndexVolume { block_volume: "b".into(), blocks: (0..200).map(|i| IndexedBlock { hash: format!("{i:064}"), size: 1 }).collect() };
        let plain = VolumeCipher::plain();
        let v1 = seal_json(&plain, &idx).unwrap();
        let v2 = seal_json(&plain.clone().with_metadata_compression(true), &idx).unwrap();
        assert!(v2.len() < v1.len() / 2, "compressed metadata is smaller");
        assert!(v2[7..].starts_with(&ZSTD_MAGIC));
        // Either reader opens both.
        for data in [&v1, &v2] {
            let back: IndexVolume = open_json(&plain, data).unwrap();
            assert_eq!(back.blocks.len(), 200);
        }
    }

    #[test]
    fn zip_volumes_all_methods() {
        for m in [ZipMethod::Deflate, ZipMethod::Stored, ZipMethod::Bzip2, ZipMethod::Zstd] {
            let mut w = BlockVolumeWriter::with_format(VolumeFormat::Zip, m);
            w.add("aa".into(), &[7u8; 5000], 6).unwrap();
            w.add("bb".into(), b"already-compressed", 0).unwrap();
            let bytes = w.finish();
            assert!(bytes.starts_with(b"PK"), "{m:?} produces a real ZIP");
            let a = zip::ZipArchive::new(std::io::Cursor::new(&bytes)).unwrap();
            assert_eq!(a.len(), 2);
            let map = read_block_volume(&bytes).unwrap();
            assert_eq!(map["aa"], vec![7u8; 5000]);
            assert_eq!(map["bb"], b"already-compressed");
        }
    }
}
