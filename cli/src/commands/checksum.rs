//! Checksum command - compute and verify file hashes.

use crate::output;
use crate::OutputFormat;
use serde::Serialize;
use std::io::Read;
use std::path::Path;

#[derive(Debug, Serialize)]
struct ChecksumResult {
    path: String,
    algorithm: String,
    hash: String,
    size: u64,
    verified: Option<bool>,
}

pub async fn execute(
    paths: Vec<String>,
    algorithm: String,
    verify: Option<String>,
    format: &OutputFormat,
) -> Result<u8, Box<dyn std::error::Error>> {
    // Validate algorithm
    let algo = algorithm.to_lowercase();
    if !["md5", "sha1", "sha256"].contains(&algo.as_str()) {
        output::print_error(
            format,
            &format!("Unsupported algorithm '{algorithm}'. Use: md5, sha1, sha256"),
        );
        return Ok(2);
    }

    let mut results: Vec<ChecksumResult> = Vec::new();
    let mut all_verified = true;

    for path_str in &paths {
        let path = Path::new(path_str);
        if !path.exists() {
            output::print_error(format, &format!("File not found: {path_str}"));
            return Ok(2);
        }

        if path.is_dir() {
            output::print_error(format, &format!("Cannot checksum directory: {path_str}. Specify individual files."));
            return Ok(2);
        }

        let size = path.metadata()?.len();
        let hash = compute_hash(path, &algo)?;

        let verified = verify.as_ref().map(|expected| {
            let matches = hash.eq_ignore_ascii_case(expected);
            if !matches {
                all_verified = false;
            }
            matches
        });

        results.push(ChecksumResult {
            path: path_str.clone(),
            algorithm: algo.clone(),
            hash: hash.clone(),
            size,
            verified,
        });
    }

    // Output
    match format {
        OutputFormat::Human => {
            println!();
            for result in &results {
                println!("  {}  {}", result.hash, result.path);
                if let Some(verified) = result.verified {
                    if verified {
                        output::print_success(format, "Hash verified OK");
                    } else {
                        output::print_error(format, &format!(
                            "Hash MISMATCH! Expected: {}",
                            verify.as_deref().unwrap_or("?")
                        ));
                    }
                }
            }
            println!();
        }
        _ => output::print_data(format, &results),
    }

    if verify.is_some() && !all_verified {
        Ok(2) // Verification failure
    } else {
        Ok(0)
    }
}

fn compute_hash(path: &Path, algorithm: &str) -> Result<String, Box<dyn std::error::Error>> {
    use md5::Digest;
    let file = std::fs::File::open(path)?;
    match algorithm {
        "md5" => stream_hex(file, md5::Md5::new()),
        "sha1" => stream_hex(file, sha1::Sha1::new()),
        "sha256" => stream_hex(file, sha2::Sha256::new()),
        _ => Err(format!("Unsupported algorithm: {algorithm} (use md5, sha1 or sha256)").into()),
    }
}

/// Hash a file in 64 KiB blocks (constant memory, any file size).
fn stream_hex<D: md5::Digest>(
    mut file: std::fs::File,
    mut hasher: D,
) -> Result<String, Box<dyn std::error::Error>> {
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash_of(content: &[u8], algorithm: &str) -> String {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, content).unwrap();
        compute_hash(&p, algorithm).unwrap()
    }

    #[test]
    fn matches_standard_test_vectors() {
        assert_eq!(hash_of(b"abc", "md5"), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hash_of(b"abc", "sha1"), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hash_of(b"abc", "sha256"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(hash_of(b"", "md5"), "d41d8cd98f00b204e9800998ecf8427e");
    }

    #[test]
    fn multi_block_files_hash_correctly() {
        // 1,000,000 × 'a' is a standard SHA-256 vector and spans many blocks.
        assert_eq!(
            hash_of(&vec![b'a'; 1_000_000], "sha256"),
            "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0"
        );
    }

    #[test]
    fn rejects_unknown_algorithm() {
        let dir = tempfile::TempDir::new().unwrap();
        let p = dir.path().join("f");
        std::fs::write(&p, "x").unwrap();
        assert!(compute_hash(&p, "crc32").is_err());
    }
}
