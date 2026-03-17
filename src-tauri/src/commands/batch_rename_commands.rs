//! Batch Rename Engine commands (T-060).
//!
//! Provides live preview, token substitution, find/replace (literal + regex),
//! sequential numbering, case transformations, undo support, and compat warnings.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

/// A single rename preview entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenamePreviewEntry {
    pub original_path: String,
    pub original_name: String,
    pub new_name: String,
    pub changed: bool,
    pub warnings: Vec<String>,
}

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

/// Generate a live preview of batch rename results.
#[tauri::command]
pub async fn batch_rename_preview(params: RenameParams) -> Result<Vec<RenamePreviewEntry>, AppError> {
    let mut entries = Vec::new();
    let mut counter = params.counter_start;

    // Windows reserved names for compat warnings
    let reserved: &[&str] = &[
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4",
        "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1", "LPT2",
        "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let invalid_chars = ['<', '>', ':', '"', '|', '?', '*'];

    // Build regex if needed
    let find_regex = if params.use_regex {
        params.find.as_ref().and_then(|f| regex::Regex::new(f).ok())
    } else {
        None
    };

    for file_path in &params.files {
        let path = Path::new(file_path);
        let filename = path
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        let stem = path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let ext = path
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();

        let parent_name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        let date_str = std::fs::metadata(file_path)
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.format("%Y%m%d").to_string()
            })
            .unwrap_or_else(|| "unknown".into());

        let mut new_name = filename.clone();

        // 1. Apply pattern with tokens
        if let Some(ref pat) = params.pattern {
            let counter_str = if params.counter_pad > 0 {
                format!("{:0>width$}", counter, width = params.counter_pad)
            } else {
                counter.to_string()
            };

            new_name = pat
                .replace("{name}", &stem)
                .replace("{ext}", &ext)
                .replace("{num}", &counter_str)
                .replace("{date}", &date_str)
                .replace("{parent}", &parent_name)
                .replace("{counter}", &counter_str);

            // Re-add extension if not in pattern
            if !new_name.contains('.') && !ext.is_empty() {
                new_name = format!("{new_name}.{ext}");
            }
        }

        // 2. Apply find/replace
        if let Some(ref find_str) = params.find {
            let replace_str = params.replace.as_deref().unwrap_or("");
            if let Some(ref re) = find_regex {
                new_name = re.replace_all(&new_name, replace_str).to_string();
            } else {
                new_name = new_name.replace(find_str.as_str(), replace_str);
            }
        }

        // 3. Apply case transform
        if let Some(ref case) = params.case_transform {
            new_name = match case.to_lowercase().as_str() {
                "upper" => new_name.to_uppercase(),
                "lower" => new_name.to_lowercase(),
                "title" => title_case(&new_name),
                "sentence" => sentence_case(&new_name),
                _ => new_name,
            };
        }

        // 4. Generate compat warnings
        let mut warnings = Vec::new();
        let stem_upper = new_name.split('.').next().unwrap_or("").to_uppercase();
        if reserved.contains(&stem_upper.as_str()) {
            warnings.push(format!("'{new_name}' is a reserved name on Windows"));
        }
        for ch in &invalid_chars {
            if new_name.contains(*ch) {
                warnings.push(format!("Character '{ch}' is invalid on Windows"));
            }
        }
        if new_name.ends_with('.') || new_name.ends_with(' ') {
            warnings.push("Trailing dot/space not allowed on Windows".into());
        }
        if new_name.len() > 255 {
            warnings.push(format!("Filename exceeds 255 characters ({} chars)", new_name.len()));
        }
        if new_name.is_empty() {
            warnings.push("Resulting filename would be empty".into());
        }

        let changed = new_name != filename;
        entries.push(RenamePreviewEntry {
            original_path: file_path.clone(),
            original_name: filename,
            new_name,
            changed,
            warnings,
        });

        counter += params.counter_step;
    }

    Ok(entries)
}

/// Apply batch renames. Returns an undo ID.
#[tauri::command]
pub async fn batch_rename_apply(params: RenameParams) -> Result<RenameResult, AppError> {
    let previews = batch_rename_preview(params).await?;
    let undo_id = uuid::Uuid::new_v4().to_string();
    let mut undo_renames = Vec::new();
    let mut success_count = 0;
    let mut fail_count = 0;
    let mut errors = Vec::new();

    for entry in &previews {
        if !entry.changed {
            continue;
        }

        let old_path = Path::new(&entry.original_path);
        let new_path = old_path
            .parent()
            .unwrap_or(Path::new("."))
            .join(&entry.new_name);

        match std::fs::rename(old_path, &new_path) {
            Ok(()) => {
                undo_renames.push((
                    new_path.display().to_string(),
                    entry.original_path.clone(),
                ));
                success_count += 1;
            }
            Err(e) => {
                fail_count += 1;
                errors.push(format!("{}: {e}", entry.original_name));
            }
        }
    }

    // Store undo record
    if !undo_renames.is_empty() {
        let mut store = UNDO_STORE.lock().map_err(|e| {
            AppError::internal(format!("Lock error: {e}"))
        })?;
        store.insert(undo_id.clone(), UndoRecord { renames: undo_renames });
    }

    Ok(RenameResult {
        success_count,
        fail_count,
        undo_id,
        errors,
    })
}

/// Undo a batch rename operation.
#[tauri::command]
pub async fn batch_rename_undo(undo_id: String) -> Result<RenameResult, AppError> {
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
        match std::fs::rename(current_path, original_path) {
            Ok(()) => success_count += 1,
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

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    let rest: String = chars.collect::<String>().to_lowercase();
                    format!("{upper}{rest}")
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sentence_case(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            format!("{upper}{}", chars.collect::<String>())
        }
        None => String::new(),
    }
}
