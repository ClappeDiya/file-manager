//! Action execution for automation rules.
//!
//! Dispatches rule actions to existing engines (fs_engine, transfer_engine,
//! sync_engine, archive_commands). Each action is a thin wrapper around
//! existing platform capabilities.

use super::services::{ActionServices, SERVICES_UNAVAILABLE};
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
    execute_rule_action_with(rule, triggered_file, dry_run, None).await
}

/// [`execute_rule_action`] with the engines that sync, transfer, command and
/// notification actions delegate to. `None` makes those actions fail with an
/// explanation rather than pretend to succeed.
pub async fn execute_rule_action_with(
    rule: &AutomationRule,
    triggered_file: Option<&str>,
    dry_run: bool,
    services: Option<&dyn ActionServices>,
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

    let result = dispatch_action_with(&rule.action, triggered_file, services).await;

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

/// Validate that a path does not contain '..' traversal components.
/// If `watch_dir` is provided, verify the path stays within it.
fn validate_automation_path(path: &std::path::Path, watch_dir: Option<&std::path::Path>) -> Result<(), String> {
    // Reject paths with .. components
    for component in path.components() {
        if let std::path::Component::ParentDir = component {
            return Err("Path contains '..' traversal".to_string());
        }
    }
    // If watch_dir specified, verify path stays within it
    if let Some(boundary) = watch_dir {
        if let (Ok(canonical), Ok(boundary_canonical)) = (std::fs::canonicalize(path), std::fs::canonicalize(boundary)) {
            if !canonical.starts_with(&boundary_canonical) {
                return Err(format!("Path {} is outside watch directory {}", canonical.display(), boundary_canonical.display()));
            }
        }
    }
    Ok(())
}

/// Validate that a rename target name does not contain path separators or '..'.
fn validate_rename_name(name: &str) -> Result<(), String> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err("Rename target cannot contain path separators or '..'".to_string());
    }
    Ok(())
}

/// Dispatch an action without engine services (see [`dispatch_action_with`]).
#[cfg(test)]
async fn dispatch_action(
    action: &RuleAction,
    triggered_file: Option<&str>,
) -> Result<Vec<String>, String> {
    dispatch_action_with(action, triggered_file, None).await
}

fn require_services(services: Option<&dyn ActionServices>) -> Result<&dyn ActionServices, String> {
    services.ok_or_else(|| SERVICES_UNAVAILABLE.to_string())
}

/// Dispatch an action to the appropriate engine.
async fn dispatch_action_with(
    action: &RuleAction,
    triggered_file: Option<&str>,
    services: Option<&dyn ActionServices>,
) -> Result<Vec<String>, String> {
    match action {
        RuleAction::MoveFile { dest_path } => {
            let source = triggered_file.ok_or("No source file for move action")?;
            let source_path = Path::new(source);
            let dest_dir = Path::new(dest_path);

            // Validate paths against traversal attacks
            validate_automation_path(source_path, None)?;
            validate_automation_path(dest_dir, None)?;

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

            // Validate paths against traversal attacks
            validate_automation_path(source_path, None)?;
            validate_automation_path(dest_dir, None)?;

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

            // Validate source path against traversal
            validate_automation_path(source_path, None)?;

            let parent = source_path.parent().ok_or("Cannot determine parent directory")?;
            let file_name = source_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("Cannot determine filename")?;

            let new_name = apply_rename_pattern(file_name, pattern);

            // Validate that the new name doesn't contain path separators or traversal
            validate_rename_name(&new_name)?;
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
            let source_path = Path::new(source);
            let dest_dir = Path::new(dest_path);
            validate_automation_path(source_path, None)?;
            validate_automation_path(dest_dir, None)?;
            let fmt = format.trim().to_lowercase();
            let ext = match fmt.as_str() {
                "zip" | "tar" | "7z" => fmt.clone(),
                "tar.gz" | "tgz" => "tar.gz".to_string(),
                other => return Err(format!("Unsupported archive format '{other}'. Use zip, tar, tar.gz or 7z")),
            };
            if !dest_dir.exists() {
                std::fs::create_dir_all(dest_dir)
                    .map_err(|e| format!("Cannot create destination directory: {e}"))?;
            }
            let base = source_path
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or("Cannot determine source name")?;
            let target = dest_dir.join(format!("{base}.{ext}"));
            let target = if target.exists() { find_unique_name(&target) } else { target };
            let output = target.to_string_lossy().to_string();
            let result = crate::commands::archive_commands::archive_create(
                crate::commands::archive_commands::CreateArchiveParams {
                    output_path: output.clone(),
                    source_paths: vec![source.to_string()],
                    format: fmt,
                    password: None,
                    compression_level: None,
                },
            )
            .await
            .map_err(|e| format!("Compress failed: {e}"))?;
            if !result.success {
                return Err(format!("Compress failed: {}", result.errors.join("; ")));
            }
            Ok(vec![output])
        }

        RuleAction::CreateFolder { path } => {
            let dir = Path::new(path);
            validate_automation_path(dir, None)?;
            std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create folder: {e}"))?;
            Ok(vec![path.clone()])
        }

        RuleAction::RunSync { sync_pair_id } => {
            let outcome = require_services(services)?.run_sync(sync_pair_id).await?;
            Ok(vec![format!("sync_pair:{sync_pair_id} ({outcome})")])
        }

        RuleAction::TransferToConnector {
            connector_protocol,
            connection_id,
            remote_path,
        } => {
            let source = triggered_file.ok_or("No source file for transfer action")?;
            validate_automation_path(Path::new(source), None)?;
            let remote = require_services(services)?
                .transfer_to_connector(connector_protocol, connection_id, source, remote_path)
                .await?;
            Ok(vec![remote])
        }

        RuleAction::ExecuteCustomCommand { command_id } => {
            let files = triggered_file.map(|f| vec![f.to_string()]).unwrap_or_default();
            let outcome = require_services(services)?
                .run_custom_command(command_id, files.clone())
                .await?;
            tracing::info!("Automation custom command {command_id}: {outcome}");
            Ok(files)
        }

        RuleAction::Notify { title, body } => {
            require_services(services)?.notify(title, body).await?;
            Ok(triggered_file.map(|f| vec![f.to_string()]).unwrap_or_default())
        }

        RuleAction::ReplayOp {
            kind,
            source_paths,
            dest_path,
        } => {
            // Pinned-operation replay: re-run a captured copy/move exactly
            // as it was first performed. Source list and destination both
            // travel with the rule, so `triggered_file` is irrelevant.
            let dest_dir = Path::new(dest_path);
            validate_automation_path(dest_dir, None)?;
            if !dest_dir.exists() {
                std::fs::create_dir_all(dest_dir)
                    .map_err(|e| format!("Cannot create destination directory: {e}"))?;
            }

            let mut affected = Vec::with_capacity(source_paths.len());
            for source in source_paths {
                let source_path = Path::new(source);
                validate_automation_path(source_path, None)?;
                if !source_path.exists() {
                    return Err(format!(
                        "Replay source no longer exists: {source} (the original file was moved or deleted)"
                    ));
                }
                let file_name = source_path
                    .file_name()
                    .ok_or_else(|| format!("Cannot determine filename for {source}"))?;
                let target = dest_dir.join(file_name);
                let final_target = if target.exists() {
                    find_unique_name(&target)
                } else {
                    target
                };
                match kind.as_str() {
                    "move" => std::fs::rename(source_path, &final_target)
                        .map_err(|e| format!("Replay move failed for {source}: {e}"))?,
                    "copy" => {
                        std::fs::copy(source_path, &final_target)
                            .map_err(|e| format!("Replay copy failed for {source}: {e}"))?;
                    }
                    other => {
                        return Err(format!("Unsupported ReplayOp kind: '{other}'"));
                    }
                }
                affected.push(final_target.to_string_lossy().to_string());
            }
            Ok(affected)
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

    // ── ReplayOp (Operation Pin) ─────────────────────────────────────

    #[tokio::test]
    async fn test_replay_op_copy_replays_multiple_sources() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        let b = dir.path().join("b.txt");
        std::fs::write(&a, "alpha").unwrap();
        std::fs::write(&b, "beta").unwrap();

        let dest = dir.path().join("backup");
        let action = RuleAction::ReplayOp {
            kind: "copy".to_string(),
            source_paths: vec![
                a.to_string_lossy().to_string(),
                b.to_string_lossy().to_string(),
            ],
            dest_path: dest.to_string_lossy().to_string(),
        };

        let result = dispatch_action(&action, None).await.unwrap();
        assert_eq!(result.len(), 2);
        // Sources still exist (it's a copy, not a move)
        assert!(a.exists());
        assert!(b.exists());
        // Targets exist at the destination
        assert!(dest.join("a.txt").exists());
        assert!(dest.join("b.txt").exists());
    }

    #[tokio::test]
    async fn test_replay_op_move_replays_and_removes_sources() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        std::fs::write(&a, "alpha").unwrap();

        let dest = dir.path().join("archive");
        let action = RuleAction::ReplayOp {
            kind: "move".to_string(),
            source_paths: vec![a.to_string_lossy().to_string()],
            dest_path: dest.to_string_lossy().to_string(),
        };

        let result = dispatch_action(&action, None).await.unwrap();
        assert_eq!(result.len(), 1);
        // Source is gone after a move
        assert!(!a.exists());
        // Target exists at the destination
        assert!(dest.join("a.txt").exists());
    }

    #[tokio::test]
    async fn test_replay_op_missing_source_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let action = RuleAction::ReplayOp {
            kind: "copy".to_string(),
            source_paths: vec![dir.path().join("ghost.txt").to_string_lossy().to_string()],
            dest_path: dir.path().join("dst").to_string_lossy().to_string(),
        };

        let result = dispatch_action(&action, None).await;
        assert!(result.is_err());
        let err = result.err().unwrap();
        assert!(
            err.contains("no longer exists"),
            "expected friendly missing-source error, got: {err}"
        );
    }

    #[tokio::test]
    async fn test_replay_op_unknown_kind_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.txt");
        std::fs::write(&a, "alpha").unwrap();

        let action = RuleAction::ReplayOp {
            kind: "delete".to_string(),
            source_paths: vec![a.to_string_lossy().to_string()],
            dest_path: dir.path().join("dst").to_string_lossy().to_string(),
        };

        let result = dispatch_action(&action, None).await;
        assert!(result.is_err());
        // Source must NOT have been touched
        assert!(a.exists());
    }

    // ── Engine-backed actions (regression: these used to log and report
    // success without doing anything) ──

    use crate::automation_engine::services::mock::RecordingServices;
    use crate::automation_engine::services::SERVICES_UNAVAILABLE;

    fn rule_with(action: RuleAction) -> AutomationRule {
        AutomationRule::new("t".into(), crate::automation_engine::RuleTrigger::Manual, action)
    }

    #[tokio::test]
    async fn engine_actions_fail_without_services_instead_of_faking_success() {
        let actions = vec![
            RuleAction::RunSync { sync_pair_id: "p1".into() },
            RuleAction::ExecuteCustomCommand { command_id: "c1".into() },
            RuleAction::Notify { title: "a".into(), body: "b".into() },
        ];
        for action in actions {
            let log = execute_rule_action(&rule_with(action.clone()), None, false).await;
            assert_eq!(log.status, AutomationStatus::Failed, "{action:?} must not report success");
            assert_eq!(log.error_message.as_deref(), Some(SERVICES_UNAVAILABLE));
        }
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.txt");
        std::fs::write(&f, "x").unwrap();
        let transfer = RuleAction::TransferToConnector {
            connector_protocol: "sftp".into(),
            connection_id: "c".into(),
            remote_path: "/in".into(),
        };
        let log = execute_rule_action(&rule_with(transfer), f.to_str(), false).await;
        assert_eq!(log.status, AutomationStatus::Failed);
    }

    #[tokio::test]
    async fn engine_actions_delegate_to_services() {
        let svc = RecordingServices::default();
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("report.csv");
        std::fs::write(&f, "a,b").unwrap();
        let file = f.to_str().unwrap();

        let sync = execute_rule_action_with(
            &rule_with(RuleAction::RunSync { sync_pair_id: "pair-9".into() }), None, false, Some(&svc),
        ).await;
        assert_eq!(sync.status, AutomationStatus::Success);

        let transfer = execute_rule_action_with(
            &rule_with(RuleAction::TransferToConnector {
                connector_protocol: "sftp".into(),
                connection_id: "conn-1".into(),
                remote_path: "/upload/".into(),
            }),
            Some(file), false, Some(&svc),
        ).await;
        assert_eq!(transfer.status, AutomationStatus::Success);
        assert_eq!(transfer.files_affected, vec!["/upload/report.csv".to_string()]);

        let cmd = execute_rule_action_with(
            &rule_with(RuleAction::ExecuteCustomCommand { command_id: "lint".into() }), Some(file), false, Some(&svc),
        ).await;
        assert_eq!(cmd.status, AutomationStatus::Success);

        let note = execute_rule_action_with(
            &rule_with(RuleAction::Notify { title: "New file".into(), body: "arrived".into() }), Some(file), false, Some(&svc),
        ).await;
        assert_eq!(note.status, AutomationStatus::Success);

        assert_eq!(
            svc.calls(),
            vec![
                "sync:pair-9".to_string(),
                format!("transfer:sftp:conn-1:{file}->/upload/report.csv"),
                format!("command:lint:{file}"),
                "notify:New file:arrived".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn service_errors_are_reported_as_failures() {
        let svc = RecordingServices { fail_with: Some("pair not found".into()), ..Default::default() };
        let log = execute_rule_action_with(
            &rule_with(RuleAction::RunSync { sync_pair_id: "nope".into() }), None, false, Some(&svc),
        ).await;
        assert_eq!(log.status, AutomationStatus::Failed);
        assert_eq!(log.error_message.as_deref(), Some("pair not found"));
    }

    #[tokio::test]
    async fn dry_run_never_calls_services() {
        let svc = RecordingServices::default();
        let log = execute_rule_action_with(
            &rule_with(RuleAction::RunSync { sync_pair_id: "p".into() }), None, true, Some(&svc),
        ).await;
        assert_eq!(log.status, AutomationStatus::Success);
        assert!(svc.calls().is_empty());
    }

    #[tokio::test]
    async fn compress_folder_creates_a_real_archive() {
        if std::process::Command::new("zip").arg("-v").output().is_err() {
            eprintln!("zip not installed; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("photos");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("a.jpg"), b"jpeg").unwrap();
        let out = dir.path().join("archives");

        let action = RuleAction::CompressFolder {
            dest_path: out.to_string_lossy().to_string(),
            format: "zip".into(),
        };
        let first = dispatch_action(&action, src.to_str()).await.unwrap();
        assert_eq!(first, vec![out.join("photos.zip").to_string_lossy().to_string()]);
        assert!(out.join("photos.zip").metadata().unwrap().len() > 0);

        // A second run never overwrites the first archive.
        let second = dispatch_action(&action, src.to_str()).await.unwrap();
        assert_eq!(second, vec![out.join("photos (2).zip").to_string_lossy().to_string()]);
    }

    #[tokio::test]
    async fn compress_folder_rejects_unknown_format() {
        let dir = tempfile::tempdir().unwrap();
        let action = RuleAction::CompressFolder {
            dest_path: dir.path().to_string_lossy().to_string(),
            format: "rar".into(),
        };
        let err = dispatch_action(&action, dir.path().to_str()).await.unwrap_err();
        assert!(err.contains("Unsupported archive format"), "{err}");
    }

    #[tokio::test]
    async fn create_folder_makes_missing_parents() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("a/b/c");
        let action = RuleAction::CreateFolder { path: target.to_string_lossy().to_string() };
        dispatch_action(&action, None).await.unwrap();
        assert!(target.is_dir());
        // Idempotent.
        dispatch_action(&action, None).await.unwrap();
    }
}
