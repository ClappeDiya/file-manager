//! IPC commands for the unified Operation Ledger.
//!
//! These commands expose [`OperationLedger`] to the frontend so future timeline
//! and "what happened" UI can be built without inventing per-engine endpoints.

use crate::core::error::AppError;
use crate::ledger::{LedgerEvent, LedgerQuery, LedgerSinceSummary, OperationLedger};
use crate::storage::Repository;
use tauri::State;

/// Config key used to persist the "last seen" timestamp between app launches.
/// Reused across command calls so future slices can wire a notification
/// channel or background refresh without reinventing the key.
const LEDGER_LAST_SEEN_KEY: &str = "ledger.last_seen_at";

/// Most recent ledger events across all engines.
#[tauri::command]
pub async fn ledger_recent(
    limit: Option<u32>,
    ledger: State<'_, OperationLedger>,
) -> Result<Vec<LedgerEvent>, AppError> {
    ledger.recent(limit.unwrap_or(100)).await
}

/// Filtered query — engine, kind, subject path, correlation id are all
/// AND-combined when provided.
#[tauri::command]
pub async fn ledger_query(
    query: LedgerQuery,
    ledger: State<'_, OperationLedger>,
) -> Result<Vec<LedgerEvent>, AppError> {
    ledger.query(query).await
}

/// Total event count. Useful for status bar / observability surfaces.
#[tauri::command]
pub async fn ledger_count(ledger: State<'_, OperationLedger>) -> Result<i64, AppError> {
    ledger.count().await
}

/// Manually prune events older than `days` days. Defaults to 30.
#[tauri::command]
pub async fn ledger_prune(
    days: Option<u32>,
    ledger: State<'_, OperationLedger>,
) -> Result<usize, AppError> {
    ledger.prune(days.unwrap_or(30)).await
}

/// "What happened while you were away?" — returns a compact summary of
/// ledger events that occurred strictly after the last time this command
/// was called (persisted in the `config` table under
/// [`LEDGER_LAST_SEEN_KEY`]), then atomically advances the timestamp to
/// "now".
///
/// Powers a zero-effort startup toast like `"12 operations ran while you
/// were away — 3 automations fired, 1 sync ran"`. First-ever call returns
/// `total = 0` (no baseline) and seeds the timestamp. Idempotent within
/// a single session because the timestamp advances after each call.
///
/// Guarantees:
/// - No new schema (reuses `config` + `operation_ledger` tables)
/// - One GROUP BY query, constant time regardless of history length
/// - Non-intrusive: frontend decides whether/how to surface the summary
/// - Safe for first-run: no entry → seed and return an empty summary
#[tauri::command]
pub async fn ledger_since_last_seen(
    repo: State<'_, Repository>,
    ledger: State<'_, OperationLedger>,
) -> Result<LedgerSinceSummary, AppError> {
    // Pull the previous "last seen" marker. First run ⇒ seed with "now"
    // and return an empty summary so the UI has nothing to show.
    let previous = repo.get_config(LEDGER_LAST_SEEN_KEY).await?;
    let now_iso = chrono::Utc::now().to_rfc3339();

    let summary = match previous {
        Some(ts) if !ts.is_empty() => ledger.since(ts).await?,
        _ => LedgerSinceSummary {
            since: now_iso.clone(),
            total: 0,
            by_engine: std::collections::BTreeMap::new(),
            by_status: std::collections::BTreeMap::new(),
        },
    };

    // Advance the marker to "now" so the next call covers the gap.
    // Done AFTER the query so a failed query never loses events.
    repo.set_config(LEDGER_LAST_SEEN_KEY, &now_iso).await?;

    Ok(summary)
}
