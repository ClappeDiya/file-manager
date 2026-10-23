//! Versions in the app: notes, tags and pins, retention previews, sources
//! of shared repositories and copying versions to another repository.
//! The engine side lives in `ufop_core::backup::{versions, retention,
//! shared, copy}`.

use super::{blocking, lock_options, maintenance, BackupDestination, BackupJob, BackupManager};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use ufop_core::backup::backend::{RetryPolicy, Throttle};
use ufop_core::backup::copy::{self, CopyOptions};
use ufop_core::backup::db::BackupDb;
use ufop_core::backup::engine::Repo;
use ufop_core::backup::lock::{self as repo_lock, LockKind};
use ufop_core::backup::shared::{self, SourceInfo};
use ufop_core::backup::versions::{self as core_versions, ClientIdentity};
use ufop_core::backup::{BackupOptions, Fileset, OpContext, RetentionPolicy};

/// The job with its identity filled in: the job id and name unless the
/// job overrides them (the host and user name default to this computer's
/// in the engine).
pub fn with_identity(job: &BackupJob) -> BackupJob {
    let mut j = job.clone();
    let id = &mut j.options.identity;
    if id.job_id.trim().is_empty() {
        id.job_id = job.id.clone();
    }
    if id.source_label.trim().is_empty() {
        id.source_label = job.name.clone();
    }
    j
}

/// The identity a job writes versions with.
pub fn job_identity(job: &BackupJob) -> ClientIdentity {
    with_identity(job).options.identity.resolve()
}

/// Where [`BackupOperation::CopyVersions`](super::BackupOperation::CopyVersions) copies to.
// One value per copy request; its size does not matter.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CopyTarget {
    /// Another job's repository.
    Job { job_id: String },
    /// A destination without a job (created when empty).
    Destination {
        destination: BackupDestination,
        /// Its passphrase (empty or none: unencrypted).
        #[serde(default)]
        passphrase: Option<String>,
        #[serde(default)]
        prefix: Option<String>,
        /// Settings for a new repository (format, chunker, volume size, …).
        #[serde(default)]
        options: Option<BackupOptions>,
    },
}

/// The destination side of a copy.
struct Target {
    backend: Box<dyn ufop_core::backup::Backend>,
    pass: Option<String>,
    options: BackupOptions,
    /// Job database (path, key, encrypt) or `None` for a job-less destination.
    db: Option<(std::path::PathBuf, Option<ufop_core::backup::db::DbKey>, bool)>,
    job_id: Option<String>,
}

impl BackupManager {
    /// Versions of the job's own source (`source`: another source id, or
    /// `*` for all), optionally only those carrying every tag in `tags`
    /// (`key=value`, or `key` for any value).
    pub async fn list_versions_in(&self, job_id: &str, source: Option<String>, tags: Vec<String>) -> Result<Vec<Fileset>, AppError> {
        let filter = core_versions::parse_tags(&tags)?;
        self.with_db_in(job_id, source, move |db| Ok(core_versions::filter_by_tags(db.filesets()?, &filter))).await
    }

    pub async fn list_files_in(&self, job_id: &str, version: usize, prefix: Option<String>, source: Option<String>) -> Result<Vec<ufop_core::backup::FileEntry>, AppError> {
        self.with_db_in(job_id, source, move |db| {
            let mut files = files_under(db, version, prefix)?;
            files.truncate(20_000);
            Ok(files)
        })
        .await
    }

    /// [`Self::list_files_in`], or one sorted/filtered page of every file
    /// (no 20 000-row cap) when `page` is given.
    pub async fn list_files_listing(
        &self,
        job_id: &str,
        version: usize,
        prefix: Option<String>,
        source: Option<String>,
        page: Option<super::paging::PageRequest>,
    ) -> Result<super::paging::Listing<ufop_core::backup::FileEntry>, AppError> {
        let Some(page) = page else {
            return Ok(super::paging::Listing::All(self.list_files_in(job_id, version, prefix, source).await?));
        };
        self.with_db_in(job_id, source, move |db| Ok(super::paging::Listing::Page(super::paging::paginate(files_under(db, version, prefix)?, &page)?))).await
    }

    pub async fn search_in(&self, job_id: &str, pattern: String, version: Option<usize>, source: Option<String>) -> Result<Vec<maintenance::FoundFile>, AppError> {
        self.with_db_in(job_id, source, move |db| maintenance::find(db, &pattern, version, 2000)).await
    }

    pub async fn compare_in(&self, job_id: &str, a: usize, b: usize, source: Option<String>) -> Result<maintenance::VersionComparison, AppError> {
        self.with_db_in(job_id, source, move |db| maintenance::compare_versions(db, a, b)).await
    }

    /// Sources known to the job's database (own first). Run the
    /// `sync_sources` operation first to see other clients' latest versions.
    pub async fn list_sources(&self, job_id: &str) -> Result<Vec<SourceInfo>, AppError> {
        let job = self.get_job(job_id).await?;
        let own = job_identity(&job).source_id();
        self.with_db_in(job_id, None, move |db| shared::list_sources(db, Some(&own))).await
    }

    /// What retention would keep and delete, without changing anything:
    /// the job's policy or `policy` (the job editor previews unsaved
    /// changes), on the job's own source or `source`. A legal hold keeps
    /// every version.
    pub async fn preview_retention(&self, job_id: &str, policy: Option<RetentionPolicy>, source: Option<String>) -> Result<maintenance::RetentionPreview, AppError> {
        let job = self.get_job(job_id).await?;
        let policy = policy.unwrap_or_else(|| job.retention.clone());
        policy.validate()?;
        let held = job.legal_hold.is_some();
        self.with_db_in(job_id, source, move |db| maintenance::preview_retention(db, &policy, held)).await
    }

    async fn copy_target(&self, job: &BackupJob, target: CopyTarget) -> Result<Target, AppError> {
        match target {
            CopyTarget::Job { job_id } => {
                if job_id == job.id {
                    return Err(AppError::validation("Choose another backup to copy to"));
                }
                let mut dj = self.get_job(&job_id).await?;
                if let Err(e) = self.global_defaults().await.and_then(|d| d.apply(&mut dj)) {
                    tracing::warn!("Global defaults not applied to '{}': {}", dj.name, e.user_message());
                }
                let dj = self.resolve_job_secrets(&dj).await?;
                if dj.read_only {
                    return Err(Self::read_only_refusal(&dj, "receive copied versions"));
                }
                let pass = if dj.encrypted { self.passphrase(&dj.id).await? } else { None };
                if dj.encrypted && pass.is_none() {
                    return Err(AppError::validation(format!("The passphrase of '{}' is missing from the credential store", dj.name)));
                }
                let backend = self.build_backend(&dj).await?;
                let db = self.open_job_db(&dj).await?;
                Ok(Target { backend, pass, options: with_identity(&dj).options, db: Some(db), job_id: Some(dj.id.clone()) })
            }
            CopyTarget::Destination { destination, passphrase, prefix, options } => {
                let backend = self.backend_for(&destination, Throttle::default(), RetryPolicy { retries: 3, ..Default::default() }, None).await?;
                let mut options = options.unwrap_or_default();
                if let Some(p) = prefix.filter(|p| !p.trim().is_empty()) {
                    options.prefix = p;
                }
                Ok(Target { backend, pass: passphrase.filter(|p| !p.is_empty()), options, db: None, job_id: None })
            }
        }
    }

    /// Copy versions of `job` to another repository: shared lock on the
    /// job's repository, exclusive lock on the target.
    pub(super) async fn copy_versions(&self, job: &BackupJob, opts: CopyOptions, target: CopyTarget, ctx: &OpContext) -> Result<serde_json::Value, AppError> {
        let job = with_identity(job);
        let src_backend = self.build_backend(&job).await?;
        let src_pass = if job.encrypted { self.passphrase(&job.id).await? } else { None };
        let (src_path, src_key, src_want) = self.open_job_db(&job).await?;
        let t = self.copy_target(&job, target).await?;
        let kdf = self.inner.kdf;
        let ctx = ctx.clone();
        let target_job = t.job_id.clone();
        let v = blocking(move || {
            let (src_lo, dst_lo) = (lock_options(&job.options), lock_options(&t.options));
            let dst_kind = if opts.dry_run { LockKind::Shared } else { LockKind::Exclusive };
            let (rep, mut warnings) = repo_lock::with_lock(src_backend.as_ref(), &job.options.prefix, LockKind::Shared, "copy_versions", &src_lo, &ctx, || {
                let o = maintenance::JobDbOptions { key: src_key.as_ref(), encrypt: src_want };
                let sdb = maintenance::open_job_db(&src_path, &o, None, None, &job.options.prefix, &ctx)?.0;
                let src = Repo::open(src_backend.as_ref(), &sdb, src_pass.as_deref(), &job.options, false, kdf)?;
                let (rep, w) = repo_lock::with_lock(t.backend.as_ref(), &t.options.prefix, dst_kind, "copy_versions", &dst_lo, &ctx, || {
                    let ddb = match &t.db {
                        Some((path, key, want)) => {
                            let o = maintenance::JobDbOptions { key: key.as_ref(), encrypt: *want };
                            maintenance::open_job_db(path, &o, Some(t.backend.as_ref()), t.pass.as_deref(), &t.options.prefix, &ctx)?.0
                        }
                        None => {
                            // A job-less destination: learn its blocks from the destination itself.
                            let d = BackupDb::open_in_memory()?;
                            if ufop_core::backup::format::read_header(t.backend.as_ref(), &t.options.prefix).is_ok() {
                                maintenance::recreate_database(t.backend.as_ref(), &d, t.pass.as_deref(), &t.options.prefix, &ctx)?;
                            }
                            d
                        }
                    };
                    let dst_opts = BackupOptions { dry_run: opts.dry_run, ..t.options.clone() };
                    let dst = Repo::open(t.backend.as_ref(), &ddb, t.pass.as_deref(), &dst_opts, true, kdf)?;
                    copy::copy_versions(&src, &dst, &opts, &dst_opts, &ctx)
                })?;
                Ok((rep, w))
            })?;
            let (rep, w) = rep;
            warnings.extend(w);
            let mut v = serde_json::to_value(&rep)?;
            super::add_warnings(&mut v, warnings);
            Ok(v)
        })
        .await?;
        if let Some(id) = target_job {
            self.refresh_stats(&id, None).await;
        }
        Ok(v)
    }
}

/// Every file of a version, optionally only those under `prefix`.
fn files_under(db: &ufop_core::backup::db::BackupDb, version: usize, prefix: Option<String>) -> Result<Vec<ufop_core::backup::FileEntry>, AppError> {
    let fs = db.fileset_by_version(version)?;
    let mut files = db.files(fs.id)?;
    if let Some(p) = prefix.filter(|p| !p.is_empty()) {
        files.retain(|f| f.path.starts_with(&p));
    }
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_defaults_to_the_job_and_keeps_overrides() {
        let job = BackupJob { id: "j1".into(), name: "Docs".into(), ..Default::default() };
        let i = with_identity(&job).options.identity;
        assert_eq!((i.job_id.as_str(), i.source_label.as_str(), i.hostname.as_str()), ("j1", "Docs", ""));
        let mut moved = job.clone();
        moved.options.identity.hostname = "old-laptop".into();
        moved.options.identity.job_id = "old-job".into();
        let r = job_identity(&moved);
        assert_eq!((r.hostname.as_str(), r.job_id.as_str()), ("old-laptop", "old-job"));
        assert_ne!(r.source_id(), job_identity(&job).source_id());
    }

    #[test]
    fn copy_target_serde_shape() {
        let t: CopyTarget = serde_json::from_value(serde_json::json!({"type": "job", "job_id": "x"})).unwrap();
        assert!(matches!(t, CopyTarget::Job { job_id } if job_id == "x"));
        let t: CopyTarget = serde_json::from_value(serde_json::json!({"type": "destination", "destination": {"type": "local", "path": "/mnt/b"}})).unwrap();
        assert!(matches!(t, CopyTarget::Destination { passphrase: None, .. }));
    }
}
