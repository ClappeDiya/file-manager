//! `ufop verify-release SHA256SUMS [SHA256SUMS.minisig] [--pubkey …]`:
//! check the minisign signature of a release's checksum list and the
//! SHA-256 of every downloaded file it lists (the same checks as
//! `scripts/verify-release.sh`, without needing minisign or OpenSSL).

use clap::Args;
use minisign_verify::{PublicKey, Signature};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

/// Release signing key (the Tauri updater's minisign key, key id
/// F5189997328AA80E; `src-tauri/tauri.conf.json` → plugins.updater.pubkey).
pub const RELEASE_PUBKEY: &str = "RWQOqigyl5kY9Vj4LeRcZ2DMlAEY85jWjDqOZ94gBgYhSd1TOsJ0nxzZ";

#[derive(Args, Debug, Clone)]
pub struct VerifyReleaseArgs {
    /// The checksum list (SHA256SUMS)
    pub sums: PathBuf,
    /// Its minisign signature (default: <SUMS>.minisig; Tauri `.sig` files work too)
    pub signature: Option<PathBuf>,
    /// Public key: the `RW…` line, a minisign.pub file's text, or Tauri's base64 form (default: the UFOP release key)
    #[arg(long, conflicts_with = "pubkey_file")]
    pub pubkey: Option<String>,
    /// Read the public key from this file
    #[arg(long)]
    pub pubkey_file: Option<PathBuf>,
    /// Folder with the downloaded files (default: the folder of SUMS)
    #[arg(long)]
    pub dir: Option<PathBuf>,
    /// Fail when a listed file is not in the folder
    #[arg(long)]
    pub require_all: bool,
    /// Only check the signature, not the files
    #[arg(long)]
    pub signature_only: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct FileCheck {
    pub name: String,
    /// `ok`, `mismatch` or `missing`.
    pub status: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct VerifyReport {
    pub signature_ok: bool,
    pub trusted_comment: String,
    pub files: Vec<FileCheck>,
    pub ok: bool,
}

/// A public key in any accepted form.
pub fn parse_pubkey(s: &str) -> Result<PublicKey, String> {
    let t = s.trim();
    if t.contains("untrusted comment") {
        return PublicKey::decode(t).map_err(|e| format!("invalid public key: {e}"));
    }
    if t.starts_with("RW") {
        return PublicKey::from_base64(t).map_err(|e| format!("invalid public key: {e}"));
    }
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD.decode(t).map_err(|_| "invalid public key (expected RW…, a minisign.pub or Tauri's base64 form)".to_string())?;
    parse_pubkey(&String::from_utf8_lossy(&decoded))
}

/// A minisign signature file, or Tauri's base64 `.sig` form of one.
pub fn parse_signature(s: &str) -> Result<Signature, String> {
    let t = s.trim();
    if t.starts_with("untrusted comment:") {
        return Signature::decode(t).map_err(|e| format!("invalid signature: {e}"));
    }
    use base64::Engine;
    let decoded = base64::engine::general_purpose::STANDARD.decode(t).map_err(|_| "invalid signature file".to_string())?;
    Signature::decode(String::from_utf8_lossy(&decoded).trim()).map_err(|e| format!("invalid signature: {e}"))
}

/// `(hash, file name)` lines of a SHA256SUMS file (`*name` = binary mode).
pub fn parse_sums(text: &str) -> Vec<(String, String)> {
    text.lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let (h, rest) = l.split_once(' ')?;
            let name = rest.trim_start_matches(' ').trim_start_matches('*');
            (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()) && !name.is_empty()).then(|| (h.to_ascii_lowercase(), name.to_string()))
        })
        .collect()
}

fn sha256_file(p: &Path) -> std::io::Result<String> {
    let mut f = std::fs::File::open(p)?;
    let mut h = Sha256::new();
    std::io::copy(&mut f, &mut h)?;
    Ok(hex::encode(h.finalize()))
}

/// Verify the signature and (unless `signature_only`) the files.
pub fn verify(a: &VerifyReleaseArgs) -> Result<VerifyReport, String> {
    let key_text = match (&a.pubkey, &a.pubkey_file) {
        (Some(k), _) => k.clone(),
        (None, Some(f)) => std::fs::read_to_string(f).map_err(|e| format!("cannot read {}: {e}", f.display()))?,
        (None, None) => RELEASE_PUBKEY.to_string(),
    };
    let pk = parse_pubkey(&key_text)?;
    let sig_path = a.signature.clone().unwrap_or_else(|| PathBuf::from(format!("{}.minisig", a.sums.display())));
    let sig = parse_signature(&std::fs::read_to_string(&sig_path).map_err(|e| format!("cannot read {}: {e}", sig_path.display()))?)?;
    let sums = std::fs::read(&a.sums).map_err(|e| format!("cannot read {}: {e}", a.sums.display()))?;
    let signature_ok = pk.verify(&sums, &sig, true).is_ok();
    let mut files = Vec::new();
    if signature_ok && !a.signature_only {
        let dir = a.dir.clone().unwrap_or_else(|| a.sums.parent().map(Path::to_path_buf).filter(|p| !p.as_os_str().is_empty()).unwrap_or_else(|| PathBuf::from(".")));
        for (hash, name) in parse_sums(&String::from_utf8_lossy(&sums)) {
            // Names are plain file names in the release folder.
            if name.contains("..") || Path::new(&name).is_absolute() {
                files.push(FileCheck { name, status: "mismatch".into() });
                continue;
            }
            let p = dir.join(&name);
            let status = if !p.is_file() {
                "missing"
            } else if sha256_file(&p).map_err(|e| format!("cannot read {}: {e}", p.display()))? == hash {
                "ok"
            } else {
                "mismatch"
            };
            files.push(FileCheck { name, status: status.into() });
        }
    }
    let bad = files.iter().any(|f| f.status == "mismatch" || (a.require_all && f.status == "missing"));
    let any_checked = a.signature_only || files.iter().any(|f| f.status == "ok");
    Ok(VerifyReport { signature_ok, trusted_comment: sig.trusted_comment().to_string(), ok: signature_ok && !bad && any_checked, files })
}

pub fn execute(a: &VerifyReleaseArgs, format: &crate::OutputFormat) -> Result<u8, Box<dyn std::error::Error>> {
    let r = verify(a)?;
    if matches!(format, crate::OutputFormat::Human) {
        println!("Signature: {}{}", if r.signature_ok { "OK" } else { "BAD" }, if r.signature_ok { format!(" ({})", r.trusted_comment) } else { String::new() });
        for f in &r.files {
            println!("  {:<8} {}", f.status.to_uppercase(), f.name);
        }
        if r.signature_ok && !a.signature_only && !r.files.iter().any(|f| f.status == "ok") {
            println!("None of the listed files is in the folder.");
        }
        println!("{}", if r.ok { "Release verified." } else { "Verification FAILED." });
    } else {
        crate::output::print_data(format, &r);
    }
    Ok(if r.ok { 0 } else { 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Made with scripts/lib/minisign-openssl.sh (a throwaway key).
    const PUBKEY: &str = "untrusted comment: minisign public key 22D497644DD54E7B\nRWQi1JdkTdVOezokXxrGep7FEpBFtuMcmaW0EzL0NA74eJZAhTsq09tv\n";
    const SUMS: &str = "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03  a.txt\ne258d248fda94c63753607f7c4494ee0fcbe92f1a76bfdac795c9d84101eb317  b.bin\n";
    const SIG_TAURI_FORM: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IHNpZ25hdHVyZSBmcm9tIG1pbmlzaWduLW9wZW5zc2wKUlVRaTFKZGtUZFZPZTgvSXpnTVZRTjJjcGpjSGRTbEkvNVNxYlIzck9LT2xqN2VnRlh3c2NBNVBlSHF4dEQ0ZFhvcC8zQ0w5N1F5NlJYUkIzWG9PV0FxWUZBK2pGN0VJeXdrPQp0cnVzdGVkIGNvbW1lbnQ6IHRpbWVzdGFtcDoxIGZpbGU6U0hBMjU2U1VNUwpweC9CZ2hCUG9XVUJldGlyUXVHYVhkNGtobytYYTVyRkRtcjN5MlNYdzE2all1bkZZZ0RWUnpQLzIyakZzcDM2NUk3dmZhTlNnbndSY3lpT01FRy9EZz09Cg==";

    fn release(dir: &Path) -> VerifyReleaseArgs {
        std::fs::write(dir.join("SHA256SUMS"), SUMS).unwrap();
        std::fs::write(dir.join("SHA256SUMS.minisig"), SIG_TAURI_FORM).unwrap();
        std::fs::write(dir.join("a.txt"), "hello\n").unwrap();
        std::fs::write(dir.join("b.bin"), "world\n").unwrap();
        VerifyReleaseArgs { sums: dir.join("SHA256SUMS"), signature: None, pubkey: Some(PUBKEY.into()), pubkey_file: None, dir: None, require_all: false, signature_only: false }
    }

    #[test]
    fn verifies_signature_and_files() {
        let t = tempfile::tempdir().unwrap();
        let a = release(t.path());
        let r = verify(&a).unwrap();
        assert!(r.signature_ok && r.ok, "{r:?}");
        assert_eq!(r.trusted_comment, "timestamp:1 file:SHA256SUMS");
        assert_eq!(r.files.iter().map(|f| f.status.as_str()).collect::<Vec<_>>(), ["ok", "ok"]);
        // The plain minisign form of the same signature and the bare key line work too.
        use base64::Engine;
        let plain = base64::engine::general_purpose::STANDARD.decode(SIG_TAURI_FORM).unwrap();
        std::fs::write(t.path().join("SHA256SUMS.minisig"), plain).unwrap();
        let r = verify(&VerifyReleaseArgs { pubkey: Some("RWQi1JdkTdVOezokXxrGep7FEpBFtuMcmaW0EzL0NA74eJZAhTsq09tv".into()), ..a }).unwrap();
        assert!(r.ok);
    }

    #[test]
    fn tampering_missing_files_and_wrong_keys_fail() {
        let t = tempfile::tempdir().unwrap();
        let a = release(t.path());
        std::fs::write(t.path().join("b.bin"), "tampered\n").unwrap();
        let r = verify(&a).unwrap();
        assert!(r.signature_ok && !r.ok);
        assert_eq!(r.files[1], FileCheck { name: "b.bin".into(), status: "mismatch".into() });
        std::fs::remove_file(t.path().join("b.bin")).unwrap();
        assert!(verify(&a).unwrap().ok, "a missing download is fine by default");
        assert!(!verify(&VerifyReleaseArgs { require_all: true, ..a.clone() }).unwrap().ok);
        std::fs::write(t.path().join("SHA256SUMS"), format!("{SUMS}0000000000000000000000000000000000000000000000000000000000000000  c\n")).unwrap();
        assert!(!verify(&a).unwrap().signature_ok, "an edited checksum list breaks the signature");
        std::fs::write(t.path().join("SHA256SUMS"), SUMS).unwrap();
        assert!(!verify(&VerifyReleaseArgs { pubkey: Some(RELEASE_PUBKEY.into()), ..a.clone() }).unwrap().signature_ok, "another key");
        assert!(parse_pubkey("garbage").is_err());
    }

    #[test]
    fn release_key_matches_the_updater_key() {
        let conf: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../src-tauri/tauri.conf.json")).unwrap()).unwrap();
        let tauri_form = conf["plugins"]["updater"]["pubkey"].as_str().unwrap();
        assert!(parse_pubkey(tauri_form).is_ok() && parse_pubkey(RELEASE_PUBKEY).is_ok());
        use base64::Engine;
        let decoded = String::from_utf8(base64::engine::general_purpose::STANDARD.decode(tauri_form).unwrap()).unwrap();
        assert_eq!(decoded.lines().nth(1), Some(RELEASE_PUBKEY));
        let sh = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../scripts/verify-release.sh")).unwrap();
        assert!(sh.contains(&format!("DEFAULT_PUBKEY=\"{RELEASE_PUBKEY}\"")));
    }

    #[test]
    fn parses_sums_lines() {
        let s = parse_sums("# c\nabc  x\n5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03 *bin.exe\r\n\n");
        assert_eq!(s, vec![("5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03".to_string(), "bin.exe".to_string())]);
    }
}
