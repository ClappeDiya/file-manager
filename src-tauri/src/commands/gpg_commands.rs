//! OpenPGP file actions (encrypt, decrypt, sign, verify) and the keyring,
//! through the user's GnuPG (`ufop_core::gpg`). Outputs are written next
//! to their inputs under a free name; passphrases go to gpg on stdin.

use crate::core::error::AppError;
use serde::Serialize;
use std::path::{Path, PathBuf};
use ufop_core::gpg::{opened_name, sealed_name, unique_path, EncryptOptions, Gpg, GpgKey, SignatureCheck};

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
    op: impl Fn(&Path, &Path) -> Result<Option<SignatureCheck>, AppError>,
) -> Vec<GpgOutcome> {
    inputs
        .into_iter()
        .map(|input| {
            let src = PathBuf::from(&input);
            let out = unique_path(&name(&src));
            let res = if !src.is_file() {
                Err(AppError::validation(format!("Not a file: {input}")))
            } else {
                super::policy_commands::enforce_path(&out).and_then(|_| op(&src, &out))
            };
            match res {
                Ok(signature) => GpgOutcome { input, output: Some(out.to_string_lossy().into()), signature, error: None },
                Err(e) => GpgOutcome { input, output: None, signature: None, error: Some(e.user_message()) },
            }
        })
        .collect()
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
        Ok(each(paths, |p| sealed_name(p, armor, false), |src, out| g.encrypt_file(src, out, &options, passphrase.as_deref()).map(|_| None)))
    })
    .await
}

#[tauri::command]
pub async fn gpg_decrypt_files(paths: Vec<String>, passphrase: Option<String>) -> Result<Vec<GpgOutcome>, AppError> {
    blocking(move |g| Ok(each(paths, opened_name, |src, out| g.decrypt_file(src, out, passphrase.as_deref()))))
        .await
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
            |p| sealed_name(p, armor, detached),
            |src, out| g.sign_file(src, out, key.as_deref(), detached, armor, passphrase.as_deref()).map(|_| None),
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
            |p| sealed_name(p, false, false),
            |_, out| {
                std::fs::write(out, "sealed").unwrap();
                Ok(None)
            },
        );
        assert_eq!(out[0].output.as_deref(), Some(s(&d.path().join("a.txt (2).gpg")).as_str()), "existing output kept");
        assert_eq!(std::fs::read_to_string(d.path().join("a.txt.gpg")).unwrap(), "existing");
        assert!(out[1].error.as_deref().unwrap().contains("Not a file"));
    }
}
