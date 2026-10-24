//! Robocopy-equivalent file copy primitives used by the sync executor.
//!
//! - Restartable copy through a `.ufop-partial` file (/Z)
//! - Bandwidth throttling with an inter-packet gap (/IPG) and the shared
//!   machine-wide limit / timetable (`crate::throttle`)
//! - Timestamp preservation (/COPY:T, /DCOPY:T)
//! - Symbolic links copied as links (/SL)
//! - Zero-length "tree only" copies (/CREATE)
//! - Per-file retries with a wait (/R, /W)
//! - Run-hours windows (/RH)
//! - Robocopy-style run logs (/LOG, /LOG+)

use crate::error::AppError;
use crate::sync_types::*;
use chrono::{Local, Timelike};
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Block size used for throttled / restartable copies (matches Robocopy's 64 KiB).
const BLOCK_SIZE: usize = 64 * 1024;

/// Suffix of the in-progress file written in restartable mode.
pub const PARTIAL_SUFFIX: &str = ".ufop-partial";

fn sync_err(message: String, advice: &str) -> AppError {
    AppError::Sync {
        message,
        advice: advice.to_string(),
    }
}

/// Copy one file honouring the pair's copy options. Returns bytes written.
pub fn copy_file(src: &Path, dst: &Path, opts: &CopyOptions) -> Result<u64, AppError> {
    if let Some(parent) = dst.parent() {
        fs::create_dir_all(parent).map_err(|e| {
            sync_err(
                format!("Cannot create directory {}: {}", parent.display(), e),
                "Check destination permissions.",
            )
        })?;
    }

    #[cfg(unix)]
    if opts.symlinks == SymlinkMode::CopyLink {
        if let Ok(meta) = fs::symlink_metadata(src) {
            if meta.file_type().is_symlink() {
                let target = fs::read_link(src)?;
                if fs::symlink_metadata(dst).is_ok() {
                    fs::remove_file(dst)?;
                }
                std::os::unix::fs::symlink(&target, dst).map_err(|e| {
                    sync_err(
                        format!("Cannot create link {}: {}", dst.display(), e),
                        "Check destination permissions and that the filesystem supports links.",
                    )
                })?;
                return Ok(0);
            }
        }
    }

    let bytes = if opts.create_only {
        fs::File::create(dst).map_err(|e| {
            sync_err(
                format!("Cannot create {}: {}", dst.display(), e),
                "Check destination permissions.",
            )
        })?;
        0
    } else if copies_blockwise(opts) {
        copy_blockwise(src, dst, opts)?
    } else {
        fs::copy(src, dst).map_err(|e| {
            sync_err(
                format!("Copy failed {}: {}", src.display(), e),
                "Check file permissions.",
            )
        })?
    };

    if opts.copy_timestamps {
        if let Ok(modified) = fs::metadata(src).and_then(|m| m.modified()) {
            set_mtime(dst, modified);
        }
    }
    // /COPY:SOU, /A+ /A-, /M — Windows only; a no-op elsewhere.
    super::winattr::after_copy(src, dst, opts)?;
    Ok(bytes)
}

/// Reject option values the engine would otherwise have to guess about.
pub fn validate_options(opts: &CopyOptions) -> Result<(), AppError> {
    if !opts.run_hours.trim().is_empty() && parse_run_hours(&opts.run_hours).is_none() {
        return Err(sync_err(
            format!(
                "\"{}\" is not a valid run-hours window.",
                opts.run_hours.trim()
            ),
            "Use HHMM-HHMM, for example 2200-0600.",
        ));
    }
    for letters in [
        &opts.include_attributes,
        &opts.exclude_attributes,
        &opts.add_attributes,
        &opts.remove_attributes,
    ] {
        super::winattr::parse_letters(letters)?;
    }
    if opts.threads == 0 || opts.threads > 128 {
        return Err(sync_err(
            format!("{} parallel copies is out of range.", opts.threads),
            "Use between 1 and 128 threads.",
        ));
    }
    Ok(())
}

/// Whether files are copied block by block: for /Z, /IPG, or whenever the
/// bandwidth timetable (shared with backups and transfers; the backup caps
/// do not apply here) has an upload limit, so that a window opening during
/// a copy paces the rest of that file too; each block asks for the rate in
/// force at that moment.
fn copies_blockwise(opts: &CopyOptions) -> bool {
    opts.restartable || opts.inter_packet_gap_ms > 0 || crate::throttle::limits_uploads()
}

/// Block copy with optional resume (/Z) and inter-packet gap (/IPG).
fn copy_blockwise(src: &Path, dst: &Path, opts: &CopyOptions) -> Result<u64, AppError> {
    let mut reader = fs::File::open(src).map_err(|e| {
        sync_err(
            format!("Cannot open {}: {}", src.display(), e),
            "Check file permissions.",
        )
    })?;
    let src_len = reader.metadata().map(|m| m.len()).unwrap_or(0);

    let target: PathBuf = if opts.restartable {
        partial_path(dst)
    } else {
        dst.to_path_buf()
    };

    // Resume from whatever a previous interrupted run already wrote.
    let resume_from = if opts.restartable {
        fs::metadata(&target)
            .map(|m| m.len())
            .ok()
            .filter(|&len| len <= src_len)
            .unwrap_or(0)
    } else {
        0
    };

    let mut writer = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(resume_from == 0)
        .open(&target)
        .map_err(|e| {
            sync_err(
                format!("Cannot write {}: {}", target.display(), e),
                "Check destination permissions.",
            )
        })?;
    if resume_from > 0 {
        reader.seek(SeekFrom::Start(resume_from))?;
        writer.seek(SeekFrom::Start(resume_from))?;
    }

    let gap = Duration::from_millis(opts.inter_packet_gap_ms as u64);
    let mut buffer = vec![0u8; BLOCK_SIZE];
    let mut written = 0u64;
    loop {
        let block_started = std::time::Instant::now();
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        writer.write_all(&buffer[..n])?;
        written += n as u64;
        crate::throttle::TIMETABLE_UPLOAD.wait(n, block_started);
        if !gap.is_zero() {
            std::thread::sleep(gap);
        }
    }
    writer.flush()?;
    drop(writer);

    if let Ok(perms) = fs::metadata(src).map(|m| m.permissions()) {
        let _ = fs::set_permissions(&target, perms);
    }
    if opts.restartable {
        fs::rename(&target, dst).map_err(|e| {
            sync_err(
                format!("Cannot finalize {}: {}", dst.display(), e),
                "Check destination permissions.",
            )
        })?;
    }
    Ok(resume_from + written)
}

/// Path of the in-progress file for a restartable copy.
pub fn partial_path(dst: &Path) -> PathBuf {
    let mut name = dst.file_name().unwrap_or_default().to_os_string();
    name.push(PARTIAL_SUFFIX);
    dst.with_file_name(name)
}

/// Best-effort: set a file or directory's modification time.
pub fn set_mtime(path: &Path, time: std::time::SystemTime) {
    let file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .or_else(|_| fs::File::open(path));
    if let Ok(f) = file {
        let _ = f.set_modified(time);
    }
}

/// Run `op` with Robocopy-style retries (/R:n, /W:n).
pub fn with_retries<T>(
    opts: &CopyOptions,
    mut op: impl FnMut() -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut attempt = 0;
    loop {
        match op() {
            Ok(v) => return Ok(v),
            Err(e) if attempt < opts.retries => {
                attempt += 1;
                tracing::debug!("retry {attempt}/{} after error: {e}", opts.retries);
                std::thread::sleep(Duration::from_secs(opts.retry_wait_secs as u64));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Parse a /RH window "HHMM-HHMM" into minutes-of-day. `None` = invalid.
pub fn parse_run_hours(spec: &str) -> Option<(u32, u32)> {
    let (a, b) = spec.trim().split_once('-')?;
    let parse = |s: &str| -> Option<u32> {
        let s = s.trim().replace(':', "");
        if s.len() != 4 || !s.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        let h: u32 = s[..2].parse().ok()?;
        let m: u32 = s[2..].parse().ok()?;
        (h <= 23 && m <= 59).then_some(h * 60 + m)
    };
    Some((parse(a)?, parse(b)?))
}

/// True when `minute_of_day` falls inside the window (windows may wrap midnight).
pub fn in_window(window: (u32, u32), minute_of_day: u32) -> bool {
    let (start, end) = window;
    if start == end {
        true
    } else if start < end {
        minute_of_day >= start && minute_of_day < end
    } else {
        minute_of_day >= start || minute_of_day < end
    }
}

/// True when copying is allowed right now under the pair's /RH setting.
pub fn within_run_hours(opts: &CopyOptions) -> bool {
    if opts.run_hours.trim().is_empty() {
        return true;
    }
    match parse_run_hours(&opts.run_hours) {
        Some(window) => {
            let now = Local::now();
            in_window(window, now.hour() * 60 + now.minute())
        }
        None => true,
    }
}

/// Robocopy-compatible exit code bitmask.
pub fn exit_code(copied: u64, extras: u64, mismatches: u64, failures: u32) -> u8 {
    let mut code = 0;
    if copied > 0 {
        code |= 1;
    }
    if extras > 0 {
        code |= 2;
    }
    if mismatches > 0 {
        code |= 4;
    }
    if failures > 0 {
        code |= 8;
    }
    code
}

/// Plain-language meaning of a Robocopy exit code.
pub fn describe_exit_code(code: u8) -> &'static str {
    match code {
        0 => "No changes — source and destination already in sync.",
        1 => "Files copied successfully.",
        2 => "Extra files found at the destination; nothing copied.",
        3 => "Files copied; extra files found at the destination.",
        c if c & 16 != 0 => "Fatal error — nothing was copied.",
        c if c & 8 != 0 => "Some files could not be copied.",
        _ => "Completed with mismatches — check the report.",
    }
}

/// Append a Robocopy-style run log to `opts.log_file` (/LOG, /LOG+).
pub fn write_log(pair: &SyncPair, report: &SyncReport, lines: &[String]) -> Result<(), AppError> {
    let opts = &pair.copy_options;
    if opts.log_file.trim().is_empty() {
        return Ok(());
    }
    let path = Path::new(opts.log_file.trim());
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent)?;
        }
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(opts.log_append)
        .truncate(!opts.log_append)
        .open(path)
        .map_err(|e| {
            sync_err(
                format!("Cannot open log file {}: {}", path.display(), e),
                "Choose a log location you can write to.",
            )
        })?;

    let sep = "-".repeat(78);
    let mut out = String::new();
    out.push_str(&format!(
        "{sep}\n   UFOP Sync :: Robocopy-compatible log\n{sep}\n"
    ));
    out.push_str(&format!("  Started : {}\n", report.started_at.to_rfc2822()));
    out.push_str(&format!("   Source : {}\n", pair.source_path));
    out.push_str(&format!("     Dest : {}\n", pair.dest_path));
    out.push_str(&format!(
        "  Options : {}\n{sep}\n",
        super::robocopy::options_summary(pair)
    ));
    for line in lines {
        out.push_str(line);
        out.push('\n');
    }
    out.push_str(&format!(
        "{sep}\n   Copied : {}   Modified : {}   Deleted : {}   Skipped : {}   Failed : {}\n    Bytes : {}\n    Ended : {} ({} ms)\n Exit code: {} — {}\n\n",
        report.files_added,
        report.files_modified,
        report.files_deleted,
        report.files_skipped,
        report.errors,
        report.bytes_transferred,
        report.ended_at.to_rfc2822(),
        report.duration_ms,
        report.exit_code,
        describe_exit_code(report.exit_code),
    ));
    file.write_all(out.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Regression: the backup machine-wide upload cap (and the console's
    /// managed cap) also paced every sync copy, a change for anyone who had
    /// set one for backups. Sync follows only the bandwidth timetable.
    #[test]
    fn backup_machine_caps_do_not_slow_sync_copies() {
        let _g = crate::throttle::test_lock();
        let dir = TempDir::new().unwrap();
        let (src, dst) = (dir.path().join("a.bin"), dir.path().join("b.bin"));
        fs::write(&src, vec![1u8; 256 * 1024]).unwrap();
        crate::throttle::GLOBAL_UPLOAD.set(64 * 1024);
        let started = std::time::Instant::now();
        let r = copy_file(&src, &dst, &CopyOptions::default());
        crate::throttle::GLOBAL_UPLOAD.set(0);
        r.unwrap();
        assert!(started.elapsed() < Duration::from_secs(2), "paced by the backup cap: {:?}", started.elapsed());
        assert_eq!(fs::read(&dst).unwrap().len(), 256 * 1024);
    }

    /// Regression: block-by-block copying (where the timetable is applied)
    /// was chosen only when a window was active as a file started, so a
    /// window opening during a long copy did not slow that file. Any
    /// configured timetable now copies block by block, and each block asks
    /// the timetable for the rate in force at that moment.
    #[test]
    fn a_configured_timetable_copies_block_by_block() {
        let _g = crate::throttle::test_lock();
        let opts = CopyOptions::default();
        crate::throttle::set_schedule(vec![]).unwrap();
        assert!(!copies_blockwise(&opts), "no timetable: plain copy");
        // A window that is not active now (on another weekday).
        let today = chrono::Datelike::weekday(&chrono::Local::now()).num_days_from_monday() as u8;
        let later = crate::throttle::BandwidthWindow { days: vec![(today + 3) % 7], start: "00:00".into(), end: "23:59".into(), upload_bps: 1_000, download_bps: 0 };
        crate::throttle::set_schedule(vec![later]).unwrap();
        let blockwise = copies_blockwise(&opts);
        crate::throttle::set_schedule(vec![]).unwrap();
        assert!(blockwise, "a window can open mid-file");
    }

    #[test]
    fn restartable_copy_resumes_partial_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("big.bin");
        let dst = dir.path().join("out/big.bin");
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&src, &data).unwrap();
        fs::create_dir_all(dst.parent().unwrap()).unwrap();
        // Simulate an interrupted run that wrote the first 70 000 bytes.
        fs::write(partial_path(&dst), &data[..70_000]).unwrap();

        let opts = CopyOptions {
            restartable: true,
            ..Default::default()
        };
        let n = copy_file(&src, &dst, &opts).unwrap();
        assert_eq!(n, data.len() as u64);
        assert_eq!(fs::read(&dst).unwrap(), data);
        assert!(!partial_path(&dst).exists());
    }

    #[test]
    fn copy_preserves_mtime_by_default() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "x").unwrap();
        let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000_000);
        set_mtime(&src, old);
        copy_file(&src, &dst, &CopyOptions::default()).unwrap();
        assert_eq!(fs::metadata(&dst).unwrap().modified().unwrap(), old);
    }

    #[test]
    fn create_only_writes_zero_length_file() {
        let dir = TempDir::new().unwrap();
        let src = dir.path().join("a.txt");
        let dst = dir.path().join("b.txt");
        fs::write(&src, "content").unwrap();
        let opts = CopyOptions {
            create_only: true,
            ..Default::default()
        };
        copy_file(&src, &dst, &opts).unwrap();
        assert_eq!(fs::metadata(&dst).unwrap().len(), 0);
    }

    #[test]
    fn retries_until_success() {
        let opts = CopyOptions {
            retries: 3,
            retry_wait_secs: 0,
            ..Default::default()
        };
        let mut calls = 0;
        let r = with_retries(&opts, || {
            calls += 1;
            if calls < 3 {
                Err(sync_err("boom".into(), ""))
            } else {
                Ok(calls)
            }
        });
        assert_eq!(r.unwrap(), 3);
    }

    #[test]
    fn retries_give_up_after_limit() {
        let opts = CopyOptions {
            retries: 1,
            retry_wait_secs: 0,
            ..Default::default()
        };
        let mut calls = 0;
        let r: Result<(), _> = with_retries(&opts, || {
            calls += 1;
            Err(sync_err("boom".into(), ""))
        });
        assert!(r.is_err());
        assert_eq!(calls, 2);
    }

    #[test]
    fn run_hours_parse_and_window() {
        assert_eq!(parse_run_hours("2200-0600"), Some((1320, 360)));
        assert_eq!(parse_run_hours("22:00-06:00"), Some((1320, 360)));
        assert_eq!(parse_run_hours("2500-0600"), None);
        assert_eq!(parse_run_hours("nonsense"), None);
        // Wrapping window
        assert!(in_window((1320, 360), 23 * 60));
        assert!(in_window((1320, 360), 60));
        assert!(!in_window((1320, 360), 12 * 60));
        // Normal window
        assert!(in_window((540, 1020), 600));
        assert!(!in_window((540, 1020), 1020));
    }

    #[test]
    fn exit_code_bits() {
        assert_eq!(exit_code(0, 0, 0, 0), 0);
        assert_eq!(exit_code(5, 0, 0, 0), 1);
        assert_eq!(exit_code(5, 2, 0, 0), 3);
        assert_eq!(exit_code(0, 0, 1, 1), 12);
    }

    #[cfg(unix)]
    #[test]
    fn symlink_copied_as_link() {
        let dir = TempDir::new().unwrap();
        fs::write(dir.path().join("target.txt"), "t").unwrap();
        let src = dir.path().join("link");
        std::os::unix::fs::symlink("target.txt", &src).unwrap();
        let dst = dir.path().join("out/link");
        let opts = CopyOptions {
            symlinks: SymlinkMode::CopyLink,
            ..Default::default()
        };
        copy_file(&src, &dst, &opts).unwrap();
        assert_eq!(fs::read_link(&dst).unwrap(), PathBuf::from("target.txt"));
    }
}

#[cfg(test)]
mod validate_tests {
    use super::*;

    #[test]
    fn validates_options() {
        assert!(validate_options(&CopyOptions::default()).is_ok());
        let bad_hours = CopyOptions {
            run_hours: "late".into(),
            ..Default::default()
        };
        assert!(validate_options(&bad_hours).is_err());
        let bad_attr = CopyOptions {
            include_attributes: "Z".into(),
            ..Default::default()
        };
        assert!(validate_options(&bad_attr).is_err());
        let bad_threads = CopyOptions {
            threads: 0,
            ..Default::default()
        };
        assert!(validate_options(&bad_threads).is_err());
    }
}
