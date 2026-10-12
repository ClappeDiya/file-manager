//! `ufop copy` — a Robocopy-compatible copy that runs the same engine as the
//! desktop app's sync pairs.
//!
//! ```text
//! ufop copy C:\Data D:\Backup /MIR /R:3 /W:5 /MT:8
//! ufop copy ~/Photos /mnt/nas/photos *.jpg /E /XO /LOG+:photos.log
//! ```
//!
//! Exit codes follow Robocopy (not the CLI's usual 0–4), so existing scripts
//! and Task Scheduler jobs that check `ERRORLEVEL` keep working:
//! 0 nothing to do, 1 copied, 2 extras, 4 mismatches, 8 failures, 16 fatal.

use crate::output;
use crate::OutputFormat;
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use ufop_core::sync::executor::{self, ExecutorConfig, RunHooks, SyncProgress};
use ufop_core::sync::{copier, planner, robocopy};
use ufop_core::sync_types::{SyncPair, SyncPreview, SyncReport, SyncRunStatus};

/// Robocopy's "fatal error" exit code.
pub const EXIT_FATAL: u8 = 16;

#[derive(Serialize)]
struct ListOnly<'a> {
    list_only: bool,
    preview: &'a SyncPreview,
    exit_code: u8,
}

/// Build the sync pair a Robocopy job describes.
pub fn pair_from_args(
    args: &[String],
    rate_limit_bps: Option<u64>,
) -> Result<(SyncPair, robocopy::RobocopyJob), String> {
    let job = robocopy::parse_args(args).map_err(|e| e.to_string())?;
    let mut opts = job.copy_options.clone();
    // --limit maps onto Robocopy's inter-packet gap: one 64 KiB block per gap.
    if let Some(bps) = rate_limit_bps.filter(|b| *b > 0) {
        opts.inter_packet_gap_ms = opts
            .inter_packet_gap_ms
            .max(((64 * 1024 * 1000) / bps).max(1) as u32);
    }
    copier::validate_options(&opts).map_err(|e| e.to_string())?;
    let pair = SyncPair {
        name: format!("{} → {}", job.source_path, job.dest_path),
        source_path: job.source_path.clone(),
        dest_path: job.dest_path.clone(),
        mode: job.mode,
        filter: job.filter.clone(),
        copy_options: opts,
        ..SyncPair::default()
    };
    Ok((pair, job))
}

/// Exit code for a list-only (/L) run, as Robocopy reports it.
pub fn list_only_exit_code(p: &SyncPreview) -> u8 {
    copier::exit_code(
        p.total_additions + p.total_modifications,
        p.total_deletions,
        p.total_conflicts,
        0,
    )
}

fn state_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("ufop")
        .join("cli-copy")
}

fn print_notes(format: &OutputFormat, job: &robocopy::RobocopyJob) {
    for note in &job.unsupported {
        output::print_warning(format, note);
    }
    if matches!(format, OutputFormat::Human) {
        for note in &job.ignored {
            eprintln!("  note: {note}");
        }
    }
}

fn print_header(pair: &SyncPair) {
    let sep = "-".repeat(78);
    println!("{sep}");
    println!("   UFOP copy :: Robocopy-compatible");
    println!("{sep}");
    println!("   Source : {}", pair.source_path);
    println!("     Dest : {}", pair.dest_path);
    println!("  Options : {}", robocopy::options_summary(pair));
    println!("{sep}");
}

fn print_summary(report: &SyncReport) {
    let sep = "-".repeat(78);
    println!("{sep}");
    println!(
        "   Copied : {}   Updated : {}   Deleted : {}   Skipped : {}   Failed : {}",
        report.files_added,
        report.files_modified,
        report.files_deleted,
        report.files_skipped,
        report.errors
    );
    println!("    Bytes : {}", report.bytes_transferred);
    println!("    Time  : {:.1}s", report.duration_ms as f64 / 1000.0);
    println!(
        "Exit code : {} — {}",
        report.exit_code,
        copier::describe_exit_code(report.exit_code)
    );
    for msg in &report.error_messages {
        eprintln!("  {msg}");
    }
}

/// Run one copy pass; returns the Robocopy exit code.
async fn run_once(pair: &SyncPair, format: &OutputFormat, cancel: Arc<AtomicBool>) -> u8 {
    let run_id = uuid::Uuid::new_v4();
    let config = ExecutorConfig {
        pair: pair.clone(),
        run_id,
        continue_on_error: true,
        resume_from: None,
        quarantine_dir: Some(state_dir().join("quarantine")),
        rollback_dir: Some(state_dir().join("rollback").join(run_id.to_string())),
    };
    let human = matches!(format, OutputFormat::Human);
    let bar = human.then(|| {
        let bar = indicatif::ProgressBar::new(0);
        bar.set_style(
            indicatif::ProgressStyle::with_template(
                "{spinner} [{bar:30}] {bytes}/{total_bytes} {msg}",
            )
            .unwrap_or_else(|_| indicatif::ProgressStyle::default_bar())
            .progress_chars("=> "),
        );
        bar
    });
    let bar_cb = bar.clone();
    let hooks = RunHooks {
        progress: Some(Arc::new(move |p: &SyncProgress| {
            if let Some(b) = &bar_cb {
                b.set_length(p.bytes_total);
                b.set_position(p.bytes_done);
                b.set_message(match (&p.current_file, p.phase.as_str()) {
                    (_, "planning") => "comparing folders…".to_string(),
                    (Some(f), _) => format!("{}/{} {f}", p.files_done, p.files_total),
                    (None, _) => format!("{}/{}", p.files_done, p.files_total),
                });
            }
        })),
        cancel: Some(cancel),
    };

    let result =
        tokio::task::spawn_blocking(move || executor::execute_sync_with(&config, &hooks)).await;
    if let Some(b) = &bar {
        b.finish_and_clear();
    }
    match result {
        Ok(Ok(outcome)) => {
            let report = outcome.report;
            if human {
                print_summary(&report);
            } else {
                output::print_data(format, &report);
            }
            // A cancelled run is incomplete: report it as a failure, like
            // Robocopy does when it's interrupted.
            if report.status == SyncRunStatus::Cancelled {
                report.exit_code | 8
            } else {
                report.exit_code
            }
        }
        Ok(Err(e)) => {
            output::print_error(format, &e.to_string());
            EXIT_FATAL
        }
        Err(e) => {
            output::print_error(format, &format!("Copy task failed: {e}"));
            EXIT_FATAL
        }
    }
}

fn changes_in(p: &SyncPreview) -> u64 {
    p.total_additions + p.total_modifications + p.total_deletions
}

pub async fn execute(
    args: Vec<String>,
    format: &OutputFormat,
    dry_run: bool,
    rate_limit_bps: Option<u64>,
) -> Result<u8, Box<dyn std::error::Error>> {
    let (pair, job) = match pair_from_args(&args, rate_limit_bps) {
        Ok(v) => v,
        Err(e) => {
            output::print_error(format, &e);
            return Ok(EXIT_FATAL);
        }
    };
    print_notes(format, &job);
    if !std::path::Path::new(&pair.source_path).is_dir() {
        output::print_error(
            format,
            &format!("Source folder not found: {}", pair.source_path),
        );
        return Ok(EXIT_FATAL);
    }

    // /L or --dry-run: list what would happen, change nothing.
    if job.list_only || dry_run {
        let p = pair.clone();
        let preview =
            match tokio::task::spawn_blocking(move || planner::compute_preview(&p)).await? {
                Ok(p) => p,
                Err(e) => {
                    output::print_error(format, &e.to_string());
                    return Ok(EXIT_FATAL);
                }
            };
        let code = list_only_exit_code(&preview);
        if matches!(format, OutputFormat::Human) {
            print_header(&pair);
            for (label, list) in [
                ("New File", &preview.additions),
                ("Newer", &preview.modifications),
                ("*EXTRA File", &preview.deletions),
                ("Conflict", &preview.conflicts),
            ] {
                for e in list {
                    println!(
                        "  {label:<12} {:>12}  {}",
                        e.source_size.or(e.dest_size).unwrap_or(0),
                        e.relative_path
                    );
                }
            }
            for d in &preview.dirs_to_create {
                println!("  {:<12} {:>12}  {d}", "New Dir", "");
            }
            for d in &preview.dirs_to_remove {
                println!("  {:<12} {:>12}  {d}", "*EXTRA Dir", "");
            }
            println!(
                "\n  List only: {} new, {} newer, {} extra, {} skipped — nothing was changed.",
                preview.total_additions,
                preview.total_modifications,
                preview.total_deletions,
                preview.total_skipped
            );
            println!("Exit code : {code} — {}", copier::describe_exit_code(code));
        } else {
            output::print_data(
                format,
                &ListOnly {
                    list_only: true,
                    preview: &preview,
                    exit_code: code,
                },
            );
        }
        return Ok(code);
    }

    // Ctrl+C stops after the file being copied, like the desktop Cancel.
    let cancel = Arc::new(AtomicBool::new(false));
    {
        let cancel = cancel.clone();
        tokio::spawn(async move {
            if tokio::signal::ctrl_c().await.is_ok() {
                cancel.store(true, Ordering::Relaxed);
                eprintln!("\nStopping after the current file… (Ctrl+C again to force quit)");
                if tokio::signal::ctrl_c().await.is_ok() {
                    std::process::exit(EXIT_FATAL as i32);
                }
            }
        });
    }

    if matches!(format, OutputFormat::Human) {
        print_header(&pair);
    }
    let mut code = run_once(&pair, format, cancel.clone()).await;

    // /MON:n and /MOT:m keep watching the source and copy again, as Robocopy does.
    if job.monitor_changes.is_none() && job.monitor_minutes.is_none() {
        return Ok(code);
    }
    let min_changes = job.monitor_changes.unwrap_or(1) as u64;
    let interval = Duration::from_secs(job.monitor_minutes.map(|m| m as u64 * 60).unwrap_or(60));
    if matches!(format, OutputFormat::Human) {
        println!(
            "\nMonitoring {} — copying again after {} change(s), checking every {}s. Ctrl+C to stop.",
            pair.source_path,
            min_changes,
            interval.as_secs()
        );
    }
    while !cancel.load(Ordering::Relaxed) {
        tokio::select! {
            _ = tokio::time::sleep(interval) => {}
            _ = wait_for(&cancel) => break,
        }
        let p = pair.clone();
        let changed = tokio::task::spawn_blocking(move || planner::compute_preview(&p))
            .await
            .ok()
            .and_then(|r| r.ok())
            .map(|p| changes_in(&p))
            .unwrap_or(0);
        if changed >= min_changes {
            code = run_once(&pair, format, cancel.clone()).await;
        }
    }
    Ok(code)
}

async fn wait_for(flag: &AtomicBool) {
    while !flag.load(Ordering::Relaxed) {
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ufop_core::sync_types::SyncMode;

    fn args(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn builds_pair_from_robocopy_args() {
        let (pair, job) =
            pair_from_args(&args(&["src", "dst", "*.txt", "/MIR", "/R:2"]), None).unwrap();
        assert_eq!(pair.mode, SyncMode::Mirror);
        assert_eq!(pair.filter.include_patterns, vec!["*.txt"]);
        assert_eq!(pair.copy_options.retries, 2);
        assert!(!job.list_only);
    }

    #[test]
    fn rate_limit_becomes_inter_packet_gap() {
        // 64 KiB/s → one 64 KiB block per second.
        let (pair, _) = pair_from_args(&args(&["a", "b"]), Some(64 * 1024)).unwrap();
        assert_eq!(pair.copy_options.inter_packet_gap_ms, 1000);
        // An explicit, larger /IPG wins.
        let (pair, _) = pair_from_args(&args(&["a", "b", "/IPG:5000"]), Some(64 * 1024)).unwrap();
        assert_eq!(pair.copy_options.inter_packet_gap_ms, 5000);
    }

    #[test]
    fn rejects_bad_switches() {
        assert!(pair_from_args(&args(&["a", "b", "/R:x"]), None).is_err());
        assert!(pair_from_args(&args(&["onlysource"]), None).is_err());
    }

    #[tokio::test]
    async fn copies_and_returns_robocopy_exit_codes() {
        let src = tempfile::TempDir::new().unwrap();
        let dst = tempfile::TempDir::new().unwrap();
        std::fs::write(src.path().join("a.txt"), "hello").unwrap();
        let a = |extra: &[&str]| {
            let mut v = vec![
                src.path().to_string_lossy().to_string(),
                dst.path().to_string_lossy().to_string(),
            ];
            v.extend(extra.iter().map(|s| s.to_string()));
            v
        };
        let fmt = OutputFormat::Json;

        // /L lists without copying.
        assert_eq!(execute(a(&["/L"]), &fmt, false, None).await.unwrap(), 1);
        assert!(!dst.path().join("a.txt").exists());
        // A real run copies (exit 1)…
        assert_eq!(execute(a(&[]), &fmt, false, None).await.unwrap(), 1);
        assert_eq!(
            std::fs::read_to_string(dst.path().join("a.txt")).unwrap(),
            "hello"
        );
        // …and a second run has nothing to do (exit 0).
        assert_eq!(execute(a(&[]), &fmt, false, None).await.unwrap(), 0);
        // Extras at the destination under /MIR are purged (1 deleted → bit 2).
        std::fs::write(dst.path().join("extra.txt"), "x").unwrap();
        assert_eq!(execute(a(&["/MIR"]), &fmt, false, None).await.unwrap(), 2);
        assert!(!dst.path().join("extra.txt").exists());
        // Missing source is fatal.
        let missing = vec!["/definitely/not/here".to_string(), "x".to_string()];
        assert_eq!(
            execute(missing, &fmt, false, None).await.unwrap(),
            EXIT_FATAL
        );
    }
}
