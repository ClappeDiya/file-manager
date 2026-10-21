//! Remote volume formats and naming.
//!
//! dblock payload (before sealing):
//! ```text
//! count u32 LE, then per block:
//!   hash_len u8 | hash (ASCII hex) | flag u8 (0 stored, 1 deflate) | len u32 LE | data
//! ```
//! dindex and dlist payloads are JSON.

use super::crypto::VolumeCipher;
use super::{FileEntry, HashAlgorithm, VolumeFormat, ZipMethod};
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
    if cipher.is_encrypted() { ".aes" } else { "" }
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
    let rest = rest.strip_suffix(".aes").unwrap_or(rest);
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
}

/// Accumulates blocks for one dblock volume.
#[derive(Default)]
pub struct BlockVolumeWriter {
    entries: Vec<(String, u8, Vec<u8>)>,
    size: usize,
    format: VolumeFormat,
    zip_method: ZipMethod,
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

    pub fn format(&self) -> VolumeFormat {
        self.format
    }

    /// Add a block; compresses it unless `level == 0` or compression does not help.
    pub fn add(&mut self, hash: String, data: &[u8], level: u32) -> Result<(), AppError> {
        if self.format == VolumeFormat::Zip {
            // Compressed by the ZIP writer in `finish`; flag 0 = store.
            self.size += data.len() + hash.len() + 30;
            self.entries.push((hash, (level > 0) as u8, data.to_vec()));
            return Ok(());
        }
        let (flag, stored) = if level == 0 {
            (0u8, data.to_vec())
        } else {
            let mut e = DeflateEncoder::new(Vec::new(), Compression::new(level.min(9)));
            e.write_all(data)?;
            let c = e.finish()?;
            if c.len() < data.len() { (1, c) } else { (0, data.to_vec()) }
        };
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

/// Encode one block for a volume of `format`: UFOP compresses here
/// (returns flag 1 = deflate or 0 = stored); ZIP keeps raw data and only
/// records whether to compress.
pub fn encode_block(format: VolumeFormat, data: &[u8], level: u32) -> Result<(u8, Vec<u8>), AppError> {
    if format == VolumeFormat::Zip {
        return Ok(((level > 0) as u8, data.to_vec()));
    }
    if level == 0 {
        return Ok((0, data.to_vec()));
    }
    let mut e = DeflateEncoder::new(Vec::new(), Compression::new(level.min(9)));
    e.write_all(data)?;
    let c = e.finish()?;
    Ok(if c.len() < data.len() { (1, c) } else { (0, data.to_vec()) })
}

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
            let mut v = Vec::with_capacity(f.size() as usize);
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

pub fn seal_json<T: Serialize>(cipher: &VolumeCipher, v: &T) -> Result<Vec<u8>, AppError> {
    cipher.seal(&serde_json::to_vec(v)?)
}

pub fn open_json<T: for<'de> Deserialize<'de>>(cipher: &VolumeCipher, data: &[u8]) -> Result<T, AppError> {
    Ok(serde_json::from_slice(&cipher.open(data)?)?)
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
