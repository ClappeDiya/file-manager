//! OpenPGP file actions (encrypt, decrypt, re-encrypt, sign, verify) and the
//! keyring (import, export, generate), through the user's GnuPG
//! (`ufop_core::gpg`). Outputs are written next to their inputs under a
//! free name (re-encryption replaces the file); passphrases go to gpg on
//! stdin.

use crate::core::error::AppError;
use serde::Serialize;
use std::path::{Path, PathBuf};
use ufop_core::gpg::{opened_name, sealed_name, unique_path, Decryption, EncryptOptions, Gpg, GpgKey, NewKey, SignatureCheck, NO_INTEGRITY};

#[derive(Debug, Clone, Serialize)]
pub struct GpgStatus {
    pub available: bool,
    pub version: Option<String>,
    pub error: Option<String>,
}

/// Result for one input file.
#[derive(Debug, Clone, Serialize)]
pub struct GpgOutcome {
    pub input: String,
    pub output: Option<String>,
    pub signature: Option<SignatureCheck>,
    pub error: Option<String>,
    /// Decrypted without integrity protection (a legacy file, allowed explicitly).
    pub unprotected: bool,
    /// Refused because it has no integrity protection; it can be decrypted anyway on request.
    pub needs_unprotected: bool,
}

fn outcome(input: String, output: &Path, res: Result<Decryption, AppError>) -> GpgOutcome {
    match res {
        Ok(d) => GpgOutcome { input, output: Some(output.to_string_lossy().into()), signature: d.signature, error: None, unprotected: d.unprotected, needs_unprotected: false },
        Err(e) => {
            let error = e.user_message();
            GpgOutcome { input, output: None, signature: None, needs_unprotected: error.contains(NO_INTEGRITY), error: Some(error), unprotected: false }
        }
    }
}

async fn blocking<T: Send + 'static>(f: impl FnOnce(Gpg) -> Result<T, AppError> + Send + 'static) -> Result<T, AppError> {
    tokio::task::spawn_blocking(move || f(Gpg::default()))
        .await
        .map_err(|e| AppError::internal(format!("GPG task failed: {e}")))?
}

/// Run `op` for each input, writing to a free output name from `name`.
fn each(
    inputs: Vec<String>,
    name: impl Fn(&Path) -> PathBuf,
    op: impl Fn(&Path, &Path) -> Result<Decryption, AppError>,
) -> Vec<GpgOutcome> {
    inputs
        .into_iter()
        .map(|input| {
            let src = PathBuf::from(&input);
            let out = name(&src);
            let res = if !src.is_file() {
                Err(AppError::validation(format!("Not a file: {input}")))
            } else {
                super::policy_commands::enforce_path(&out).and_then(|_| op(&src, &out))
            };
            outcome(input, &out, res)
        })
        .collect()
}

fn unsigned(r: Result<(), AppError>) -> Result<Decryption, AppError> {
    r.map(|_| Decryption::default())
}

#[tauri::command]
pub async fn gpg_status() -> Result<GpgStatus, AppError> {
    blocking(|g| {
        Ok(match g.version() {
            Ok(v) => GpgStatus { available: true, version: Some(v), error: None },
            Err(e) => GpgStatus { available: false, version: None, error: Some(e.user_message()) },
        })
    })
    .await
}

#[tauri::command]
pub async fn gpg_list_keys() -> Result<Vec<GpgKey>, AppError> {
    blocking(|g| g.list_keys()).await
}

/// Import keys from a file (armored or binary) or pasted text.
#[tauri::command]
pub async fn gpg_import_keys(path: Option<String>, text: Option<String>) -> Result<u32, AppError> {
    blocking(move |g| {
        let data = match (path, text) {
            (Some(p), _) => std::fs::read(&p).map_err(|e| AppError::file_op(format!("Cannot read {p}: {e}"), "Check the file"))?,
            (None, Some(t)) => t.into_bytes(),
            _ => return Err(AppError::validation("Choose a key file or paste a key")),
        };
        g.import_keys(&data)
    })
    .await
}

#[tauri::command]
pub async fn gpg_export_public(key: String) -> Result<String, AppError> {
    blocking(move |g| g.export_public(&key)).await
}

#[tauri::command]
pub async fn gpg_encrypt_files(paths: Vec<String>, options: EncryptOptions, passphrase: Option<String>) -> Result<Vec<GpgOutcome>, AppError> {
    blocking(move |g| {
        let armor = options.armor;
        Ok(each(paths, |p| unique_path(&sealed_name(p, armor, false)), |src, out| unsigned(g.encrypt_file(src, out, &options, passphrase.as_deref()))))
    })
    .await
}

#[tauri::command]
pub async fn gpg_decrypt_files(paths: Vec<String>, passphrase: Option<String>, allow_unprotected: Option<bool>) -> Result<Vec<GpgOutcome>, AppError> {
    let allow = allow_unprotected.unwrap_or(false);
    blocking(move |g| Ok(each(paths, |p| unique_path(&opened_name(p)), |src, out| g.decrypt_file_with(src, out, passphrase.as_deref(), allow))))
        .await
}

/// Re-encrypt files in place for new recipients or a new passphrase,
/// without writing their plaintext anywhere.
#[tauri::command]
pub async fn gpg_recrypt_files(
    paths: Vec<String>,
    options: EncryptOptions,
    old_passphrase: Option<String>,
    new_passphrase: Option<String>,
    allow_unprotected: Option<bool>,
) -> Result<Vec<GpgOutcome>, AppError> {
    let allow = allow_unprotected.unwrap_or(false);
    blocking(move |g| {
        Ok(each(paths, Path::to_path_buf, |src, out| g.recrypt_file(src, out, &options, old_passphrase.as_deref(), new_passphrase.as_deref(), allow)))
    })
    .await
}

/// Create a key pair; returns its fingerprint.
#[tauri::command]
pub async fn gpg_generate_key(key: NewKey, passphrase: Option<String>) -> Result<String, AppError> {
    blocking(move |g| g.generate_key(&key, passphrase.as_deref())).await
}

#[tauri::command]
pub async fn gpg_sign_files(
    paths: Vec<String>,
    key: Option<String>,
    detached: bool,
    armor: bool,
    passphrase: Option<String>,
) -> Result<Vec<GpgOutcome>, AppError> {
    blocking(move |g| {
        Ok(each(
            paths,
            |p| unique_path(&sealed_name(p, armor, detached)),
            |src, out| unsigned(g.sign_file(src, out, key.as_deref(), detached, armor, passphrase.as_deref())),
        ))
    })
    .await
}

/// The file a detached signature (`x.sig` / `x.asc`) signs, if present.
pub fn signed_data_for(sig: &Path) -> Option<PathBuf> {
    let ext = sig.extension()?.to_string_lossy().to_lowercase();
    if ext != "sig" && ext != "asc" {
        return None;
    }
    let data = sig.with_extension("");
    data.is_file().then_some(data)
}

/// Check a signature: a detached `.sig`/`.asc` next to its file, or a
/// signed file itself.
#[tauri::command]
pub async fn gpg_verify(path: String) -> Result<SignatureCheck, AppError> {
    blocking(move |g| {
        let p = PathBuf::from(&path);
        g.verify(&p, signed_data_for(&p).as_deref())
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_signature_pairs_with_its_file() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.tar");
        std::fs::write(&f, "x").unwrap();
        assert_eq!(signed_data_for(&d.path().join("a.tar.sig")), Some(f.clone()));
        assert_eq!(signed_data_for(&d.path().join("a.tar.ASC")), Some(f.clone()), "extension case doesn't matter");
        assert_eq!(signed_data_for(&d.path().join("b.tar.sig")), None);
        assert_eq!(signed_data_for(&f), None);
    }

    #[test]
    fn outcomes_report_per_file_and_never_overwrite() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("a.txt");
        std::fs::write(&f, "x").unwrap();
        std::fs::write(d.path().join("a.txt.gpg"), "existing").unwrap();
        let s = |p: &Path| p.to_string_lossy().to_string();
        let out = each(
            vec![s(&f), s(&d.path().join("missing.txt"))],
            |p| unique_path(&sealed_name(p, false, false)),
            |_, out| {
                std::fs::write(out, "sealed").unwrap();
                Ok(Decryption::default())
            },
        );
        assert_eq!(out[0].output.as_deref(), Some(s(&d.path().join("a.txt (2).gpg")).as_str()), "existing output kept");
        assert_eq!(std::fs::read_to_string(d.path().join("a.txt.gpg")).unwrap(), "existing");
        assert!(out[1].error.as_deref().unwrap().contains("Not a file"));
        assert!(!out[1].needs_unprotected);
        // A legacy-file refusal is flagged so the dialog can offer to decrypt anyway.
        let refused = outcome("x.gpg".into(), Path::new("x"), Err(AppError::encryption(format!("{NO_INTEGRITY} (old)"), "")));
        assert!(refused.needs_unprotected && refused.output.is_none());
        let legacy = outcome("x.gpg".into(), Path::new("x"), Ok(Decryption { signature: None, unprotected: true }));
        assert!(legacy.unprotected && legacy.output.as_deref() == Some("x"));
    }
}
