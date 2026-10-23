//! Repository mirroring: copy a backup's remote files from one destination
//! to another so the copy is a complete, independently restorable backup.
//! Used for extra destinations (replicas) and by `ufop backup mirror`.
//!
//! Volumes never change once written, so a file present on both sides with
//! the same size is skipped. The repository header is copied last, making
//! a half-finished replica invisible to restores.

use super::backend::Backend;
use super::layout::LayoutBackend;
use super::volume;
use super::{hash_bytes, HashAlgorithm, OpContext, Phase, Progress};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Mutex;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MirrorOptions {
    /// Delete files on the target that the source no longer has.
    pub delete_extraneous: bool,
    /// Download each copied file again from the target and compare hashes.
    pub verify: bool,
    pub dry_run: bool,
    /// Parallel copies (0 = 2).
    pub threads: usize,
}

impl Default for MirrorOptions {
    fn default() -> Self {
        Self { delete_extraneous: true, verify: false, dry_run: false, threads: 0 }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MirrorReport {
    pub copied: usize,
    pub bytes_copied: u64,
    pub skipped_same: usize,
    pub deleted: usize,
    pub verified: usize,
    pub dry_run: bool,
    pub errors: Vec<String>,
}

/// Whether `name` belongs to the repository with `prefix`. Lock files
/// belong to one destination and are never copied or deleted.
fn ours(prefix: &str, name: &str) -> bool {
    name.strip_prefix(prefix).is_some_and(|r| r.starts_with('-')) && !super::lock::is_lock_file(prefix, name)
}

/// Make `dst` hold the same repository files as `src`.
pub fn mirror(src: &dyn Backend, dst: &dyn Backend, prefix: &str, opts: &MirrorOptions, ctx: &OpContext) -> Result<MirrorReport, AppError> {
    let header = volume::repo_header_name(prefix);
    // Both sides use the source repository's layout (sharded volumes are
    // copied into the same shard folders).
    let src_l = super::layout::for_repository(src, prefix);
    let layout = src_l.layout();
    if layout.is_sharded() && !dst.supports_subfolders() {
        return Err(AppError::state(
            format!("{} does not support subfolders, which this sharded backup needs", dst.describe()),
            "Reshard the backup to the flat layout first, or mirror to a destination with folders.",
        ));
    }
    let dst_l = LayoutBackend::new(dst, layout, prefix);
    let (src, dst): (&dyn Backend, &dyn Backend) = (&src_l, &dst_l);
    let source: Vec<_> = src.list()?.into_iter().filter(|f| ours(prefix, &f.name)).collect();
    if !source.iter().any(|f| f.name == header) {
        return Err(AppError::file_op(format!("No backup with prefix '{prefix}' at {}", src.describe()), "Check the source destination and prefix."));
    }
    if !opts.dry_run {
        dst.create_folder()?;
    }
    let target: HashMap<String, u64> = dst.list().unwrap_or_default().into_iter().filter(|f| ours(prefix, &f.name)).map(|f| (f.name, f.size)).collect();
    let mut r = MirrorReport { dry_run: opts.dry_run, ..Default::default() };
    let mut todo: Vec<(String, u64)> = Vec::new();
    for f in &source {
        if f.name == header && target.contains_key(&header) {
            r.skipped_same += 1;
        } else if f.name != header && target.get(&f.name) == Some(&f.size) {
            r.skipped_same += 1;
        } else if f.name != header {
            todo.push((f.name.clone(), f.size));
        }
    }
    let total = todo.len() as u64;
    if opts.dry_run {
        r.copied = todo.len() + usize::from(!target.contains_key(&header));
        r.bytes_copied = todo.iter().map(|t| t.1).sum();
    } else {
        let next = AtomicUsize::new(0);
        let done = AtomicU64::new(0);
        let bytes = AtomicU64::new(0);
        let copied = AtomicUsize::new(0);
        let verified = AtomicUsize::new(0);
        let errors = Mutex::new(Vec::new());
        std::thread::scope(|sc| {
            for _ in 0..(if opts.threads == 0 { 2 } else { opts.threads.min(16) }) {
                sc.spawn(|| loop {
                    if ctx.is_cancelled() {
                        return;
                    }
                    let i = next.fetch_add(1, Ordering::SeqCst);
                    let Some((name, _)) = todo.get(i) else { return };
                    let res = (|| -> Result<(), AppError> {
                        let data = src.get(name)?;
                        dst.put(name, &data)?;
                        bytes.fetch_add(data.len() as u64, Ordering::SeqCst);
                        copied.fetch_add(1, Ordering::SeqCst);
                        if opts.verify {
                            let back = dst.get(name)?;
                            if hash_bytes(HashAlgorithm::Sha256, &back) != hash_bytes(HashAlgorithm::Sha256, &data) {
                                return Err(AppError::transfer(format!("{name} differs after copying"), "Check the target destination."));
                            }
                            verified.fetch_add(1, Ordering::SeqCst);
                        }
                        Ok(())
                    })();
                    if let Err(e) = res {
                        errors.lock().map(|mut v| v.push(format!("{name}: {}", e.user_message()))).ok();
                    }
                    let n = done.fetch_add(1, Ordering::SeqCst) + 1;
                    ctx.report(&Progress { phase: Phase::Uploading, files_processed: n, files_total: total, bytes_processed: bytes.load(Ordering::SeqCst), current_path: Some(name.clone()) });
                });
            }
        });
        ctx.check()?;
        r.copied = copied.into_inner();
        r.bytes_copied = bytes.into_inner();
        r.verified = verified.into_inner();
        r.errors = errors.into_inner().unwrap_or_default();
        // Header last, and only when every volume made it.
        if r.errors.is_empty() && !target.contains_key(&header) {
            let h = src.get(&header)?;
            dst.put(&header, &h)?;
            r.copied += 1;
            r.bytes_copied += h.len() as u64;
        }
    }
    if opts.delete_extraneous && r.errors.is_empty() {
        let keep: std::collections::HashSet<&str> = source.iter().map(|f| f.name.as_str()).collect();
        for name in target.keys().filter(|n| !keep.contains(n.as_str())) {
            if !opts.dry_run {
                if let Err(e) = dst.delete(name) {
                    r.errors.push(format!("{name}: {}", e.user_message()));
                    continue;
                }
            }
            r.deleted += 1;
        }
    }
    Ok(r)
}

/// [`mirror`] under repository locks: shared on the source, exclusive on
/// the target (shared for a dry run). Returns the report with the locks'
/// warnings (stale takeover, clock skew) added to its errors-free warnings.
pub fn mirror_locked(
    src: &dyn Backend,
    dst: &dyn Backend,
    prefix: &str,
    opts: &MirrorOptions,
    lock: &super::lock::LockOptions,
    ctx: &OpContext,
) -> Result<(MirrorReport, Vec<String>), AppError> {
    use super::lock::{with_lock, LockKind};
    let target_kind = if opts.dry_run { LockKind::Shared } else { LockKind::Exclusive };
    let ((r, mut w2), mut w1) = with_lock(src, prefix, LockKind::Shared, "mirror", lock, ctx, || {
        if !opts.dry_run {
            // The target folder must exist before a lock can be written in it.
            dst.create_folder()?;
        }
        with_lock(dst, prefix, target_kind, "mirror", lock, ctx, || mirror(src, dst, prefix, opts, ctx))
    })?;
    w1.append(&mut w2);
    Ok((r, w1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;
    use crate::backup::crypto::KdfParams;
    use crate::backup::db::BackupDb;
    use crate::backup::test_util::write;
    use crate::backup::{run_backup, BackupOptions, RetentionPolicy, SourceFilter};

    #[test]
    fn mirror_copies_skips_deletes_and_is_restorable() {
        let (src, a, b, out) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "f.bin", &(0..9000u32).map(|i| (i % 241) as u8).collect::<Vec<_>>());
        let o = BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 0, ..Default::default() };
        let db = BackupDb::open_in_memory().unwrap();
        let (ba, bb) = (LocalBackend::new(a.path()), LocalBackend::new(b.path()));
        run_backup(&ba, &db, &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        std::fs::write(b.path().join("ufop-stale.dblock.aes"), b"old").unwrap();
        std::fs::write(b.path().join("other-file.txt"), b"not ours").unwrap();

        let dry = mirror(&ba, &bb, "ufop", &MirrorOptions { dry_run: true, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(dry.copied > 0 && dry.deleted == 1);
        assert!(!b.path().join("ufop-repo.json").exists(), "dry run writes nothing");

        let r = mirror(&ba, &bb, "ufop", &MirrorOptions { verify: true, threads: 3, ..Default::default() }, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        assert_eq!(r.verified + 1, r.copied, "every volume verified, header copied last");
        assert_eq!(r.deleted, 1);
        assert!(b.path().join("other-file.txt").exists(), "foreign files untouched");
        let again = mirror(&ba, &bb, "ufop", &MirrorOptions::default(), &OpContext::new()).unwrap();
        assert_eq!(again.copied, 0);

        // The replica restores on its own.
        let db2 = BackupDb::open_in_memory().unwrap();
        crate::backup::maintenance::recreate_database(&bb, &db2, Some("pw"), "ufop", &OpContext::new()).unwrap();
        let repo = crate::backup::engine::Repo::open(&bb, &db2, Some("pw"), &o, false, KdfParams::insecure_fast()).unwrap();
        let rr = crate::backup::run_restore(&repo, &crate::backup::RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
        assert!(rr.errors.is_empty() && rr.files_restored == 1);
        assert!(mirror(&bb, &ba, "nope", &MirrorOptions::default(), &OpContext::new()).is_err());
    }

    #[test]
    fn lock_files_are_never_mirrored_and_locks_are_honoured() {
        use crate::backup::lock::{LockKind, LockOptions, RepoLock};
        let (src, a, b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        write(src.path(), "f.bin", b"data");
        let o = BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 0, ..Default::default() };
        let (ba, bb) = (LocalBackend::new(a.path()), LocalBackend::new(b.path()));
        run_backup(&ba, &BackupDb::open_in_memory().unwrap(), &[src.path().into()], &SourceFilter::default(), &o, &RetentionPolicy::KeepAll, None, KdfParams::insecure_fast(), &OpContext::new()).unwrap();
        let lo = LockOptions { settle: std::time::Duration::from_millis(10), ..Default::default() };
        // A reader on the source does not stop a mirror; its lock file is not copied.
        let reader = RepoLock::acquire(&ba, "ufop", LockKind::Shared, "restore", &lo).unwrap();
        let (r, w) = mirror_locked(&ba, &bb, "ufop", &MirrorOptions::default(), &lo, &OpContext::new()).unwrap();
        assert!(r.errors.is_empty() && w.is_empty());
        assert!(!std::fs::read_dir(b.path()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains("-lock")), "no lock files copied, mirror locks released");
        drop(reader);
        // A target lock is neither deleted as extraneous nor ignored.
        let held = RepoLock::acquire(&bb, "ufop", LockKind::Shared, "verify", &lo).unwrap();
        assert_eq!(mirror(&ba, &bb, "ufop", &MirrorOptions::default(), &OpContext::new()).unwrap().deleted, 0);
        assert!(bb.get(held.file()).is_ok());
        assert!(mirror_locked(&ba, &bb, "ufop", &MirrorOptions::default(), &lo, &OpContext::new()).unwrap_err().user_message().contains("(verify)"));
        // A writer on the source blocks it too.
        drop(held);
        let _w = RepoLock::acquire(&ba, "ufop", LockKind::Exclusive, "backup", &lo).unwrap();
        assert!(mirror_locked(&ba, &bb, "ufop", &MirrorOptions::default(), &lo, &OpContext::new()).is_err());
    }
}
