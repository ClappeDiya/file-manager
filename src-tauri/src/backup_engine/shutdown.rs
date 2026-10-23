//! Graceful stop of the backup manager (server shutdown, desktop quit):
//! refuse new operations, ask running ones to stop at their next safe point
//! (the engine's cancel path writes a partial version and releases the
//! repository lock), wait up to a grace period, then close drives.

use super::{lock, BackupManager};
use crate::core::error::AppError;
use serde::Serialize;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// Default time running operations get to reach a safe point.
pub const DEFAULT_GRACE: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ShutdownReport {
    /// Operations asked to stop.
    pub cancelled: usize,
    /// Operations still running when the grace period ran out.
    pub still_running: usize,
    pub waited_ms: u64,
}

impl BackupManager {
    /// Stop accepting new operations (queued and running ones are not touched).
    pub fn begin_shutdown(&self) {
        if !self.inner.draining.swap(true, Ordering::SeqCst) {
            tracing::info!("Backup manager: not accepting new operations (shutting down)");
        }
    }

    pub fn is_shutting_down(&self) -> bool {
        self.inner.draining.load(Ordering::SeqCst)
    }

    /// Refuse when shutting down (called before an operation is queued).
    pub(super) fn ensure_accepting(&self) -> Result<(), AppError> {
        if self.is_shutting_down() {
            return Err(AppError::state("The backup service is shutting down", "Start the operation again after the restart."));
        }
        Ok(())
    }

    /// Ask every queued or running operation to stop; returns how many.
    pub fn cancel_all(&self) -> usize {
        let tasks = lock(&self.inner.tasks);
        for t in tasks.iter() {
            t.cancel.store(true, Ordering::SeqCst);
        }
        tasks.len()
    }

    pub fn active_task_count(&self) -> usize {
        lock(&self.inner.tasks).len()
    }

    /// Wait until no operation is queued or running, at most `timeout`.
    pub async fn wait_idle(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if self.active_task_count() == 0 {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    /// The whole sequence: stop accepting, cancel, wait up to `grace`, close drives.
    pub async fn shutdown(&self, grace: Duration) -> ShutdownReport {
        let t0 = Instant::now();
        self.begin_shutdown();
        let cancelled = self.cancel_all();
        if cancelled > 0 {
            tracing::info!("Stopping {cancelled} backup operation(s) at the next safe point (up to {}s)", grace.as_secs());
        }
        let idle = self.wait_idle(grace).await;
        let still_running = if idle { 0 } else { self.active_task_count() };
        if still_running > 0 {
            tracing::warn!("{still_running} backup operation(s) did not stop within {}s; their repository locks expire on their own", grace.as_secs());
        }
        self.close_all_drives_blocking();
        ShutdownReport { cancelled, still_running, waited_ms: t0.elapsed().as_millis() as u64 }
    }

    /// `shutdown` from synchronous code (the desktop app's exit hook).
    pub fn shutdown_blocking(&self, grace: Duration) -> ShutdownReport {
        let me = self.clone();
        let run = move || match tokio::runtime::Builder::new_current_thread().enable_all().build() {
            Ok(rt) => rt.block_on(me.shutdown(grace)),
            Err(_) => {
                me.begin_shutdown();
                ShutdownReport { cancelled: me.cancel_all(), ..Default::default() }
            }
        };
        // A runtime cannot be started on a thread that already drives one.
        match std::thread::spawn(run).join() {
            Ok(r) => r,
            Err(_) => ShutdownReport::default(),
        }
    }
}
