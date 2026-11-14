//! Local transfer execution is shared with the desktop guided flows.
use crate::{
    output::{self, ProgressEvent},
    OutputFormat,
};
use indicatif::ProgressBar;
use serde::Serialize;
use std::{path::Path, time::Instant};
use ufop_core::local_transfer::{self, TransferOptions, TransferOutcome};

#[derive(Serialize)]
struct TransferResult {
    status: &'static str,
    source: String,
    destination: String,
    #[serde(flatten)]
    outcome: TransferOutcome,
    duration_ms: u64,
    average_speed_bps: u64,
}

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    src: String,
    dest: String,
    overwrite: bool,
    resume: bool,
    verify: bool,
    parallel: u32,
    format: &OutputFormat,
    dry_run: bool,
    rate_limit: Option<u64>,
) -> Result<u8, Box<dyn std::error::Error>> {
    if dry_run {
        output::print_data(
            format,
            &serde_json::json!({
                "dry_run": true, "source": src, "destination": dest,
                "overwrite": overwrite, "resume": resume, "verify": verify,
                "parallel": parallel, "rate_limit_bps": rate_limit,
            }),
        );
        return Ok(0);
    }
    if src.contains("://") || dest.contains("://") {
        return Err("Remote transfer is unavailable in this command. Use a connected location in the desktop transfer queue.".into());
    }
    if resume || rate_limit.is_some() {
        return Err(
            "Use ufop copy with /Z or --limit for resumable or rate-limited local copies.".into(),
        );
    }
    let bar = matches!(format, OutputFormat::Human).then(ProgressBar::new_spinner);
    let start = Instant::now();
    let outcome = local_transfer::transfer(
        Path::new(&src),
        Path::new(&dest),
        TransferOptions {
            overwrite,
            verify,
            move_source: false,
        },
        |bytes, path| {
            if let Some(bar) = &bar {
                bar.set_message(format!("{bytes} bytes · {}", path.display()));
                bar.tick();
            } else {
                let event = ProgressEvent {
                    event: "progress".into(),
                    progress: 0.0,
                    bytes_transferred: bytes,
                    bytes_total: 0,
                    speed_bps: (bytes as f64 / start.elapsed().as_secs_f64().max(0.001)) as u64,
                    eta_seconds: None,
                    current_file: Some(path.display().to_string()),
                };
                eprintln!("{}", serde_json::to_string(&event).unwrap_or_default());
            }
        },
    )?;
    if let Some(bar) = bar {
        bar.finish_and_clear();
    }
    let partial = outcome.files_skipped > 0;
    let result = TransferResult {
        status: if partial { "partial" } else { "success" },
        source: src,
        destination: dest,
        duration_ms: start.elapsed().as_millis() as u64,
        average_speed_bps: (outcome.bytes_transferred as f64
            / start.elapsed().as_secs_f64().max(0.001)) as u64,
        outcome,
    };
    if matches!(format, OutputFormat::Human) {
        println!(
            "{} files copied, {} bytes written, {} skipped. {}",
            result.outcome.files_transferred,
            result.outcome.bytes_transferred,
            result.outcome.files_skipped,
            if result.outcome.verified {
                "Verified against the originals."
            } else {
                "Verification incomplete or not requested."
            }
        );
    } else {
        output::print_data(format, &result);
    }
    Ok(if partial { 1 } else { 0 })
}
