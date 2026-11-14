//! Search index commands: choose folders to index, run/cancel indexing,
//! and query names or contents.

use crate::core::error::AppError;
use crate::search_engine::{IndexJob, IndexRoot, SearchHit, SearchManager, SearchQuery, STALE_AFTER_SECS};
use serde::Serialize;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
pub struct IndexOverview {
    pub roots: Vec<IndexRoot>,
    pub jobs: Vec<IndexJob>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    /// The indexed folder covering the query, `None` = not indexed.
    pub root: Option<IndexRoot>,
    /// That folder is being (re)indexed now; results may be incomplete.
    pub indexing: bool,
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Result<T, AppError> + Send + 'static) -> Result<T, AppError> {
    tokio::task::spawn_blocking(f).await.map_err(|e| AppError::internal(format!("Search task failed: {e}")))?
}

fn overview(m: &SearchManager) -> Result<IndexOverview, AppError> {
    Ok(IndexOverview { roots: m.index().roots()?, jobs: m.jobs() })
}

#[tauri::command]
pub async fn search_index_roots(manager: State<'_, SearchManager>) -> Result<IndexOverview, AppError> {
    overview(&manager)
}

/// Index `path` (names, plus contents when `content`) and start a run.
#[tauri::command]
pub async fn search_index_add(path: String, content: bool, manager: State<'_, SearchManager>) -> Result<IndexOverview, AppError> {
    if !std::path::Path::new(&path).is_dir() {
        return Err(AppError::validation(format!("Not a folder: {path}")));
    }
    let root = manager.index().add_root(&path, content)?;
    manager.start(&root.path);
    overview(&manager)
}

#[tauri::command]
pub async fn search_index_refresh(path: String, manager: State<'_, SearchManager>) -> Result<IndexOverview, AppError> {
    manager.start(&path);
    overview(&manager)
}

#[tauri::command]
pub async fn search_index_remove(path: String, manager: State<'_, SearchManager>) -> Result<IndexOverview, AppError> {
    manager.cancel(&path);
    manager.index().remove_root(&path)?;
    overview(&manager)
}

#[tauri::command]
pub async fn search_index_cancel(path: String, manager: State<'_, SearchManager>) -> Result<IndexOverview, AppError> {
    manager.cancel(&path);
    overview(&manager)
}

/// Query the index. A covered folder whose index is older than
/// `STALE_AFTER_SECS` is refreshed in the background while the current
/// results are returned.
#[tauri::command]
pub async fn search_files_query(
    text: String,
    root: Option<String>,
    content: Option<bool>,
    limit: Option<usize>,
    manager: State<'_, SearchManager>,
) -> Result<SearchResponse, AppError> {
    let covering = match root.as_deref() {
        Some(r) => manager.index().root_covering(r)?,
        None => None,
    };
    if let Some(c) = &covering {
        let stale = c
            .indexed_at
            .as_deref()
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map_or(true, |t| (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds() > STALE_AFTER_SECS);
        if stale {
            manager.start(&c.path);
        }
    }
    let q = SearchQuery { text, root, content: content.unwrap_or(false), limit };
    let index = manager.inner().index_arc();
    let hits = blocking(move || index.search(&q)).await?;
    let indexing = covering.as_ref().is_some_and(|c| manager.is_running(&c.path));
    Ok(SearchResponse { hits, root: covering, indexing })
}
