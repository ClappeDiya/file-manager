//! GPG (OpenPGP) volume encryption through the `gpg` executable.
//!
//! Two modes:
//! - **symmetric**: `gpg --symmetric --cipher-algo AES256` with the backup
//!   passphrase (sent on stdin, never on the command line);
//! - **recipients**: `gpg --encrypt -r <key>…`. Restores need the matching
//!   secret key in the keyring; the passphrase, if given, unlocks it.
//!
//! Volumes never touch disk in plaintext: when a passphrase is needed it is
//! the first line on gpg's stdin and the data follows.

use super::modules::EncryptionModule;
use crate::error::AppError;
use crate::gpg::Gpg;
#[cfg(test)]
use std::process::Command;

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

impl GpgModule {
    fn gpg(&self) -> Gpg {
        Gpg { executable: self.executable.clone(), homedir: self.homedir.clone() }
    }

    fn command(&self) -> std::process::Command {
        self.gpg().command()
    }

    fn with_pass(&self, c: &mut std::process::Command) -> Option<Vec<u8>> {
        Gpg::with_pass(c, self.passphrase.as_deref())
    }

    fn run(c: std::process::Command, stdin: Vec<u8>, what: &str) -> Result<Vec<u8>, AppError> {
        Gpg::run(c, stdin, what)
    }

    /// `gpg --version` first line (diagnostics).
    pub fn version(&self) -> Result<String, AppError> {
        self.gpg().version()
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
        let mut stdin = self.with_pass(&mut c).ok_or_else(|| AppError::validation("GPG symmetric encryption needs a passphrase (or set recipients)"))?;
        stdin.extend_from_slice(plain);
        c.args(["--symmetric", "--cipher-algo", "AES256", "--s2k-digest-algo", "SHA512", "--s2k-count", "65011712", "--output", "-"]);
        Self::run(c, stdin, "encryption")
    }

    fn open(&self, sealed: &[u8]) -> Result<Vec<u8>, AppError> {
        let mut c = self.command();
        let mut stdin = self.with_pass(&mut c).unwrap_or_default();
        stdin.extend_from_slice(sealed);
        c.args(["--decrypt", "--output", "-"]);
        Self::run(c, stdin, "decryption")
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
