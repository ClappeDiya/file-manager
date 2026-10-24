//! Tauri commands for the shared include/exclude rule engine
//! (`ufop_core::filter_rules`), used by sync pairs and backup jobs: rclone
//! `--filter-from` import, the predefined groups and user-saved presets.

use crate::core::error::AppError;
use crate::storage::Repository;
use crate::sync_engine::filter_presets::{self, UserFilterPreset};
use serde::Serialize;
use tauri::State;
use ufop_core::filter_rules::{FilterRule, FILTER_GROUPS};

/// A predefined group of patterns (for the rule editors' group picker).
#[derive(Debug, Clone, Serialize)]
pub struct FilterGroup {
    pub name: String,
    pub description: String,
    pub patterns: Vec<String>,
}

/// Read rules in rclone `--filter-from` format (`+ pattern` / `- pattern`,
/// `#` comments, `!` clears). Fails with the offending line number.
#[tauri::command]
pub async fn parse_filter_rules(text: String) -> Result<Vec<FilterRule>, AppError> {
    ufop_core::filter_rules::parse_filter_file(&text)
}

/// The predefined groups (temporary files, caches, images, video, …).
#[tauri::command]
pub async fn get_filter_groups() -> Result<Vec<FilterGroup>, AppError> {
    Ok(FILTER_GROUPS
        .iter()
        .map(|(name, description, patterns)| FilterGroup {
            name: name.to_string(),
            description: description.to_string(),
            patterns: patterns.iter().map(|p| p.to_string()).collect(),
        })
        .collect())
}

/// User-saved filter presets, by name.
#[tauri::command]
pub async fn list_user_filter_presets(repo: State<'_, Repository>) -> Result<Vec<UserFilterPreset>, AppError> {
    filter_presets::list(&repo).await
}

/// Save the rules as a named preset (replaces the preset when `id` is given).
#[tauri::command]
pub async fn save_user_filter_preset(
    id: Option<String>,
    name: String,
    rules: Vec<FilterRule>,
    repo: State<'_, Repository>,
) -> Result<UserFilterPreset, AppError> {
    filter_presets::save(&repo, id, name, rules).await
}

/// Delete a user filter preset. Returns false when it did not exist.
#[tauri::command]
pub async fn delete_user_filter_preset(id: String, repo: State<'_, Repository>) -> Result<bool, AppError> {
    filter_presets::delete(&repo, id).await
}
