//! IPC commands for the unified Operation Ledger.
//!
//! These commands expose [`OperationLedger`] to the frontend so future timeline
//! and "what happened" UI can be built without inventing per-engine endpoints.

use crate::core::error::AppError;
use crate::ledger::{
    LedgerEvent, LedgerPathHit, LedgerQuery, LedgerSinceSummary, OperationLedger,
};
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

/// Return every distinct path under `dir_path` that has any ledger
/// activity, optionally filtered to events after `since_iso`.
///
/// Powers the per-file **activity dots** rendered inline in the file
/// list: the frontend calls this once per directory navigation and
/// builds a path → most-recent-touch map, then each row checks its own
/// path against the map and renders a colored dot if found. One SQL
/// scan per directory, no polling, no per-file round-trips.
#[tauri::command]
pub async fn ledger_directory_activity(
    dir_path: String,
    since_iso: Option<String>,
    limit: Option<u32>,
    ledger: State<'_, OperationLedger>,
) -> Result<Vec<LedgerPathHit>, AppError> {
    if dir_path.is_empty() {
        return Err(AppError::file_op(
            "dir_path is required",
            "Internal usage error: pass the current pane's directory.",
        ));
    }
    ledger
        .directory_activity(dir_path, since_iso, limit.unwrap_or(1000))
        .await
}

/// Universal Path Recall — returns distinct paths the ledger has ever
/// seen, unioning subject and target columns, ranked most-recent-first.
///
/// The optional `query` is a case-insensitive substring filter that
/// lets the frontend do type-to-filter fuzzy matching cheaply in the
/// backend instead of pulling the whole set on every keystroke. `limit`
/// defaults to 100 and is clamped to `[1, 500]` by the ledger.
///
/// This powers the Instant Jump picker (`Cmd+Shift+O`) — one keybind,
/// type three letters, navigate to any path you've ever touched across
/// every connector and session.
#[tauri::command]
pub async fn ledger_recent_paths(
    limit: Option<u32>,
    query: Option<String>,
    ledger: State<'_, OperationLedger>,
) -> Result<Vec<LedgerPathHit>, AppError> {
    ledger.recent_paths(limit.unwrap_or(100), query).await
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

/// Rolling-window engine pulse — returns a `LedgerSinceSummary` covering
/// the last `window_secs` seconds (default 300 = 5 minutes). Unlike
/// `ledger_since_last_seen`, this command is **idempotent** — it never
/// advances any stored timestamp, so the status bar can poll it cheaply
/// every few seconds without losing event windows.
///
/// Powers the ambient Engine Pulse indicator in the status bar: one
/// lightweight GROUP BY query per poll, showing total recent ops, errors,
/// and which engines are active — all from the existing `since()` method.
#[tauri::command]
pub async fn ledger_get_pulse(
    window_secs: Option<u64>,
    ledger: State<'_, OperationLedger>,
) -> Result<LedgerSinceSummary, AppError> {
    let secs = window_secs.unwrap_or(300).clamp(10, 3600);
    let cutoff = chrono::Utc::now() - chrono::Duration::seconds(secs as i64);
    ledger.since(cutoff.to_rfc3339()).await
}
