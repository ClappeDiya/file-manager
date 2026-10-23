//! Per-job figures for monitoring (`ufop-server`'s `/metrics`), read from
//! the run history (`backup_runs`), so they survive restarts.

use super::BackupManager;
use crate::core::error::AppError;
use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct RunCount {
    pub job_id: String,
    pub operation: String,
    /// `success`, `warning`, `error` or `cancelled`.
    pub status: String,
    pub count: u64,
}

/// The newest backup run of a job.
#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct LastBackup {
    pub job_id: String,
    pub status: String,
    pub duration_secs: f64,
    pub bytes_uploaded: u64,
    pub files_examined: u64,
    /// Unix time of the newest successful (or warning) backup, if any.
    pub last_success_unix: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct RunMetrics {
    pub counts: Vec<RunCount>,
    pub last: Vec<LastBackup>,
}

fn unix(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts).ok().map(|t| t.timestamp())
}

impl BackupManager {
    pub async fn run_metrics(&self) -> Result<RunMetrics, AppError> {
        self.inner
            .pool
            .execute(|c| {
                let mut st = c.prepare("SELECT job_id, operation, status, COUNT(*) FROM backup_runs GROUP BY job_id, operation, status ORDER BY job_id, operation, status")?;
                let counts = st
                    .query_map([], |r| Ok(RunCount { job_id: r.get(0)?, operation: r.get(1)?, status: r.get(2)?, count: r.get::<_, i64>(3)? as u64 }))?
                    .collect::<Result<Vec<_>, _>>()?;
                let mut st = c.prepare("SELECT job_id, MAX(finished_at) FROM backup_runs WHERE operation = 'backup' AND status IN ('success', 'warning') GROUP BY job_id")?;
                let success: std::collections::HashMap<String, Option<String>> = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
                let mut st = c.prepare(
                    "SELECT r.job_id, r.status, r.started_at, r.finished_at, r.report_json FROM backup_runs r
                     WHERE r.operation = 'backup' AND r.started_at = (SELECT MAX(started_at) FROM backup_runs WHERE job_id = r.job_id AND operation = 'backup')
                     ORDER BY r.job_id",
                )?;
                let last = st
                    .query_map([], |r| {
                        let (job_id, status, started, finished, report): (String, String, String, Option<String>, Option<String>) = (r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?);
                        Ok((job_id, status, started, finished, report))
                    })?
                    .collect::<Result<Vec<_>, _>>()?
                    .into_iter()
                    .map(|(job_id, status, started, finished, report)| {
                        let rep: serde_json::Value = report.and_then(|r| serde_json::from_str(&r).ok()).unwrap_or_default();
                        let n = |k: &str| rep.get(k).and_then(|v| v.as_u64()).unwrap_or(0);
                        let duration_secs = match (unix_ms(&started), finished.as_deref().and_then(unix_ms)) {
                            (Some(a), Some(b)) if b >= a => (b - a) as f64 / 1000.0,
                            _ => 0.0,
                        };
                        let last_success_unix = success.get(&job_id).cloned().flatten().as_deref().and_then(unix);
                        LastBackup { job_id, status, duration_secs, bytes_uploaded: n("bytes_uploaded"), files_examined: n("files_examined"), last_success_unix }
                    })
                    .collect();
                Ok(RunMetrics { counts, last })
            })
            .await
    }
}

fn unix_ms(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts).ok().map(|t| t.timestamp_millis())
}
