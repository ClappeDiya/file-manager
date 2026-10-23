//! Batch Rename Engine commands (T-060).
//!
//! Provides live preview, token substitution, find/replace (literal + regex),
//! sequential numbering, case transformations, undo support, and compat warnings.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use crate::commands::file_ops_commands::record_fs_ok;
use crate::ledger::OperationLedger;
use crate::metadata_engine::MetadataStore;

/// A single rename preview entry.
pub use ufop_core::naming::RenamePlanEntry as RenamePreviewEntry;
use ufop_core::naming::{plan_rename, RenameSpec};

/// Rename operation parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameParams {
    pub files: Vec<String>,
    pub pattern: Option<String>,
    pub find: Option<String>,
    pub replace: Option<String>,
    pub use_regex: bool,
    pub case_transform: Option<String>,
    pub counter_start: u32,
    pub counter_step: u32,
    pub counter_pad: usize,
}

/// Result of applying renames.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenameResult {
    pub success_count: usize,
    pub fail_count: usize,
    pub undo_id: String,
    pub errors: Vec<String>,
}

/// Undo record for reverting renames.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct UndoRecord {
    renames: Vec<(String, String)>, // (new_path, original_path)
}

// Use a simple static mutex for undo storage
static UNDO_STORE: std::sync::LazyLock<Mutex<HashMap<String, UndoRecord>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

impl RenameParams {
    fn spec(&self) -> RenameSpec {
        RenameSpec {
            pattern: self.pattern.clone(),
            find: self.find.clone(),
            replace: self.replace.clone(),
            use_regex: self.use_regex,
            case_transform: self.case_transform.clone(),
            counter_start: self.counter_start as u64,
            counter_step: self.counter_step as u64,
            counter_pad: self.counter_pad,
        }
    }
}

/// Generate a live preview of batch rename results (shared pipeline in
/// `ufop_core::naming`, the same one `ufop rename` uses).
#[tauri::command]
pub async fn batch_rename_preview(params: RenameParams) -> Result<Vec<RenamePreviewEntry>, AppError> {
    let mut entries = plan_rename(&params.files, &params.spec()).map_err(|e| AppError::validation(e.to_string()))?;
    // Name clashes are reported on the entries that cause them.
    for problem in ufop_core::naming::rename::collisions(&entries) {
        if let Some(e) = entries.iter_mut().find(|e| problem.contains(&format!("'{}'", e.new_name))) {
            e.warnings.push(problem);
        }
    }
    Ok(entries)
}

/// Apply batch renames. Returns an undo ID.
///
/// Never overwrites: the batch is refused if a new name collides with
/// another entry or an existing file, and renames happen in two phases so
/// chains and swaps work. Each rename is recorded in the operation ledger
/// (so it shows in the timeline and universal undo) and carries the file's
/// metadata along.
#[tauri::command]
pub async fn batch_rename_apply(
    params: RenameParams,
    ledger: tauri::State<'_, OperationLedger>,
    metadata: tauri::State<'_, MetadataStore>,
) -> Result<RenameResult, AppError> {
    let previews = batch_rename_preview(params).await?;
    let undo_id = uuid::Uuid::new_v4().to_string();
    let outcome = tokio::task::spawn_blocking(move || ufop_core::naming::rename::apply(&previews))
        .await
        .map_err(|e| AppError::internal(format!("Rename task failed: {e}")))?;
    let outcome = match outcome {
        Ok(o) => o,
        Err(reason) => {
            return Ok(RenameResult { success_count: 0, fail_count: 1, undo_id, errors: vec![reason] });
        }
    };

    let (sources, dests): (Vec<String>, Vec<String>) = outcome.renamed.iter().cloned().unzip();
    if !sources.is_empty() {
        record_fs_ok(&ledger, "rename", &sources, &dests, &undo_id).await;
        metadata.follow_fs_op("rename", &sources, &dests, false).await;
        let mut store = UNDO_STORE.lock().map_err(|e| AppError::internal(format!("Lock error: {e}")))?;
        store.insert(
            undo_id.clone(),
            UndoRecord { renames: outcome.renamed.iter().map(|(from, to)| (to.clone(), from.clone())).collect() },
        );
    }

    Ok(RenameResult { success_count: sources.len(), fail_count: 0, undo_id, errors: outcome.errors })
}

/// Undo a batch rename operation.
#[tauri::command]
pub async fn batch_rename_undo(
    undo_id: String,
    metadata: tauri::State<'_, MetadataStore>,
) -> Result<RenameResult, AppError> {
    let record = {
        let mut store = UNDO_STORE.lock().map_err(|e| {
            AppError::internal(format!("Lock error: {e}"))
        })?;
        store.remove(&undo_id)
    };

    let record = record.ok_or_else(|| {
        AppError::not_found(format!("No undo record found for ID: {undo_id}"))
    })?;

    let mut success_count = 0;
    let mut fail_count = 0;
    let mut errors = Vec::new();

    for (current_path, original_path) in &record.renames {
        if Path::new(original_path).exists() {
            fail_count += 1;
            errors.push(format!("{original_path} exists again; not overwriting it"));
            continue;
        }
        match std::fs::rename(current_path, original_path) {
            Ok(()) => {
                success_count += 1;
                metadata.follow_fs_op("rename", std::slice::from_ref(original_path), std::slice::from_ref(current_path), true).await;
            }
            Err(e) => {
                fail_count += 1;
                errors.push(format!("{current_path}: {e}"));
            }
        }
    }

    Ok(RenameResult {
        success_count,
        fail_count,
        undo_id,
        errors,
    })
}

/// Render a name or path template against a sample file, exactly as
/// automation and rename will (strict: unknown tokens are errors). Used by
/// editors for a live preview; the Rust engine is the only implementation.
#[tauri::command]
pub async fn naming_preview(
    template: String,
    sample_path: String,
    match_regex: Option<String>,
    as_path: Option<bool>,
) -> Result<String, AppError> {
    use ufop_core::naming::{render, render_path, TokenContext, UnknownTokens};
    let mut ctx = TokenContext::for_path(Path::new(&sample_path)).with_counter(1, 0);
    // A sample that doesn't exist (an example name): dates use "now", as
    // the engines do for files not written yet.
    if ctx.modified.is_none() {
        ctx.modified = Some(chrono::Utc::now());
    }
    if let Some(re) = match_regex.as_deref().filter(|r| !r.is_empty()) {
        let re = regex::Regex::new(re).map_err(|e| AppError::validation(format!("Invalid regular expression: {e}")))?;
        if !ctx.capture_from(&re) {
            return Err(AppError::validation(format!("'{}' does not match the pattern", ctx.file_name)));
        }
    }
    let out = if as_path.unwrap_or(false) {
        render_path(&template, &ctx, UnknownTokens::Error)
    } else {
        render(&template, &ctx, UnknownTokens::Error)
    };
    out.map_err(|e| AppError::validation(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn naming_preview_is_strict_and_supports_captures() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("INV-2026-0042.pdf");
        std::fs::write(&f, b"%PDF").unwrap();
        let p = f.to_string_lossy().to_string();
        assert_eq!(
            naming_preview("/archive/{1}/{name|lower}.{ext}".into(), p.clone(), Some(r"^INV-(\d{4})".into()), Some(true)).await.unwrap(),
            "/archive/2026/inv-2026-0042.pdf"
        );
        assert!(naming_preview("{nmae}".into(), p.clone(), None, None).await.is_err());
        // An example name that doesn't exist still renders date tokens.
        let year = chrono::Local::now().format("%Y").to_string();
        assert_eq!(naming_preview("{name}-{yyyy}.{ext}".into(), "/no/such/report.pdf".into(), None, None).await.unwrap(), format!("report-{year}.pdf"));
        assert!(naming_preview("{1}".into(), p, Some("^nope".into()), None).await.unwrap_err().to_string().contains("does not match"));
    }
}
