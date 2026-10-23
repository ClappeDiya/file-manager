//! OpenPGP through the `gpg` executable: running it safely (batch mode,
//! passphrases on stdin, never argv), the user's keys with their expiry,
//! and file-level encrypt / decrypt / sign / verify, re-encryption in
//! memory, legacy (no integrity protection) files on request, and key
//! generation.
//!
//! Plaintext never goes to a temporary file: when gpg needs both a
//! passphrase and data on stdin, the passphrase is the first line
//! (`--passphrase-fd 0` reads exactly one line) and the data follows.
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

/// Decrypting refused a file without integrity protection.
pub const NO_INTEGRITY: &str = "This file was encrypted without integrity protection";

/// Largest file [`Gpg::recrypt_file`] re-encrypts in memory.
pub const MAX_RECRYPT_BYTES: u64 = 512 * 1024 * 1024;

/// Outcome of a decryption.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct Decryption {
    /// The embedded signature, if the file was signed.
    pub signature: Option<SignatureCheck>,
    /// Decrypted although it had no integrity protection (allowed explicitly).
    pub unprotected: bool,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum KeyAlgorithm {
    /// Ed25519 signing key with a Curve25519 encryption subkey.
    #[default]
    Ed25519,
    /// RSA 4096 signing key with an RSA 4096 encryption subkey (for older software).
    Rsa4096,
}

/// A key pair to generate.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct NewKey {
    pub name: String,
    pub email: String,
    #[serde(default)]
    pub algorithm: KeyAlgorithm,
    /// `2y`, `90d`, `2030-12-31`, `never`; empty = 2 years.
    #[serde(default)]
    pub expires: String,
}

impl NewKey {
    /// `Name <email>`, checked.
    fn user_id(&self) -> Result<String, AppError> {
        let (name, email) = (self.name.trim(), self.email.trim());
        if name.is_empty() || name.chars().count() > 100 || name.contains(['<', '>', '\n', '\r']) {
            return Err(AppError::validation("Enter a name (up to 100 characters, without < or >)"));
        }
        let at = email.find('@').unwrap_or(0);
        if at == 0 || at + 1 >= email.len() || email.contains(|c: char| c.is_whitespace() || c == '<' || c == '>') || email.len() > 254 {
            return Err(AppError::validation("Enter an e-mail address like name@example.com"));
        }
        Ok(format!("{name} <{email}>"))
    }
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
    pub(crate) fn run_full(c: Command, stdin: Vec<u8>, what: &str) -> Result<(Vec<u8>, String), AppError> {
        let (ok, out, stderr) = Self::exec(c, stdin)?;
        if !ok {
            return Err(gpg_err(format!("gpg {what} failed: {}", last_reason(&stderr))));
        }
        Ok((out, stderr))
    }

    /// Run gpg; returns (success, stdout, stderr).
    fn exec(mut c: Command, stdin: Vec<u8>) -> Result<(bool, Vec<u8>, String), AppError> {
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
        Ok((out.status.success(), out.stdout, String::from_utf8_lossy(&out.stderr).to_string()))
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
        let (mut c, stdin) = self.encrypt_command(opts, passphrase)?;
        c.arg("--output").arg(output).arg(input);
        Self::run(c, stdin, "encryption").map(|_| ())
    }

    /// Encrypt `plain` in memory (it goes to gpg on stdin, after the passphrase line when there is one).
    pub fn encrypt_bytes(&self, plain: &[u8], opts: &EncryptOptions, passphrase: Option<&str>) -> Result<Vec<u8>, AppError> {
        let (mut c, mut stdin) = self.encrypt_command(opts, passphrase)?;
        c.args(["--output", "-"]);
        stdin.extend_from_slice(plain);
        Self::run(c, stdin, "encryption")
    }

    /// The encryption command and the passphrase line it reads from stdin.
    fn encrypt_command(&self, opts: &EncryptOptions, passphrase: Option<&str>) -> Result<(Command, Vec<u8>), AppError> {
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
        Ok((c, stdin))
    }

    /// Decrypt `input` to `output`; reports the embedded signature, if any.
    /// Files without integrity protection are refused (see [`Gpg::decrypt_file_with`]).
    pub fn decrypt_file(&self, input: &Path, output: &Path, passphrase: Option<&str>) -> Result<Option<SignatureCheck>, AppError> {
        self.decrypt_file_with(input, output, passphrase, false).map(|d| d.signature)
    }

    /// Decrypt `input` to `output`. A legacy file without integrity
    /// protection (no MDC, typical before 2003) is refused with
    /// [`NO_INTEGRITY`] unless `allow_unprotected`; then it decrypts and
    /// the result says so.
    pub fn decrypt_file_with(&self, input: &Path, output: &Path, passphrase: Option<&str>, allow_unprotected: bool) -> Result<Decryption, AppError> {
        let (mut c, stdin) = self.decrypt_command(passphrase, allow_unprotected);
        c.arg("--output").arg(output).arg(input);
        Self::decryption(c, stdin).map(|(_, d)| d).inspect_err(|_| {
            let _ = std::fs::remove_file(output);
        })
    }

    /// Decrypt `sealed` in memory.
    pub fn decrypt_bytes(&self, sealed: &[u8], passphrase: Option<&str>, allow_unprotected: bool) -> Result<(Vec<u8>, Decryption), AppError> {
        let (mut c, mut stdin) = self.decrypt_command(passphrase, allow_unprotected);
        c.args(["--output", "-"]);
        stdin.extend_from_slice(sealed);
        Self::decryption(c, stdin)
    }

    fn decrypt_command(&self, passphrase: Option<&str>, allow_unprotected: bool) -> (Command, Vec<u8>) {
        let mut c = self.command();
        let stdin = Self::with_pass(&mut c, passphrase).unwrap_or_default();
        if allow_unprotected {
            c.arg("--ignore-mdc-error");
        }
        c.args(["--status-fd", "2", "--decrypt"]);
        (c, stdin)
    }

    fn decryption(c: Command, stdin: Vec<u8>) -> Result<(Vec<u8>, Decryption), AppError> {
        let (ok, out, err) = Self::exec(c, stdin)?;
        if !ok {
            if err.lines().any(|l| l.starts_with("[GNUPG:] ERROR nomdc_with_legacy_cipher")) {
                return Err(AppError::encryption(
                    format!("{NO_INTEGRITY} (an old OpenPGP format), so changes to it can't be detected"),
                    "If you trust where the file came from (files made before 2003 often look like this), decrypt it anyway.",
                ));
            }
            return Err(gpg_err(format!("gpg decryption failed: {}", last_reason(&err))));
        }
        let check = parse_status(&err);
        // `DECRYPTION_INFO <mdc> <cipher> [<aead>]`: neither MDC nor AEAD means no integrity protection.
        let unprotected = err.lines().filter_map(|l| l.strip_prefix("[GNUPG:] DECRYPTION_INFO ")).any(|info| {
            let f: Vec<&str> = info.split_whitespace().collect();
            f.first() == Some(&"0") && f.get(2).map_or(true, |a| *a == "0")
        });
        let signature = (check.valid || check.key_id.is_some() || check.missing_key).then_some(check);
        Ok((out, Decryption { signature, unprotected }))
    }

    /// Re-encrypt `input` for new recipients or a new passphrase, writing
    /// `output` (which may be `input`) atomically. The plaintext stays in
    /// memory (files up to [`MAX_RECRYPT_BYTES`]) and is wiped afterwards.
    pub fn recrypt_file(&self, input: &Path, output: &Path, opts: &EncryptOptions, old_passphrase: Option<&str>, new_passphrase: Option<&str>, allow_unprotected: bool) -> Result<Decryption, AppError> {
        let len = std::fs::metadata(input).map_err(|e| AppError::file_op(format!("Cannot read {}: {e}", input.display()), "Check the file"))?.len();
        if len > MAX_RECRYPT_BYTES {
            return Err(AppError::validation(format!("{} is larger than {} MB; decrypt and encrypt it in two steps instead", input.display(), MAX_RECRYPT_BYTES / 1024 / 1024)));
        }
        let sealed = std::fs::read(input).map_err(|e| AppError::file_op(format!("Cannot read {}: {e}", input.display()), "Check the file"))?;
        let (mut plain, d) = self.decrypt_bytes(&sealed, old_passphrase, allow_unprotected)?;
        let resealed = self.encrypt_bytes(&plain, opts, new_passphrase);
        plain.fill(0);
        let resealed = resealed?;
        let name = output.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let tmp = output.with_file_name(format!(".{name}.{}.ufop-tmp", uuid::Uuid::new_v4().simple()));
        let write = std::fs::write(&tmp, &resealed).and_then(|_| std::fs::rename(&tmp, output));
        if let Err(e) = write {
            let _ = std::fs::remove_file(&tmp);
            return Err(AppError::file_op(format!("Cannot write {}: {e}", output.display()), "Check free space and permissions"));
        }
        Ok(d)
    }

    /// Create a key pair that can sign and encrypt; returns its fingerprint.
    /// Without a passphrase the secret key is stored unprotected.
    pub fn generate_key(&self, key: &NewKey, passphrase: Option<&str>) -> Result<String, AppError> {
        let uid = key.user_id()?;
        let expires = key.expires.trim().to_lowercase();
        let expires = if expires.is_empty() { "2y".to_string() } else { expires };
        if !regex::Regex::new(r"^(never|\d{1,4}[dwmy]?|\d{4}-\d{2}-\d{2})$").expect("static pattern").is_match(&expires) {
            return Err(AppError::validation("Expiry is a number of days, weeks, months or years (e.g. 2y), a date (YYYY-MM-DD) or never"));
        }
        let (algo, usage) = match key.algorithm {
            KeyAlgorithm::Ed25519 => ("future-default", "default"),
            KeyAlgorithm::Rsa4096 => ("rsa4096", "cert,sign"),
        };
        let gen = |args: &[&str]| -> Result<String, AppError> {
            let mut c = self.command();
            let stdin = match Self::with_pass(&mut c, passphrase) {
                Some(p) => p,
                None => {
                    c.args(["--pinentry-mode", "loopback", "--passphrase", ""]);
                    Vec::new()
                }
            };
            c.args(["--status-fd", "2"]).args(args);
            Self::run_full(c, stdin, "key generation").map(|(_, err)| err)
        };
        let err = gen(&["--quick-generate-key", &uid, algo, usage, &expires])?;
        let fingerprint = err
            .lines()
            .find_map(|l| l.strip_prefix("[GNUPG:] KEY_CREATED "))
            .and_then(|r| r.split_whitespace().nth(1))
            .map(str::to_string)
            .ok_or_else(|| gpg_err("gpg did not report the new key".into()))?;
        if key.algorithm == KeyAlgorithm::Rsa4096 {
            gen(&["--quick-add-key", &fingerprint, "rsa4096", "encr", &expires])?;
        }
        Ok(fingerprint)
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

    #[test]
    fn legacy_files_recrypt_in_memory_and_generated_keys() {
        if !gpg_available() {
            return;
        }
        let (_home, g) = keyring();
        let dir = tempfile::tempdir().unwrap();
        let plain = dir.path().join("old.txt");
        std::fs::write(&plain, "from 1999").unwrap();

        // A legacy message without integrity protection (no MDC).
        let legacy = dir.path().join("old.txt.gpg");
        let mut c = g.command();
        let pass = Gpg::with_pass(&mut c, Some("pw")).unwrap();
        c.args(["--rfc2440", "--symmetric", "--output"]).arg(&legacy).arg(&plain);
        Gpg::run(c, pass, "legacy").unwrap();
        let out = dir.path().join("out.txt");
        let refused = g.decrypt_file(&legacy, &out, Some("pw")).unwrap_err();
        assert!(refused.user_message().contains(NO_INTEGRITY), "{}", refused.user_message());
        assert!(!out.exists(), "the refused plaintext is removed");
        let d = g.decrypt_file_with(&legacy, &out, Some("pw"), true).unwrap();
        assert!(d.unprotected);
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "from 1999");
        // Modern files are protected.
        let modern = dir.path().join("new.gpg");
        g.encrypt_file(&plain, &modern, &EncryptOptions::default(), Some("pw")).unwrap();
        assert!(!g.decrypt_file_with(&modern, &dir.path().join("n.txt"), Some("pw"), true).unwrap().unprotected);

        // In memory, with the passphrase line ahead of the data (binary data with newlines included).
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        let sealed = g.encrypt_bytes(&data, &EncryptOptions::default(), Some("line one")).unwrap();
        assert_eq!(g.decrypt_bytes(&sealed, Some("line one"), false).unwrap().0, data);
        assert!(g.decrypt_bytes(&sealed, Some("line two"), false).is_err());

        // Generated keys: Ed25519 with a passphrase, RSA without.
        let bad = |k: NewKey| g.generate_key(&k, None).unwrap_err().user_message();
        assert!(bad(NewKey { name: "".into(), email: "a@b.c".into(), ..Default::default() }).contains("name"));
        assert!(bad(NewKey { name: "A <x>".into(), email: "a@b.c".into(), ..Default::default() }).contains("name"));
        assert!(bad(NewKey { name: "A".into(), email: "nobody".into(), ..Default::default() }).contains("e-mail"));
        assert!(bad(NewKey { name: "A".into(), email: "a@b.c".into(), expires: "soon".into(), ..Default::default() }).contains("Expiry"));
        let ed = g.generate_key(&NewKey { name: "Ed Example".into(), email: "ed@ufop.invalid".into(), expires: "1y".into(), ..Default::default() }, Some("keypass")).unwrap();
        let rsa = g.generate_key(&NewKey { name: "Rsa Example".into(), email: "rsa@ufop.invalid".into(), algorithm: KeyAlgorithm::Rsa4096, expires: "never".into() }, None).unwrap();
        let keys = g.list_keys().unwrap();
        let find = |fp: &str| keys.iter().find(|k| k.fingerprint == fp).unwrap().clone();
        let (ed_key, rsa_key) = (find(&ed), find(&rsa));
        assert!(ed_key.secret && ed_key.can_sign && ed_key.can_encrypt && ed_key.expires.is_some(), "{ed_key:?}");
        assert!(rsa_key.secret && rsa_key.can_sign && rsa_key.can_encrypt && rsa_key.expires.is_none(), "{rsa_key:?}");
        assert_eq!(ed_key.user_ids, vec!["Ed Example <ed@ufop.invalid>"]);

        // Recrypt in place: passphrase → key, then key → new passphrase; old secrets stop working.
        let doc = dir.path().join("doc.gpg");
        g.encrypt_file(&plain, &doc, &EncryptOptions::default(), Some("old")).unwrap();
        let to_ed = EncryptOptions { recipients: vec![ed.clone()], ..Default::default() };
        g.recrypt_file(&doc, &doc, &to_ed, Some("old"), None, false).unwrap();
        assert!(g.decrypt_bytes(&std::fs::read(&doc).unwrap(), Some("old"), false).is_err());
        assert_eq!(g.decrypt_bytes(&std::fs::read(&doc).unwrap(), Some("keypass"), false).unwrap().0, b"from 1999");
        g.recrypt_file(&doc, &doc, &EncryptOptions::default(), Some("keypass"), Some("new"), false).unwrap();
        assert_eq!(g.decrypt_bytes(&std::fs::read(&doc).unwrap(), Some("new"), false).unwrap().0, b"from 1999");
        // A failed recrypt (wrong passphrase) leaves the file as it was, and no temporary files.
        let before = std::fs::read(&doc).unwrap();
        assert!(g.recrypt_file(&doc, &doc, &to_ed, Some("wrong"), None, false).is_err());
        assert_eq!(std::fs::read(&doc).unwrap(), before);
        assert!(std::fs::read_dir(dir.path()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().contains("ufop-tmp")));
        // Legacy files recrypt only when allowed, into a protected file.
        let fresh = dir.path().join("fresh.gpg");
        assert!(g.recrypt_file(&legacy, &fresh, &to_ed, Some("pw"), None, false).is_err());
        assert!(g.recrypt_file(&legacy, &fresh, &to_ed, Some("pw"), None, true).unwrap().unprotected);
        assert!(!g.decrypt_bytes(&std::fs::read(&fresh).unwrap(), Some("keypass"), false).unwrap().1.unprotected);
    }
}
