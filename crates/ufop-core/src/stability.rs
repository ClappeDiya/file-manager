//! Changing-source guard: don't copy a file that is still being written.
//!
//! Two checks, for two situations:
//! - [`settled`] is stateless: a file whose modification time is at least
//!   `quiet` old is assumed complete. The sync planner uses it to defer
//!   fresh files to the next pass.
//! - [`wait_until_stable`] watches a file until its size and modification
//!   time stop changing for `quiet` (with a deadline). Watcher-triggered
//!   automations use it, because they fire the moment a file appears.

use std::path::Path;
use std::time::{Duration, Instant, SystemTime};

/// Size and modification time, or `None` if the file is gone.
pub fn fingerprint(path: &Path) -> Option<(u64, Option<SystemTime>)> {
    let m = std::fs::metadata(path).ok()?;
    Some((m.len(), m.modified().ok()))
}

/// Whether `meta` was last modified at least `quiet` before `now`. Files
/// with no modification time, or one in the future, count as settled
/// (nothing better to go on; never block forever).
pub fn settled(modified: Option<SystemTime>, quiet: Duration, now: SystemTime) -> bool {
    match modified {
        Some(t) => now.duration_since(t).map(|age| age >= quiet).unwrap_or(true),
        None => true,
    }
}

/// Outcome of waiting for a file to stop changing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stability {
    /// Unchanged for the quiet period.
    Stable,
    /// Still changing at the deadline.
    StillChanging,
    /// Removed (or never existed).
    Gone,
}

/// Block until `path` is unchanged for `quiet`, checking every `poll`, for
/// at most `timeout`.
pub fn wait_until_stable_blocking(path: &Path, quiet: Duration, poll: Duration, timeout: Duration) -> Stability {
    let deadline = Instant::now() + timeout;
    let Some(mut last) = fingerprint(path) else { return Stability::Gone };
    let mut unchanged_since = Instant::now();
    loop {
        if unchanged_since.elapsed() >= quiet {
            return Stability::Stable;
        }
        if Instant::now() >= deadline {
            return Stability::StillChanging;
        }
        std::thread::sleep(poll.min(quiet));
        match fingerprint(path) {
            None => return Stability::Gone,
            Some(now) if now != last => {
                last = now;
                unchanged_since = Instant::now();
            }
            Some(_) => {}
        }
    }
}

/// Async [`wait_until_stable_blocking`].
#[cfg(feature = "agent")]
pub async fn wait_until_stable(path: &Path, quiet: Duration, poll: Duration, timeout: Duration) -> Stability {
    let deadline = Instant::now() + timeout;
    let Some(mut last) = fingerprint(path) else { return Stability::Gone };
    let mut unchanged_since = Instant::now();
    loop {
        if unchanged_since.elapsed() >= quiet {
            return Stability::Stable;
        }
        if Instant::now() >= deadline {
            return Stability::StillChanging;
        }
        tokio::time::sleep(poll.min(quiet)).await;
        match fingerprint(path) {
            None => return Stability::Gone,
            Some(now) if now != last => {
                last = now;
                unchanged_since = Instant::now();
            }
            Some(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn tmp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("ufop-stability-{}-{name}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d.join("f.bin")
    }

    #[test]
    fn settled_uses_age() {
        let now = SystemTime::now();
        assert!(!settled(Some(now - Duration::from_secs(1)), Duration::from_secs(5), now));
        assert!(settled(Some(now - Duration::from_secs(10)), Duration::from_secs(5), now));
        assert!(settled(Some(now + Duration::from_secs(60)), Duration::from_secs(5), now), "clock skew never blocks");
        assert!(settled(None, Duration::from_secs(5), now));
    }

    #[test]
    fn stable_file_and_missing_file() {
        let p = tmp("stable");
        std::fs::write(&p, b"done").unwrap();
        let s = wait_until_stable_blocking(&p, Duration::from_millis(100), Duration::from_millis(20), Duration::from_secs(2));
        assert_eq!(s, Stability::Stable);
        assert_eq!(
            wait_until_stable_blocking(&p.with_file_name("nope"), Duration::from_millis(50), Duration::from_millis(10), Duration::from_secs(1)),
            Stability::Gone
        );
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }

    #[test]
    fn growing_file_is_not_stable_until_writes_stop() {
        let p = tmp("growing");
        let mut f = std::fs::File::create(&p).unwrap();
        let writer = std::thread::spawn(move || {
            for _ in 0..8 {
                f.write_all(&[0u8; 1024]).unwrap();
                f.flush().unwrap();
                std::thread::sleep(Duration::from_millis(40));
            }
        });
        // Quiet period longer than the gap between writes: still changing
        // at a deadline that falls inside the writing window.
        let early = wait_until_stable_blocking(&p, Duration::from_millis(150), Duration::from_millis(10), Duration::from_millis(200));
        assert_eq!(early, Stability::StillChanging);
        writer.join().unwrap();
        let later = wait_until_stable_blocking(&p, Duration::from_millis(150), Duration::from_millis(10), Duration::from_secs(3));
        assert_eq!(later, Stability::Stable);
        assert_eq!(std::fs::metadata(&p).unwrap().len(), 8 * 1024);
        std::fs::remove_dir_all(p.parent().unwrap()).ok();
    }
}
