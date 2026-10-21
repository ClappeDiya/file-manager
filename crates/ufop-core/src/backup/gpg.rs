//! GPG (OpenPGP) volume encryption through the `gpg` executable.
//!
//! Two modes:
//! - **symmetric**: `gpg --symmetric --cipher-algo AES256` with the backup
//!   passphrase (sent on stdin, never on the command line);
//! - **recipients**: `gpg --encrypt -r <key>…`. Restores need the matching
//!   secret key in the keyring; the passphrase, if given, unlocks it.
//!
//! Plaintext only touches disk while sealing a volume in symmetric mode
//! (stdin carries the passphrase there), in a private temporary file that is
//! removed straight away.

use super::modules::EncryptionModule;
use crate::error::AppError;
use std::io::Write;
use std::process::{Command, Stdio};

#[derive(Clone, Debug, Default)]
pub struct GpgModule {
    /// `gpg` binary (empty = `$UFOP_GPG`, else `gpg` on PATH).
    pub executable: String,
    /// `--homedir` (empty = the default keyring / `$GNUPGHOME`).
    pub homedir: String,
    pub passphrase: Option<String>,
    /// Key ids, fingerprints or e-mail addresses; empty = symmetric.
    pub recipients: Vec<String>,
}

fn gpg_err(msg: String) -> AppError {
    AppError::encryption(msg, "Check that GnuPG is installed, the passphrase is right and (for key encryption) the secret key is in the keyring.")
}

impl GpgModule {
    fn command(&self) -> Command {
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

    fn with_pass(&self, c: &mut Command) -> Option<Vec<u8>> {
        let p = self.passphrase.as_deref().filter(|p| !p.is_empty())?;
        // Without --no-symkey-cache, gpg-agent reuses a cached symmetric key
        // and a wrong passphrase would still decrypt.
        c.args(["--pinentry-mode", "loopback", "--no-symkey-cache", "--passphrase-fd", "0"]);
        Some(format!("{p}\n").into_bytes())
    }

    fn run(mut c: Command, stdin: Vec<u8>, what: &str) -> Result<Vec<u8>, AppError> {
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
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            return Err(gpg_err(format!("gpg {what} failed: {}", err.trim().lines().last().unwrap_or("unknown error"))));
        }
        Ok(out.stdout)
    }

    /// Private temporary file holding `data`; removed when dropped.
    fn temp_file(data: &[u8]) -> Result<TempFile, AppError> {
        let dir = std::env::temp_dir().join(format!("ufop-gpg-{}", uuid::Uuid::new_v4().simple()));
        std::fs::create_dir(&dir)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        }
        let path = dir.join("data");
        std::fs::write(&path, data)?;
        Ok(TempFile { dir, path })
    }

    /// `gpg --version` first line (diagnostics).
    pub fn version(&self) -> Result<String, AppError> {
        let mut c = self.command();
        c.arg("--version");
        let out = Self::run(c, Vec::new(), "--version")?;
        Ok(String::from_utf8_lossy(&out).lines().next().unwrap_or_default().to_string())
    }
}

struct TempFile {
    dir: std::path::PathBuf,
    path: std::path::PathBuf,
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

impl EncryptionModule for GpgModule {
    fn name(&self) -> &str {
        "gpg"
    }

    fn seal(&self, plain: &[u8]) -> Result<Vec<u8>, AppError> {
        let mut c = self.command();
        if !self.recipients.is_empty() {
            c.args(["--encrypt", "--trust-model", "always"]);
            for r in &self.recipients {
                c.arg("--recipient").arg(r);
            }
            c.args(["--output", "-"]);
            return Self::run(c, plain.to_vec(), "encryption");
        }
        let pass = self.with_pass(&mut c).ok_or_else(|| AppError::validation("GPG symmetric encryption needs a passphrase (or set recipients)"))?;
        let tmp = Self::temp_file(plain)?;
        c.args(["--symmetric", "--cipher-algo", "AES256", "--s2k-digest-algo", "SHA512", "--s2k-count", "65011712", "--output", "-"]).arg(&tmp.path);
        Self::run(c, pass, "encryption")
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, AppError> {
        let mut c = self.command();
        let stdin = self.with_pass(&mut c);
        c.args(["--decrypt", "--output", "-"]);
        match stdin {
            Some(pass) => {
                let tmp = Self::temp_file(sealed)?;
                c.arg(&tmp.path);
                Self::run(c, pass, "decryption")
            }
            None => Self::run(c, sealed.to_vec(), "decryption"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpg_available() -> bool {
        Command::new("gpg").arg("--version").output().is_ok_and(|o| o.status.success())
    }

    #[test]
    fn symmetric_and_recipient_round_trips() {
        if !gpg_available() {
            return;
        }
        let home = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(home.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let h = home.path().to_string_lossy().into_owned();
        let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();

        let sym = GpgModule { homedir: h.clone(), passphrase: Some("correct horse".into()), ..Default::default() };
        let sealed = sym.seal(&data).unwrap();
        assert!(sealed.len() > 100 && sealed != data);
        assert_eq!(sym.open(&sealed).unwrap(), data);
        let wrong = GpgModule { passphrase: Some("nope".into()), ..sym.clone() };
        assert!(wrong.open(&sealed).is_err());
        assert!(GpgModule { passphrase: None, ..sym.clone() }.seal(&data).is_err());
        assert!(sym.version().unwrap().contains("GnuPG"));

        // Key encryption: generate a throwaway key without a passphrase.
        let ok = Command::new("gpg")
            .args(["--batch", "--homedir", &h, "--passphrase", "", "--quick-gen-key", "UFOP Test <gpg@ufop.invalid>", "default", "default", "never"])
            .output()
            .unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        let key = GpgModule { homedir: h.clone(), recipients: vec!["gpg@ufop.invalid".into()], ..Default::default() };
        let sealed = key.seal(&data).unwrap();
        assert_eq!(key.open(&sealed).unwrap(), data);
        let other = tempfile::tempdir().unwrap();
        assert!(GpgModule { homedir: other.path().to_string_lossy().into(), ..Default::default() }.open(&sealed).is_err(), "needs the secret key");
    }
}
