//! Run-level manager operations: back up every job at once and estimate
//! what the next backup of a job will do.

use super::{BackupManager, BackupRun};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use ufop_core::backup::estimate::{self, BackupEstimate, EstimateOptions};

/// Which jobs "Back up all now" starts.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RunAllFilter {
    /// Only these jobs (empty = every job).
    pub job_ids: Vec<String>,
    /// Only jobs with a schedule turned on.
    pub scheduled_only: bool,
    pub dry_run: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartedRun {
    pub job_id: String,
    pub job_name: String,
    pub task_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkippedRun {
    pub job_id: String,
    pub job_name: String,
    pub reason: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RunAllReport {
    pub started: Vec<StartedRun>,
    pub skipped: Vec<SkippedRun>,
}

/// Throughput of recent successful backups (bytes uploaded per second).
pub fn history_throughput(runs: &[BackupRun]) -> Option<u64> {
    let samples: Vec<(u64, u64)> = runs
        .iter()
        .filter(|r| r.operation == "backup" && (r.status == "success" || r.status == "warning"))
        .filter(|r| r.report.get("dry_run").and_then(|d| d.as_bool()) != Some(true))
        .filter_map(|r| Some((r.report.get("bytes_uploaded")?.as_u64()?, r.report.get("duration_ms")?.as_u64()?)))
        .collect();
    estimate::throughput_from_history(&samples)
}

impl BackupManager {
    /// Queue a backup of every matching job. Runs go through the normal
    /// queue, so at most `max_concurrent_runs` run at once; read-only and
    /// busy jobs are skipped with a reason.
    pub async fn run_all(&self, filter: RunAllFilter) -> Result<RunAllReport, AppError> {
        let mut jobs = self.list_jobs().await;
        jobs.sort_by_key(|j| j.name.to_lowercase());
        let mut report = RunAllReport::default();
        for id in filter.job_ids.iter().filter(|id| !jobs.iter().any(|j| &j.id == *id)) {
            report.skipped.push(SkippedRun { job_id: id.clone(), job_name: String::new(), reason: "no such job".into() });
        }
        for job in jobs.into_iter().filter(|j| filter.job_ids.is_empty() || filter.job_ids.contains(&j.id)) {
            let skip = |reason: &str| SkippedRun { job_id: job.id.clone(), job_name: job.name.clone(), reason: reason.into() };
            if job.read_only {
                report.skipped.push(skip("read-only (restore and browse only)"));
                continue;
            }
            if filter.scheduled_only && !job.schedule.enabled {
                report.skipped.push(skip("no schedule"));
                continue;
            }
            if self.is_job_busy(&job.id) {
                report.skipped.push(skip("already queued or running"));
                continue;
            }
            match self.start_backup(&job.id, filter.dry_run).await {
                Ok(task_id) => report.started.push(StartedRun { job_id: job.id.clone(), job_name: job.name.clone(), task_id }),
                Err(e) => report.skipped.push(skip(&e.user_message())),
            }
        }
        Ok(report)
    }

    /// What the next backup of a job reads and at most uploads, and how
    /// long it may take (from recent runs, or `assumed_bytes_per_sec`).
    pub async fn estimate(&self, job_id: &str, assumed_bytes_per_sec: Option<u64>) -> Result<BackupEstimate, AppError> {
        let job = self.effective_job(job_id).await?;
        let runs = self.run_history(Some(job_id.to_string()), 20).await?;
        let eo = EstimateOptions {
            history_bytes_per_sec: history_throughput(&runs),
            assumed_bytes_per_sec: assumed_bytes_per_sec.unwrap_or(estimate::DEFAULT_ASSUMED_BYTES_PER_SEC),
            ..Default::default()
        };
        let filter = job.filter.clone().compile()?;
        let sources: Vec<std::path::PathBuf> = job.sources.iter().filter(|s| !s.trim().is_empty()).map(std::path::PathBuf::from).collect();
        let opts = job.options.clone();
        let stdin = job.stdin_command.as_ref().filter(|c| c.is_set()).map(|c| c.virtual_path());
        let mut est = self.with_db(job_id, move |db| estimate::estimate(db, &sources, &filter, &opts, &eo)).await?;
        if let Some(p) = stdin {
            est.warnings.push(format!("The output of the backup command ({p}) is not included: its size is known only when it runs"));
        }
        Ok(est)
    }
}
