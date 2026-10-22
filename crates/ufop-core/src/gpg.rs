//! OpenPGP through the `gpg` executable: running it safely (batch mode,
//! passphrases on stdin, never argv), the user's keys with their expiry,
//! and file-level encrypt / decrypt / sign / verify.
//!
//! Used by the file manager's GPG actions and by the backup engine's GPG
//! volume cipher (`backup::gpg`).

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// How to reach gpg: binary (empty = `$UFOP_GPG`, else `gpg` on PATH) and
/// keyring (`--homedir`, empty = default / `$GNUPGHOME`).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Gpg {
    #[serde(default)]
    pub executable: String,
    #[serde(default)]
    pub homedir: String,
}

pub(crate) fn gpg_err(msg: String) -> AppError {
    AppError::encryption(msg, "Check that GnuPG is installed, the passphrase is right and the needed key is in the keyring.")
}

/// A key in the keyring (from `--with-colons`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GpgKey {
    pub fingerprint: String,
    pub key_id: String,
    pub user_ids: Vec<String>,
    /// Unix seconds.
    pub created: Option<i64>,
    pub expires: Option<i64>,
    pub expired: bool,
    pub revoked: bool,
    /// Usable for encryption (`e`) / signing (`s`), from the key and its subkeys.
    pub can_encrypt: bool,
    pub can_sign: bool,
    /// The secret key is available (decrypt / sign).
    pub secret: bool,
    pub algorithm: String,
}

/// Outcome of a signature check.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignatureCheck {
    /// The signature is cryptographically good.
    pub valid: bool,
    /// Signer's key id / fingerprint and user id, as gpg reports them.
    pub key_id: Option<String>,
    pub fingerprint: Option<String>,
    pub user_id: Option<String>,
    /// Unix seconds.
    pub signed_at: Option<i64>,
    /// Good signature, but the key has expired or been revoked.
    pub key_expired: bool,
    pub key_revoked: bool,
    /// The public key needed to check it isn't in the keyring.
    pub missing_key: bool,
    pub message: String,
}

/// Options for encrypting a file.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EncryptOptions {
    /// Key ids / fingerprints / e-mails; empty = passphrase (symmetric).
    #[serde(default)]
    pub recipients: Vec<String>,
    /// ASCII armor (`.asc`) instead of binary (`.gpg`).
    #[serde(default)]
    pub armor: bool,
    /// Also sign with this secret key.
    #[serde(default)]
    pub sign_with: Option<String>,
}

/// Name for the encrypted/signed copy, next to the input: `x.pdf.gpg`.
pub fn sealed_name(input: &Path, armor: bool, detached_sig: bool) -> PathBuf {
    let ext = match (detached_sig, armor) {
        (true, true) => "asc",
        (true, false) => "sig",
        (false, true) => "asc",
        (false, false) => "gpg",
    };
    let mut name = input.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{ext}"));
    input.with_file_name(name)
}

/// Name for the decrypted copy: drops `.gpg` / `.pgp` / `.asc`, else adds `.decrypted`.
pub fn opened_name(input: &Path) -> PathBuf {
    let ext = input.extension().map(|e| e.to_string_lossy().to_lowercase());
    match ext.as_deref() {
        Some("gpg" | "pgp" | "asc") => input.with_extension(""),
        _ => {
            let mut name = input.file_name().unwrap_or_default().to_os_string();
            name.push(".decrypted");
            input.with_file_name(name)
        }
    }
}

/// `p`, or `stem (n).ext` if it exists.
pub fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
    let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..10_000)
        .map(|n| p.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|c| !c.exists())
        .unwrap_or_else(|| p.to_path_buf())
}

impl Gpg {
    pub(crate) fn command(&self) -> Command {
        let exe = if !self.executable.trim().is_empty() {
            self.executable.trim().to_string()
        } else {
            std::env::var("UFOP_GPG").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "gpg".into())
        };
        let mut c = Command::new(exe);
        c.args(["--batch", "--yes", "--no-tty", "--quiet"]);
        if !self.homedir.trim().is_empty() {
            c.arg("--homedir").arg(self.homedir.trim());
        }
        c.stdout(Stdio::piped()).stderr(Stdio::piped());
        c
    }

    /// Read the passphrase from stdin; returns the bytes to send.
    pub(crate) fn with_pass(c: &mut Command, passphrase: Option<&str>) -> Option<Vec<u8>> {
        let p = passphrase.filter(|p| !p.is_empty())?;
        // Without --no-symkey-cache, gpg-agent reuses a cached symmetric key
        // and a wrong passphrase would still decrypt.
        c.args(["--pinentry-mode", "loopback", "--no-symkey-cache", "--passphrase-fd", "0"]);
        Some(format!("{p}\n").into_bytes())
    }

    /// Run gpg; returns (stdout, stderr) on success.
    pub(crate) fn run_full(mut c: Command, stdin: Vec<u8>, what: &str) -> Result<(Vec<u8>, String), AppError> {
        c.stdin(Stdio::piped());
        let mut child = c.spawn().map_err(|e| gpg_err(format!("Could not start gpg: {e}")))?;
        let mut sin = child.stdin.take().ok_or_else(|| gpg_err("gpg stdin unavailable".into()))?;
        // Feed stdin on a thread so large outputs cannot deadlock the pipes.
        let writer = std::thread::spawn(move || {
            let r = sin.write_all(&stdin);
            drop(sin);
            r
        });
        let out = child.wait_with_output().map_err(|e| gpg_err(format!("gpg failed: {e}")))?;
        let _ = writer.join();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        if !out.status.success() {
            return Err(gpg_err(format!("gpg {what} failed: {}", last_reason(&stderr))));
        }
        Ok((out.stdout, stderr))
    }

    pub(crate) fn run(c: Command, stdin: Vec<u8>, what: &str) -> Result<Vec<u8>, AppError> {
        Self::run_full(c, stdin, what).map(|(o, _)| o)
    }

    /// `gpg --version` first line.
    pub fn version(&self) -> Result<String, AppError> {
        let mut c = self.command();
        c.arg("--version");
        let out = Self::run(c, Vec::new(), "--version")?;
        Ok(String::from_utf8_lossy(&out).lines().next().unwrap_or_default().to_string())
    }

    /// Public keys (with `secret` set where the secret key is present).
    pub fn list_keys(&self) -> Result<Vec<GpgKey>, AppError> {
        let list = |secret: bool| -> Result<String, AppError> {
            let mut c = self.command();
            c.args(["--with-colons", "--fixed-list-mode", if secret { "--list-secret-keys" } else { "--list-keys" }]);
            Self::run(c, Vec::new(), "key listing").map(|o| String::from_utf8_lossy(&o).to_string())
        };
        let mut keys = parse_colons(&list(false)?);
        let secret: std::collections::HashSet<String> = parse_colons(&list(true)?).into_iter().map(|k| k.fingerprint).collect();
        for k in &mut keys {
            k.secret = secret.contains(&k.fingerprint);
        }
        Ok(keys)
    }

    /// Import keys from armored or binary key data; returns the count imported.
    pub fn import_keys(&self, data: &[u8]) -> Result<u32, AppError> {
        let mut c = self.command();
        c.args(["--status-fd", "2", "--import"]);
        let (_, err) = Self::run_full(c, data.to_vec(), "import")?;
        Ok(err
            .lines()
            .find_map(|l| l.strip_prefix("[GNUPG:] IMPORT_RES "))
            .and_then(|r| r.split_whitespace().nth(2))
            .and_then(|n| n.parse().ok())
            .unwrap_or(0))
    }

    /// Armored public key block for sharing.
    pub fn export_public(&self, key: &str) -> Result<String, AppError> {
        let mut c = self.command();
        c.args(["--armor", "--export", key]);
        let out = Self::run(c, Vec::new(), "export")?;
        if out.is_empty() {
            return Err(AppError::validation(format!("No public key matches {key}")));
        }
        Ok(String::from_utf8_lossy(&out).to_string())
    }

    pub fn encrypt_file(&self, input: &Path, output: &Path, opts: &EncryptOptions, passphrase: Option<&str>) -> Result<(), AppError> {
        let mut c = self.command();
        let stdin = if opts.recipients.is_empty() {
            let pass = Self::with_pass(&mut c, passphrase).ok_or_else(|| AppError::validation("Choose recipients, or enter a passphrase to encrypt with"))?;
            c.args(["--symmetric", "--cipher-algo", "AES256", "--s2k-digest-algo", "SHA512", "--s2k-count", "65011712"]);
            if let Some(k) = &opts.sign_with {
                c.arg("--sign").arg("--local-user").arg(k);
            }
            pass
        } else {
            c.args(["--encrypt", "--trust-model", "always"]);
            for r in &opts.recipients {
                c.arg("--recipient").arg(r);
            }
            if let Some(k) = &opts.sign_with {
                c.arg("--sign").arg("--local-user").arg(k);
            }
            Self::with_pass(&mut c, passphrase).unwrap_or_default()
        };
        if opts.armor {
            c.arg("--armor");
        }
        c.arg("--output").arg(output).arg(input);
        Self::run(c, stdin, "encryption").map(|_| ())
    }

    /// Decrypt `input` to `output`; reports the embedded signature, if any.
    pub fn decrypt_file(&self, input: &Path, output: &Path, passphrase: Option<&str>) -> Result<Option<SignatureCheck>, AppError> {
        let mut c = self.command();
        let stdin = Self::with_pass(&mut c, passphrase).unwrap_or_default();
        c.args(["--status-fd", "2", "--decrypt", "--output"]).arg(output).arg(input);
        let (_, err) = Self::run_full(c, stdin, "decryption").inspect_err(|_| {
            let _ = std::fs::remove_file(output);
        })?;
        let check = parse_status(&err);
        Ok((check.valid || check.key_id.is_some() || check.missing_key).then_some(check))
    }

    /// Sign `input`: detached signature (`.sig`/`.asc`) or a signed copy.
    pub fn sign_file(&self, input: &Path, output: &Path, key: Option<&str>, detached: bool, armor: bool, passphrase: Option<&str>) -> Result<(), AppError> {
        let mut c = self.command();
        let stdin = Self::with_pass(&mut c, passphrase).unwrap_or_default();
        c.arg(if detached { "--detach-sign" } else { "--sign" });
        if let Some(k) = key.filter(|k| !k.is_empty()) {
            c.arg("--local-user").arg(k);
        }
        if armor {
            c.arg("--armor");
        }
        c.arg("--output").arg(output).arg(input);
        Self::run(c, stdin, "signing").map(|_| ())
    }

    /// Check a detached signature (`signature` + `data`), or a signed file.
    pub fn verify(&self, signature: &Path, data: Option<&Path>) -> Result<SignatureCheck, AppError> {
        let mut c = self.command();
        c.args(["--status-fd", "1", "--verify"]).arg(signature);
        if let Some(d) = data {
            c.arg(d);
        }
        c.stdin(Stdio::piped());
        let child = c.spawn().map_err(|e| gpg_err(format!("Could not start gpg: {e}")))?;
        let out = child.wait_with_output().map_err(|e| gpg_err(format!("gpg failed: {e}")))?;
        // A bad signature exits non-zero but is an answer, not an error.
        let mut check = parse_status(&String::from_utf8_lossy(&out.stdout));
        if check.message.is_empty() {
            check.message = last_reason(&String::from_utf8_lossy(&out.stderr));
        }
        Ok(check)
    }
}

fn last_reason(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| !l.starts_with("[GNUPG:]"))
        .map(|l| l.trim_start_matches("gpg: ").trim())
        .rfind(|l| !l.is_empty())
        .unwrap_or("unknown error")
        .to_string()
}

fn num(s: &str) -> Option<i64> {
    s.parse().ok().filter(|n| *n > 0)
}

/// Parse `gpg --with-colons --fixed-list-mode` output.
pub fn parse_colons(text: &str) -> Vec<GpgKey> {
    let mut keys: Vec<GpgKey> = Vec::new();
    // fpr lines follow the record they belong to: only the primary's counts.
    let mut want_fpr = false;
    for line in text.lines() {
        let f: Vec<&str> = line.split(':').collect();
        match f.first().copied() {
            Some("pub" | "sec") if f.len() > 11 => {
                let validity = f[1];
                keys.push(GpgKey {
                    fingerprint: String::new(),
                    key_id: f[4].to_string(),
                    user_ids: Vec::new(),
                    created: num(f[5]),
                    expires: num(f[6]),
                    expired: validity == "e",
                    revoked: validity == "r",
                    // Field 12 holds the usable capabilities of the whole key in caps.
                    can_encrypt: f[11].contains('E'),
                    can_sign: f[11].contains('S'),
                    secret: f[0] == "sec",
                    algorithm: match f[3] {
                        "1" => "RSA",
                        "17" => "DSA",
                        "18" => "ECDH",
                        "19" => "ECDSA",
                        "22" => "EdDSA",
                        _ => "other",
                    }
                    .to_string(),
                });
                want_fpr = true;
            }
            Some("fpr") if want_fpr && f.len() > 9 => {
                if let Some(k) = keys.last_mut() {
                    k.fingerprint = f[9].to_string();
                }
                want_fpr = false;
            }
            Some("uid") if f.len() > 9 => {
                if let Some(k) = keys.last_mut() {
                    if f[1] != "r" {
                        k.user_ids.push(f[9].replace("\\x3a", ":"));
                    }
                }
            }
            Some("sub" | "ssb") => want_fpr = false,
            _ => {}
        }
    }
    keys
}

/// Parse `--status-fd` lines of a verify / decrypt run.
pub fn parse_status(text: &str) -> SignatureCheck {
    let mut c = SignatureCheck::default();
    for line in text.lines().filter_map(|l| l.strip_prefix("[GNUPG:] ")) {
        let mut parts = line.splitn(3, ' ');
        let (tag, a, rest) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        match tag {
            "GOODSIG" => {
                c.valid = true;
                c.key_id = Some(a.to_string());
                c.user_id = Some(rest.to_string());
                c.message = format!("Good signature from {rest}");
            }
            "EXPKEYSIG" | "REVKEYSIG" => {
                c.valid = true;
                c.key_id = Some(a.to_string());
                c.user_id = Some(rest.to_string());
                if tag == "EXPKEYSIG" {
                    c.key_expired = true;
                    c.message = format!("Good signature from {rest}, but the key has expired");
                } else {
                    c.key_revoked = true;
                    c.message = format!("Good signature from {rest}, but the key was revoked");
                }
            }
            "BADSIG" => {
                c.valid = false;
                c.key_id = Some(a.to_string());
                c.user_id = Some(rest.to_string());
                c.message = format!("BAD signature claimed to be from {rest}: the file was changed or the signature is forged");
            }
            "VALIDSIG" => {
                c.fingerprint = Some(a.to_string());
                c.signed_at = rest.split_whitespace().nth(1).and_then(num);
            }
            "NO_PUBKEY" => {
                c.missing_key = true;
                c.key_id = Some(a.to_string());
                c.message = format!("Can't check: public key {a} isn't in your keyring");
            }
            "NODATA" if c.message.is_empty() => c.message = "No signature found".into(),
            _ => {}
        }
    }
    c
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_key_listing() {
        let text = "\
tru::1:1700000000:0:3:1:5
pub:u:255:22:AAAABBBBCCCCDDDD:1700000000:1900000000::u:::scESC:::::ed25519:::0:
fpr:::::::::0123456789ABCDEF0123456789ABCDEFAAAABBBB:
uid:u::::1700000000::HASH::Ada Lovelace <ada@example.org>::::::::::0:
uid:r::::1700000000::HASH2::Old Name <old@example.org>::::::::::0:
sub:u:255:18:1111222233334444:1700000000:1900000000:::::e:::::cv25519::
fpr:::::::::FFFFFFFFFFFFFFFFFFFFFFFFFFFFFFFF11112222:
pub:e:3072:1:9999888877776666:1500000000:1600000000::-:::sc:::::::::0:
fpr:::::::::99998888777766669999888877776666AAAA0000:
uid:e::::1500000000::H3::Expired Key \\x3a test::::::::::0:
";
        let keys = parse_colons(text);
        assert_eq!(keys.len(), 2);
        let a = &keys[0];
        assert_eq!(a.fingerprint, "0123456789ABCDEF0123456789ABCDEFAAAABBBB", "primary fingerprint, not the subkey's");
        assert_eq!(a.user_ids, vec!["Ada Lovelace <ada@example.org>"], "revoked uids dropped");
        assert_eq!((a.created, a.expires, a.expired, a.can_encrypt, a.can_sign), (Some(1700000000), Some(1900000000), false, true, true));
        assert_eq!(a.algorithm, "EdDSA");
        let b = &keys[1];
        assert!(b.expired && !b.can_encrypt && !b.can_sign);
        assert_eq!(b.user_ids, vec!["Expired Key : test"]);
    }

    #[test]
    fn parses_signature_status() {
        let good = parse_status("[GNUPG:] NEWSIG\n[GNUPG:] GOODSIG AAAABBBBCCCCDDDD Ada <ada@example.org>\n[GNUPG:] VALIDSIG 0123ABCD 2026-10-01 1790000000 0 4 0 22 10 00 0123ABCD\n");
        assert!(good.valid && !good.key_expired);
        assert_eq!((good.fingerprint.as_deref(), good.signed_at), (Some("0123ABCD"), Some(1790000000)));
        let bad = parse_status("[GNUPG:] BADSIG AAAA Ada <ada@example.org>\n");
        assert!(!bad.valid && bad.message.starts_with("BAD signature"));
        let exp = parse_status("[GNUPG:] EXPKEYSIG AAAA Ada\n");
        assert!(exp.valid && exp.key_expired);
        let missing = parse_status("[GNUPG:] ERRSIG AAAA 22 10 00 1790000000 9 -\n[GNUPG:] NO_PUBKEY AAAA\n");
        assert!(!missing.valid && missing.missing_key);
    }

    #[test]
    fn output_names() {
        let p = Path::new("/d/report.pdf");
        assert_eq!(sealed_name(p, false, false), Path::new("/d/report.pdf.gpg"));
        assert_eq!(sealed_name(p, true, false), Path::new("/d/report.pdf.asc"));
        assert_eq!(sealed_name(p, false, true), Path::new("/d/report.pdf.sig"));
        assert_eq!(opened_name(Path::new("/d/report.pdf.gpg")), Path::new("/d/report.pdf"));
        assert_eq!(opened_name(Path::new("/d/report.PGP")), Path::new("/d/report"));
        assert_eq!(opened_name(Path::new("/d/blob")), Path::new("/d/blob.decrypted"));
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("a.txt");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(unique_path(&f), dir.path().join("a (2).txt"));
    }

    fn gpg_available() -> bool {
        Command::new("gpg").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    fn keyring() -> (tempfile::TempDir, Gpg) {
        let home = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let g = Gpg { homedir: home.path().to_string_lossy().into(), ..Default::default() };
        (home, g)
    }

    fn gen_key(g: &Gpg, email: &str) {
        let out = Command::new("gpg")
            .args(["--batch", "--homedir", &g.homedir, "--passphrase", "", "--quick-gen-key", &format!("Test <{email}>"), "default", "default", "2y"])
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }

    #[test]
    fn file_round_trips_signatures_and_keys() {
        if !gpg_available() {
            return;
        }
        let (_home, g) = keyring();
        gen_key(&g, "alice@ufop.invalid");
        let keys = g.list_keys().unwrap();
        assert_eq!(keys.len(), 1);
        let k = &keys[0];
        assert!(k.secret && k.can_encrypt && k.can_sign && k.expires.is_some() && !k.expired);
        assert_eq!(k.fingerprint.len(), 40);

        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("doc.txt");
        std::fs::write(&plain, "secret plans").unwrap();

        // Recipient encryption, armored and signed.
        let sealed = sealed_name(&plain, true, false);
        let opts = EncryptOptions { recipients: vec!["alice@ufop.invalid".into()], armor: true, sign_with: Some(k.fingerprint.clone()) };
        g.encrypt_file(&plain, &sealed, &opts, None).unwrap();
        assert!(std::fs::read_to_string(&sealed).unwrap().starts_with("-----BEGIN PGP MESSAGE-----"));
        let out = dir.path().join("out.txt");
        let sig = g.decrypt_file(&sealed, &out, None).unwrap().expect("signed");
        assert!(sig.valid, "{sig:?}");
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "secret plans");

        // Symmetric: right and wrong passphrase.
        let sym = dir.path().join("doc.txt.gpg");
        g.encrypt_file(&plain, &sym, &EncryptOptions::default(), Some("hunter2")).unwrap();
        assert!(g.encrypt_file(&plain, &dir.path().join("x.gpg"), &EncryptOptions::default(), None).is_err());
        let wrong = dir.path().join("wrong.txt");
        assert!(g.decrypt_file(&sym, &wrong, Some("nope")).is_err());
        assert!(!wrong.exists(), "no partial output left behind");
        let right = dir.path().join("right.txt");
        assert!(g.decrypt_file(&sym, &right, Some("hunter2")).unwrap().is_none(), "unsigned");
        assert_eq!(std::fs::read_to_string(&right).unwrap(), "secret plans");

        // Detached signature: good, then tampered.
        let sigf = sealed_name(&plain, false, true);
        g.sign_file(&plain, &sigf, None, true, false, None).unwrap();
        assert!(g.verify(&sigf, Some(&plain)).unwrap().valid);
        std::fs::write(&plain, "secret plans!").unwrap();
        let bad = g.verify(&sigf, Some(&plain)).unwrap();
        assert!(!bad.valid && bad.message.contains("BAD"), "{bad:?}");

        // Export / import into another keyring: verify then works there.
        let public = g.export_public(&k.fingerprint).unwrap();
        let (_h2, other) = keyring();
        std::fs::write(&plain, "secret plans").unwrap();
        let missing = other.verify(&sigf, Some(&plain)).unwrap();
        assert!(missing.missing_key && !missing.valid);
        assert_eq!(other.import_keys(public.as_bytes()).unwrap(), 1);
        assert!(other.verify(&sigf, Some(&plain)).unwrap().valid);
        assert!(other.list_keys().unwrap().iter().all(|k| !k.secret));
        assert!(other.export_public("nobody@nowhere.invalid").is_err());
    }
}
