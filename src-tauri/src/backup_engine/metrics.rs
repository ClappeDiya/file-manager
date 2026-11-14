//! Per-job figures for monitoring, read from the run history
//! (`backup_runs`), so they survive restarts, and their Prometheus text
//! exposition (format 0.0.4, written by hand). `ufop-server`'s `/metrics`
//! and the desktop app's optional loopback exporter (`metrics_exporter`)
//! both render the backup part with [`render_backup`].

use super::BackupManager;
use crate::core::error::AppError;
use serde::Serialize;
use std::collections::BTreeMap;
use std::fmt::Write;

/// A label value escaped for the text format.
pub fn esc(v: &str) -> String {
    v.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n")
}

fn labels(pairs: &[(&str, &str)]) -> String {
    if pairs.is_empty() {
        return String::new();
    }
    let inner: Vec<String> = pairs.iter().map(|(k, v)| format!("{k}=\"{}\"", esc(v))).collect();
    format!("{{{}}}", inner.join(","))
}

/// A text exposition being written.
#[derive(Default)]
pub struct Exposition(pub String);

impl Exposition {
    pub fn head(&mut self, name: &str, kind: &str, help: &str) {
        let _ = writeln!(self.0, "# HELP {name} {help}\n# TYPE {name} {kind}");
    }
    pub fn sample(&mut self, name: &str, l: &[(&str, &str)], v: impl std::fmt::Display) {
        let _ = writeln!(self.0, "{name}{} {v}", labels(l));
    }
}

fn ts_secs(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| t.timestamp())
}

/// Backup jobs, runs, the scheduler, running operations and the volume cache.
pub async fn render_backup(mgr: &BackupManager, o: &mut Exposition) {
    // Jobs and runs.
    let jobs = mgr.list_jobs().await;
    o.head("ufop_backup_job_info", "gauge", "One series per backup job, carrying its name.");
    for j in &jobs {
        o.sample("ufop_backup_job_info", &[("job_id", &j.id), ("job_name", &j.name), ("read_only", if j.read_only { "true" } else { "false" })], 1);
    }
    o.head("ufop_backup_versions", "gauge", "Versions in the job's repository (as of its last run).");
    for j in &jobs {
        o.sample("ufop_backup_versions", &[("job_id", &j.id)], j.stats.version_count);
    }
    o.head("ufop_backup_source_bytes", "gauge", "Size of the job's sources at its last backup.");
    for j in &jobs {
        o.sample("ufop_backup_source_bytes", &[("job_id", &j.id)], j.stats.source_size);
    }
    o.head("ufop_backup_destination_bytes", "gauge", "Size of the job's repository at its last run.");
    for j in &jobs {
        o.sample("ufop_backup_destination_bytes", &[("job_id", &j.id)], j.stats.backend_size);
    }
    match mgr.run_metrics().await {
        Ok(rm) => {
            o.head("ufop_backup_runs_total", "counter", "Finished operations by job, operation and result (from the run history).");
            for c in &rm.counts {
                o.sample("ufop_backup_runs_total", &[("job_id", &c.job_id), ("operation", &c.operation), ("result", &c.status)], c.count);
            }
            o.head("ufop_backup_last_success_timestamp_seconds", "gauge", "Unix time the job's newest successful backup finished.");
            for l in rm.last.iter().filter(|l| l.last_success_unix.is_some()) {
                o.sample("ufop_backup_last_success_timestamp_seconds", &[("job_id", &l.job_id)], l.last_success_unix.unwrap_or(0));
            }
            o.head("ufop_backup_last_duration_seconds", "gauge", "Duration of the job's newest backup run.");
            for l in &rm.last {
                o.sample("ufop_backup_last_duration_seconds", &[("job_id", &l.job_id), ("result", &l.status)], l.duration_secs);
            }
            o.head("ufop_backup_last_uploaded_bytes", "gauge", "Bytes uploaded by the job's newest backup run.");
            for l in &rm.last {
                o.sample("ufop_backup_last_uploaded_bytes", &[("job_id", &l.job_id)], l.bytes_uploaded);
            }
            o.head("ufop_backup_last_files", "gauge", "Files examined by the job's newest backup run.");
            for l in &rm.last {
                o.sample("ufop_backup_last_files", &[("job_id", &l.job_id)], l.files_examined);
            }
        }
        Err(e) => tracing::warn!("Run metrics unavailable: {}", e.user_message()),
    }

    // Scheduler, tasks, locks.
    let status = mgr.status().await;
    let mut running: BTreeMap<String, u64> = BTreeMap::new();
    let mut locked: BTreeMap<String, u64> = BTreeMap::new();
    for t in status.tasks.iter().filter(|t| t.state == "running") {
        *running.entry(t.operation.clone()).or_default() += 1;
        locked.insert(t.job_id.clone(), 1);
    }
    o.head("ufop_operations_running", "gauge", "Operations running now, by operation.");
    if running.is_empty() {
        o.sample("ufop_operations_running", &[("operation", "backup")], 0);
    }
    for (op, n) in &running {
        o.sample("ufop_operations_running", &[("operation", op)], n);
    }
    o.head("ufop_operations_queued", "gauge", "Operations waiting for a free slot (queue length).");
    o.sample("ufop_operations_queued", &[], status.tasks.iter().filter(|t| t.state == "queued").count());
    o.head("ufop_repository_lock_held", "gauge", "1 while an operation of this job runs and holds its repository lock.");
    for j in &jobs {
        o.sample("ufop_repository_lock_held", &[("job_id", &j.id)], locked.get(&j.id).copied().unwrap_or(0));
    }
    o.head("ufop_scheduler_running", "gauge", "1 when the scheduler loop runs.");
    o.sample("ufop_scheduler_running", &[], u8::from(status.running));
    o.head("ufop_scheduler_paused", "gauge", "1 while scheduled work is paused.");
    o.sample("ufop_scheduler_paused", &[], u8::from(status.paused));
    o.head("ufop_scheduler_next_run_timestamp_seconds", "gauge", "Unix time of the next scheduled run, by job and operation.");
    for n in status.next_runs.iter().chain(status.next_maintenance.iter()) {
        if let Some(t) = ts_secs(&n.next_run_at) {
            o.sample("ufop_scheduler_next_run_timestamp_seconds", &[("job_id", &n.job_id), ("operation", &n.operation)], t);
        }
    }

    // Volume cache.
    if let Ok(c) = mgr.cache_stats().await {
        o.head("ufop_volume_cache_bytes", "gauge", "Bytes in the local volume cache.");
        o.sample("ufop_volume_cache_bytes", &[], c.bytes);
        o.head("ufop_volume_cache_max_bytes", "gauge", "Volume cache limit (0 = off).");
        o.sample("ufop_volume_cache_max_bytes", &[], c.max_bytes);
        o.head("ufop_volume_cache_entries", "gauge", "Volumes in the cache.");
        o.sample("ufop_volume_cache_entries", &[], c.entries);
        o.head("ufop_volume_cache_hits_total", "counter", "Cache hits since start.");
        o.sample("ufop_volume_cache_hits_total", &[], c.hits);
        o.head("ufop_volume_cache_misses_total", "counter", "Cache misses since start.");
        o.sample("ufop_volume_cache_misses_total", &[], c.misses);
        o.head("ufop_volume_cache_evictions_total", "counter", "Volumes evicted since start.");
        o.sample("ufop_volume_cache_evictions_total", &[], c.evictions);
    }
}

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
