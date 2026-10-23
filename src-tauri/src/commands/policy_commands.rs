//! Filename policies: rules on which names may be written where.
//!
//! The rules live in the config table (`filename_policies`, JSON) and are
//! loaded into `ufop_core::policy`'s active set at startup and on every
//! save; file operations, sync planning, transfers and automation all
//! check that one set.

use crate::core::error::AppError;
use crate::storage::Repository;
use std::path::Path;
use tauri::State;
use ufop_core::policy::{self, FilenameRule, PolicySet, PolicyViolation};

const CONFIG_KEY: &str = "filename_policies";

/// Refuse an operation whose targets break a rule.
pub fn enforce(violations: Vec<PolicyViolation>) -> Result<(), AppError> {
    if violations.is_empty() {
        return Ok(());
    }
    Err(AppError::file_op(
        format!("Blocked by filename policy: {}", policy::describe(&violations)),
        "Choose another name or folder, or change the rule in Settings → Filename policies.",
    ))
}

/// Check a copy/move of `sources` into `dest_dir` against the active rules.
pub fn enforce_copy(sources: &[String], dest_dir: &str) -> Result<(), AppError> {
    enforce(policy::check_copy(sources, Path::new(dest_dir)))
}

/// Check one file about to exist at `target`.
pub fn enforce_path(target: &Path) -> Result<(), AppError> {
    enforce(policy::active().check(target).err().into_iter().collect())
}

pub async fn load_rules(repo: &Repository) -> Result<Vec<FilenameRule>, AppError> {
    Ok(match repo.get_config(CONFIG_KEY).await? {
        Some(json) => serde_json::from_str(&json).unwrap_or_else(|e| {
            tracing::warn!("Unreadable filename policies ignored: {e}");
            Vec::new()
        }),
        None => Vec::new(),
    })
}

/// Load the stored rules into the active set (startup).
pub async fn activate_stored(repo: &Repository) {
    match load_rules(repo).await.map(PolicySet::new) {
        Ok(Ok(set)) => policy::set_active(set),
        Ok(Err(e)) => tracing::warn!("Filename policies not applied: {e}"),
        Err(e) => tracing::warn!("Filename policies not loaded: {e}"),
    }
}

/// Validate, persist and activate `rules`.
pub async fn save_rules(repo: &Repository, mut rules: Vec<FilenameRule>) -> Result<Vec<FilenameRule>, AppError> {
    for r in &mut rules {
        if r.id.trim().is_empty() {
            r.id = uuid::Uuid::new_v4().to_string();
        }
    }
    let set = PolicySet::new(rules.clone()).map_err(|e| AppError::validation(e.to_string()))?;
    let json = serde_json::to_string(&rules).map_err(|e| AppError::internal(e.to_string()))?;
    repo.set_config(CONFIG_KEY, &json).await?;
    policy::set_active(set);
    Ok(rules)
}

#[tauri::command]
pub async fn filename_policies_list(repo: State<'_, Repository>) -> Result<Vec<FilenameRule>, AppError> {
    load_rules(&repo).await
}

#[tauri::command]
pub async fn filename_policies_save(rules: Vec<FilenameRule>, repo: State<'_, Repository>) -> Result<Vec<FilenameRule>, AppError> {
    save_rules(&repo, rules).await
}

/// Which of `paths` (files about to exist) the active rules refuse.
#[tauri::command]
pub async fn filename_policy_check(paths: Vec<String>) -> Result<Vec<PolicyViolation>, AppError> {
    Ok(policy::active().check_all(&paths))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn rules_persist_validate_and_activate() {
        let repo = Repository::open_in_memory().await.unwrap();
        assert!(load_rules(&repo).await.unwrap().is_empty());
        let bad = FilenameRule { required_pattern: Some("(".into()), ..FilenameRule::new("x", "") };
        assert!(save_rules(&repo, vec![bad]).await.is_err());
        assert!(load_rules(&repo).await.unwrap().is_empty(), "invalid rules are not stored");

        // Unique extension: the active set is process-wide.
        let rule = FilenameRule { blocked_extensions: vec!["zz-policy-cmd".into()], ..FilenameRule::new("", "") };
        let saved = save_rules(&repo, vec![rule]).await.unwrap();
        assert!(!saved[0].id.is_empty(), "ids are assigned");
        assert_eq!(load_rules(&repo).await.unwrap(), saved);
        let err = enforce_path(Path::new("/tmp/a.zz-policy-cmd")).unwrap_err();
        assert!(err.to_string().contains("Blocked by filename policy"));
        assert!(enforce_path(Path::new("/tmp/a.txt")).is_ok());

        policy::set_active(PolicySet::default());
        activate_stored(&repo).await;
        assert!(enforce_path(Path::new("/tmp/a.zz-policy-cmd")).is_err(), "restored at startup");
        save_rules(&repo, vec![]).await.unwrap();
        assert!(enforce_path(Path::new("/tmp/a.zz-policy-cmd")).is_ok());
    }
}
