//! Repository lock lease: keeps two programs (two computers, the app and
//! the CLI, two jobs on one folder) from changing a repository at the same
//! time, and keeps readers from seeing a repository while a writer rewrites
//! it.
//!
//! - **Exclusive** (backup, compact, repair, purge, delete versions,
//!   retention, reshard, format upgrade, password change, mirror target):
//!   `{prefix}-lock.json`. Nobody else may hold any lock.
//! - **Shared** (restore, verify, list, browse, recovery download, mirror
//!   source): one file per reader, `{prefix}-lock-r-{id}.json`. Readers run
//!   alongside each other, never alongside a live exclusive lock.
//!
//! Lock files are unencrypted JSON ([`LockInfo`]: owner, host, process,
//! app version, operation, acquired / expires). A lock is a *lease*: it
//! expires after the TTL (30 minutes by default) unless the holder renews
//! it, which [`with_lock`] does on a background thread every TTL/3. An
//! expired lock is stale (its holder crashed or lost the connection) and
//! is taken over with a warning.
//!
//! Acquisition writes the lock, reads it back to catch a concurrent writer
//! that overwrote it, waits a short settle time and then checks the other
//! side again (exclusive: readers; shared: the exclusive lock). Both sides
//! write before they check, so of two programs starting together at least
//! one sees the other and backs off.
//!
//! Read-only connections ([`Backend::is_read_only`]) cannot write a lock:
//! their shared locks are best effort: they refuse to start while a live
//! exclusive lock exists, but write nothing.
//!
//! When the lock is written, the destination's modification time of the
//! lock file is compared with this computer's clock (see
//! [`check_clock_skew`]): leases and version timestamps assume clocks
//! roughly agree.

use super::backend::Backend;
use super::{BackupOptions, OpContext};
use crate::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Mutex};
use std::time::Duration;

/// Default lease length.
pub const DEFAULT_LOCK_TTL_SECS: u64 = 30 * 60;
/// Default clock-skew warning threshold.
pub const DEFAULT_SKEW_WARN_SECS: u64 = 5 * 60;
/// Default clock-skew abort threshold.
pub const DEFAULT_SKEW_ABORT_SECS: u64 = 60 * 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LockKind {
    /// One writer, nobody else.
    #[default]
    Exclusive,
    /// Any number of readers, no writer.
    Shared,
}

/// Contents of a lock file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct LockInfo {
    /// Random id of this acquisition.
    pub owner_id: String,
    pub hostname: String,
    pub username: String,
    pub pid: u32,
    /// Program that holds the lock (`ufop-desktop`, `ufop-cli`, `ufop-server`).
    pub app: String,
    pub version: String,
    /// Operation name (`backup`, `restore`, `compact`, …).
    pub operation: String,
    pub kind: LockKind,
    pub acquired_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl LockInfo {
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }

    /// "laptop (backup) since 2026-10-23 10:00:00 UTC".
    pub fn holder(&self) -> String {
        let host = if self.hostname.is_empty() { "another computer" } else { &self.hostname };
        format!("{host} ({}) since {}", self.operation, self.acquired_at.format("%Y-%m-%d %H:%M:%S UTC"))
    }
}

/// One lock file found on the destination.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockEntry {
    /// File name on the destination.
    pub file: String,
    #[serde(flatten)]
    pub info: LockInfo,
    /// Expired (or unreadable): it will be taken over by the next operation.
    pub stale: bool,
    /// The file could not be parsed.
    pub damaged: bool,
}

/// Every lock currently on the destination.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockStatus {
    pub locks: Vec<LockEntry>,
    /// A live exclusive lock exists (writers and readers must wait).
    pub exclusive: bool,
    /// Live shared locks (writers must wait).
    pub readers: usize,
    pub checked_at: DateTime<Utc>,
}

impl LockStatus {
    /// "Repository in use by laptop (backup) since …" for the live lock, if any.
    pub fn summary(&self) -> Option<String> {
        let live: Vec<&LockEntry> = self.locks.iter().filter(|l| !l.stale).collect();
        let first = live.iter().find(|l| l.info.kind == LockKind::Exclusive).or_else(|| live.first())?;
        let more = if live.len() > 1 { format!(" and {} other(s)", live.len() - 1) } else { String::new() };
        Some(format!("Repository in use by {}{more}", first.info.holder()))
    }
}

/// Result of [`break_lock`].
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BreakLockReport {
    /// Lock files deleted.
    pub removed: Vec<String>,
    /// Who held them.
    pub holders: Vec<String>,
}

/// Lease and safety settings for one acquisition.
#[derive(Debug, Clone)]
pub struct LockOptions {
    /// Lease length; renewed every `ttl / 3`.
    pub ttl: Duration,
    /// Wait after writing before checking the other side again.
    pub settle: Duration,
    /// Warn when the destination clock differs by at least this (0 = never).
    pub skew_warn_secs: u64,
    /// Refuse to run when it differs by at least this (0 = never).
    pub skew_abort_secs: u64,
    /// Name of the program, recorded in the lock.
    pub app: String,
}

impl Default for LockOptions {
    fn default() -> Self {
        Self {
            ttl: Duration::from_secs(DEFAULT_LOCK_TTL_SECS),
            settle: Duration::from_millis(200),
            skew_warn_secs: DEFAULT_SKEW_WARN_SECS,
            skew_abort_secs: DEFAULT_SKEW_ABORT_SECS,
            app: "ufop".into(),
        }
    }
}

impl LockOptions {
    /// Lease and clock settings of a job's options.
    pub fn from_backup_options(o: &BackupOptions, app: &str) -> Self {
        Self {
            ttl: Duration::from_secs(if o.lock_ttl_secs == 0 { DEFAULT_LOCK_TTL_SECS } else { o.lock_ttl_secs.max(10) }),
            skew_warn_secs: o.clock_skew_warn_secs,
            skew_abort_secs: o.clock_skew_abort_secs,
            app: app.into(),
            ..Default::default()
        }
    }
}

/// `{prefix}-lock.json`: the exclusive lock.
pub fn lock_name(prefix: &str) -> String {
    format!("{prefix}-lock.json")
}

/// `{prefix}-lock-r-{id}.json`: one reader's shared lock.
pub fn reader_lock_name(prefix: &str, id: &str) -> String {
    format!("{prefix}-lock-r-{id}.json")
}

/// Whether `name` is a lock file of the repository with `prefix`. Lock
/// files are not volumes: listings, mirrors, recovery and "unknown file"
/// checks skip them.
pub fn is_lock_file(prefix: &str, name: &str) -> bool {
    let name = super::layout::logical(name);
    name == lock_name(prefix) || name.strip_prefix(&format!("{prefix}-lock-r-")).is_some_and(|r| r.ends_with(".json"))
}

fn now() -> DateTime<Utc> {
    Utc::now()
}

fn to_chrono(d: Duration) -> chrono::Duration {
    chrono::Duration::from_std(d).unwrap_or_else(|_| chrono::Duration::seconds(DEFAULT_LOCK_TTL_SECS as i64))
}

/// Read one lock file: `None` when it does not exist, `Err` text when damaged.
/// Reads directly and lists only when the read fails (listings are slow on
/// large destinations).
fn read_lock(backend: &dyn Backend, name: &str) -> Result<Option<Result<LockInfo, String>>, AppError> {
    match backend.get(name) {
        Ok(d) => Ok(Some(serde_json::from_slice::<LockInfo>(&d).map_err(|e| e.to_string()))),
        Err(e) => {
            if backend.list()?.iter().any(|f| f.name == name) {
                Ok(Some(Err(e.user_message())))
            } else {
                Ok(None)
            }
        }
    }
}

/// Every lock file of `prefix` with its parsed contents.
pub fn status(backend: &dyn Backend, prefix: &str) -> Result<LockStatus, AppError> {
    let t = now();
    let mut s = LockStatus { checked_at: t, ..Default::default() };
    let mut names: Vec<String> = backend.list()?.into_iter().map(|f| f.name).filter(|n| is_lock_file(prefix, n)).collect();
    names.sort();
    for file in names {
        let (info, damaged) = match backend.get(&file).map_err(|e| e.user_message()).and_then(|d| serde_json::from_slice::<LockInfo>(&d).map_err(|e| e.to_string())) {
            Ok(i) => (i, false),
            Err(_) => (
                LockInfo { operation: "unknown".into(), kind: if file == lock_name(prefix) { LockKind::Exclusive } else { LockKind::Shared }, ..Default::default() },
                true,
            ),
        };
        let stale = damaged || info.is_expired(t);
        if !stale {
            match info.kind {
                LockKind::Exclusive => s.exclusive = true,
                LockKind::Shared => s.readers += 1,
            }
        }
        s.locks.push(LockEntry { file, info, stale, damaged });
    }
    Ok(s)
}

/// Force-unlock: delete every lock file of `prefix`. Needs `confirmed`;
/// with `expected_owner` only the lock with that owner id is removed, and
/// nothing happens when it changed since the caller looked.
pub fn break_lock(backend: &dyn Backend, prefix: &str, confirmed: bool, expected_owner: Option<&str>) -> Result<BreakLockReport, AppError> {
    if !confirmed {
        return Err(AppError::validation(
            "Breaking a repository lock needs explicit confirmation: only do it when the program named in the lock is no longer running",
        ));
    }
    if backend.is_read_only() {
        return Err(AppError::PermissionDenied {
            message: "This connection is read-only and cannot remove lock files".into(),
            advice: "Break the lock from a connection that can write to the destination.".into(),
        });
    }
    let st = status(backend, prefix)?;
    let targets: Vec<&LockEntry> = match expected_owner.filter(|o| !o.is_empty()) {
        Some(o) => {
            let hit: Vec<&LockEntry> = st.locks.iter().filter(|l| l.info.owner_id == o).collect();
            if hit.is_empty() {
                return Err(AppError::state(
                    "The lock changed since it was shown (it was released or someone else holds it now)",
                    "Refresh the lock status and decide again.",
                ));
            }
            hit
        }
        None => st.locks.iter().collect(),
    };
    let mut r = BreakLockReport::default();
    for l in targets {
        backend.delete(&l.file)?;
        tracing::warn!("Broke repository lock {} held by {}", l.file, l.info.holder());
        r.removed.push(l.file.clone());
        r.holders.push(l.info.holder());
    }
    Ok(r)
}

fn in_use(info: &LockInfo) -> AppError {
    AppError::state(
        format!("Repository in use by {}", info.holder()),
        format!(
            "Wait for that operation to finish and try again. If that program is no longer running, its lock expires at {} or can be removed with Break lock.",
            info.expires_at.format("%Y-%m-%d %H:%M:%S UTC")
        ),
    )
}

/// Compare the destination's time stamp of a file just written (between
/// `before` and `after` on this computer's clock). Returns a warning, or
/// an error at the abort threshold. Signed skew: positive = destination ahead.
pub fn check_clock_skew(
    backend: &dyn Backend,
    name: &str,
    before: DateTime<Utc>,
    after: DateTime<Utc>,
    warn_secs: u64,
    abort_secs: u64,
) -> Result<Option<String>, AppError> {
    if warn_secs == 0 && abort_secs == 0 {
        return Ok(None);
    }
    let remote = match backend.modified(name) {
        Ok(Some(t)) => t,
        Ok(None) => return Ok(None),
        Err(e) => {
            tracing::debug!("Clock check skipped: {}", e.user_message());
            return Ok(None);
        }
    };
    // Many file systems keep whole seconds: allow one second of rounding.
    let skew = if remote < before - chrono::Duration::seconds(1) {
        (remote - before).num_seconds()
    } else if remote > after + chrono::Duration::seconds(1) {
        (remote - after).num_seconds()
    } else {
        0
    };
    let abs = skew.unsigned_abs();
    let describe = || {
        let dir = if skew > 0 { "ahead of" } else { "behind" };
        format!("The destination's clock is {} {dir} this computer's clock", human_secs(abs))
    };
    if abort_secs > 0 && abs >= abort_secs {
        return Err(AppError::state(
            describe(),
            "Lock leases and version times would be wrong. Correct the time on this computer or the storage server (turn on automatic time / NTP), or raise the clock-skew limit in the job's advanced options.",
        ));
    }
    if warn_secs > 0 && abs >= warn_secs {
        return Ok(Some(format!("{}; turn on automatic time (NTP) on both", describe())));
    }
    Ok(None)
}

fn human_secs(s: u64) -> String {
    match s {
        0..=119 => format!("{s} s"),
        120..=7199 => format!("{} min", (s + 30) / 60),
        _ => format!("{:.1} h", s as f64 / 3600.0),
    }
}

/// A held repository lock. Released on drop (and by [`RepoLock::release`]).
pub struct RepoLock<'a> {
    backend: &'a dyn Backend,
    file: String,
    info: Mutex<LockInfo>,
    /// False for best-effort shared locks on read-only connections.
    written: bool,
    ttl: Duration,
    released: AtomicBool,
    lost: AtomicBool,
    warnings: Vec<String>,
}

impl std::fmt::Debug for RepoLock<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RepoLock").field("file", &self.file).field("written", &self.written).finish()
    }
}

impl<'a> RepoLock<'a> {
    /// Take the lock or fail with "Repository in use by …".
    pub fn acquire(backend: &'a dyn Backend, prefix: &str, kind: LockKind, operation: &str, opts: &LockOptions) -> Result<Self, AppError> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let excl = lock_name(prefix);
        let file = match kind {
            LockKind::Exclusive => excl.clone(),
            LockKind::Shared => reader_lock_name(prefix, &id),
        };
        if kind == LockKind::Exclusive && backend.is_read_only() {
            return Err(AppError::PermissionDenied {
                message: format!("{} is connected read-only; '{operation}' needs to write", backend.describe()),
                advice: "Turn off 'Read-only' for this job's destination to back up or run maintenance.".into(),
            });
        }
        let mut warnings = Vec::new();
        let t = now();
        let take_over = |file: &str, info: Option<&LockInfo>, warnings: &mut Vec<String>| -> Result<(), AppError> {
            let who = info.map(|i| i.holder()).unwrap_or_else(|| "an unreadable lock file".into());
            let msg = format!("Took over a stale repository lock held by {who} (expired; that program probably stopped)");
            tracing::warn!("{msg}");
            warnings.push(msg);
            if !backend.is_read_only() {
                backend.delete(file)?;
            }
            Ok(())
        };
        // 1. Who holds it now?
        let current = status(backend, prefix)?;
        for l in &current.locks {
            let relevant = kind == LockKind::Exclusive || l.info.kind == LockKind::Exclusive;
            if !relevant {
                continue;
            }
            if l.stale {
                take_over(&l.file, (!l.damaged).then_some(&l.info), &mut warnings)?;
            } else {
                return Err(in_use(&l.info));
            }
        }
        let info = LockInfo {
            owner_id: id,
            hostname: crate::session::hostname(),
            username: crate::session::user_name(),
            pid: std::process::id(),
            app: opts.app.clone(),
            version: env!("CARGO_PKG_VERSION").into(),
            operation: operation.into(),
            kind,
            acquired_at: t,
            expires_at: t + to_chrono(opts.ttl),
        };
        if backend.is_read_only() {
            // Best effort: nothing can be written on a read-only connection.
            return Ok(Self { backend, file, info: Mutex::new(info), written: false, ttl: opts.ttl, released: AtomicBool::new(false), lost: AtomicBool::new(false), warnings });
        }
        // 2. Write, then read back (a concurrent writer may have overwritten it).
        let before = now();
        backend.put(&file, &serde_json::to_vec_pretty(&info)?)?;
        let after = now();
        let mut guard = Self { backend, file, info: Mutex::new(info), written: true, ttl: opts.ttl, released: AtomicBool::new(false), lost: AtomicBool::new(false), warnings };
        guard.verify_owner()?;
        // 3. Settle, then check again: the other side writes before it checks too.
        if !opts.settle.is_zero() {
            std::thread::sleep(opts.settle);
        }
        guard.verify_owner()?;
        let again = status(backend, prefix)?;
        let conflict = again.locks.iter().find(|l| {
            !l.stale && l.file != guard.file && (kind == LockKind::Exclusive || l.info.kind == LockKind::Exclusive)
        });
        if let Some(l) = conflict {
            let e = in_use(&l.info);
            guard.release_quietly();
            return Err(e);
        }
        // 4. Clock skew between this computer and the destination.
        match check_clock_skew(backend, &guard.file, before, after, opts.skew_warn_secs, opts.skew_abort_secs) {
            Ok(Some(w)) => guard.warnings.push(w),
            Ok(None) => {}
            Err(e) => {
                guard.release_quietly();
                return Err(e);
            }
        }
        Ok(guard)
    }

    /// The lock file still holds our owner id.
    fn verify_owner(&mut self) -> Result<(), AppError> {
        let mine = self.info.lock().unwrap_or_else(|e| e.into_inner()).owner_id.clone();
        match read_lock(self.backend, &self.file)? {
            Some(Ok(i)) if i.owner_id == mine => Ok(()),
            Some(Ok(i)) => {
                // Someone else wrote it at the same moment: theirs now.
                self.released.store(true, Ordering::SeqCst);
                Err(in_use(&i))
            }
            _ => {
                self.released.store(true, Ordering::SeqCst);
                Err(AppError::state(
                    "Another program changed the repository lock while it was being taken",
                    "Try again in a moment; if it keeps happening, check what else uses this destination.",
                ))
            }
        }
    }

    /// Extend the lease by one TTL. Fails (and marks the lock lost) when
    /// the file now belongs to someone else or was removed (Break lock).
    pub fn renew(&self) -> Result<(), AppError> {
        if !self.written || self.released.load(Ordering::SeqCst) {
            return Ok(());
        }
        let mut info = self.info.lock().unwrap_or_else(|e| e.into_inner());
        match read_lock(self.backend, &self.file)? {
            Some(Ok(i)) if i.owner_id == info.owner_id => {}
            other => {
                self.lost.store(true, Ordering::SeqCst);
                let who = match other {
                    Some(Ok(i)) => format!("it is now held by {}", i.holder()),
                    _ => "it was removed (Break lock)".to_string(),
                };
                return Err(AppError::state(format!("The repository lock was lost: {who}"), "The operation stops; run it again."));
            }
        }
        info.expires_at = now() + to_chrono(self.ttl);
        self.backend.put(&self.file, &serde_json::to_vec_pretty(&*info)?)
    }

    /// The lease was taken over or removed while held.
    pub fn is_lost(&self) -> bool {
        self.lost.load(Ordering::SeqCst)
    }

    /// Whether the lock file was written (false: best-effort read-only lock).
    pub fn is_written(&self) -> bool {
        self.written
    }

    pub fn info(&self) -> LockInfo {
        self.info.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub fn file(&self) -> &str {
        &self.file
    }

    /// Warnings from taking the lock (stale takeover, clock skew).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Delete the lock file if it is still ours.
    pub fn release(&self) -> Result<(), AppError> {
        if self.released.swap(true, Ordering::SeqCst) || !self.written {
            return Ok(());
        }
        let mine = self.info.lock().unwrap_or_else(|e| e.into_inner()).owner_id.clone();
        match read_lock(self.backend, &self.file)? {
            Some(Ok(i)) if i.owner_id == mine => self.backend.delete(&self.file),
            _ => Ok(()),
        }
    }

    fn release_quietly(&self) {
        if let Err(e) = self.release() {
            tracing::warn!("Could not release repository lock {}: {}", self.file, e.user_message());
        }
    }
}

impl Drop for RepoLock<'_> {
    fn drop(&mut self) {
        self.release_quietly();
    }
}

/// Run `f` while holding a lock on `prefix`, renewing the lease in the
/// background every TTL/3 and releasing it afterwards (also on error or
/// panic). When the lease is lost the operation is cancelled through `ctx`.
/// Returns `f`'s result and the lock's warnings.
pub fn with_lock<T>(
    backend: &dyn Backend,
    prefix: &str,
    kind: LockKind,
    operation: &str,
    opts: &LockOptions,
    ctx: &OpContext,
    f: impl FnOnce() -> Result<T, AppError>,
) -> Result<(T, Vec<String>), AppError> {
    let lock = RepoLock::acquire(backend, prefix, kind, operation, opts)?;
    let renew_every = (opts.ttl / 3).max(Duration::from_millis(50));
    let lost_msg: Mutex<Option<String>> = Mutex::new(None);
    let result = std::thread::scope(|scope| {
        let (stop_tx, stop_rx) = mpsc::channel::<()>();
        if lock.is_written() {
            let (lock, lost_msg) = (&lock, &lost_msg);
            scope.spawn(move || loop {
                match stop_rx.recv_timeout(renew_every) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if let Err(e) = lock.renew() {
                            if lock.is_lost() {
                                tracing::warn!("{}", e.user_message());
                                *lost_msg.lock().unwrap_or_else(|e| e.into_inner()) = Some(e.user_message());
                                ctx.cancel();
                                return;
                            }
                            // Transient destination error: try again next interval.
                            tracing::warn!("Could not renew the repository lock: {}", e.user_message());
                        }
                    }
                    _ => return,
                }
            });
        }
        let r = f();
        drop(stop_tx);
        r
    });
    let mut warnings = lock.warnings().to_vec();
    if let Some(m) = lost_msg.into_inner().unwrap_or_else(|e| e.into_inner()) {
        warnings.push(m);
    }
    lock.release()?;
    result.map(|v| (v, warnings))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::{LocalBackend, ReadOnlyBackend, RemoteFile};

    fn fast() -> LockOptions {
        LockOptions { settle: Duration::from_millis(30), ..Default::default() }
    }

    #[test]
    fn exclusive_excludes_everyone_and_releases_on_drop() {
        let d = tempfile::tempdir().unwrap();
        let b = LocalBackend::new(d.path());
        let l = RepoLock::acquire(&b, "ufop", LockKind::Exclusive, "backup", &fast()).unwrap();
        assert!(d.path().join("ufop-lock.json").is_file());
        let info: LockInfo = serde_json::from_slice(&std::fs::read(d.path().join("ufop-lock.json")).unwrap()).unwrap();
        assert_eq!((info.operation.as_str(), info.kind, info.pid), ("backup", LockKind::Exclusive, std::process::id()));
        assert!(info.expires_at > info.acquired_at + chrono::Duration::minutes(29));
        for kind in [LockKind::Exclusive, LockKind::Shared] {
            let e = RepoLock::acquire(&b, "ufop", kind, "restore", &fast()).unwrap_err();
            assert!(e.user_message().contains("Repository in use by"), "{e}");
            assert!(e.user_message().contains("(backup)"));
        }
        // Another prefix in the same folder is another repository.
        drop(RepoLock::acquire(&b, "other", LockKind::Exclusive, "backup", &fast()).unwrap());
        let st = status(&b, "ufop").unwrap();
        assert!(st.exclusive && st.readers == 0);
        assert!(st.summary().unwrap().starts_with("Repository in use by vm") || st.summary().unwrap().starts_with("Repository in use by"));
        drop(l);
        assert!(!d.path().join("ufop-lock.json").exists());
        assert!(status(&b, "ufop").unwrap().locks.is_empty());
    }

    #[test]
    fn readers_share_but_block_writers() {
        let d = tempfile::tempdir().unwrap();
        let b = LocalBackend::new(d.path());
        let r1 = RepoLock::acquire(&b, "ufop", LockKind::Shared, "restore", &fast()).unwrap();
        let r2 = RepoLock::acquire(&b, "ufop", LockKind::Shared, "verify", &fast()).unwrap();
        assert_ne!(r1.file(), r2.file());
        assert!(r1.file().starts_with("ufop-lock-r-") && is_lock_file("ufop", r1.file()));
        assert_eq!(status(&b, "ufop").unwrap().readers, 2);
        let e = RepoLock::acquire(&b, "ufop", LockKind::Exclusive, "compact", &fast()).unwrap_err();
        assert!(e.user_message().contains("(restore)") || e.user_message().contains("(verify)"));
        assert!(!d.path().join("ufop-lock.json").exists(), "the failed writer cleaned up");
        drop(r1);
        drop(r2);
        drop(RepoLock::acquire(&b, "ufop", LockKind::Exclusive, "compact", &fast()).unwrap());
        assert!(b.list().unwrap().is_empty());
    }

    #[test]
    fn two_acquirers_racing_get_exactly_one_lock() {
        for _ in 0..5 {
            let d = tempfile::tempdir().unwrap();
            let b = LocalBackend::new(d.path());
            let barrier = std::sync::Barrier::new(2);
            let wins = std::sync::atomic::AtomicUsize::new(0);
            let held = Mutex::new(Vec::new());
            std::thread::scope(|s| {
                for kind in [LockKind::Exclusive, LockKind::Exclusive] {
                    let (b, barrier, wins, held) = (&b, &barrier, &wins, &held);
                    s.spawn(move || {
                        barrier.wait();
                        if let Ok(l) = RepoLock::acquire(b, "ufop", kind, "backup", &fast()) {
                            wins.fetch_add(1, Ordering::SeqCst);
                            // Hold it until both tried.
                            held.lock().unwrap().push(l.info());
                            std::thread::sleep(Duration::from_millis(150));
                        }
                    });
                }
            });
            assert_eq!(wins.load(Ordering::SeqCst), 1, "{:?}", held.lock().unwrap());
            assert!(b.list().unwrap().is_empty(), "winner released");
        }
        // A writer and a reader starting together never both run.
        for _ in 0..5 {
            let d = tempfile::tempdir().unwrap();
            let b = LocalBackend::new(d.path());
            let barrier = std::sync::Barrier::new(2);
            let running = std::sync::atomic::AtomicUsize::new(0);
            let overlap = AtomicBool::new(false);
            std::thread::scope(|s| {
                for kind in [LockKind::Exclusive, LockKind::Shared] {
                    let (b, barrier, running, overlap) = (&b, &barrier, &running, &overlap);
                    s.spawn(move || {
                        barrier.wait();
                        if let Ok(_l) = RepoLock::acquire(b, "ufop", kind, "op", &fast()) {
                            if running.fetch_add(1, Ordering::SeqCst) > 0 {
                                overlap.store(true, Ordering::SeqCst);
                            }
                            std::thread::sleep(Duration::from_millis(100));
                            running.fetch_sub(1, Ordering::SeqCst);
                        }
                    });
                }
            });
            assert!(!overlap.load(Ordering::SeqCst));
        }
    }

    fn stale_lock(dir: &std::path::Path, name: &str, kind: LockKind) {
        let old = LockInfo {
            owner_id: "dead".into(),
            hostname: "old-pc".into(),
            operation: "backup".into(),
            kind,
            acquired_at: Utc::now() - chrono::Duration::hours(3),
            expires_at: Utc::now() - chrono::Duration::hours(2),
            ..Default::default()
        };
        std::fs::write(dir.join(name), serde_json::to_vec(&old).unwrap()).unwrap();
    }

    #[test]
    fn stale_locks_are_taken_over_with_a_warning() {
        let d = tempfile::tempdir().unwrap();
        let b = LocalBackend::new(d.path());
        stale_lock(d.path(), "ufop-lock.json", LockKind::Exclusive);
        stale_lock(d.path(), "ufop-lock-r-dead.json", LockKind::Shared);
        let st = status(&b, "ufop").unwrap();
        assert_eq!(st.locks.len(), 2);
        assert!(st.locks.iter().all(|l| l.stale) && !st.exclusive && st.summary().is_none());
        let l = RepoLock::acquire(&b, "ufop", LockKind::Exclusive, "compact", &fast()).unwrap();
        assert_eq!(l.warnings().len(), 2, "{:?}", l.warnings());
        assert!(l.warnings()[0].contains("old-pc"));
        assert!(!d.path().join("ufop-lock-r-dead.json").exists());
        drop(l);
        // A damaged lock file counts as stale too.
        std::fs::write(d.path().join("ufop-lock.json"), b"{not json").unwrap();
        assert!(status(&b, "ufop").unwrap().locks[0].damaged);
        let l = RepoLock::acquire(&b, "ufop", LockKind::Shared, "restore", &fast()).unwrap();
        assert!(l.warnings()[0].contains("unreadable"));
    }

    #[test]
    fn renewal_extends_the_lease_and_detects_loss() {
        let d = tempfile::tempdir().unwrap();
        let b = LocalBackend::new(d.path());
        let opts = LockOptions { ttl: Duration::from_millis(600), ..fast() };
        let read = || serde_json::from_slice::<LockInfo>(&std::fs::read(d.path().join("ufop-lock.json")).unwrap()).unwrap();
        let ctx = OpContext::new();
        let ((first, later), w) = with_lock(&b, "ufop", LockKind::Exclusive, "backup", &opts, &ctx, || {
            let first = read().expires_at;
            // Longer than the TTL: without renewal the lock would expire.
            std::thread::sleep(Duration::from_millis(900));
            let later = read();
            assert!(!later.is_expired(Utc::now()), "renewed in the background");
            Ok((first, later.expires_at))
        })
        .unwrap();
        assert!(later > first, "{first} → {later}");
        assert!(w.is_empty());
        assert!(!d.path().join("ufop-lock.json").exists(), "released afterwards");
        // Break lock while held: renewal notices and cancels the operation.
        let ctx = OpContext::new();
        let (cancelled, w) = with_lock(&b, "ufop", LockKind::Exclusive, "backup", &opts, &ctx, || {
            break_lock(&b, "ufop", true, None).unwrap();
            for _ in 0..50 {
                if ctx.is_cancelled() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(ctx.is_cancelled())
        })
        .unwrap();
        assert!(cancelled);
        assert!(w.iter().any(|w| w.contains("lock was lost")), "{w:?}");
    }

    #[test]
    fn released_on_error_and_panic() {
        let d = tempfile::tempdir().unwrap();
        let b = LocalBackend::new(d.path());
        let r: Result<((), Vec<String>), AppError> = with_lock(&b, "ufop", LockKind::Exclusive, "backup", &fast(), &OpContext::new(), || Err(AppError::validation("boom")));
        assert!(r.is_err());
        assert!(b.list().unwrap().is_empty());
        let p = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = with_lock(&b, "ufop", LockKind::Shared, "restore", &fast(), &OpContext::new(), || -> Result<(), AppError> { panic!("crash") });
        }));
        assert!(p.is_err());
        assert!(b.list().unwrap().is_empty(), "the guard released it while unwinding");
    }

    #[test]
    fn break_lock_needs_confirmation_and_the_right_owner() {
        let d = tempfile::tempdir().unwrap();
        let b = LocalBackend::new(d.path());
        let l = RepoLock::acquire(&b, "ufop", LockKind::Exclusive, "backup", &fast()).unwrap();
        assert!(break_lock(&b, "ufop", false, None).is_err());
        assert!(break_lock(&b, "ufop", true, Some("someone-else")).is_err());
        let r = break_lock(&b, "ufop", true, Some(&l.info().owner_id)).unwrap();
        assert_eq!(r.removed, ["ufop-lock.json"]);
        assert!(l.renew().is_err() && l.is_lost());
        drop(l);
        assert!(b.list().unwrap().is_empty());
    }

    #[test]
    fn read_only_connections_take_best_effort_shared_locks() {
        let d = tempfile::tempdir().unwrap();
        let rw = LocalBackend::new(d.path());
        let ro = ReadOnlyBackend::new(LocalBackend::new(d.path()));
        let l = RepoLock::acquire(&ro, "ufop", LockKind::Shared, "restore", &fast()).unwrap();
        assert!(!l.is_written());
        assert!(rw.list().unwrap().is_empty(), "nothing written");
        drop(l);
        assert!(RepoLock::acquire(&ro, "ufop", LockKind::Exclusive, "backup", &fast()).is_err());
        let w = RepoLock::acquire(&rw, "ufop", LockKind::Exclusive, "backup", &fast()).unwrap();
        assert!(RepoLock::acquire(&ro, "ufop", LockKind::Shared, "restore", &fast()).unwrap_err().user_message().contains("in use"));
        assert!(break_lock(&ro, "ufop", true, None).is_err());
        drop(w);
    }

    /// A destination whose clock runs `offset` ahead of ours.
    struct Skewed(LocalBackend, chrono::Duration);
    impl Backend for Skewed {
        fn describe(&self) -> String { self.0.describe() }
        fn list(&self) -> Result<Vec<RemoteFile>, AppError> { self.0.list() }
        fn put(&self, n: &str, d: &[u8]) -> Result<(), AppError> { self.0.put(n, d) }
        fn get(&self, n: &str) -> Result<Vec<u8>, AppError> { self.0.get(n) }
        fn delete(&self, n: &str) -> Result<(), AppError> { self.0.delete(n) }
        fn create_folder(&self) -> Result<(), AppError> { self.0.create_folder() }
        fn modified(&self, n: &str) -> Result<Option<DateTime<Utc>>, AppError> { Ok(self.0.modified(n)?.map(|t| t + self.1)) }
    }

    #[test]
    fn clock_skew_warns_and_aborts() {
        let d = tempfile::tempdir().unwrap();
        let ok = Skewed(LocalBackend::new(d.path()), chrono::Duration::seconds(20));
        assert!(RepoLock::acquire(&ok, "ufop", LockKind::Exclusive, "backup", &fast()).unwrap().warnings().is_empty());
        let warn = Skewed(LocalBackend::new(d.path()), chrono::Duration::minutes(-10));
        let l = RepoLock::acquire(&warn, "ufop", LockKind::Exclusive, "backup", &fast()).unwrap();
        assert!(l.warnings()[0].contains("10 min behind"), "{:?}", l.warnings());
        drop(l);
        let bad = Skewed(LocalBackend::new(d.path()), chrono::Duration::hours(3));
        let e = RepoLock::acquire(&bad, "ufop", LockKind::Shared, "restore", &fast()).unwrap_err();
        assert!(e.user_message().contains("3.0 h ahead"), "{e}");
        assert!(bad.list().unwrap().is_empty(), "aborted lock released");
        // Thresholds of 0 turn the check off.
        let off = LockOptions { skew_warn_secs: 0, skew_abort_secs: 0, ..fast() };
        assert!(RepoLock::acquire(&bad, "ufop", LockKind::Exclusive, "backup", &off).unwrap().warnings().is_empty());
        // Backends without time stamps skip the check.
        assert_eq!(check_clock_skew(&ReadOnlyBackend::new(Skewed(LocalBackend::new(d.path()), chrono::Duration::zero())), "missing", Utc::now(), Utc::now(), 1, 2).unwrap(), None);
    }
}
