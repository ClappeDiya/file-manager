//! Settings shared across jobs: global option defaults (applied to every
//! option a job leaves at the built-in value) and the destination library
//! (named destinations that several jobs can reference).

use super::{BackupDestination, BackupJob};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use ufop_core::backup::backend::{RetryPolicy, Throttle};
use ufop_core::backup::BackupOptions;

/// Config-table keys.
pub const DEFAULTS_KEY: &str = "backup.global_defaults";
pub const LIBRARY_KEY: &str = "backup.destination_library";

/// Defaults for all jobs. `options` holds only the `BackupOptions` fields
/// that were set (snake_case keys, engine JSON values).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalDefaults {
    pub options: serde_json::Map<String, serde_json::Value>,
    pub throttle: Option<Throttle>,
    pub retry: Option<RetryPolicy>,
}

impl GlobalDefaults {
    /// Reject unknown option names and values the engine cannot parse.
    pub fn validate(&self) -> Result<(), AppError> {
        let base = serde_json::to_value(BackupOptions::default())?;
        let known = base.as_object().map(|o| o.keys().cloned().collect::<Vec<_>>()).unwrap_or_default();
        for k in self.options.keys() {
            if !known.contains(k) {
                return Err(AppError::validation(format!("Unknown backup option '{k}'")));
            }
            if matches!(k.as_str(), "prefix" | "block_size" | "block_hash" | "file_hash" | "dry_run" | "task_config" | "changed_paths" | "deleted_paths") {
                return Err(AppError::validation(format!("'{k}' is per job and cannot have a global default")));
            }
        }
        let mut merged = base;
        if let Some(o) = merged.as_object_mut() {
            o.extend(self.options.clone());
        }
        serde_json::from_value::<BackupOptions>(merged).map_err(|e| AppError::validation(format!("Invalid default value: {e}")))?;
        Ok(())
    }

    /// Effective job: every option the job leaves at the built-in default
    /// takes the global default instead.
    pub fn apply(&self, job: &mut BackupJob) -> Result<(), AppError> {
        if !self.options.is_empty() {
            let builtin = serde_json::to_value(BackupOptions::default())?;
            let mut cur = serde_json::to_value(&job.options)?;
            if let (Some(c), Some(b)) = (cur.as_object_mut(), builtin.as_object()) {
                for (k, v) in &self.options {
                    if c.get(k) == b.get(k) {
                        c.insert(k.clone(), v.clone());
                    }
                }
            }
            job.options = serde_json::from_value(cur)?;
        }
        if let Some(t) = self.throttle {
            if job.throttle == Throttle::default() {
                job.throttle = t;
            }
        }
        if let Some(r) = self.retry {
            if job.retry == BackupJob::default().retry {
                job.retry = r;
            }
        }
        Ok(())
    }
}

/// A named destination in the library.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SavedDestination {
    pub id: String,
    pub name: String,
    pub destination: BackupDestination,
    #[serde(default)]
    pub created_at: String,
}

impl SavedDestination {
    pub fn validate(&self) -> Result<(), AppError> {
        if self.name.trim().is_empty() {
            return Err(AppError::validation("Give the destination a name"));
        }
        match &self.destination {
            BackupDestination::Library { .. } => Err(AppError::validation("A library destination cannot point at another library entry")),
            BackupDestination::Local { path, .. } if path.trim().is_empty() => Err(AppError::validation("Choose a destination folder")),
            _ => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_fill_only_untouched_options() {
        let mut d = GlobalDefaults::default();
        d.options.insert("upload_concurrency".into(), 6.into());
        d.options.insert("compression_level".into(), 9.into());
        d.throttle = Some(Throttle { upload_bps: 1000, download_bps: 0 });
        d.validate().unwrap();
        let mut j = BackupJob::default();
        j.options.compression_level = 1; // set by the job → kept
        d.apply(&mut j).unwrap();
        assert_eq!(j.options.upload_concurrency, 6);
        assert_eq!(j.options.compression_level, 1);
        assert_eq!(j.throttle.upload_bps, 1000);

        let mut bad = GlobalDefaults::default();
        bad.options.insert("nope".into(), 1.into());
        assert!(bad.validate().is_err());
        let mut bad = GlobalDefaults::default();
        bad.options.insert("block_size".into(), 1.into());
        assert!(bad.validate().is_err());
        let mut bad = GlobalDefaults::default();
        bad.options.insert("usn_policy".into(), "sometimes".into());
        assert!(bad.validate().is_err());
    }

    #[test]
    fn library_entries_validate() {
        let ok = SavedDestination { id: "x".into(), name: "NAS".into(), destination: BackupDestination::Local { path: "/mnt/nas".into(), alternate_paths: vec![], marker_file: None }, created_at: String::new() };
        ok.validate().unwrap();
        let nested = SavedDestination { destination: BackupDestination::Library { destination_id: "x".into(), remote_path: String::new() }, ..ok.clone() };
        assert!(nested.validate().is_err());
    }
}
