//! Job configuration portability: JSON export/import (optionally with the
//! passphrase, optionally encrypted with a separate export passphrase),
//! the secret-stripped form embedded in remote file lists, and a
//! command-line representation for scripting.

use super::{BackupDestination, BackupJob, JobStats};
use crate::core::error::AppError;
use base64::Engine;
use serde::{Deserialize, Serialize};
use ufop_core::backup::crypto::{KdfParams, VolumeCipher};
use ufop_core::backup::RetentionPolicy;

const FORMAT: &str = "ufop-backup-config";

#[derive(Debug, Serialize, Deserialize)]
struct ExportFile {
    format: String,
    version: u32,
    encrypted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    salt: Option<String>,
    /// Plain export: the payload object. Encrypted: base64 of the sealed payload.
    payload: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
struct Payload {
    job: BackupJob,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    passphrase: Option<String>,
}

/// Job JSON safe to store remotely: no stats, no report callback URLs
/// (which often embed tokens). Passphrases never live in the job itself.
pub fn stripped_config(job: &BackupJob) -> Result<serde_json::Value, AppError> {
    let mut j = job.clone();
    j.stats = JobStats::default();
    j.reports.http_urls.clear();
    Ok(serde_json::to_value(j)?)
}

/// Export a job. `passphrase` is included only when given;
/// `export_passphrase` encrypts the whole file.
pub fn export_job(job: &BackupJob, passphrase: Option<String>, export_passphrase: Option<&str>, include_stats: bool) -> Result<String, AppError> {
    let mut j = job.clone();
    if !include_stats {
        j.stats = JobStats::default();
    }
    let payload = Payload { job: j, passphrase };
    let file = match export_passphrase.filter(|p| !p.is_empty()) {
        None => ExportFile { format: FORMAT.into(), version: 1, encrypted: false, salt: None, payload: serde_json::to_value(&payload)? },
        Some(p) => {
            let salt = VolumeCipher::random_salt();
            let c = VolumeCipher::derive(p, &salt, KdfParams::default())?;
            let sealed = c.seal(&serde_json::to_vec(&payload)?)?;
            ExportFile {
                format: FORMAT.into(),
                version: 1,
                encrypted: true,
                salt: Some(hex::encode(salt)),
                payload: base64::engine::general_purpose::STANDARD.encode(sealed).into(),
            }
        }
    };
    Ok(serde_json::to_string_pretty(&file)?)
}

/// Whether an export file needs a passphrase to import.
pub fn is_encrypted_export(json: &str) -> bool {
    serde_json::from_str::<ExportFile>(json).map(|f| f.encrypted).unwrap_or(false)
}

/// Parse an export (or a bare job JSON, e.g. from remote discovery). The
/// returned job has a fresh id; statistics are kept only on request.
pub fn import_job(json: &str, export_passphrase: Option<&str>, import_stats: bool) -> Result<(BackupJob, Option<String>), AppError> {
    let payload: Payload = match serde_json::from_str::<ExportFile>(json) {
        Ok(f) if f.format == FORMAT => {
            if f.encrypted {
                let p = export_passphrase.filter(|p| !p.is_empty()).ok_or_else(|| AppError::validation("This export is encrypted; enter its passphrase"))?;
                let salt = hex::decode(f.salt.unwrap_or_default()).map_err(|_| AppError::validation("Export salt is malformed"))?;
                let sealed = base64::engine::general_purpose::STANDARD
                    .decode(f.payload.as_str().unwrap_or_default())
                    .map_err(|_| AppError::validation("Export payload is malformed"))?;
                let c = VolumeCipher::derive(p, &salt, KdfParams::default())?;
                serde_json::from_slice(&c.open(&sealed).map_err(|_| AppError::encryption("Wrong export passphrase", "Check the passphrase used when exporting."))?)?
            } else {
                serde_json::from_value(f.payload)?
            }
        }
        _ => Payload { job: serde_json::from_str(json).map_err(|e| AppError::validation(format!("Not a backup configuration: {e}")))?, passphrase: None },
    };
    let mut job = payload.job;
    job.id = String::new();
    if !import_stats {
        job.stats = JobStats::default();
    }
    Ok((job, payload.passphrase))
}

fn quote(s: &str) -> String {
    shlex::try_quote(s).map(|c| c.into_owned()).unwrap_or_else(|_| format!("'{s}'"))
}

/// `ufop backup run …` equivalent of a job (local destinations only carry
/// a path; connection destinations name the saved connection id).
pub fn cli_command(job: &BackupJob) -> String {
    let mut parts = vec!["ufop".to_string(), "backup".into(), "run".into()];
    for s in &job.sources {
        parts.push("--source".into());
        parts.push(quote(s));
    }
    match &job.destination {
        BackupDestination::Local { path, .. } => {
            parts.push("--dest".into());
            parts.push(quote(path));
        }
        BackupDestination::Connection { connection_id, remote_path } => {
            parts.push("--dest".into());
            parts.push(quote(&format!("connection://{connection_id}{remote_path}")));
        }
    }
    parts.push("--dbpath".into());
    parts.push(quote(&super::default_data_dir().join(format!("{}.sqlite", job.id)).to_string_lossy()));
    if job.encrypted {
        parts.push("--passphrase-env".into());
        parts.push("UFOP_PASSPHRASE".into());
    } else {
        parts.push("--no-encryption".into());
    }
    let o = &job.options;
    parts.push(format!("--prefix={}", o.prefix));
    parts.push(format!("--block-size={}", o.block_size));
    parts.push(format!("--volume-size={}", o.volume_size));
    parts.push(format!("--compression-level={}", o.compression_level));
    match &job.retention {
        RetentionPolicy::KeepAll => {}
        RetentionPolicy::KeepDays { days } => parts.push(format!("--keep-days={days}")),
        RetentionPolicy::KeepCount { count } => parts.push(format!("--keep-versions={count}")),
        RetentionPolicy::Smart => parts.push("--retention-policy=smart".into()),
        RetentionPolicy::Custom { rules } => parts.push(format!("--retention-policy={}", quote(rules))),
    }
    for r in &job.filter.rules {
        let flag = match r.action {
            ufop_core::backup::FilterAction::Include => "--include",
            ufop_core::backup::FilterAction::Exclude => "--exclude",
        };
        let pat = match r.kind {
            ufop_core::backup::FilterKind::Regex => format!("[{}]", r.pattern),
            ufop_core::backup::FilterKind::Group => format!("{{{}}}", r.pattern),
            _ => r.pattern.clone(),
        };
        parts.push(format!("{flag}={}", quote(&pat)));
    }
    if let Some(m) = job.filter.max_file_size {
        parts.push(format!("--skip-files-larger-than={m}"));
    }
    if job.filter.exclude_hidden {
        parts.push("--exclude-hidden".into());
    }
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn job() -> BackupJob {
        BackupJob {
            id: "abc".into(),
            name: "Docs".into(),
            sources: vec!["/home/me/Docs".into()],
            destination: BackupDestination::Local { path: "/mnt/backup".into(), alternate_paths: vec![], marker_file: None },
            stats: JobStats { version_count: 4, ..Default::default() },
            ..Default::default()
        }
    }

    #[test]
    fn plain_and_encrypted_roundtrip() {
        let j = job();
        let plain = export_job(&j, None, None, false).unwrap();
        assert!(!plain.contains("\"version_count\": 4"));
        let (imp, pass) = import_job(&plain, None, false).unwrap();
        assert_eq!(imp.name, "Docs");
        assert!(imp.id.is_empty() && pass.is_none());

        let enc = export_job(&j, Some("secret".into()), Some("exportpw"), true).unwrap();
        assert!(is_encrypted_export(&enc));
        assert!(!enc.contains("secret") && !enc.contains("Docs"));
        assert!(import_job(&enc, None, false).is_err());
        assert!(import_job(&enc, Some("wrong"), false).is_err());
        let (imp, pass) = import_job(&enc, Some("exportpw"), true).unwrap();
        assert_eq!(pass.as_deref(), Some("secret"));
        assert_eq!(imp.stats.version_count, 4);
    }

    #[test]
    fn bare_job_and_stripping() {
        let mut j = job();
        j.reports.http_urls = vec!["https://x/?token=1".into()];
        let v = stripped_config(&j).unwrap();
        assert!(v["reports"]["http_urls"].as_array().unwrap().is_empty());
        let (imp, _) = import_job(&v.to_string(), None, false).unwrap();
        assert_eq!(imp.sources, j.sources);
        assert!(import_job("{\"nope\": 1", None, false).is_err());
    }

    #[test]
    fn cli_export() {
        let mut j = job();
        j.retention = RetentionPolicy::KeepCount { count: 3 };
        let c = cli_command(&j);
        assert!(c.starts_with("ufop backup run --source /home/me/Docs --dest /mnt/backup"));
        assert!(c.contains("--keep-versions=3") && c.contains("--passphrase-env UFOP_PASSPHRASE"));
    }
}
