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
    /// Machine-wide caps (all jobs together).
    pub caps: TransferCaps,
}

/// Machine-wide transfer caps; the console's managed settings can override each.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TransferCaps {
    /// Bytes per second for all uploads together (0 = unlimited).
    pub upload_bps: u64,
    pub download_bps: u64,
    /// Operations that may run at the same time (0 = 1).
    pub max_concurrent_runs: u32,
}

impl TransferCaps {
    pub fn concurrency(&self) -> usize {
        self.max_concurrent_runs.clamp(1, ufop_core::agent::MAX_CONCURRENT_RUNS) as usize
    }

    /// Each field the console manages replaces the machine's own.
    pub fn with_managed(mut self, m: Option<&ufop_core::agent::ManagedTransfers>) -> Self {
        if let Some(m) = m {
            if let Some(k) = m.max_upload_kbps {
                self.upload_bps = k * 1024;
            }
            if let Some(k) = m.max_download_kbps {
                self.download_bps = k * 1024;
            }
            if let Some(n) = m.max_concurrent_runs {
                self.max_concurrent_runs = n;
            }
        }
        self
    }
}

impl GlobalDefaults {
    /// Reject unknown option names and values the engine cannot parse.
    pub fn validate(&self) -> Result<(), AppError> {
        super::templates::validate_options(&self.options)?;
        if self.caps.max_concurrent_runs > ufop_core::agent::MAX_CONCURRENT_RUNS {
            return Err(AppError::validation(format!("At most {} operations can run at the same time", ufop_core::agent::MAX_CONCURRENT_RUNS)));
        }
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
