//! eDiscovery export: one zip with what an investigation needs about a
//! matter, each part hashed in a manifest.
//!
//! - `audit/audit-log.jsonl` — the complete tamper-evident audit log (as
//!   exported for external verification) and `audit/verification.json`;
//! - `ledger/events.jsonl` — file operations in the time range that touched
//!   the given paths (or all paths);
//! - `backups/<job>/` — per backup job: its configuration without secrets,
//!   its legal hold, its versions and, per version, the backed-up files
//!   under the given paths (path, size, modified time, hash);
//! - `files/<job>/…` — optionally, those files' content from the newest
//!   version, restored (up to a size limit);
//! - `manifest.json` — the request, the result, and SHA-256 and size of
//!   every other entry; `README.txt` explains how to check it.
//!
//! Jobs default to every job under legal hold.

use crate::backup_engine::{portability, BackupJob, BackupManager, BackupOperation};
use crate::core::error::AppError;
use crate::ledger::OperationLedger;
use crate::security::audit::AuditTrail;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};

pub const FORMAT: &str = "ufop-ediscovery/1";
const MAX_LEDGER_EVENTS: u32 = 200_000;
const DEFAULT_MAX_FILE_BYTES: u64 = 2 * 1024 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EDiscoveryRequest {
    /// Case or matter reference (recorded in the manifest).
    pub matter: String,
    /// Where to write the `.zip`.
    pub output: String,
    /// Paths of interest (files or folders); empty = everything.
    pub paths: Vec<String>,
    /// RFC 3339 bounds for ledger events.
    pub since: Option<String>,
    pub until: Option<String>,
    /// Backup jobs to include; empty = every job under legal hold.
    pub job_ids: Vec<String>,
    /// Restore the matching files of each job's newest version into the bundle.
    pub include_files: bool,
    /// Largest total size of restored files (default 2 GB).
    pub max_file_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManifestEntry {
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EDiscoveryReport {
    pub output: String,
    /// SHA-256 of the whole zip.
    pub sha256: String,
    pub bytes: u64,
    pub entries: usize,
    pub ledger_events: usize,
    pub jobs: Vec<String>,
    pub restored_files: usize,
    /// Restored files stopped at the size limit.
    pub files_truncated: bool,
    pub audit_ok: bool,
}

/// A zip being written, with every entry hashed for the manifest.
struct Bundle {
    zip: zip::ZipWriter<std::fs::File>,
    entries: Vec<ManifestEntry>,
}

impl Bundle {
    fn add(&mut self, name: &str, data: &[u8]) -> Result<(), AppError> {
        self.zip.start_file(name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).map_err(zip_err)?;
        self.zip.write_all(data)?;
        self.entries.push(ManifestEntry { path: name.into(), bytes: data.len() as u64, sha256: hex::encode(Sha256::digest(data)) });
        Ok(())
    }

    fn add_file(&mut self, name: &str, path: &Path) -> Result<u64, AppError> {
        self.zip.start_file(name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated).large_file(true)).map_err(zip_err)?;
        let mut f = std::fs::File::open(path)?;
        let mut h = Sha256::new();
        let mut buf = vec![0u8; 1024 * 1024];
        let mut n = 0u64;
        loop {
            let k = std::io::Read::read(&mut f, &mut buf)?;
            if k == 0 {
                break;
            }
            h.update(&buf[..k]);
            self.zip.write_all(&buf[..k])?;
            n += k as u64;
        }
        self.entries.push(ManifestEntry { path: name.into(), bytes: n, sha256: hex::encode(h.finalize()) });
        Ok(n)
    }

    fn jsonl<T: Serialize>(&mut self, name: &str, items: &[T]) -> Result<(), AppError> {
        let mut s = String::new();
        for i in items {
            s.push_str(&serde_json::to_string(i)?);
            s.push('\n');
        }
        self.add(name, s.as_bytes())
    }
}

fn zip_err(e: zip::result::ZipError) -> AppError {
    AppError::file_op(format!("Writing the export failed: {e}"), "Check the output folder and free space.")
}

/// `name` made safe as one path component in the bundle.
fn slug(name: &str) -> String {
    let s: String = name.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect();
    s.trim_matches('-').chars().take(60).collect::<String>()
}

fn under(path: &str, roots: &[String]) -> bool {
    roots.is_empty()
        || roots.iter().any(|r| {
            let r = r.trim_end_matches(['/', '\\']);
            path == r || path.strip_prefix(r).is_some_and(|rest| rest.starts_with(['/', '\\']))
        })
}

/// Build the bundle at `req.output`.
pub async fn export(req: &EDiscoveryRequest, mgr: &BackupManager, ledger: &OperationLedger, audit: &AuditTrail, by: &str) -> Result<EDiscoveryReport, AppError> {
    let matter = req.matter.trim();
    if matter.is_empty() {
        return Err(AppError::validation("Give the matter or case reference"));
    }
    let output = PathBuf::from(req.output.trim());
    if output.as_os_str().is_empty() || output.extension().map_or(true, |e| !e.eq_ignore_ascii_case("zip")) {
        return Err(AppError::validation("Choose where to save the export (a .zip file)"));
    }
    if output.exists() {
        return Err(AppError::validation(format!("{} already exists", output.display())));
    }
    for (label, v) in [("since", &req.since), ("until", &req.until)] {
        if let Some(t) = v {
            chrono::DateTime::parse_from_rfc3339(t).map_err(|_| AppError::validation(format!("'{label}' must be an RFC 3339 time, e.g. 2026-01-31T00:00:00Z")))?;
        }
    }
    // Jobs: the requested ones, else every job under legal hold.
    let all = mgr.list_jobs().await;
    let jobs: Vec<BackupJob> = if req.job_ids.is_empty() {
        all.into_iter().filter(|j| j.legal_hold.is_some()).collect()
    } else {
        let mut v = Vec::new();
        for id in &req.job_ids {
            v.push(all.iter().find(|j| &j.id == id).cloned().ok_or_else(|| AppError::not_found(format!("Backup job {id} not found")))?);
        }
        v
    };

    let part = output.with_extension("zip.part");
    let file = std::fs::File::create(&part)?;
    let mut b = Bundle { zip: zip::ZipWriter::new(file), entries: Vec::new() };
    let result = async {
        // Audit log: complete (a filtered chain could not be verified) + its verification.
        let verification = audit.verify().await?;
        b.add("audit/audit-log.jsonl", audit.export_jsonl().await?.as_bytes())?;
        b.add("audit/verification.json", serde_json::to_string_pretty(&verification)?.as_bytes())?;

        // Ledger events touching the paths.
        let events = ledger.export_range(req.since.clone(), req.until.clone(), req.paths.clone(), MAX_LEDGER_EVENTS).await?;
        b.jsonl("ledger/events.jsonl", &events)?;

        // Backups.
        let budget = req.max_file_bytes.unwrap_or(DEFAULT_MAX_FILE_BYTES);
        let (mut restored, mut restored_bytes, mut truncated) = (0usize, 0u64, false);
        let mut job_names = Vec::new();
        for job in &jobs {
            let dir = format!("backups/{}-{}", slug(&job.name), &job.id[..8.min(job.id.len())]);
            job_names.push(job.name.clone());
            b.add(&format!("{dir}/job.json"), serde_json::to_string_pretty(&portability::stripped_config(job)?)?.as_bytes())?;
            b.add(&format!("{dir}/legal-hold.json"), serde_json::to_string_pretty(&job.legal_hold)?.as_bytes())?;
            let versions = mgr.list_versions(&job.id).await.unwrap_or_default();
            b.add(&format!("{dir}/versions.json"), serde_json::to_string_pretty(&versions)?.as_bytes())?;
            for v in 0..versions.len() {
                let files: Vec<_> = mgr.list_files(&job.id, v, None).await.unwrap_or_default().into_iter().filter(|f| under(&f.path, &req.paths)).collect();
                b.jsonl(&format!("{dir}/files-v{v}.jsonl"), &files)?;
            }
            if req.include_files && !versions.is_empty() && !truncated {
                let tmp = ufop_core::session::user_temp_subdir("ediscovery").join(uuid::Uuid::new_v4().simple().to_string());
                std::fs::create_dir_all(&tmp)?;
                let options = ufop_core::backup::RestoreOptions { version: 0, paths: req.paths.clone(), target: Some(tmp.clone()), preserve_full_paths: true, ..Default::default() };
                let run = mgr.execute(&job.id, BackupOperation::Restore { options, overrides: Default::default() }).await;
                if let Err(e) = run {
                    let _ = std::fs::remove_dir_all(&tmp);
                    return Err(e);
                }
                let mut stack = vec![tmp.clone()];
                let mut found = Vec::new();
                while let Some(p) = stack.pop() {
                    for e in std::fs::read_dir(&p)?.filter_map(Result::ok) {
                        let ft = e.file_type()?;
                        if ft.is_dir() {
                            stack.push(e.path());
                        } else if ft.is_file() {
                            found.push(e.path());
                        }
                    }
                }
                found.sort();
                for f in found {
                    let len = std::fs::metadata(&f)?.len();
                    if restored_bytes + len > budget {
                        truncated = true;
                        break;
                    }
                    let rel = f.strip_prefix(&tmp).unwrap_or(&f).to_string_lossy().replace('\\', "/");
                    restored_bytes += b.add_file(&format!("files/{}-{}/{rel}", slug(&job.name), &job.id[..8.min(job.id.len())]), &f)?;
                    restored += 1;
                }
                let _ = std::fs::remove_dir_all(&tmp);
            }
        }

        b.add(
            "README.txt",
            format!(
                "eDiscovery export ({FORMAT}) for matter \"{matter}\".\n\nmanifest.json lists every other file with its SHA-256 and size; check them with any SHA-256 tool.\naudit/audit-log.jsonl is the complete tamper-evident audit log (hash chain with signed anchors); audit/verification.json is its check at export time.\nledger/events.jsonl lists file operations; backups/ describes each backup job, its legal hold, versions and backed-up files; files/ holds restored file contents when requested.\n"
            )
            .as_bytes(),
        )?;
        let manifest = serde_json::json!({
            "format": FORMAT,
            "matter": matter,
            "generated_at": Utc::now().to_rfc3339(),
            "generated_by": by,
            "request": { "paths": req.paths, "since": req.since, "until": req.until, "job_ids": jobs.iter().map(|j| &j.id).collect::<Vec<_>>(), "include_files": req.include_files, "max_file_bytes": budget },
            "result": { "ledger_events": events.len(), "ledger_truncated": events.len() as u32 >= MAX_LEDGER_EVENTS, "restored_files": restored, "restored_bytes": restored_bytes, "files_truncated": truncated },
            "audit": verification,
            "entries": b.entries,
        });
        let manifest = serde_json::to_vec_pretty(&manifest)?;
        b.zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).map_err(zip_err)?;
        b.zip.write_all(&manifest)?;
        Ok::<_, AppError>((events.len(), job_names, restored, truncated, verification.ok))
    }
    .await;
    let (ledger_events, job_names, restored_files, files_truncated, audit_ok) = match result {
        Ok(r) => r,
        Err(e) => {
            drop(b);
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    };
    let entries = b.entries.len() + 1;
    b.zip.finish().map_err(zip_err)?.sync_all()?;
    std::fs::rename(&part, &output)?;
    let data = std::fs::read(&output)?;
    Ok(EDiscoveryReport {
        output: output.to_string_lossy().into_owned(),
        sha256: hex::encode(Sha256::digest(&data)),
        bytes: data.len() as u64,
        entries,
        ledger_events,
        jobs: job_names,
        restored_files,
        files_truncated,
        audit_ok,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn path_scope_and_slugs() {
        let roots = vec!["/data/legal/".to_string()];
        assert!(under("/data/legal", &roots) && under("/data/legal/a.pdf", &roots));
        assert!(!under("/data/legal-old/a.pdf", &roots) && !under("/data", &roots));
        assert!(under("/anything", &[]));
        assert_eq!(slug("Q3 / Finance: \"x\""), "Q3---Finance---x");
    }
}
