//! Transfer history management (T-018).
//!
//! - Transfer history: timestamp, source, dest, size, duration, speed, status, errors
//! - History searchable by path, date, status, connection
//! - Export history as CSV/JSON
//! - 90-day retention (configurable)

use crate::core::error::AppError;
use crate::core::types::{
    ConflictPolicy, ExportFormat, TransferHistoryFilter, TransferHistoryRecord, TransferJob,
    TransferStatus,
};
use crate::storage::pool::DbPool;
use chrono::{Duration, Utc};
use uuid::Uuid;

/// Transfer history manager backed by SQLite.
#[derive(Clone, Debug)]
pub struct TransferHistory {
    pool: DbPool,
    /// Retention period in days (default: 90).
    retention_days: u32,
}

impl TransferHistory {
    pub fn new(pool: DbPool) -> Self {
        Self {
            pool,
            retention_days: 90,
        }
    }

    pub fn with_retention(mut self, days: u32) -> Self {
        self.retention_days = days;
        self
    }

    /// Set retention period in days.
    pub fn set_retention_days(&mut self, days: u32) {
        self.retention_days = days;
    }

    /// Record a completed (or failed) transfer to history.
    pub async fn record_transfer(
        &self,
        job: &TransferJob,
        started_at: chrono::DateTime<Utc>,
        duration_ms: Option<u64>,
        checksum_verified: bool,
        checksum_match: Option<bool>,
        conflict_resolution: Option<String>,
    ) -> Result<(), AppError> {
        let record = TransferHistoryRecord {
            id: job.id,
            source_path: job.source_path.clone(),
            dest_path: job.dest_path.clone(),
            total_bytes: job.total_bytes,
            transferred_bytes: job.transferred_bytes,
            status: job.status,
            started_at,
            completed_at: Some(Utc::now()),
            duration_ms,
            speed_bps: job.speed_bps,
            error_message: job.error_message.clone(),
            connection_id: job.connection_id,
            checksum_verified,
            checksum_match,
            conflict_policy: job.conflict_policy,
            conflict_resolution,
            retry_count: job.retry_count,
        };
        self.save_record(&record).await
    }

    /// Save a history record to the database.
    pub async fn save_record(&self, record: &TransferHistoryRecord) -> Result<(), AppError> {
        let record = record.clone();
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO transfer_history_log (id, source_path, dest_path, total_bytes, transferred_bytes,
                     status, started_at, completed_at, duration_ms, speed_bps, error_message,
                     connection_id, checksum_verified, checksum_match, conflict_policy,
                     conflict_resolution, retry_count)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                     ON CONFLICT(id) DO UPDATE SET
                        transferred_bytes = ?5,
                        status = ?6,
                        completed_at = ?8,
                        duration_ms = ?9,
                        speed_bps = ?10,
                        error_message = ?11,
                        checksum_verified = ?13,
                        checksum_match = ?14,
                        conflict_resolution = ?16,
                        retry_count = ?17",
                    rusqlite::params![
                        record.id.to_string(),
                        record.source_path,
                        record.dest_path,
                        record.total_bytes as i64,
                        record.transferred_bytes as i64,
                        record.status.as_str(),
                        record.started_at.to_rfc3339(),
                        record.completed_at.map(|dt| dt.to_rfc3339()),
                        record.duration_ms.map(|d| d as i64),
                        record.speed_bps as i64,
                        record.error_message,
                        record.connection_id.map(|id| id.to_string()),
                        record.checksum_verified,
                        record.checksum_match,
                        record.conflict_policy.map(|p| p.as_str().to_string()),
                        record.conflict_resolution,
                        record.retry_count as i64,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Search transfer history with filters.
    pub async fn search(
        &self,
        filter: TransferHistoryFilter,
    ) -> Result<Vec<TransferHistoryRecord>, AppError> {
        self.pool
            .execute(move |conn| {
                let mut conditions = Vec::new();
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                let mut param_idx = 1;

                if let Some(ref path) = filter.path_contains {
                    conditions.push(format!(
                        "(source_path LIKE ?{param_idx} OR dest_path LIKE ?{param_idx})"
                    ));
                    params.push(Box::new(format!("%{path}%")));
                    param_idx += 1;
                }

                if let Some(status) = filter.status {
                    conditions.push(format!("status = ?{param_idx}"));
                    params.push(Box::new(status.as_str().to_string()));
                    param_idx += 1;
                }

                if let Some(conn_id) = filter.connection_id {
                    conditions.push(format!("connection_id = ?{param_idx}"));
                    params.push(Box::new(conn_id.to_string()));
                    param_idx += 1;
                }

                if let Some(from) = filter.date_from {
                    conditions.push(format!("started_at >= ?{param_idx}"));
                    params.push(Box::new(from.to_rfc3339()));
                    param_idx += 1;
                }

                if let Some(to) = filter.date_to {
                    conditions.push(format!("started_at <= ?{param_idx}"));
                    params.push(Box::new(to.to_rfc3339()));
                    param_idx += 1;
                }

                let where_clause = if conditions.is_empty() {
                    String::new()
                } else {
                    format!("WHERE {}", conditions.join(" AND "))
                };

                let limit = filter.limit.unwrap_or(100);
                let offset = filter.offset.unwrap_or(0);

                let sql = format!(
                    "SELECT id, source_path, dest_path, total_bytes, transferred_bytes,
                            status, started_at, completed_at, duration_ms, speed_bps,
                            error_message, connection_id, checksum_verified, checksum_match,
                            conflict_policy, conflict_resolution, retry_count
                     FROM transfer_history_log
                     {where_clause}
                     ORDER BY started_at DESC
                     LIMIT ?{param_idx} OFFSET ?{}",
                    param_idx + 1
                );
                params.push(Box::new(limit as i64));
                params.push(Box::new(offset as i64));

                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();

                let mut stmt = conn.prepare(&sql)?;
                let records = stmt
                    .query_map(param_refs.as_slice(), |row| {
                        Ok(TransferHistoryRecord {
                            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                            source_path: row.get(1)?,
                            dest_path: row.get(2)?,
                            total_bytes: row.get::<_, i64>(3)? as u64,
                            transferred_bytes: row.get::<_, i64>(4)? as u64,
                            status: TransferStatus::from_str_lossy(&row.get::<_, String>(5)?),
                            started_at: chrono::DateTime::parse_from_rfc3339(
                                &row.get::<_, String>(6)?,
                            )
                            .map(|dt| dt.with_timezone(&chrono::Utc))
                            .unwrap_or_else(|_| chrono::Utc::now()),
                            completed_at: row
                                .get::<_, Option<String>>(7)?
                                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                                .map(|dt| dt.with_timezone(&chrono::Utc)),
                            duration_ms: row.get::<_, Option<i64>>(8)?.map(|d| d as u64),
                            speed_bps: row.get::<_, i64>(9)? as u64,
                            error_message: row.get(10)?,
                            connection_id: row
                                .get::<_, Option<String>>(11)?
                                .and_then(|s| Uuid::parse_str(&s).ok()),
                            checksum_verified: row.get(12)?,
                            checksum_match: row.get(13)?,
                            conflict_policy: row
                                .get::<_, Option<String>>(14)?
                                .map(|s| ConflictPolicy::from_str_lossy(&s)),
                            conflict_resolution: row.get(15)?,
                            retry_count: row.get::<_, i64>(16)? as u32,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(records)
            })
            .await
    }

    /// Export transfer history in the specified format.
    pub async fn export(
        &self,
        filter: TransferHistoryFilter,
        format: ExportFormat,
    ) -> Result<String, AppError> {
        let records = self.search(filter).await?;

        match format {
            ExportFormat::Json => {
                serde_json::to_string_pretty(&records).map_err(|e| {
                    AppError::Transfer {
                        message: format!("Failed to export as JSON: {e}"),
                        advice: "Try again or check disk space.".to_string(),
                    }
                })
            }
            ExportFormat::Csv => {
                let mut wtr = csv::Writer::from_writer(Vec::new());

                // Write header
                wtr.write_record([
                    "id",
                    "source_path",
                    "dest_path",
                    "total_bytes",
                    "transferred_bytes",
                    "status",
                    "started_at",
                    "completed_at",
                    "duration_ms",
                    "speed_bps",
                    "error_message",
                    "connection_id",
                    "checksum_verified",
                    "checksum_match",
                    "conflict_policy",
                    "conflict_resolution",
                    "retry_count",
                ])
                .map_err(|e| AppError::Transfer {
                    message: format!("CSV write error: {e}"),
                    advice: "Try again.".to_string(),
                })?;

                for r in &records {
                    wtr.write_record(&[
                        r.id.to_string(),
                        r.source_path.clone(),
                        r.dest_path.clone(),
                        r.total_bytes.to_string(),
                        r.transferred_bytes.to_string(),
                        r.status.as_str().to_string(),
                        r.started_at.to_rfc3339(),
                        r.completed_at.map(|dt| dt.to_rfc3339()).unwrap_or_default(),
                        r.duration_ms.map(|d| d.to_string()).unwrap_or_default(),
                        r.speed_bps.to_string(),
                        r.error_message.clone().unwrap_or_default(),
                        r.connection_id.map(|id| id.to_string()).unwrap_or_default(),
                        r.checksum_verified.to_string(),
                        r.checksum_match.map(|b| b.to_string()).unwrap_or_default(),
                        r.conflict_policy.map(|p| p.as_str().to_string()).unwrap_or_default(),
                        r.conflict_resolution.clone().unwrap_or_default(),
                        r.retry_count.to_string(),
                    ])
                    .map_err(|e| AppError::Transfer {
                        message: format!("CSV write error: {e}"),
                        advice: "Try again.".to_string(),
                    })?;
                }

                let data = wtr.into_inner().map_err(|e| AppError::Transfer {
                    message: format!("CSV finalization error: {e}"),
                    advice: "Try again.".to_string(),
                })?;

                String::from_utf8(data).map_err(|e| AppError::Transfer {
                    message: format!("CSV encoding error: {e}"),
                    advice: "Try again.".to_string(),
                })
            }
        }
    }

    /// Clean up records older than the retention period.
    pub async fn cleanup_old_records(&self) -> Result<u32, AppError> {
        let cutoff = Utc::now() - Duration::days(self.retention_days as i64);
        let cutoff_str = cutoff.to_rfc3339();
        self.pool
            .execute(move |conn| {
                let deleted = conn.execute(
                    "DELETE FROM transfer_history_log WHERE started_at < ?1",
                    rusqlite::params![cutoff_str],
                )?;
                Ok(deleted as u32)
            })
            .await
    }

    /// Get count of records matching the filter.
    pub async fn count(&self, filter: TransferHistoryFilter) -> Result<u64, AppError> {
        self.pool
            .execute(move |conn| {
                let mut conditions = Vec::new();
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                let mut param_idx = 1;

                if let Some(ref path) = filter.path_contains {
                    conditions.push(format!(
                        "(source_path LIKE ?{param_idx} OR dest_path LIKE ?{param_idx})"
                    ));
                    params.push(Box::new(format!("%{path}%")));
                    param_idx += 1;
                }

                if let Some(status) = filter.status {
                    conditions.push(format!("status = ?{param_idx}"));
                    params.push(Box::new(status.as_str().to_string()));
                    let _ = param_idx; // Suppress unused warning
                }

                let where_clause = if conditions.is_empty() {
                    String::new()
                } else {
                    format!("WHERE {}", conditions.join(" AND "))
                };

                let sql = format!(
                    "SELECT COUNT(*) FROM transfer_history_log {where_clause}"
                );

                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();

                let count: i64 = conn.query_row(&sql, param_refs.as_slice(), |row| row.get(0))?;
                Ok(count as u64)
            })
            .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::TransferPriority;
    use crate::storage::pool::DbPool;

    async fn setup_db() -> DbPool {
        let pool = DbPool::open_in_memory().unwrap();
        // Run base migrations
        pool.execute(|conn| {
            crate::storage::migrations::run_migrations(conn)?;
            Ok(())
        })
        .await
        .unwrap();
        pool
    }

    fn make_test_job(status: TransferStatus) -> TransferJob {
        TransferJob {
            id: Uuid::new_v4(),
            source_path: "/test/source.txt".to_string(),
            dest_path: "/test/dest.txt".to_string(),
            total_bytes: 1000,
            transferred_bytes: 1000,
            status,
            priority: TransferPriority::Normal,
            sort_order: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            error_message: None,
            retry_count: 0,
            partial_checksum: None,
            conflict_policy: None,
            connection_id: None,
            verify_checksum: false,
            checksum_algorithm: None,
            source_checksum: None,
            dest_checksum: None,
            speed_bps: 100000,
            eta_seconds: None,
        }
    }

    #[tokio::test]
    async fn test_record_and_search_history() {
        let pool = setup_db().await;
        let history = TransferHistory::new(pool);

        let job = make_test_job(TransferStatus::Completed);
        history
            .record_transfer(&job, Utc::now(), Some(5000), true, Some(true), None)
            .await
            .unwrap();

        let results = history.search(TransferHistoryFilter::default()).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, job.id);
        assert!(results[0].checksum_verified);
        assert_eq!(results[0].checksum_match, Some(true));
    }

    #[tokio::test]
    async fn test_search_by_status() {
        let pool = setup_db().await;
        let history = TransferHistory::new(pool);

        let completed = make_test_job(TransferStatus::Completed);
        let failed = make_test_job(TransferStatus::Failed);

        history
            .record_transfer(&completed, Utc::now(), Some(1000), false, None, None)
            .await
            .unwrap();
        history
            .record_transfer(&failed, Utc::now(), Some(2000), false, None, None)
            .await
            .unwrap();

        let filter = TransferHistoryFilter {
            status: Some(TransferStatus::Completed),
            ..Default::default()
        };
        let results = history.search(filter).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].status, TransferStatus::Completed);
    }

    #[tokio::test]
    async fn test_search_by_path() {
        let pool = setup_db().await;
        let history = TransferHistory::new(pool);

        let mut job = make_test_job(TransferStatus::Completed);
        job.source_path = "/home/user/documents/report.pdf".to_string();
        history
            .record_transfer(&job, Utc::now(), Some(1000), false, None, None)
            .await
            .unwrap();

        let filter = TransferHistoryFilter {
            path_contains: Some("report".to_string()),
            ..Default::default()
        };
        let results = history.search(filter).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_export_json() {
        let pool = setup_db().await;
        let history = TransferHistory::new(pool);

        let job = make_test_job(TransferStatus::Completed);
        history
            .record_transfer(&job, Utc::now(), Some(1000), false, None, None)
            .await
            .unwrap();

        let json = history
            .export(TransferHistoryFilter::default(), ExportFormat::Json)
            .await
            .unwrap();

        assert!(json.contains("completed"));
        assert!(json.contains("source_path"));
    }

    #[tokio::test]
    async fn test_export_csv() {
        let pool = setup_db().await;
        let history = TransferHistory::new(pool);

        let job = make_test_job(TransferStatus::Completed);
        history
            .record_transfer(&job, Utc::now(), Some(1000), false, None, None)
            .await
            .unwrap();

        let csv = history
            .export(TransferHistoryFilter::default(), ExportFormat::Csv)
            .await
            .unwrap();

        assert!(csv.contains("id,source_path,dest_path"));
        assert!(csv.contains("completed"));
    }

    #[tokio::test]
    async fn test_cleanup_old_records() {
        let pool = setup_db().await;
        let history = TransferHistory::new(pool).with_retention(0); // 0 days = delete everything

        let job = make_test_job(TransferStatus::Completed);
        history
            .record_transfer(&job, Utc::now() - Duration::days(1), Some(1000), false, None, None)
            .await
            .unwrap();

        let deleted = history.cleanup_old_records().await.unwrap();
        assert_eq!(deleted, 1);

        let results = history.search(TransferHistoryFilter::default()).await.unwrap();
        assert!(results.is_empty());
    }

    #[tokio::test]
    async fn test_count() {
        let pool = setup_db().await;
        let history = TransferHistory::new(pool);

        for _ in 0..5 {
            let job = make_test_job(TransferStatus::Completed);
            history
                .record_transfer(&job, Utc::now(), Some(1000), false, None, None)
                .await
                .unwrap();
        }

        let count = history.count(TransferHistoryFilter::default()).await.unwrap();
        assert_eq!(count, 5);
    }
}
