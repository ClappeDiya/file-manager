//! Legal hold on a backup job: while held, nothing in the backup set can be
//! removed —
//!
//! - retention is suspended (every backup keeps all versions);
//! - deleting versions and purging files (also broken ones) is refused,
//!   except as a dry run;
//! - the job itself cannot be deleted.
//!
//! Placing and releasing a hold is explicit (never through the job editor,
//! which keeps the stored hold) and is recorded with the matter and reason.

use super::{BackupJob, BackupManager, BackupOperation};
use crate::core::error::AppError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use ufop_core::backup::RetentionPolicy;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct LegalHold {
    /// Case or matter reference.
    pub matter: String,
    pub reason: String,
    pub placed_by: String,
    pub placed_at: String,
}

fn held(job: &BackupJob) -> AppError {
    let h = job.legal_hold.as_ref().map(|h| h.matter.clone()).unwrap_or_default();
    AppError::PermissionDenied {
        message: format!("\"{}\" is under legal hold ({h}): nothing in it can be removed", job.name),
        advice: "Release the hold first (it is recorded in the audit log).".into(),
    }
}

/// Refuse operations that would remove data from a held backup set.
pub fn guard(job: &BackupJob, op: &BackupOperation) -> Result<(), AppError> {
    if job.legal_hold.is_none() {
        return Ok(());
    }
    match op {
        BackupOperation::DeleteVersions { .. } => Err(held(job)),
        BackupOperation::Purge { dry_run: false, .. } | BackupOperation::PurgeBroken { dry_run: false } => Err(held(job)),
        _ => Ok(()),
    }
}

/// The retention a backup run applies: everything is kept while held.
pub fn effective_retention(job: &BackupJob) -> RetentionPolicy {
    if job.legal_hold.is_some() {
        RetentionPolicy::KeepAll
    } else {
        job.retention.clone()
    }
}

/// Deleting a held job is refused.
pub fn guard_delete(job: &BackupJob) -> Result<(), AppError> {
    if job.legal_hold.is_some() {
        Err(held(job))
    } else {
        Ok(())
    }
}

impl BackupManager {
    /// Place a legal hold on a job (replacing an existing one's matter and reason).
    pub async fn place_legal_hold(&self, job_id: &str, matter: &str, reason: &str, by: &str) -> Result<BackupJob, AppError> {
        let matter = matter.trim();
        let reason = reason.trim();
        if matter.is_empty() || matter.chars().count() > 200 {
            return Err(AppError::validation("Give the matter or case reference (up to 200 characters)"));
        }
        if reason.chars().count() > 2000 {
            return Err(AppError::validation("The reason is limited to 2000 characters"));
        }
        let mut job = self.get_job(job_id).await?;
        job.legal_hold = Some(LegalHold { matter: matter.into(), reason: reason.into(), placed_by: by.into(), placed_at: Utc::now().to_rfc3339() });
        job.updated_at = Utc::now().to_rfc3339();
        self.persist(&job).await?;
        self.inner.jobs.write().await.insert(job.id.clone(), job.clone());
        Ok(job)
    }

    /// Release a job's legal hold; returns the hold that was released.
    pub async fn release_legal_hold(&self, job_id: &str) -> Result<(BackupJob, LegalHold), AppError> {
        let mut job = self.get_job(job_id).await?;
        let hold = job.legal_hold.take().ok_or_else(|| AppError::validation(format!("\"{}\" is not under legal hold", job.name)))?;
        job.updated_at = Utc::now().to_rfc3339();
        self.persist(&job).await?;
        self.inner.jobs.write().await.insert(job.id.clone(), job.clone());
        Ok((job, hold))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guards_removal_but_not_other_work() {
        let mut job = BackupJob { name: "Finance".into(), retention: RetentionPolicy::KeepCount { count: 3 }, ..Default::default() };
        assert!(guard(&job, &BackupOperation::DeleteVersions { versions: vec![1], allow_full_removal: false }).is_ok());
        assert_eq!(effective_retention(&job), RetentionPolicy::KeepCount { count: 3 });
        job.legal_hold = Some(LegalHold { matter: "Case 2026-17".into(), ..Default::default() });
        let e = guard(&job, &BackupOperation::DeleteVersions { versions: vec![1], allow_full_removal: false }).unwrap_err();
        assert!(e.to_string().contains("Case 2026-17"));
        assert!(guard(&job, &BackupOperation::Purge { paths: vec!["/x".into()], versions: None, dry_run: false }).is_err());
        assert!(guard(&job, &BackupOperation::Purge { paths: vec!["/x".into()], versions: None, dry_run: true }).is_ok());
        assert!(guard(&job, &BackupOperation::PurgeBroken { dry_run: false }).is_err());
        for ok in [BackupOperation::Backup { dry_run: false }, BackupOperation::Compact, BackupOperation::Verify { full: true }, BackupOperation::ListBroken] {
            assert!(guard(&job, &ok).is_ok());
        }
        assert_eq!(effective_retention(&job), RetentionPolicy::KeepAll);
        assert!(guard_delete(&job).is_err());
    }
}
