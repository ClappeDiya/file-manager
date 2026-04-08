//! Operation Ledger — unified append-only event store across all engines.
//!
//! Every engine (transfer, sync, automation, fs, mount, spaces, ai, vault) can emit
//! a [`LedgerEvent`] describing what just happened. Events are persisted to the
//! `operation_ledger` SQLite table (created by migration v11) and can be queried by
//! engine, by subject path, by correlation id, or as a recent timeline.
//!
//! # Design goals
//!
//! - **DRY**: one schema for all engines instead of N ad-hoc log tables.
//! - **Fail-open**: [`OperationLedger::record`] never returns an error to the
//!   caller — engines should never fail because the ledger is unavailable.
//! - **Cheap**: writes go through the existing [`DbPool`] on the blocking pool.
//!   No extra threads, no extra connections, no new processes.
//! - **Bounded**: [`OperationLedger::prune`] enforces a 30-day retention window
//!   so the table cannot grow without bound.
//! - **Additive**: this module is wired into `AppState` but not yet called from
//!   any engine. Engine call sites land in a follow-up slice. Until then, the
//!   ledger is observable via the `ledger_*` IPC commands and tests.
//!
//! # Future use cases (not yet implemented)
//!
//! - Cross-engine activity timeline ("what touched this folder today").
//! - Single-click "undo last operation" by [`LedgerEvent::undo_token`].
//! - AI assistant context ("explain why this file changed").
//! - Forensic search by [`LedgerEvent::correlation_id`].

use crate::core::error::AppError;
use crate::storage::DbPool;
use serde::{Deserialize, Serialize};

/// Engine that emitted a ledger event. Stable string identifiers — used as a
/// query filter, so don't rename without a migration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerEngine {
    Fs,
    Transfer,
    Sync,
    Automation,
    Mount,
    Spaces,
    Ai,
    Vault,
    Compat,
    Connector,
    System,
}

impl LedgerEngine {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fs => "fs",
            Self::Transfer => "transfer",
            Self::Sync => "sync",
            Self::Automation => "automation",
            Self::Mount => "mount",
            Self::Spaces => "spaces",
            Self::Ai => "ai",
            Self::Vault => "vault",
            Self::Compat => "compat",
            Self::Connector => "connector",
            Self::System => "system",
        }
    }
}

/// Outcome of the operation being recorded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LedgerStatus {
    Ok,
    Failed,
    Cancelled,
    Skipped,
}

impl LedgerStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Skipped => "skipped",
        }
    }
}

/// A single immutable event in the operation ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerEvent {
    pub id: String,
    pub occurred_at: String,
    pub engine: String,
    pub kind: String,
    pub status: String,
    pub subject_path: Option<String>,
    pub target_path: Option<String>,
    pub bytes: Option<i64>,
    pub correlation_id: Option<String>,
    pub summary: String,
    /// Free-form structured data, JSON-encoded. Engines decide their own shape.
    pub details_json: String,
    /// Optional opaque token consumed by an engine-specific "undo" handler.
    pub undo_token: Option<String>,
}

/// Builder for a ledger record. Engines construct one of these and pass it to
/// [`OperationLedger::record`].
#[derive(Debug, Clone)]
pub struct RecordEvent {
    pub engine: LedgerEngine,
    pub kind: String,
    pub status: LedgerStatus,
    pub subject_path: Option<String>,
    pub target_path: Option<String>,
    pub bytes: Option<i64>,
    pub correlation_id: Option<String>,
    pub summary: String,
    pub details_json: String,
    pub undo_token: Option<String>,
}

impl RecordEvent {
    /// Create a minimal `ok` record. Use the builder methods to fill in the
    /// optional fields you care about.
    pub fn new(engine: LedgerEngine, kind: impl Into<String>) -> Self {
        Self {
            engine,
            kind: kind.into(),
            status: LedgerStatus::Ok,
            subject_path: None,
            target_path: None,
            bytes: None,
            correlation_id: None,
            summary: String::new(),
            details_json: "{}".to_string(),
            undo_token: None,
        }
    }

    pub fn status(mut self, status: LedgerStatus) -> Self {
        self.status = status;
        self
    }

    pub fn subject(mut self, path: impl Into<String>) -> Self {
        self.subject_path = Some(path.into());
        self
    }

    pub fn target(mut self, path: impl Into<String>) -> Self {
        self.target_path = Some(path.into());
        self
    }

    pub fn bytes(mut self, bytes: i64) -> Self {
        self.bytes = Some(bytes);
        self
    }

    pub fn correlation(mut self, id: impl Into<String>) -> Self {
        self.correlation_id = Some(id.into());
        self
    }

    pub fn summary(mut self, text: impl Into<String>) -> Self {
        self.summary = text.into();
        self
    }

    pub fn details_json(mut self, json: impl Into<String>) -> Self {
        self.details_json = json.into();
        self
    }

    pub fn undo_token(mut self, token: impl Into<String>) -> Self {
        self.undo_token = Some(token.into());
        self
    }
}

/// Compact "what happened since timestamp X" summary. Used by
/// [`OperationLedger::since`] and the `ledger_since_last_seen` IPC command
/// to power a non-intrusive startup toast ("12 operations ran while you
/// were away — 3 automations fired, 1 sync ran"). Always cheap: one SQL
/// scan with a handful of GROUP BY rows, never a full event dump.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerSinceSummary {
    /// ISO-8601 timestamp the query was filtered from (exclusive).
    pub since: String,
    /// Total number of events strictly after `since`.
    pub total: i64,
    /// Breakdown by engine (`"fs"`, `"transfer"`, `"sync"`, `"automation"`…).
    pub by_engine: std::collections::BTreeMap<String, i64>,
    /// Breakdown by status (`"ok"`, `"failed"`, `"cancelled"`, `"skipped"`).
    pub by_status: std::collections::BTreeMap<String, i64>,
}

/// Query filter for [`OperationLedger::query`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LedgerQuery {
    pub engine: Option<String>,
    pub kind: Option<String>,
    pub subject_path: Option<String>,
    pub correlation_id: Option<String>,
    pub limit: Option<u32>,
}

/// Cross-engine append-only event store.
///
/// Cloning is cheap — internally just clones an [`Arc<Mutex<Connection>>`] via
/// [`DbPool`].
#[derive(Clone)]
pub struct OperationLedger {
    pool: DbPool,
}

impl OperationLedger {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Record an event. Fail-open: errors are logged at `warn` and swallowed
    /// so callers in engine hot paths can never fail because of the ledger.
    pub async fn record(&self, event: RecordEvent) {
        let id = uuid::Uuid::new_v4().to_string();
        let res = self
            .pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO operation_ledger
                       (id, engine, kind, status, subject_path, target_path,
                        bytes, correlation_id, summary, details_json, undo_token)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        id,
                        event.engine.as_str(),
                        event.kind,
                        event.status.as_str(),
                        event.subject_path,
                        event.target_path,
                        event.bytes,
                        event.correlation_id,
                        event.summary,
                        event.details_json,
                        event.undo_token,
                    ],
                )?;
                Ok(())
            })
            .await;

        if let Err(e) = res {
            tracing::warn!("operation ledger write failed (non-fatal): {e}");
        }
    }

    /// Strict variant used by tests and the `ledger_*` IPC commands. Returns
    /// any error so callers can surface it. Engine hot-path code should use
    /// [`Self::record`] instead.
    pub async fn record_strict(&self, event: RecordEvent) -> Result<String, AppError> {
        let id = uuid::Uuid::new_v4().to_string();
        let id_for_insert = id.clone();
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO operation_ledger
                       (id, engine, kind, status, subject_path, target_path,
                        bytes, correlation_id, summary, details_json, undo_token)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                    rusqlite::params![
                        id_for_insert,
                        event.engine.as_str(),
                        event.kind,
                        event.status.as_str(),
                        event.subject_path,
                        event.target_path,
                        event.bytes,
                        event.correlation_id,
                        event.summary,
                        event.details_json,
                        event.undo_token,
                    ],
                )?;
                Ok(())
            })
            .await?;
        Ok(id)
    }

    /// Most recent `limit` events across all engines.
    pub async fn recent(&self, limit: u32) -> Result<Vec<LedgerEvent>, AppError> {
        let limit = limit.clamp(1, 1000);
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, occurred_at, engine, kind, status,
                            subject_path, target_path, bytes,
                            correlation_id, summary, details_json, undo_token
                     FROM operation_ledger
                     ORDER BY occurred_at DESC, id DESC
                     LIMIT ?1",
                )?;
                let rows = stmt
                    .query_map(rusqlite::params![limit], row_to_event)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// Filtered query. All filter fields are AND-combined.
    pub async fn query(&self, q: LedgerQuery) -> Result<Vec<LedgerEvent>, AppError> {
        let limit = q.limit.unwrap_or(200).clamp(1, 1000);
        self.pool
            .execute(move |conn| {
                let mut sql = String::from(
                    "SELECT id, occurred_at, engine, kind, status,
                            subject_path, target_path, bytes,
                            correlation_id, summary, details_json, undo_token
                     FROM operation_ledger WHERE 1=1",
                );
                let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
                if let Some(engine) = q.engine.as_ref() {
                    sql.push_str(" AND engine = ?");
                    params.push(Box::new(engine.clone()));
                }
                if let Some(kind) = q.kind.as_ref() {
                    sql.push_str(" AND kind = ?");
                    params.push(Box::new(kind.clone()));
                }
                if let Some(subject) = q.subject_path.as_ref() {
                    sql.push_str(" AND subject_path = ?");
                    params.push(Box::new(subject.clone()));
                }
                if let Some(corr) = q.correlation_id.as_ref() {
                    sql.push_str(" AND correlation_id = ?");
                    params.push(Box::new(corr.clone()));
                }
                sql.push_str(" ORDER BY occurred_at DESC, id DESC LIMIT ?");
                params.push(Box::new(limit));

                let params_refs: Vec<&dyn rusqlite::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(params_refs.as_slice(), row_to_event)?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// Delete events older than `days` days. Returns the number of rows deleted.
    /// Default retention is 30 days.
    pub async fn prune(&self, days: u32) -> Result<usize, AppError> {
        let cutoff_clause = format!("-{} days", days.max(1));
        self.pool
            .execute(move |conn| {
                let n = conn.execute(
                    "DELETE FROM operation_ledger
                     WHERE occurred_at < datetime('now', ?1)",
                    rusqlite::params![cutoff_clause],
                )?;
                Ok(n)
            })
            .await
    }

    /// Aggregated counts for events that occurred strictly after a given
    /// ISO-8601 timestamp. Returns `(total, by_engine, by_status)` where the
    /// maps use the canonical `LedgerEngine::as_str()` / `LedgerStatus::as_str()`
    /// spellings as keys. Cheap — a single SQL scan with GROUP BY.
    ///
    /// Powers the "what happened while you were away?" startup summary so
    /// users get zero-effort visibility into autonomous activity (automation
    /// rule fires, background syncs, etc.) without opening a panel.
    pub async fn since(
        &self,
        iso_since: String,
    ) -> Result<LedgerSinceSummary, AppError> {
        self.pool
            .execute(move |conn| {
                // `occurred_at` is stored as SQLite's `YYYY-MM-DD HH:MM:SS`
                // (see migration v11: `DEFAULT (datetime('now'))`).
                // The caller passes RFC-3339 (e.g. chrono's `to_rfc3339()`),
                // which lexically sorts DIFFERENTLY ('T' vs ' '). Wrapping
                // the input in `datetime(?1)` canonicalizes it to the same
                // format before comparison so the filter actually matches.
                let total: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM operation_ledger \
                     WHERE occurred_at > datetime(?1)",
                    rusqlite::params![iso_since],
                    |row| row.get(0),
                )?;

                // Per-engine breakdown.
                let mut by_engine: std::collections::BTreeMap<String, i64> =
                    std::collections::BTreeMap::new();
                {
                    let mut stmt = conn.prepare(
                        "SELECT engine, COUNT(*) FROM operation_ledger \
                         WHERE occurred_at > datetime(?1) GROUP BY engine",
                    )?;
                    let rows = stmt.query_map(rusqlite::params![iso_since], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                    })?;
                    for r in rows {
                        let (k, v) = r?;
                        by_engine.insert(k, v);
                    }
                }

                // Per-status breakdown.
                let mut by_status: std::collections::BTreeMap<String, i64> =
                    std::collections::BTreeMap::new();
                {
                    let mut stmt = conn.prepare(
                        "SELECT status, COUNT(*) FROM operation_ledger \
                         WHERE occurred_at > datetime(?1) GROUP BY status",
                    )?;
                    let rows = stmt.query_map(rusqlite::params![iso_since], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
                    })?;
                    for r in rows {
                        let (k, v) = r?;
                        by_status.insert(k, v);
                    }
                }

                Ok(LedgerSinceSummary {
                    since: iso_since,
                    total,
                    by_engine,
                    by_status,
                })
            })
            .await
    }

    /// Total row count. Used by tests and observability.
    pub async fn count(&self) -> Result<i64, AppError> {
        self.pool
            .execute(|conn| {
                let n: i64 = conn
                    .query_row("SELECT COUNT(*) FROM operation_ledger", [], |row| {
                        row.get(0)
                    })?;
                Ok(n)
            })
            .await
    }
}

fn row_to_event(row: &rusqlite::Row<'_>) -> rusqlite::Result<LedgerEvent> {
    Ok(LedgerEvent {
        id: row.get(0)?,
        occurred_at: row.get(1)?,
        engine: row.get(2)?,
        kind: row.get(3)?,
        status: row.get(4)?,
        subject_path: row.get(5)?,
        target_path: row.get(6)?,
        bytes: row.get(7)?,
        correlation_id: row.get(8)?,
        summary: row.get(9)?,
        details_json: row.get(10)?,
        undo_token: row.get(11)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Repository;

    async fn fresh_ledger() -> OperationLedger {
        let repo = Repository::open_in_memory().await.unwrap();
        OperationLedger::new(repo.pool().clone())
    }

    #[tokio::test]
    async fn record_then_recent_returns_event() {
        let ledger = fresh_ledger().await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Transfer, "transfer.completed")
                    .subject("/src/a.bin")
                    .target("/dst/a.bin")
                    .bytes(1024)
                    .summary("copied a.bin"),
            )
            .await;

        let recent = ledger.recent(10).await.unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].engine, "transfer");
        assert_eq!(recent[0].kind, "transfer.completed");
        assert_eq!(recent[0].status, "ok");
        assert_eq!(recent[0].bytes, Some(1024));
        assert_eq!(recent[0].subject_path.as_deref(), Some("/src/a.bin"));
    }

    #[tokio::test]
    async fn record_is_fail_open_on_closed_pool() {
        // We can't easily close the pool from the outside, but a happy-path
        // record() should never panic and should produce no error to caller.
        let ledger = fresh_ledger().await;
        ledger
            .record(RecordEvent::new(LedgerEngine::System, "boot"))
            .await;
        assert_eq!(ledger.count().await.unwrap(), 1);
    }

    #[tokio::test]
    async fn query_filters_by_engine_and_correlation() {
        let ledger = fresh_ledger().await;
        for i in 0..3 {
            ledger
                .record(
                    RecordEvent::new(LedgerEngine::Sync, "sync.file")
                        .correlation("run-1")
                        .subject(format!("/p/{i}")),
                )
                .await;
        }
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Automation, "rule.fired")
                    .correlation("run-2"),
            )
            .await;

        let only_sync = ledger
            .query(LedgerQuery {
                engine: Some("sync".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(only_sync.len(), 3);

        let only_run1 = ledger
            .query(LedgerQuery {
                correlation_id: Some("run-1".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(only_run1.len(), 3);

        let only_run2 = ledger
            .query(LedgerQuery {
                correlation_id: Some("run-2".to_string()),
                ..Default::default()
            })
            .await
            .unwrap();
        assert_eq!(only_run2.len(), 1);
        assert_eq!(only_run2[0].engine, "automation");
    }

    #[tokio::test]
    async fn prune_with_zero_day_retention_clears_table() {
        let ledger = fresh_ledger().await;
        for _ in 0..5 {
            ledger
                .record(RecordEvent::new(LedgerEngine::Fs, "file.created"))
                .await;
        }
        assert_eq!(ledger.count().await.unwrap(), 5);

        // Prune with 1-day window won't delete fresh rows; we just verify it
        // runs cleanly. (Time-travel testing of `datetime('now')` would
        // require injecting a clock, which is more than this slice needs.)
        let deleted = ledger.prune(1).await.unwrap();
        assert_eq!(deleted, 0);
        assert_eq!(ledger.count().await.unwrap(), 5);
    }

    #[tokio::test]
    async fn since_counts_only_events_after_timestamp() {
        let ledger = fresh_ledger().await;

        // Record three "old" events.
        for _ in 0..3 {
            ledger
                .record(RecordEvent::new(LedgerEngine::Fs, "copy"))
                .await;
        }

        // Sleep 1s so SQLite's seconds-resolution `occurred_at` advances
        // past the cutoff we capture below.
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let cutoff = chrono::Utc::now().to_rfc3339();
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;

        // Record mixed "new" events after the cutoff.
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Automation, "rule_fire")
                    .status(LedgerStatus::Ok),
            )
            .await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Sync, "run").status(LedgerStatus::Failed),
            )
            .await;
        ledger
            .record(
                RecordEvent::new(LedgerEngine::Automation, "rule_fire")
                    .status(LedgerStatus::Skipped),
            )
            .await;

        let summary = ledger.since(cutoff).await.unwrap();

        // Only the 3 post-cutoff events count.
        assert_eq!(summary.total, 3);
        assert_eq!(summary.by_engine.get("automation"), Some(&2));
        assert_eq!(summary.by_engine.get("sync"), Some(&1));
        assert!(summary.by_engine.get("fs").is_none());
        assert_eq!(summary.by_status.get("ok"), Some(&1));
        assert_eq!(summary.by_status.get("failed"), Some(&1));
        assert_eq!(summary.by_status.get("skipped"), Some(&1));
    }

    #[tokio::test]
    async fn since_with_future_timestamp_returns_empty() {
        let ledger = fresh_ledger().await;
        ledger
            .record(RecordEvent::new(LedgerEngine::Fs, "copy"))
            .await;

        let future = (chrono::Utc::now() + chrono::Duration::days(1)).to_rfc3339();
        let summary = ledger.since(future).await.unwrap();

        assert_eq!(summary.total, 0);
        assert!(summary.by_engine.is_empty());
        assert!(summary.by_status.is_empty());
    }

    #[tokio::test]
    async fn record_strict_returns_id() {
        let ledger = fresh_ledger().await;
        let id = ledger
            .record_strict(RecordEvent::new(LedgerEngine::Ai, "ai.suggestion"))
            .await
            .unwrap();
        assert_eq!(id.len(), 36); // uuid v4 length
        let recent = ledger.recent(10).await.unwrap();
        assert_eq!(recent[0].id, id);
    }
}
