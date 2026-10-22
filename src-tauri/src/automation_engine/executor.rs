//! Action execution for automation rules.
//!
//! A run takes the rule's steps (`action` then `steps`) and applies them to
//! each file it works on: the file that triggered it, or the files its
//! source selection picks. Destinations, renames, command arguments and
//! message texts are `{token}` templates (see `ufop_core::naming`);
//! transient failures are retried with the rule's retry policy; what
//! happens when a target exists is the rule's overwrite policy.

use super::evaluator::evaluate_rule;
use super::services::{ActionServices, SERVICES_UNAVAILABLE};
use super::{AutomationLog, AutomationRule, AutomationStatus, RuleAction, SourceSelection, StepResult};
use crate::core::types::ConflictPolicy;
use chrono::Utc;
use std::path::{Path, PathBuf};
use ufop_core::naming::{render, render_path, TokenContext, UnknownTokens};
use uuid::Uuid;

/// Upper bound on files one source-selection run touches.
pub const MAX_SELECTED_FILES: usize = 10_000;

/// Everything an action needs besides the action itself.
pub struct ExecEnv<'a> {
    pub services: Option<&'a dyn ActionServices>,
    pub overwrite: ConflictPolicy,
    /// Regex captures from the rule's `NameRegex` condition.
    pub captures: Vec<String>,
    pub rule_name: &'a str,
    /// Approval hash stored on the rule (for `RunCommand`).
    pub command_approval: Option<&'a str>,
    /// Extra template values (e.g. webhook payload fields).
    pub extra: std::collections::HashMap<String, String>,
}

impl<'a> ExecEnv<'a> {
    pub fn new(services: Option<&'a dyn ActionServices>) -> Self {
        Self {
            services,
            overwrite: ConflictPolicy::Rename,
            captures: Vec::new(),
            rule_name: "",
            command_approval: None,
            extra: Default::default(),
        }
    }

    fn for_rule(rule: &'a AutomationRule, services: Option<&'a dyn ActionServices>) -> Self {
        Self {
            services,
            overwrite: rule.options.overwrite,
            captures: Vec::new(),
            rule_name: &rule.name,
            command_approval: rule.options.command_approval.as_deref(),
            extra: Default::default(),
        }
    }

    fn token_ctx(&self, file: Option<&str>) -> TokenContext {
        let mut ctx = file.map(|f| TokenContext::for_path(Path::new(f))).unwrap_or_default();
        ctx.captures = self.captures.clone();
        ctx.extra = self.extra.clone();
        ctx.extra.entry("rule".into()).or_insert_with(|| self.rule_name.to_string());
        if let Some(f) = file {
            ctx.extra.entry("path".into()).or_insert_with(|| f.to_string());
        }
        ctx
    }

    /// Resolve a destination path template for `file`.
    fn path(&self, template: &str, file: Option<&str>) -> Result<String, String> {
        if !template.contains('{') {
            return Ok(template.to_string());
        }
        render_path(template, &self.token_ctx(file), UnknownTokens::Error).map_err(|e| format!("Destination '{template}': {e}"))
    }

    /// Resolve a text template (messages, command arguments).
    fn text(&self, template: &str, file: Option<&str>) -> Result<String, String> {
        if !template.contains('{') {
            return Ok(template.to_string());
        }
        render(template, &self.token_ctx(file), UnknownTokens::Error).map_err(|e| format!("Template '{template}': {e}"))
    }
}

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
    execute_rule(rule, triggered_file, dry_run, services, Default::default()).await
}

/// Files a source selection picks: matching the rule's conditions, newest
/// first, at most `newest` (or [`MAX_SELECTED_FILES`]).
pub fn select_files(rule: &AutomationRule, source: &SourceSelection) -> Result<Vec<(String, Vec<String>)>, String> {
    let root = Path::new(&source.path);
    validate_automation_path(root, None)?;
    if !root.is_dir() {
        return Err(format!("Source folder does not exist: {}", source.path));
    }
    let mut found: Vec<(std::time::SystemTime, String, Vec<String>)> = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("Cannot read {}: {e}", dir.display()))?;
        for entry in entries.flatten() {
            let p = entry.path();
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue; // never follow links out of the source folder
            }
            if ft.is_dir() && source.recursive {
                stack.push(p.clone());
            }
            if ft.is_dir() && !source.include_dirs {
                continue;
            }
            let ps = p.to_string_lossy().to_string();
            let m = evaluate_rule(rule, &ps);
            if m.matched {
                let mtime = entry.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
                found.push((mtime, ps, m.captures));
            }
            if found.len() > MAX_SELECTED_FILES * 4 {
                return Err(format!("More than {} matching files; narrow the conditions", MAX_SELECTED_FILES * 4));
            }
        }
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let limit = source.newest.unwrap_or(MAX_SELECTED_FILES).min(MAX_SELECTED_FILES);
    Ok(found.into_iter().take(limit).map(|(_, p, c)| (p, c)).collect())
}

/// Run every step of `rule` for each target file and summarise the run in
/// one log entry. `extra` adds template values (webhook fields).
pub async fn execute_rule(
    rule: &AutomationRule,
    triggered_file: Option<&str>,
    dry_run: bool,
    services: Option<&dyn ActionServices>,
    extra: std::collections::HashMap<String, String>,
) -> AutomationLog {
    let started = Utc::now().to_rfc3339();
    let trigger_event = match triggered_file {
        Some(file) => format!("file: {file}"),
        None => "manual".to_string(),
    };
    let finish = |status, files_affected, error_message, attempts, steps| AutomationLog {
        id: Uuid::new_v4().to_string(),
        rule_id: rule.id.clone(),
        rule_name: rule.name.clone(),
        triggered_at: started.clone(),
        completed_at: Some(Utc::now().to_rfc3339()),
        trigger_event: trigger_event.clone(),
        status,
        files_affected,
        error_message,
        attempts,
        steps,
    };

    // Targets: the triggering file (conditions already checked by the
    // watcher, captures recomputed here), the source selection, or none.
    let targets: Vec<(Option<String>, Vec<String>)> = match (triggered_file, &rule.options.source) {
        (Some(f), _) => vec![(Some(f.to_string()), evaluate_rule(rule, f).captures)],
        (None, Some(src)) => match select_files(rule, src) {
            Ok(files) => files.into_iter().map(|(p, c)| (Some(p), c)).collect(),
            Err(e) => return finish(AutomationStatus::Failed, vec![], Some(e), 1, vec![]),
        },
        (None, None) => vec![(None, Vec::new())],
    };
    if targets.is_empty() {
        return finish(AutomationStatus::Skipped, vec![], Some("No files matched".into()), 1, vec![]);
    }

    if dry_run {
        let files: Vec<String> = targets.iter().filter_map(|(f, _)| f.clone()).collect();
        let steps = targets
            .iter()
            .flat_map(|(f, _)| rule.all_steps().into_iter().enumerate().map(move |(i, (a, _))| (i, a, f.clone())))
            .map(|(i, a, f)| StepResult { step: i, action: action_kind(a).into(), file: f, ok: true, attempts: 0, message: Some("dry run".into()) })
            .collect();
        return finish(AutomationStatus::Success, files, Some("Dry run — no action taken".to_string()), 0, steps);
    }

    let mut env = ExecEnv::for_rule(rule, services);
    env.extra = extra;
    let mut affected = Vec::new();
    let mut results = Vec::new();
    let mut first_error: Option<String> = None;
    let mut max_attempts = 1;
    for (file, captures) in &targets {
        env.captures = captures.clone();
        // A step that moves or renames the file hands the new path on.
        let mut current = file.clone();
        for (i, (action, continue_on_error)) in rule.all_steps().into_iter().enumerate() {
            let retried = ufop_core::retry::retry_async(
                &rule.options.retry,
                |_| dispatch(action, current.as_deref(), &env),
                |e: &String| is_transient(action, e),
                |attempt, e, d| tracing::warn!("Automation '{}' step {} failed (attempt {}), retrying in {d:?}: {e}", rule.name, i + 1, attempt + 1),
            )
            .await;
            max_attempts = max_attempts.max(retried.attempts);
            match retried.result {
                Ok(paths) => {
                    if moves_file(action) {
                        if let Some(p) = paths.first() {
                            current = Some(p.clone());
                        }
                    }
                    results.push(StepResult { step: i, action: action_kind(action).into(), file: file.clone(), ok: true, attempts: retried.attempts, message: None });
                    affected.extend(paths);
                }
                Err(e) => {
                    results.push(StepResult { step: i, action: action_kind(action).into(), file: file.clone(), ok: false, attempts: retried.attempts, message: Some(e.clone()) });
                    let label = file.as_deref().map(|f| format!("{f}: ")).unwrap_or_default();
                    first_error.get_or_insert(format!("{label}{e}"));
                    if !continue_on_error {
                        break;
                    }
                }
            }
        }
    }
    affected.dedup();
    let failed = results.iter().any(|r| !r.ok);
    let status = if failed { AutomationStatus::Failed } else { AutomationStatus::Success };
    finish(status, affected, first_error, max_attempts, results)
}

/// Snake-case name of an action (for logs and step results).
pub fn action_kind(action: &RuleAction) -> &'static str {
    match action {
        RuleAction::MoveFile { .. } => "move_file",
        RuleAction::CopyFile { .. } => "copy_file",
        RuleAction::RenameFile { .. } => "rename_file",
        RuleAction::DeleteFile { .. } => "delete_file",
        RuleAction::CompressFolder { .. } => "compress_folder",
        RuleAction::RunSync { .. } => "run_sync",
        RuleAction::TransferToConnector { .. } => "transfer_to_connector",
        RuleAction::ExecuteCustomCommand { .. } => "execute_custom_command",
        RuleAction::Notify { .. } => "notify",
        RuleAction::CreateFolder { .. } => "create_folder",
        RuleAction::ExtractArchive { .. } => "extract_archive",
        RuleAction::RunCommand { .. } => "run_command",
        RuleAction::SendMessage { .. } => "send_message",
        RuleAction::ReplayOp { .. } => "replay_op",
    }
}

/// Actions after which the file lives at a new path.
fn moves_file(action: &RuleAction) -> bool {
    matches!(action, RuleAction::MoveFile { .. } | RuleAction::RenameFile { .. })
}

/// Whether retrying `action` after `error` could help. Validation and
/// configuration problems never do; network, busy and locked conditions can.
pub fn is_transient(action: &RuleAction, error: &str) -> bool {
    let e = error.to_lowercase();
    let permanent = [
        "traversal", "outside", "no source", "unsupported", "invalid", "not approved", "does not exist", "not found",
        "cannot determine", "requires confirmation", "blocked", "already exists", "too many", "template", "destination '",
        SERVICES_UNAVAILABLE,
    ];
    if permanent.iter().any(|p| e.contains(&p.to_lowercase())) {
        return false;
    }
    match action {
        // Remote and engine actions: most failures are network/server side.
        RuleAction::RunSync { .. } | RuleAction::TransferToConnector { .. } | RuleAction::SendMessage { .. } | RuleAction::Notify { .. } => true,
        // Local file actions: only "busy" style OS errors.
        _ => ["busy", "in use", "being used", "temporarily", "timed out", "would block", "interrupted", "sharing violation", "resource deadlock"]
            .iter()
            .any(|p| e.contains(p)),
    }
}

/// Hash that approves exactly one command line for a rule.
pub fn command_fingerprint(program: &str, args: &[String], working_dir: Option<&str>) -> String {
    use sha2::{Digest, Sha256};
    let mut h = Sha256::new();
    h.update(program.as_bytes());
    for a in args {
        h.update([0u8]);
        h.update(a.as_bytes());
    }
    h.update([1u8]);
    h.update(working_dir.unwrap_or("").as_bytes());
    hex::encode(h.finalize())
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

/// What to do about an existing `target` under `policy`: `Some(path)` to
/// write to (the same path to replace it, a new one to keep both), `None`
/// to skip.
fn resolve_target(source: &Path, target: PathBuf, policy: ConflictPolicy) -> Result<Option<PathBuf>, String> {
    // Filename policy first: a blocked name is refused whatever the overwrite choice.
    let violations = if source.is_dir() {
        ufop_core::policy::check_copy(&[source], target.parent().unwrap_or(Path::new("")))
    } else {
        ufop_core::policy::active().check(&target).err().into_iter().collect()
    };
    if !violations.is_empty() {
        return Err(format!("Blocked by filename policy: {}", ufop_core::policy::describe(&violations)));
    }
    if !target.exists() {
        return Ok(Some(target));
    }
    let meta = |p: &Path| std::fs::metadata(p).map_err(|e| format!("Cannot read {}: {e}", p.display()));
    Ok(match policy {
        ConflictPolicy::Rename | ConflictPolicy::AskEachTime => Some(find_unique_name(&target)),
        ConflictPolicy::SkipAlways => None,
        ConflictPolicy::OverwriteAlways => Some(target),
        ConflictPolicy::OverwriteIfNewer => {
            let (s, t) = (meta(source)?.modified().ok(), meta(&target)?.modified().ok());
            (s > t).then_some(target)
        }
        ConflictPolicy::OverwriteIfLarger => (meta(source)?.len() > meta(&target)?.len()).then_some(target),
    })
}

/// Remove an existing target that is about to be replaced.
fn clear_target(target: &Path) -> Result<(), String> {
    if !target.exists() {
        return Ok(());
    }
    if target.is_dir() {
        std::fs::remove_dir_all(target)
    } else {
        std::fs::remove_file(target)
    }
    .map_err(|e| format!("Cannot replace {}: {e}", target.display()))
}

/// Rename, falling back to copy + delete across filesystems (a plain
/// rename fails with EXDEV when the destination is another drive).
fn move_path(source: &Path, target: &Path) -> Result<(), String> {
    match std::fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(e) if is_cross_device(&e) && source.is_file() => {
            std::fs::copy(source, target).map_err(|e| format!("Move failed: {e}"))?;
            std::fs::remove_file(source).map_err(|e| format!("Moved a copy but could not remove the original: {e}"))
        }
        Err(e) => Err(format!("Move failed: {e}")),
    }
}

fn is_cross_device(e: &std::io::Error) -> bool {
    // EXDEV is 18 on Unix; ERROR_NOT_SAME_DEVICE is 17 on Windows (where
    // 17 on Unix is EEXIST, which must not trigger a copy).
    #[cfg(windows)]
    const CROSS_DEVICE: i32 = 17;
    #[cfg(not(windows))]
    const CROSS_DEVICE: i32 = 18;
    e.raw_os_error() == Some(CROSS_DEVICE)
}

/// Copy or move `source` into the directory `dest_dir` honouring `policy`.
/// Returns the final path, or `None` when skipped.
fn place(source: &Path, dest_dir: &Path, policy: ConflictPolicy, mv: bool) -> Result<Option<PathBuf>, String> {
    validate_automation_path(source, None)?;
    validate_automation_path(dest_dir, None)?;
    if !dest_dir.exists() {
        std::fs::create_dir_all(dest_dir).map_err(|e| format!("Cannot create destination directory: {e}"))?;
    }
    let file_name = source.file_name().ok_or("Cannot determine filename")?;
    let Some(target) = resolve_target(source, dest_dir.join(file_name), policy)? else {
        return Ok(None);
    };
    if target == source {
        return Ok(Some(target));
    }
    if target.exists() {
        clear_target(&target)?;
    }
    if mv {
        move_path(source, &target)?;
    } else if source.is_dir() {
        copy_dir(source, &target)?;
    } else {
        std::fs::copy(source, &target).map_err(|e| format!("Copy failed: {e}"))?;
    }
    Ok(Some(target))
}

fn copy_dir(src: &Path, dst: &Path) -> Result<(), String> {
    std::fs::create_dir_all(dst).map_err(|e| format!("Copy failed: {e}"))?;
    for entry in std::fs::read_dir(src).map_err(|e| format!("Copy failed: {e}"))?.flatten() {
        let (from, to) = (entry.path(), dst.join(entry.file_name()));
        match entry.file_type() {
            Ok(t) if t.is_dir() => copy_dir(&from, &to)?,
            Ok(t) if t.is_file() => {
                std::fs::copy(&from, &to).map_err(|e| format!("Copy failed for {}: {e}", from.display()))?;
            }
            _ => {} // links and specials are not copied
        }
    }
    Ok(())
}

fn require_services(services: Option<&dyn ActionServices>) -> Result<&dyn ActionServices, String> {
    services.ok_or_else(|| SERVICES_UNAVAILABLE.to_string())
}

/// Dispatch an action without engine services (see [`dispatch_action_with`]).
#[cfg(test)]
async fn dispatch_action(
    action: &RuleAction,
    triggered_file: Option<&str>,
) -> Result<Vec<String>, String> {
    dispatch_action_with(action, triggered_file, None).await
}

/// Dispatch with default options (rename on conflict, no captures).
pub async fn dispatch_action_with(
    action: &RuleAction,
    triggered_file: Option<&str>,
    services: Option<&dyn ActionServices>,
) -> Result<Vec<String>, String> {
    dispatch(action, triggered_file, &ExecEnv::new(services)).await
}

/// Run one action for one file (or none). Returns the paths it produced.
pub async fn dispatch(action: &RuleAction, triggered_file: Option<&str>, env: &ExecEnv<'_>) -> Result<Vec<String>, String> {
    let services = env.services;
    match action {
        RuleAction::MoveFile { dest_path } | RuleAction::CopyFile { dest_path } => {
            let mv = matches!(action, RuleAction::MoveFile { .. });
            let what = if mv { "move" } else { "copy" };
            let source = triggered_file.ok_or(format!("No source file for {what} action"))?;
            let dest = env.path(dest_path, Some(source))?;
            match place(Path::new(source), Path::new(&dest), env.overwrite, mv)? {
                Some(p) => Ok(vec![p.to_string_lossy().to_string()]),
                None => Ok(vec![]), // skipped by the overwrite policy
            }
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

            // `{token}` templates; otherwise one of the named presets.
            let new_name = if pattern.contains('{') {
                ufop_core::naming::tokens::sanitize_segment(env.text(pattern, Some(source))?)
            } else {
                apply_rename_pattern(file_name, pattern)
            };

            // Validate that the new name doesn't contain path separators or traversal
            validate_rename_name(&new_name)?;
            if new_name.is_empty() {
                return Err("Rename pattern produced an empty name".into());
            }
            if new_name == file_name {
                return Ok(vec![source.to_string()]);
            }
            let Some(dest) = resolve_target(source_path, parent.join(&new_name), env.overwrite)? else {
                return Ok(vec![]);
            };
            if dest.exists() && dest != source_path {
                clear_target(&dest)?;
            }
            std::fs::rename(source, &dest).map_err(|e| format!("Rename failed: {e}"))?;
            Ok(vec![dest.to_string_lossy().to_string()])
        }

        RuleAction::DeleteFile { to_trash } => {
            let source = triggered_file.ok_or("No source file for delete action")?;
            validate_automation_path(Path::new(source), None)?;

            if *to_trash {
                // Move to a .ufop_trash directory next to the file
                let source_path = Path::new(source);
                let parent = source_path.parent().ok_or("Cannot determine parent")?;
                let trash_dir = parent.join(".ufop_trash");
                std::fs::create_dir_all(&trash_dir)
                    .map_err(|e| format!("Cannot create trash directory: {e}"))?;

                let file_name = source_path.file_name().ok_or("Cannot determine filename")?;
                let target = trash_dir.join(file_name);
                let trash_path = if target.exists() { find_unique_name(&target) } else { target };
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
            let dest = env.path(dest_path, Some(source))?;
            let dest_dir = Path::new(&dest);
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
            let Some(target) = resolve_target(source_path, dest_dir.join(format!("{base}.{ext}")), env.overwrite)? else {
                return Ok(vec![]);
            };
            clear_target(&target)?;
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

        RuleAction::ExtractArchive { dest_path, max_entries, delete_archive } => {
            let source = triggered_file.ok_or("No archive for extract action")?;
            let source_path = Path::new(source);
            validate_automation_path(source_path, None)?;
            let dest = PathBuf::from(env.path(dest_path, Some(source))?);
            validate_automation_path(&dest, None)?;
            let info = crate::commands::archive_commands::archive_info(source.to_string())
                .await
                .map_err(|e| format!("Cannot read archive: {e}"))?;
            let members = info.total_files + info.total_dirs;
            if members > *max_entries {
                return Err(format!("Archive has {members} entries; the limit is {max_entries} (too many to extract)"));
            }
            // Extract into a private folder, then place each top-level entry
            // with the overwrite policy, so existing files are never
            // clobbered by the archive tool's own rules.
            std::fs::create_dir_all(&dest).map_err(|e| format!("Cannot create destination directory: {e}"))?;
            let staging = dest.join(format!(".ufop-extract-{}", Uuid::new_v4().simple()));
            let result = crate::commands::archive_commands::archive_extract(crate::commands::archive_commands::ExtractArchiveParams {
                archive_path: source.to_string(),
                dest_dir: Some(staging.to_string_lossy().to_string()),
                password: None,
                selected_entries: None,
            })
            .await;
            let placed = (|| -> Result<Vec<String>, String> {
                let result = result.map_err(|e| format!("Extract failed: {e}"))?;
                if !result.success {
                    return Err(format!("Extract failed: {}", result.errors.join("; ")));
                }
                let mut out = Vec::new();
                let entries = std::fs::read_dir(&staging).map_err(|e| format!("Extract failed: {e}"))?;
                for entry in entries.flatten() {
                    if let Some(p) = place(&entry.path(), &dest, env.overwrite, true)? {
                        out.push(p.to_string_lossy().to_string());
                    }
                }
                Ok(out)
            })();
            let _ = std::fs::remove_dir_all(&staging);
            let placed = placed?;
            if *delete_archive {
                std::fs::remove_file(source_path).map_err(|e| format!("Extracted, but could not delete the archive: {e}"))?;
            }
            Ok(placed)
        }

        RuleAction::CreateFolder { path } => {
            let path = env.path(path, triggered_file)?;
            let dir = Path::new(&path);
            validate_automation_path(dir, None)?;
            std::fs::create_dir_all(dir).map_err(|e| format!("Cannot create folder: {e}"))?;
            Ok(vec![path])
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
            let remote_dir = env.text(remote_path, Some(source))?;
            let remote = require_services(services)?
                .transfer_to_connector(connector_protocol, connection_id, source, &remote_dir)
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

        RuleAction::RunCommand { program, args, working_dir, timeout_secs } => {
            let approved = command_fingerprint(program, args, working_dir.as_deref());
            if !env.command_approval.is_some_and(|a| a.split(',').any(|x| x == approved)) {
                return Err("This command is not approved. Open the rule and approve its command line.".into());
            }
            run_command(program, args, working_dir.as_deref(), *timeout_secs, triggered_file, env).await
        }

        RuleAction::Notify { title, body } => {
            let (title, body) = (env.text(title, triggered_file)?, env.text(body, triggered_file)?);
            require_services(services)?.notify(&title, &body).await?;
            Ok(triggered_file.map(|f| vec![f.to_string()]).unwrap_or_default())
        }

        RuleAction::SendMessage { notification, title, body } => {
            let (title, body) = (env.text(title, triggered_file)?, env.text(body, triggered_file)?);
            require_services(services)?.send_message(notification, &title, &body).await?;
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
                let mv = match kind.as_str() {
                    "move" => true,
                    "copy" => false,
                    other => return Err(format!("Unsupported ReplayOp kind: '{other}'")),
                };
                if let Some(p) = place(source_path, dest_dir, env.overwrite, mv)? {
                    affected.push(p.to_string_lossy().to_string());
                }
            }
            Ok(affected)
        }
    }
}

/// Run an approved program directly (never through a shell) with templated
/// arguments, a timeout, and the file in `UFOP_FILE`.
async fn run_command(
    program: &str,
    args: &[String],
    working_dir: Option<&str>,
    timeout_secs: u64,
    file: Option<&str>,
    env: &ExecEnv<'_>,
) -> Result<Vec<String>, String> {
    let args: Vec<String> = args.iter().map(|a| env.text(a, file)).collect::<Result<_, _>>()?;
    let mut cmd = tokio::process::Command::new(program);
    cmd.args(&args).kill_on_drop(true).stdin(std::process::Stdio::null());
    if let Some(dir) = working_dir {
        cmd.current_dir(env.path(dir, file)?);
    }
    cmd.env("UFOP_RULE", env.rule_name);
    if let Some(f) = file {
        cmd.env("UFOP_FILE", f);
    }
    let timeout = std::time::Duration::from_secs(timeout_secs.clamp(1, 24 * 3600));
    let output = match tokio::time::timeout(timeout, cmd.output()).await {
        Err(_) => return Err(format!("Command timed out after {}s", timeout.as_secs())),
        Ok(Err(e)) => return Err(format!("Cannot start '{program}': {e}")),
        Ok(Ok(o)) => o,
    };
    if output.status.success() {
        Ok(file.map(|f| vec![f.to_string()]).unwrap_or_default())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let tail: String = stderr.trim().chars().rev().take(500).collect::<Vec<_>>().into_iter().rev().collect();
        Err(format!(
            "Command exited with {}: {tail}",
            output.status.code().map(|c| c.to_string()).unwrap_or_else(|| "a signal".into())
        ))
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
    #[test]
    fn filename_policy_refuses_blocked_targets() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("payload.zz-auto-policy");
        std::fs::write(&src, "x").unwrap();
        // Unique extension: the active set is process-wide.
        ufop_core::policy::set_active(
            ufop_core::policy::PolicySet::new(vec![ufop_core::policy::FilenameRule {
                blocked_extensions: vec!["zz-auto-policy".into()],
                ..ufop_core::policy::FilenameRule::new("t", "")
            }])
            .unwrap(),
        );
        let out = super::place(&src, &dir.path().join("out"), crate::core::types::ConflictPolicy::Rename, false);
        ufop_core::policy::set_active(ufop_core::policy::PolicySet::default());
        assert!(out.unwrap_err().starts_with("Blocked by filename policy"));
        assert!(!dir.path().join("out/payload.zz-auto-policy").exists());
    }

    #[test]
    fn only_cross_device_errors_fall_back_to_copy() {
        use super::is_cross_device;
        use std::io::Error;
        #[cfg(not(windows))]
        {
            assert!(is_cross_device(&Error::from_raw_os_error(18)));
            // EEXIST: the target exists; copying over it would be wrong.
            assert!(!is_cross_device(&Error::from_raw_os_error(17)));
        }
        #[cfg(windows)]
        assert!(is_cross_device(&Error::from_raw_os_error(17)));
        assert!(!is_cross_device(&Error::other("x")));
    }

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
            condition_tree: None,
            steps: vec![],
            options: Default::default(),
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

    // ── Automation v1.5 ──

    use crate::automation_engine::{ActionStep, ConditionNode, RuleCondition, RuleOptions, SourceSelection};
    use crate::core::types::ConflictPolicy;

    fn rule_steps(first: RuleAction, more: Vec<ActionStep>, options: RuleOptions) -> AutomationRule {
        let mut r = AutomationRule::new("v15".into(), crate::automation_engine::RuleTrigger::Manual, first);
        r.steps = more;
        r.options = options;
        r
    }

    fn write(p: &std::path::Path, data: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, data).unwrap();
    }

    #[tokio::test]
    async fn destinations_are_token_templates_with_regex_captures() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("in/INV-2026-07.pdf");
        write(&f, "%PDF");
        let mut rule = rule_steps(
            RuleAction::CopyFile { dest_path: format!("{}/out/{{1}}/{{ext|upper}}", dir.path().display()) },
            vec![],
            RuleOptions::default(),
        );
        rule.conditions = vec![RuleCondition::NameRegex { pattern: r"^INV-(\d{4})".into() }];
        let log = execute_rule_action(&rule, f.to_str(), false).await;
        assert_eq!(log.status, AutomationStatus::Success, "{:?}", log.error_message);
        assert!(dir.path().join("out/2026/PDF/INV-2026-07.pdf").exists());

        // A misspelt token fails the run instead of creating a literal folder.
        let bad = rule_steps(RuleAction::CopyFile { dest_path: format!("{}/{{yeer}}", dir.path().display()) }, vec![], RuleOptions::default());
        let log = execute_rule_action(&bad, f.to_str(), false).await;
        assert_eq!(log.status, AutomationStatus::Failed);
        assert!(log.error_message.unwrap().contains("Unknown token {yeer}"));
    }

    #[tokio::test]
    async fn rename_templates_and_move_chains() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("Scan 0042.PDF");
        write(&f, "x");
        let rule = rule_steps(
            RuleAction::RenameFile { pattern: "{name|slug}.{ext|lower}".into() },
            vec![ActionStep { action: RuleAction::MoveFile { dest_path: format!("{}/filed", dir.path().display()) }, continue_on_error: false }],
            RuleOptions::default(),
        );
        let log = execute_rule_action(&rule, f.to_str(), false).await;
        assert_eq!(log.status, AutomationStatus::Success, "{:?}", log.error_message);
        assert!(dir.path().join("filed/scan-0042.pdf").exists(), "the move step acted on the renamed file");
        assert_eq!(log.steps.len(), 2);
    }

    #[tokio::test]
    async fn several_destinations_and_on_error_behaviour() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("report.csv");
        write(&f, "a,b");
        let copy_to = |sub: &str| RuleAction::CopyFile { dest_path: format!("{}/{sub}", dir.path().display()) };
        let failing = RuleAction::CopyFile { dest_path: "/../escape".into() };

        let keep_going = rule_steps(
            copy_to("a"),
            vec![
                ActionStep { action: failing.clone(), continue_on_error: true },
                ActionStep { action: copy_to("b"), continue_on_error: false },
            ],
            RuleOptions::default(),
        );
        let log = execute_rule_action(&keep_going, f.to_str(), false).await;
        assert_eq!(log.status, AutomationStatus::Failed, "a failed step fails the run");
        assert!(dir.path().join("a/report.csv").exists() && dir.path().join("b/report.csv").exists());

        let stop = rule_steps(
            copy_to("c"),
            vec![ActionStep { action: failing, continue_on_error: false }, ActionStep { action: copy_to("d"), continue_on_error: false }],
            RuleOptions::default(),
        );
        execute_rule_action(&stop, f.to_str(), false).await;
        assert!(dir.path().join("c/report.csv").exists());
        assert!(!dir.path().join("d").exists(), "steps after a stopping failure don't run");
    }

    #[tokio::test]
    async fn overwrite_policies() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("in/data.txt");
        write(&src, "new");
        let out = dir.path().join("out");
        write(&out.join("data.txt"), "old");
        let copy = |policy| rule_steps(RuleAction::CopyFile { dest_path: out.to_string_lossy().to_string() }, vec![], RuleOptions { overwrite: policy, ..Default::default() });

        let skipped = execute_rule_action(&copy(ConflictPolicy::SkipAlways), src.to_str(), false).await;
        assert!(skipped.files_affected.is_empty());
        assert_eq!(std::fs::read_to_string(out.join("data.txt")).unwrap(), "old");

        execute_rule_action(&copy(ConflictPolicy::Rename), src.to_str(), false).await;
        assert!(out.join("data (2).txt").exists());

        // Source is newer than the destination: replaced.
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::File::options().write(true).open(out.join("data.txt")).unwrap().set_modified(old).unwrap();
        execute_rule_action(&copy(ConflictPolicy::OverwriteIfNewer), src.to_str(), false).await;
        assert_eq!(std::fs::read_to_string(out.join("data.txt")).unwrap(), "new");

        write(&out.join("data.txt"), "much longer existing content");
        execute_rule_action(&copy(ConflictPolicy::OverwriteIfLarger), src.to_str(), false).await;
        assert_eq!(std::fs::read_to_string(out.join("data.txt")).unwrap(), "much longer existing content");

        execute_rule_action(&copy(ConflictPolicy::OverwriteAlways), src.to_str(), false).await;
        assert_eq!(std::fs::read_to_string(out.join("data.txt")).unwrap(), "new");
    }

    #[tokio::test]
    async fn transient_failures_are_retried_permanent_ones_are_not() {
        use std::sync::atomic::{AtomicU32, Ordering};
        struct Flaky(AtomicU32);
        impl ActionServices for Flaky {
            fn run_sync(&self, _: &str) -> crate::core::traits::BoxFuture<'_, Result<String, String>> {
                let n = self.0.fetch_add(1, Ordering::SeqCst);
                Box::pin(async move { if n < 2 { Err("connection reset by peer".into()) } else { Ok("ok".into()) } })
            }
            fn transfer_to_connector(&self, _: &str, _: &str, _: &str, _: &str) -> crate::core::traits::BoxFuture<'_, Result<String, String>> {
                self.0.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Err("Saved connection c not found".into()) })
            }
            fn run_custom_command(&self, _: &str, _: Vec<String>) -> crate::core::traits::BoxFuture<'_, Result<String, String>> {
                Box::pin(async { Ok(String::new()) })
            }
            fn notify(&self, _: &str, _: &str) -> crate::core::traits::BoxFuture<'_, Result<(), String>> {
                Box::pin(async { Ok(()) })
            }
            fn send_message(&self, _: &crate::backup_engine::notify::Notification, _: &str, _: &str) -> crate::core::traits::BoxFuture<'_, Result<(), String>> {
                Box::pin(async { Ok(()) })
            }
        }
        let retry = ufop_core::retry::RetryPolicy { retries: 3, delay_ms: 1, exponential: true, max_delay_ms: 5 };
        let svc = Flaky(AtomicU32::new(0));
        let sync = rule_steps(RuleAction::RunSync { sync_pair_id: "p".into() }, vec![], RuleOptions { retry, ..Default::default() });
        let log = execute_rule_action_with(&sync, None, false, Some(&svc)).await;
        assert_eq!((log.status, log.attempts), (AutomationStatus::Success, 3));

        let svc = Flaky(AtomicU32::new(0));
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x");
        write(&f, "x");
        let transfer = rule_steps(
            RuleAction::TransferToConnector { connector_protocol: "sftp".into(), connection_id: "c".into(), remote_path: "/".into() },
            vec![],
            RuleOptions { retry, ..Default::default() },
        );
        let log = execute_rule_action_with(&transfer, f.to_str(), false, Some(&svc)).await;
        assert_eq!((log.status, log.attempts), (AutomationStatus::Failed, 1), "'not found' is permanent");
        assert_eq!(svc.0.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn run_command_requires_approval_and_honours_timeout() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("input.txt");
        write(&f, "x");
        let touch = RuleAction::RunCommand { program: "touch".into(), args: vec!["{path}.done".into()], working_dir: None, timeout_secs: 10 };
        let mut rule = rule_steps(touch.clone(), vec![], RuleOptions::default());
        let log = execute_rule_action(&rule, f.to_str(), false).await;
        assert!(log.error_message.unwrap().contains("not approved"));
        assert!(!dir.path().join("input.txt.done").exists());

        if let RuleAction::RunCommand { program, args, working_dir, .. } = &touch {
            rule.options.command_approval = Some(command_fingerprint(program, args, working_dir.as_deref()));
        }
        let log = execute_rule_action(&rule, f.to_str(), false).await;
        assert_eq!(log.status, AutomationStatus::Success, "{:?}", log.error_message);
        assert!(dir.path().join("input.txt.done").exists(), "arguments are templated");

        // Editing the command line voids the approval.
        rule.action = RuleAction::RunCommand { program: "rm".into(), args: vec!["{path}".into()], working_dir: None, timeout_secs: 10 };
        assert!(execute_rule_action(&rule, f.to_str(), false).await.error_message.unwrap().contains("not approved"));
        assert!(f.exists());

        let sleep = RuleAction::RunCommand { program: "sleep".into(), args: vec!["5".into()], working_dir: None, timeout_secs: 1 };
        let mut slow = rule_steps(sleep, vec![], RuleOptions::default());
        slow.options.command_approval = Some(command_fingerprint("sleep", &["5".into()], None));
        let started = std::time::Instant::now();
        let log = execute_rule_action(&slow, None, false).await;
        assert!(log.error_message.unwrap().contains("timed out"));
        assert!(started.elapsed() < std::time::Duration::from_secs(4));

        let fails = RuleAction::RunCommand { program: "false".into(), args: vec![], working_dir: None, timeout_secs: 5 };
        let mut r = rule_steps(fails, vec![], RuleOptions::default());
        r.options.command_approval = Some(command_fingerprint("false", &[], None));
        assert!(execute_rule_action(&r, None, false).await.error_message.unwrap().contains("exited with 1"));
    }

    #[tokio::test]
    async fn source_selection_newest_n_and_expiry() {
        let dir = tempfile::tempdir().unwrap();
        let logs = dir.path().join("logs");
        for (i, age_days) in [(1, 10u64), (2, 5), (3, 1), (4, 0)] {
            let p = logs.join(format!("app-{i}.log"));
            write(&p, "x");
            let t = std::time::SystemTime::now() - std::time::Duration::from_secs(age_days * 86_400 + 60);
            std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
        }
        write(&logs.join("keep.txt"), "x");

        // Expire logs older than 2 days, to the app trash, on a schedule.
        let mut expire = rule_steps(
            RuleAction::DeleteFile { to_trash: true },
            vec![],
            RuleOptions { source: Some(SourceSelection { path: logs.to_string_lossy().to_string(), recursive: false, newest: None, include_dirs: false }), ..Default::default() },
        );
        expire.condition_tree = Some(ConditionNode::All {
            nodes: vec![
                ConditionNode::Condition { condition: RuleCondition::ExtensionIs { extensions: vec!["log".into()] } },
                ConditionNode::Condition { condition: RuleCondition::OlderThan { seconds: 2 * 86_400 } },
            ],
        });
        let preview = execute_rule_action(&expire, None, true).await;
        assert_eq!(preview.files_affected.len(), 2, "dry run lists exactly what would go");
        assert!(logs.join("app-1.log").exists(), "dry run touches nothing");
        let run = execute_rule_action(&expire, None, false).await;
        assert_eq!(run.status, AutomationStatus::Success);
        assert!(!logs.join("app-1.log").exists() && !logs.join("app-2.log").exists());
        assert!(logs.join("app-3.log").exists() && logs.join("keep.txt").exists());
        assert!(logs.join(".ufop_trash/app-1.log").exists(), "recoverable");

        // Newest-N: copy only the most recent log.
        let mut newest = rule_steps(
            RuleAction::CopyFile { dest_path: dir.path().join("latest").to_string_lossy().to_string() },
            vec![],
            RuleOptions { source: Some(SourceSelection { path: logs.to_string_lossy().to_string(), recursive: false, newest: Some(1), include_dirs: false }), ..Default::default() },
        );
        newest.conditions = vec![RuleCondition::ExtensionIs { extensions: vec!["log".into()] }];
        execute_rule_action(&newest, None, false).await;
        let copied: Vec<_> = std::fs::read_dir(dir.path().join("latest")).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(copied, vec![std::ffi::OsString::from("app-4.log")]);

        let empty = execute_rule_action(&rule_steps(RuleAction::DeleteFile { to_trash: true }, vec![], RuleOptions { source: Some(SourceSelection { path: dir.path().join("latest").to_string_lossy().to_string(), recursive: false, newest: None, include_dirs: false }), ..Default::default() }).tap_conditions(), None, false).await;
        assert_eq!(empty.status, AutomationStatus::Skipped);
    }

    trait TapConditions {
        fn tap_conditions(self) -> Self;
    }
    impl TapConditions for AutomationRule {
        /// Conditions nothing matches.
        fn tap_conditions(mut self) -> Self {
            self.conditions = vec![RuleCondition::ExtensionIs { extensions: vec!["nomatch".into()] }];
            self
        }
    }

    #[tokio::test]
    async fn extract_archive_with_limits_and_policy() {
        if std::process::Command::new("zip").arg("-v").output().is_err() || std::process::Command::new("unzip").arg("-v").output().is_err() {
            eprintln!("zip/unzip not installed; skipping");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let content = dir.path().join("content");
        write(&content.join("a.txt"), "A");
        write(&content.join("b.txt"), "B");
        let archive = dir.path().join("bundle.zip");
        let ok = std::process::Command::new("zip").arg("-jq").arg(&archive).arg(content.join("a.txt")).arg(content.join("b.txt")).status().unwrap();
        assert!(ok.success());

        let out = dir.path().join("out");
        write(&out.join("a.txt"), "existing");
        let extract = |max| rule_steps(RuleAction::ExtractArchive { dest_path: out.to_string_lossy().to_string(), max_entries: max, delete_archive: false }, vec![], RuleOptions::default());

        let refused = execute_rule_action(&extract(1), archive.to_str(), false).await;
        assert!(refused.error_message.unwrap().contains("too many"));

        let log = execute_rule_action(&extract(100), archive.to_str(), false).await;
        assert_eq!(log.status, AutomationStatus::Success, "{:?}", log.error_message);
        assert_eq!(std::fs::read_to_string(out.join("a.txt")).unwrap(), "existing", "never clobbered by default");
        assert_eq!(std::fs::read_to_string(out.join("a (2).txt")).unwrap(), "A");
        assert_eq!(std::fs::read_to_string(out.join("b.txt")).unwrap(), "B");
        assert!(std::fs::read_dir(&out).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".ufop-extract")));
    }

    #[tokio::test]
    async fn messages_use_templates_and_services() {
        let svc = RecordingServices::default();
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("invoice.pdf");
        write(&f, "x");
        let n: crate::backup_engine::notify::Notification =
            serde_json::from_value(serde_json::json!({ "type": "teams", "credential_id": "teams-1" })).unwrap();
        let rule = rule_steps(
            RuleAction::SendMessage { notification: n, title: "New {ext} in {parent}".into(), body: "{filename} ({size} bytes)".into() },
            vec![ActionStep { action: RuleAction::Notify { title: "{rule}".into(), body: "{name}".into() }, continue_on_error: false }],
            RuleOptions::default(),
        );
        let log = execute_rule_action_with(&rule, f.to_str(), false, Some(&svc)).await;
        assert_eq!(log.status, AutomationStatus::Success, "{:?}", log.error_message);
        let parent = dir.path().file_name().unwrap().to_string_lossy().to_string();
        assert_eq!(
            svc.calls(),
            vec![format!("message:teams:New pdf in {parent}:invoice.pdf (1 bytes)"), "notify:v15:invoice".to_string()]
        );
    }

    #[test]
    fn transient_classification() {
        let sync = RuleAction::RunSync { sync_pair_id: "p".into() };
        let mv = RuleAction::MoveFile { dest_path: "/x".into() };
        assert!(is_transient(&sync, "connection timed out"));
        assert!(!is_transient(&sync, SERVICES_UNAVAILABLE));
        assert!(!is_transient(&mv, "Move failed: Permission denied"));
        assert!(is_transient(&mv, "Move failed: Resource busy"));
        assert!(!is_transient(&mv, "Path contains '..' traversal"));
    }
}
