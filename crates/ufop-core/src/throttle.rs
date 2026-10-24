//! The one bandwidth limiter, shared by backups, sync and file transfers,
//! and the bandwidth timetable.
//!
//! A [`SharedRate`] spreads transfers over a time line so that everything
//! going through it together stays at or below its rate.
//!
//! - [`GLOBAL_UPLOAD`] / [`GLOBAL_DOWNLOAD`]: the backup "machine-wide
//!   limits" (and the console's managed caps). They pace backups and
//!   restores only, as they always have. While a [`BandwidthWindow`] is
//!   active its limits replace these base rates.
//! - [`TIMETABLE_UPLOAD`]: the timetable alone (no base rate), shared by
//!   backups, sync copies and file transfers, so while a window is active
//!   all of them together stay within its upload limit. Sync copies and
//!   transfers have no direction of their own and count as uploads; a
//!   window's download limit applies to backup downloads (restores).
//!
//! Window times reuse the sync run-hours format (`HH:MM` or `HHMM`, a window
//! may wrap midnight) and weekdays the backup-schedule convention
//! (0 = Monday … 6 = Sunday, empty = every day). A window that wraps
//! midnight belongs to the day it starts on.

use crate::error::AppError;
use crate::sync::copier::{in_window, parse_run_hours};
use chrono::{Datelike, Local, NaiveDateTime, Timelike};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::RwLock;
use std::time::{Duration, Instant};

/// Which way the bytes go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Upload,
    Download,
}

/// One timetable entry: on `days`, between `start` and `end`, limit
/// uploads and downloads to these rates (bytes/second, 0 = unlimited).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct BandwidthWindow {
    /// 0 = Monday … 6 = Sunday; empty = every day.
    pub days: Vec<u8>,
    /// `HH:MM`.
    pub start: String,
    /// `HH:MM`; before `start` = the window wraps midnight.
    pub end: String,
    pub upload_bps: u64,
    pub download_bps: u64,
}

impl BandwidthWindow {
    fn minutes(&self) -> Option<(u32, u32)> {
        parse_run_hours(&format!("{}-{}", self.start.trim(), self.end.trim()))
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if self.days.iter().any(|d| *d > 6) {
            return Err(AppError::validation("Weekdays must be 0 (Monday) to 6 (Sunday)"));
        }
        if self.minutes().is_none() {
            return Err(AppError::validation(format!(
                "\"{}–{}\" is not a valid time window; use HH:MM, for example 08:00 to 18:00",
                self.start.trim(),
                self.end.trim()
            )));
        }
        Ok(())
    }

    /// Whether the window is active at local time `t`.
    pub fn active_at(&self, t: NaiveDateTime) -> bool {
        let Some((start, end)) = self.minutes() else { return false };
        let minute = t.hour() * 60 + t.minute();
        if !in_window((start, end), minute) {
            return false;
        }
        let today = t.weekday().num_days_from_monday() as u8;
        // After midnight in a wrapping window: it started yesterday.
        let day = if start > end && minute < end { (today + 6) % 7 } else { today };
        self.days.is_empty() || self.days.contains(&day)
    }

    pub fn limit(&self, d: Direction) -> u64 {
        match d {
            Direction::Upload => self.upload_bps,
            Direction::Download => self.download_bps,
        }
    }
}

/// Validate a whole timetable.
pub fn validate_schedule(windows: &[BandwidthWindow]) -> Result<(), AppError> {
    windows.iter().try_for_each(BandwidthWindow::validate)
}

/// The first window active at `t`.
pub fn window_at(windows: &[BandwidthWindow], t: NaiveDateTime) -> Option<&BandwidthWindow> {
    windows.iter().find(|w| w.active_at(t))
}

static SCHEDULE: RwLock<Vec<BandwidthWindow>> = RwLock::new(Vec::new());

/// Replace the timetable every scheduled rate follows.
pub fn set_schedule(windows: Vec<BandwidthWindow>) -> Result<(), AppError> {
    validate_schedule(&windows)?;
    *SCHEDULE.write().unwrap_or_else(|e| e.into_inner()) = windows;
    Ok(())
}

/// The current timetable.
pub fn schedule() -> Vec<BandwidthWindow> {
    SCHEDULE.read().map(|s| s.clone()).unwrap_or_default()
}

/// Whether the timetable can ever limit uploads (some window has an upload
/// limit), active now or not: a long copy that should slow down when such a
/// window opens has to be paced block by block from the start.
pub fn limits_uploads() -> bool {
    SCHEDULE.read().map(|s| s.iter().any(|w| w.upload_bps > 0)).unwrap_or(false)
}

/// A rate shared by every transfer that goes through it: each transfer
/// reserves its share of the time line, so the sum of concurrent
/// transfers stays at or below the rate.
pub struct SharedRate {
    bps: AtomicU64,
    next: std::sync::Mutex<Option<Instant>>,
    /// Follows the timetable for this direction.
    scheduled: Option<Direction>,
    /// Outside every window: unlimited (the base rate is never used).
    window_only: bool,
}

impl SharedRate {
    pub const fn new() -> Self {
        Self { bps: AtomicU64::new(0), next: std::sync::Mutex::new(None), scheduled: None, window_only: false }
    }

    /// A rate whose limit the bandwidth timetable replaces while a window is active.
    pub const fn scheduled(direction: Direction) -> Self {
        Self { bps: AtomicU64::new(0), next: std::sync::Mutex::new(None), scheduled: Some(direction), window_only: false }
    }

    /// A rate that only the timetable sets: a window's limit while one is
    /// active, unlimited otherwise.
    pub const fn window_only(direction: Direction) -> Self {
        Self { bps: AtomicU64::new(0), next: std::sync::Mutex::new(None), scheduled: Some(direction), window_only: true }
    }

    /// Base rate in bytes per second; 0 = unlimited.
    pub fn set(&self, bps: u64) {
        self.bps.store(bps, Ordering::Relaxed);
    }

    /// The base rate (outside timetable windows).
    pub fn base(&self) -> u64 {
        self.bps.load(Ordering::Relaxed)
    }

    /// The rate in force now.
    pub fn get(&self) -> u64 {
        self.rate_at(Local::now().naive_local())
    }

    /// The rate in force at local time `t` under the current timetable.
    pub fn rate_at(&self, t: NaiveDateTime) -> u64 {
        self.rate_with(&SCHEDULE.read().unwrap_or_else(|e| e.into_inner()), t)
    }

    /// The rate at `t` under `windows` (for tests and previews). A window's
    /// 0 for this direction sets no limit of its own: the base rate stays
    /// in force (so a download-only window leaves the upload cap alone).
    pub fn rate_with(&self, windows: &[BandwidthWindow], t: NaiveDateTime) -> u64 {
        match self.scheduled.and_then(|d| window_at(windows, t).map(|w| w.limit(d))).filter(|&l| l > 0) {
            Some(limit) => limit,
            None if self.window_only => 0,
            None => self.base(),
        }
    }

    /// How long a transfer of `bytes` that started at `started` must still
    /// wait; reserves its slot.
    pub fn reserve(&self, bytes: usize, started: Instant) -> Duration {
        let bps = self.get();
        if bps == 0 {
            return Duration::ZERO;
        }
        let now = Instant::now();
        let mut next = self.next.lock().unwrap_or_else(|e| e.into_inner());
        // Idle time is not banked: a slot starts no earlier than this transfer did.
        let start = next.map_or(started, |n| n.max(started));
        let end = start + Duration::from_secs_f64(bytes as f64 / bps as f64);
        *next = Some(end);
        end.saturating_duration_since(now)
    }

    /// Block the thread for the transfer's share of the time line.
    pub fn wait(&self, bytes: usize, started: Instant) {
        let d = self.reserve(bytes, started);
        if !d.is_zero() {
            std::thread::sleep(d);
        }
    }
}

impl Default for SharedRate {
    fn default() -> Self {
        Self::new()
    }
}

/// Backup machine-wide upload and download caps (on top of each job's own
/// throttle); a timetable window's non-zero limits replace them while active.
pub static GLOBAL_UPLOAD: SharedRate = SharedRate::scheduled(Direction::Upload);
pub static GLOBAL_DOWNLOAD: SharedRate = SharedRate::scheduled(Direction::Download);

/// The timetable's upload limit, shared by backups, sync and transfers
/// (unlimited outside every window).
pub static TIMETABLE_UPLOAD: SharedRate = SharedRate::window_only(Direction::Upload);

/// Reserve `bytes` on every rate in `rates`; the wait is the longest.
pub fn reserve_all(rates: &[&SharedRate], bytes: usize, started: Instant) -> Duration {
    rates.iter().map(|r| r.reserve(bytes, started)).max().unwrap_or(Duration::ZERO)
}

/// Block the thread until `bytes` fit every rate in `rates`.
pub fn wait_all(rates: &[&SharedRate], bytes: usize, started: Instant) {
    let d = reserve_all(rates, bytes, started);
    if !d.is_zero() {
        std::thread::sleep(d);
    }
}

/// Serializes tests that change the process-wide rates or timetable.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    /// 2026-10-19 is a Monday.
    fn at(day: u32, h: u32, m: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, day).unwrap().and_hms_opt(h, m, 0).unwrap()
    }

    fn window(days: &[u8], start: &str, end: &str, up: u64) -> BandwidthWindow {
        BandwidthWindow { days: days.to_vec(), start: start.into(), end: end.into(), upload_bps: up, download_bps: up * 2 }
    }

    #[test]
    fn timetable_with_an_injected_clock() {
        // Work hours on weekdays: 1 MB/s; Friday nights wrap past midnight.
        let windows = vec![window(&[0, 1, 2, 3, 4], "08:00", "18:00", 1_000_000), window(&[4], "22:00", "06:00", 5_000)];
        validate_schedule(&windows).unwrap();
        let up = SharedRate::scheduled(Direction::Upload);
        up.set(10_000_000);
        assert_eq!(up.rate_with(&windows, at(19, 9, 30)), 1_000_000, "Monday 09:30");
        assert_eq!(up.rate_with(&windows, at(19, 18, 0)), 10_000_000, "end is exclusive");
        assert_eq!(up.rate_with(&windows, at(25, 9, 30)), 10_000_000, "Sunday: base rate");
        assert_eq!(up.rate_with(&windows, at(23, 23, 0)), 5_000, "Friday 23:00");
        assert_eq!(up.rate_with(&windows, at(24, 2, 0)), 5_000, "Saturday 02:00 belongs to Friday night");
        assert_eq!(up.rate_with(&windows, at(25, 2, 0)), 10_000_000, "Sunday 02:00 does not");
        let down = SharedRate::scheduled(Direction::Download);
        assert_eq!(down.rate_with(&windows, at(19, 9, 30)), 2_000_000);
        // A window's 0 leaves that direction's normal limit in force (it does
        // not lift it); an unscheduled rate ignores the timetable.
        let open = vec![window(&[], "00:00", "00:00", 0)];
        assert_eq!(up.rate_with(&open, at(19, 12, 0)), 10_000_000);
        let plain = SharedRate::new();
        plain.set(42);
        assert_eq!(plain.rate_with(&windows, at(19, 9, 30)), 42);
    }

    /// Regression (review): a window that limits only downloads (upload 0)
    /// lifted the backup machine-wide upload cap to unlimited while active.
    #[test]
    fn download_only_window_keeps_the_upload_cap() {
        let windows = vec![BandwidthWindow { days: vec![], start: "08:00".into(), end: "18:00".into(), upload_bps: 0, download_bps: 2_000 }];
        let up = SharedRate::scheduled(Direction::Upload);
        up.set(500_000);
        assert_eq!(up.rate_with(&windows, at(19, 9, 0)), 500_000, "the upload cap stays");
        let down = SharedRate::scheduled(Direction::Download);
        down.set(9_000);
        assert_eq!(down.rate_with(&windows, at(19, 9, 0)), 2_000, "the window's download limit applies");
        // Without a base cap the direction stays unlimited.
        assert_eq!(SharedRate::scheduled(Direction::Upload).rate_with(&windows, at(19, 9, 0)), 0);
        // The timetable-only rate has no cap outside its limits either.
        assert_eq!(SharedRate::window_only(Direction::Upload).rate_with(&windows, at(19, 9, 0)), 0);
    }

    #[test]
    fn timetable_rate_has_no_base() {
        let windows = vec![window(&[], "08:00", "18:00", 1_000)];
        let tt = SharedRate::window_only(Direction::Upload);
        tt.set(5); // never used
        assert_eq!(tt.rate_with(&windows, at(19, 7, 0)), 0, "outside every window: unlimited");
        assert_eq!(tt.rate_with(&windows, at(19, 9, 0)), 1_000);
        assert_eq!(tt.rate_with(&[], at(19, 9, 0)), 0);
        // The longest wait of several rates wins.
        let (a, b) = (SharedRate::new(), SharedRate::new());
        b.set(1_000);
        let t = Instant::now();
        b.reserve(1_000, t);
        assert!(reserve_all(&[&a, &b], 1_000, t) >= Duration::from_millis(900));
    }

    #[test]
    fn invalid_windows_are_refused() {
        assert!(window(&[7], "08:00", "09:00", 1).validate().is_err());
        assert!(window(&[], "8am", "09:00", 1).validate().is_err());
        assert!(window(&[], "24:00", "09:00", 1).validate().is_err());
        assert!(window(&[], "0800", "0900", 1).validate().is_ok());
    }

    #[test]
    fn reserve_spreads_transfers() {
        let r = SharedRate::new();
        assert_eq!(r.reserve(1000, Instant::now()), Duration::ZERO, "unlimited");
        r.set(1_000_000);
        let t = Instant::now();
        let first = r.reserve(500_000, t);
        let second = r.reserve(500_000, t);
        assert!(second > first && second >= Duration::from_millis(900), "{first:?} {second:?}");
    }
}
