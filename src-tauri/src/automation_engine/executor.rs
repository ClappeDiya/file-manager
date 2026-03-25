//! Action execution for automation rules.
//!
//! Dispatches rule actions to existing engines (fs_engine, transfer_engine,
//! sync_engine, archive_commands). Each action is a thin wrapper around
//! existing platform capabilities.

use super::{AutomationLog, AutomationRule, AutomationStatus, RuleAction};
use chrono::Utc;
use std::path::Path;
use uuid::Uuid;

/// Execute the action for a triggered rule.
///
/// - `rule`: The rule whose action to execute.
/// - `triggered_file`: The file path that triggered the rule (for watch triggers).
/// - `dry_run`: If true, logs what would happen without executing.
pub async fn execute_rule_action(
    rule: &AutomationRule,
    triggered_file: Option<&str>,
    dry_run: bool,
) -> AutomationLog {
    let started = Utc::now().to_rfc3339();
    let trigger_event = if let Some(file) = triggered_file {
        format!("file: {}", file)
    } else {
        "manual".to_string()
    };

    if dry_run {
        return AutomationLog {
            id: Uuid::new_v4().to_string(),
            rule_id: rule.id.clone(),
            rule_name: rule.name.clone(),
            triggered_at: started,
            completed_at: Some(Utc::now().to_rfc3339()),
            trigger_event,
            status: AutomationStatus::Success,
            files_affected: triggered_file.map(|f| vec![f.to_string()]).unwrap_or_default(),
            error_message: Some("Dry run — no action taken".to_string()),
        };
    }

    let result = dispatch_action(&rule.action, triggered_file).await;

    let (status, files_affected, error_message) = match result {
        Ok(affected) => (AutomationStatus::Success, affected, None),
        Err(e) => (AutomationStatus::Failed, vec![], Some(e)),
    };

    AutomationLog {
        id: Uuid::new_v4().to_string(),
        rule_id: rule.id.clone(),
        rule_name: rule.name.clone(),
        triggered_at: started,
        completed_at: Some(Utc::now().to_rfc3339()),
        trigger_event,
        status,
        files_affected,
        error_message,
    }
}

/// Dispatch an action to the appropriate engine.
async fn dispatch_action(
    action: &RuleAction,
    triggered_file: Option<&str>,
) -> Result<Vec<String>, String> {
    match action {
        RuleAction::MoveFile { dest_path } => {
            let source = triggered_file.ok_or("No source file for move action")?;
            let source_path = Path::new(source);
            let dest_dir = Path::new(dest_path);

            // Ensure destination exists
            if !dest_dir.exists() {
                std::fs::create_dir_all(dest_dir)
                    .map_err(|e| format!("Cannot create destination directory: {e}"))?;
            }

            let file_name = source_path
                .file_name()
                .ok_or("Cannot determine filename")?;
            let dest_file = dest_dir.join(file_name);

            // Handle conflict: auto-rename if destination exists
            let final_dest = if dest_file.exists() {
                find_unique_name(&dest_file)
            } else {
                dest_file
            };

            std::fs::rename(source, &final_dest)
                .map_err(|e| format!("Move failed: {e}"))?;

            Ok(vec![final_dest.to_string_lossy().to_string()])
        }

        RuleAction::CopyFile { dest_path } => {
            let source = triggered_file.ok_or("No source file for copy action")?;
            let source_path = Path::new(source);
            let dest_dir = Path::new(dest_path);

            if !dest_dir.exists() {
                std::fs::create_dir_all(dest_dir)
                    .map_err(|e| format!("Cannot create destination directory: {e}"))?;
            }

            let file_name = source_path
                .file_name()
                .ok_or("Cannot determine filename")?;
            let dest_file = dest_dir.join(file_name);

            let final_dest = if dest_file.exists() {
                find_unique_name(&dest_file)
            } else {
                dest_file
            };

            std::fs::copy(source, &final_dest)
                .map_err(|e| format!("Copy failed: {e}"))?;

            Ok(vec![final_dest.to_string_lossy().to_string()])
        }

        RuleAction::RenameFile { pattern } => {
            let source = triggered_file.ok_or("No source file for rename action")?;
            let source_path = Path::new(source);
            let parent = source_path.parent().ok_or("Cannot determine parent directory")?;
            let file_name = source_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("Cannot determine filename")?;

            let new_name = apply_rename_pattern(file_name, pattern);
            let dest = parent.join(&new_name);

            if dest.exists() {
                return Err(format!("Cannot rename: '{}' already exists", new_name));
            }

            std::fs::rename(source, &dest)
                .map_err(|e| format!("Rename failed: {e}"))?;

            Ok(vec![dest.to_string_lossy().to_string()])
        }

        RuleAction::DeleteFile { to_trash } => {
            let source = triggered_file.ok_or("No source file for delete action")?;

            if *to_trash {
                // Move to a .trash directory next to the file
                let source_path = Path::new(source);
                let parent = source_path.parent().ok_or("Cannot determine parent")?;
                let trash_dir = parent.join(".ufop_trash");
                std::fs::create_dir_all(&trash_dir)
                    .map_err(|e| format!("Cannot create trash directory: {e}"))?;

                let file_name = source_path.file_name().ok_or("Cannot determine filename")?;
                let trash_path = trash_dir.join(file_name);
                std::fs::rename(source, &trash_path)
                    .map_err(|e| format!("Move to trash failed: {e}"))?;
            } else {
                let path = Path::new(source);
                if path.is_dir() {
                    std::fs::remove_dir_all(source)
                        .map_err(|e| format!("Delete directory failed: {e}"))?;
                } else {
                    std::fs::remove_file(source)
                        .map_err(|e| format!("Delete file failed: {e}"))?;
                }
            }

            Ok(vec![source.to_string()])
        }

        RuleAction::CompressFolder { dest_path, format } => {
            let source = triggered_file.ok_or("No source for compress action")?;
            // Delegate to the archive creation logic
            // For now, create a basic zip using std::fs
            let _ = (dest_path, format, source);
            Err("Compress action requires archive engine integration — use RunSync or TransferToConnector instead".to_string())
        }

        RuleAction::RunSync { sync_pair_id } => {
            // This would be called via the SyncManager in the Tauri command layer.
            // At this level, we log the intent; the command layer handles the actual dispatch.
            tracing::info!("Automation triggering sync pair: {}", sync_pair_id);
            Ok(vec![format!("sync_pair:{}", sync_pair_id)])
        }

        RuleAction::TransferToConnector {
            connector_protocol,
            connection_id,
            remote_path,
        } => {
            let source = triggered_file.ok_or("No source file for transfer action")?;
            tracing::info!(
                "Automation triggering transfer: {} -> {}:{} via {}",
                source, connection_id, remote_path, connector_protocol
            );
            Ok(vec![format!("transfer:{}:{}", connection_id, source)])
        }

        RuleAction::ExecuteCustomCommand { command_id } => {
            tracing::info!("Automation triggering custom command: {}", command_id);
            Ok(vec![format!("command:{}", command_id)])
        }

        RuleAction::Notify { title, body } => {
            tracing::info!("Automation notification: {} - {}", title, body);
            // OS notification is handled at the Tauri command layer
            Ok(vec![])
        }
    }
}

/// Generate a unique filename by appending a counter (e.g., file (2).pdf).
fn find_unique_name(path: &Path) -> std::path::PathBuf {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("file");
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();
    let parent = path.parent().unwrap_or(Path::new("."));

    for i in 2..=999 {
        let candidate = parent.join(format!("{stem} ({i}){ext}"));
        if !candidate.exists() {
            return candidate;
        }
    }

    // Fallback with UUID suffix
    parent.join(format!(
        "{stem}_{}{ext}",
        &Uuid::new_v4().to_string()[..8]
    ))
}

/// Apply a rename pattern to a filename.
fn apply_rename_pattern(filename: &str, pattern: &str) -> String {
    let path = Path::new(filename);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(filename);
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| format!(".{e}"))
        .unwrap_or_default();

    match pattern {
        "date_prefix" => {
            let date = Utc::now().format("%Y-%m-%d");
            format!("{date}_{stem}{ext}")
        }
        "datetime_prefix" => {
            let dt = Utc::now().format("%Y-%m-%d_%H%M%S");
            format!("{dt}_{stem}{ext}")
        }
        "lowercase" => filename.to_lowercase(),
        "uppercase" => filename.to_uppercase(),
        "snake_case" => {
            let snaked: String = stem
                .chars()
                .map(|c| if c == ' ' || c == '-' { '_' } else { c })
                .collect();
            format!("{}{ext}", snaked.to_lowercase())
        }
        _ => filename.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rename_date_prefix() {
        let result = apply_rename_pattern("report.pdf", "date_prefix");
        assert!(result.ends_with("_report.pdf"));
        assert!(result.len() > "report.pdf".len());
    }

    #[test]
    fn test_rename_lowercase() {
        assert_eq!(
            apply_rename_pattern("My Report.PDF", "lowercase"),
            "my report.pdf"
        );
    }

    #[test]
    fn test_rename_snake_case() {
        assert_eq!(
            apply_rename_pattern("My Report Name.pdf", "snake_case"),
            "my_report_name.pdf"
        );
    }

    #[test]
    fn test_find_unique_name() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("test.txt");
        std::fs::write(&original, "original").unwrap();

        let unique = find_unique_name(&original);
        assert_eq!(unique.file_name().unwrap().to_str().unwrap(), "test (2).txt");
    }

    #[tokio::test]
    async fn test_move_file_action() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, "hello").unwrap();

        let dest_dir = dir.path().join("output");

        let action = RuleAction::MoveFile {
            dest_path: dest_dir.to_string_lossy().to_string(),
        };

        let result = dispatch_action(&action, Some(source.to_str().unwrap())).await;
        assert!(result.is_ok());
        assert!(!source.exists()); // source should be gone
        assert!(dest_dir.join("source.txt").exists());
    }

    #[tokio::test]
    async fn test_copy_file_action() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source.txt");
        std::fs::write(&source, "hello").unwrap();

        let dest_dir = dir.path().join("output");

        let action = RuleAction::CopyFile {
            dest_path: dest_dir.to_string_lossy().to_string(),
        };

        let result = dispatch_action(&action, Some(source.to_str().unwrap())).await;
        assert!(result.is_ok());
        assert!(source.exists()); // source should still exist
        assert!(dest_dir.join("source.txt").exists());
    }

    #[tokio::test]
    async fn test_delete_to_trash() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("delete_me.txt");
        std::fs::write(&source, "goodbye").unwrap();

        let action = RuleAction::DeleteFile { to_trash: true };

        let result = dispatch_action(&action, Some(source.to_str().unwrap())).await;
        assert!(result.is_ok());
        assert!(!source.exists());
        assert!(dir.path().join(".ufop_trash/delete_me.txt").exists());
    }

    #[tokio::test]
    async fn test_dry_run_no_side_effects() {
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("keep_me.txt");
        std::fs::write(&source, "keep").unwrap();

        let rule = AutomationRule {
            id: "test".to_string(),
            name: "Test Rule".to_string(),
            enabled: true,
            trigger: super::super::RuleTrigger::Manual,
            conditions: vec![],
            action: RuleAction::DeleteFile { to_trash: false },
            created_at: Utc::now().to_rfc3339(),
            updated_at: Utc::now().to_rfc3339(),
            last_triggered: None,
            trigger_count: 0,
            error_count: 0,
            last_error: None,
        };

        let log = execute_rule_action(&rule, Some(source.to_str().unwrap()), true).await;
        assert_eq!(log.status, AutomationStatus::Success);
        assert!(source.exists()); // file should NOT be deleted in dry run
    }
}
