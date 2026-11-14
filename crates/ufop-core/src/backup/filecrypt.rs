//! Standalone file encryption with the backup's cipher (AES-256-GCM, key
//! from Argon2id), plus decryption of single backup volumes by hand.
//!
//! File format (`.ufopenc`):
//!
//! ```text
//! "UFOPENC1" | u32 LE header length | header JSON {kdf, salt, chunk_size}
//! then chunks: u32 LE sealed length | VolumeCipher::seal([u64 LE index][u8 last][data])
//! ```
//!
//! Each chunk carries its index and a last-chunk flag inside the
//! authenticated payload, so reordered, dropped or truncated chunks are
//! detected. Files are processed in chunks (default 4 MiB), so size is
//! unbounded.

use super::crypto::{KdfParams, VolumeCipher};
use super::volume::RepoHeader;
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};
use std::path::Path;

const MAGIC: &[u8; 8] = b"UFOPENC1";
pub const DEFAULT_CHUNK: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FileHeader {
    kdf: KdfParams,
    salt: String,
    chunk_size: usize,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CryptReport {
    pub input_bytes: u64,
    pub output_bytes: u64,
    pub chunks: u64,
}

fn bad(msg: &str) -> AppError {
    AppError::encryption(msg.to_string(), "Check the passphrase and that the file is a complete .ufopenc file.")
}

/// Fill `buf` as far as the reader allows; returns the bytes read.
fn read_full(r: &mut impl Read, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..])? {
            0 => break,
            k => n += k,
        }
    }
    Ok(n)
}

pub fn encrypt_stream(mut input: impl Read, mut output: impl Write, passphrase: &str, kdf: KdfParams, chunk_size: usize) -> Result<CryptReport, AppError> {
    let chunk_size = chunk_size.clamp(4096, 64 * 1024 * 1024);
    let salt = VolumeCipher::random_salt();
    let cipher = VolumeCipher::derive(passphrase, &salt, kdf)?;
    let header = serde_json::to_vec(&FileHeader { kdf, salt: hex::encode(&salt), chunk_size })?;
    output.write_all(MAGIC)?;
    output.write_all(&(header.len() as u32).to_le_bytes())?;
    output.write_all(&header)?;
    let mut rep = CryptReport { output_bytes: (MAGIC.len() + 4 + header.len()) as u64, ..Default::default() };
    let mut cur = vec![0u8; chunk_size];
    let mut next = vec![0u8; chunk_size];
    let mut n = read_full(&mut input, &mut cur)?;
    let mut index: u64 = 0;
    loop {
        let m = if n == chunk_size { read_full(&mut input, &mut next)? } else { 0 };
        let last = m == 0;
        let mut plain = Vec::with_capacity(n + 9);
        plain.extend_from_slice(&index.to_le_bytes());
        plain.push(last as u8);
        plain.extend_from_slice(&cur[..n]);
        let sealed = cipher.seal(&plain)?;
        output.write_all(&(sealed.len() as u32).to_le_bytes())?;
        output.write_all(&sealed)?;
        rep.input_bytes += n as u64;
        rep.output_bytes += 4 + sealed.len() as u64;
        rep.chunks += 1;
        if last {
            break;
        }
        std::mem::swap(&mut cur, &mut next);
        n = m;
        index += 1;
    }
    output.flush()?;
    Ok(rep)
}

pub fn decrypt_stream(mut input: impl Read, mut output: impl Write, passphrase: &str) -> Result<CryptReport, AppError> {
    let mut magic = [0u8; 8];
    if read_full(&mut input, &mut magic)? != 8 || &magic != MAGIC {
        return Err(bad("This is not a UFOP encrypted file"));
    }
    let mut len = [0u8; 4];
    read_full(&mut input, &mut len)?;
    let hlen = u32::from_le_bytes(len) as usize;
    if hlen > 64 * 1024 {
        return Err(bad("The file header is damaged"));
    }
    let mut hb = vec![0u8; hlen];
    if read_full(&mut input, &mut hb)? != hlen {
        return Err(bad("The file header is truncated"));
    }
    let h: FileHeader = serde_json::from_slice(&hb).map_err(|_| bad("The file header is damaged"))?;
    let salt = hex::decode(&h.salt).map_err(|_| bad("The file header is damaged"))?;
    let cipher = VolumeCipher::derive(passphrase, &salt, h.kdf)?;
    let max_sealed = h.chunk_size + 1024;
    let mut rep = CryptReport { input_bytes: (8 + 4 + hlen) as u64, ..Default::default() };
    let mut index: u64 = 0;
    loop {
        let got = read_full(&mut input, &mut len)?;
        if got == 0 {
            return Err(bad("The file is truncated (last chunk missing)"));
        }
        if got != 4 {
            return Err(bad("The file is truncated"));
        }
        let sl = u32::from_le_bytes(len) as usize;
        if sl > max_sealed {
            return Err(bad("A chunk is damaged"));
        }
        let mut sealed = vec![0u8; sl];
        if read_full(&mut input, &mut sealed)? != sl {
            return Err(bad("The file is truncated"));
        }
        let plain = cipher.open(&sealed).map_err(|_| {
            if index == 0 {
                AppError::encryption("The passphrase is incorrect or the file is damaged", "Enter the passphrase used to encrypt the file.")
            } else {
                bad("A chunk failed authentication (the file is damaged)")
            }
        })?;
        if plain.len() < 9 || u64::from_le_bytes(plain[..8].try_into().unwrap_or_default()) != index {
            return Err(bad("Chunks are out of order"));
        }
        output.write_all(&plain[9..])?;
        rep.input_bytes += 4 + sl as u64;
        rep.output_bytes += (plain.len() - 9) as u64;
        rep.chunks += 1;
        if plain[8] == 1 {
            break;
        }
        index += 1;
    }
    let mut extra = [0u8; 1];
    if read_full(&mut input, &mut extra)? != 0 {
        return Err(bad("Unexpected data after the last chunk"));
    }
    output.flush()?;
    Ok(rep)
}

fn temp_beside(out: &Path) -> std::path::PathBuf {
    let mut p = out.as_os_str().to_owned();
    p.push(format!(".{}.part", uuid::Uuid::new_v4().simple()));
    p.into()
}

/// Run `f` writing to a temporary file next to `output`, then rename it into
/// place; a failure leaves no partial output behind.
fn atomically(output: &Path, f: impl FnOnce(&mut std::io::BufWriter<std::fs::File>) -> Result<CryptReport, AppError>) -> Result<CryptReport, AppError> {
    if let Some(d) = output.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(d)?;
    }
    let tmp = temp_beside(output);
    let r = (|| {
        let mut w = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        let rep = f(&mut w)?;
        w.into_inner().map_err(|e| AppError::from(e.into_error()))?.sync_all()?;
        Ok(rep)
    })();
    match r {
        Ok(rep) => {
            std::fs::rename(&tmp, output)?;
            Ok(rep)
        }
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

pub fn encrypt_file(input: &Path, output: &Path, passphrase: &str, kdf: KdfParams) -> Result<CryptReport, AppError> {
    if input == output {
        return Err(AppError::validation("Choose a different output file"));
    }
    let r = std::io::BufReader::new(std::fs::File::open(input)?);
    atomically(output, |w| encrypt_stream(r, w, passphrase, kdf, DEFAULT_CHUNK))
}

pub fn decrypt_file(input: &Path, output: &Path, passphrase: &str) -> Result<CryptReport, AppError> {
    if input == output {
        return Err(AppError::validation("Choose a different output file"));
    }
    let r = std::io::BufReader::new(std::fs::File::open(input)?);
    atomically(output, |w| decrypt_stream(r, w, passphrase))
}

/// Decrypt one backup volume (dblock / dindex / dlist) using the repository
/// header (`<prefix>-repo.json`), producing the plain ZIP or UFOP container.
pub fn decrypt_volume(volume: &Path, header: &Path, output: &Path, passphrase: Option<&str>) -> Result<CryptReport, AppError> {
    let h: RepoHeader = serde_json::from_slice(&std::fs::read(header)?).map_err(|e| AppError::validation(format!("Not a repository header: {e}")))?;
    let cipher = super::engine::header_cipher(&h, passphrase, None)?;
    let data = std::fs::read(volume)?;
    let plain = if cipher.is_encrypted() { cipher.open(&data)? } else { data.clone() };
    atomically(output, |w| {
        w.write_all(&plain)?;
        Ok(CryptReport { input_bytes: data.len() as u64, output_bytes: plain.len() as u64, chunks: 1 })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fast() -> KdfParams {
        KdfParams::insecure_fast()
    }

    #[test]
    fn round_trips_across_chunk_boundaries_and_detects_tampering() {
        for size in [0usize, 1, 4095, 4096, 4097, 3 * 4096, 50_000] {
            let data: Vec<u8> = (0..size).map(|i| (i * 31 % 256) as u8).collect();
            let mut enc = Vec::new();
            let r = encrypt_stream(&data[..], &mut enc, "pw", fast(), 4096).unwrap();
            assert_eq!(r.input_bytes, size as u64);
            let mut out = Vec::new();
            decrypt_stream(&enc[..], &mut out, "pw").unwrap();
            assert_eq!(out, data, "size {size}");
            assert!(decrypt_stream(&enc[..], &mut Vec::new(), "wrong").unwrap_err().user_message().contains("incorrect"));
        }
        let data = vec![7u8; 3 * 4096 + 10];
        let mut enc = Vec::new();
        encrypt_stream(&data[..], &mut enc, "pw", fast(), 4096).unwrap();
        // Truncation after a full chunk: the last-chunk marker is missing.
        let hlen = u32::from_le_bytes(enc[8..12].try_into().unwrap()) as usize;
        let first = 12 + hlen;
        let c0 = u32::from_le_bytes(enc[first..first + 4].try_into().unwrap()) as usize;
        let cut = &enc[..first + 4 + c0];
        assert!(decrypt_stream(cut, &mut Vec::new(), "pw").unwrap_err().user_message().contains("truncated"));
        // Swapping chunks 0 and 1 is detected.
        let c1s = first + 4 + c0;
        let c1 = u32::from_le_bytes(enc[c1s..c1s + 4].try_into().unwrap()) as usize;
        let mut swapped = enc[..first].to_vec();
        swapped.extend_from_slice(&enc[c1s..c1s + 4 + c1]);
        swapped.extend_from_slice(&enc[first..c1s]);
        swapped.extend_from_slice(&enc[c1s + 4 + c1..]);
        assert!(decrypt_stream(&swapped[..], &mut Vec::new(), "pw").is_err());
        // A flipped byte fails authentication; trailing junk is rejected.
        let mut flipped = enc.clone();
        let l = flipped.len();
        flipped[l - 3] ^= 1;
        assert!(decrypt_stream(&flipped[..], &mut Vec::new(), "pw").is_err());
        let mut extra = enc.clone();
        extra.push(0);
        assert!(decrypt_stream(&extra[..], &mut Vec::new(), "pw").is_err());
        assert!(decrypt_stream(&b"not encrypted"[..], &mut Vec::new(), "pw").is_err());
    }

    #[test]
    fn files_are_written_atomically() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, c) = (d.path().join("a.bin"), d.path().join("out/a.ufopenc"), d.path().join("a.dec"));
        std::fs::write(&a, b"hello file").unwrap();
        encrypt_file(&a, &b, "pw", fast()).unwrap();
        assert!(decrypt_file(&b, &c, "bad").is_err());
        assert!(!c.exists(), "no partial output on failure");
        decrypt_file(&b, &c, "pw").unwrap();
        assert_eq!(std::fs::read(&c).unwrap(), b"hello file");
        assert!(encrypt_file(&a, &a, "pw", fast()).is_err());
        assert_eq!(std::fs::read_dir(d.path().join("out")).unwrap().count(), 1, "temp file cleaned up");
    }

    #[test]
    fn decrypts_a_single_backup_volume() {
        use crate::backup::{run_backup, BackupOptions, LocalBackend, OpContext, RetentionPolicy, SourceFilter};
        let src = tempfile::tempdir().unwrap();
        let dst = tempfile::tempdir().unwrap();
        std::fs::write(src.path().join("f.txt"), b"volume content check").unwrap();
        let be = LocalBackend::new(dst.path());
        let db = crate::backup::db::BackupDb::open_in_memory().unwrap();
        let o = BackupOptions { block_size: 1024, volume_size: 4096, compression_level: 0, quota_disable: true, ..Default::default() };
        run_backup(&be, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), fast(), &OpContext::new()).unwrap();
        let vol = std::fs::read_dir(dst.path()).unwrap().flatten().map(|e| e.path()).find(|p| p.to_string_lossy().contains("dblock")).unwrap();
        let out = dst.path().join("plain.bin");
        assert!(decrypt_volume(&vol, &dst.path().join("ufop-repo.json"), &out, Some("nope")).is_err());
        decrypt_volume(&vol, &dst.path().join("ufop-repo.json"), &out, Some("pw")).unwrap();
        let plain = std::fs::read(&out).unwrap();
        assert!(plain.windows(20).any(|w| w == b"volume content check"), "stored blocks are readable after decryption");
    }
}
