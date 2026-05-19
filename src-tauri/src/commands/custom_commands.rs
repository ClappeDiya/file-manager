//! User-defined custom commands with path substitution (WinSCP-inspired).
//!
//! Allows power users to define reusable commands with substitution patterns:
//! - `{path}` — Full path of selected file(s)
//! - `{name}` — File name only (no directory)
//! - `{dir}` — Directory containing the file
//! - `{ext}` — File extension
//! - `{remote_path}` — Remote path (for connected sessions)
//! - `{local_path}` — Local path equivalent
//!
//! Commands can be executed locally (shell) or remotely (SSH).

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use uuid::Uuid;

/// A user-defined custom command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomCommand {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    /// The command template with substitution patterns.
    pub command_template: String,
    /// Whether this runs locally (shell) or remotely (SSH).
    pub execution_mode: ExecutionMode,
    /// Whether to apply to directories as well as files.
    pub apply_to_directories: bool,
    /// Whether to apply recursively to directory contents.
    pub recursive: bool,
    /// Optional keyboard shortcut (e.g., "Ctrl+Shift+G").
    pub shortcut: Option<String>,
    /// Category for organization in menus.
    pub category: Option<String>,
    /// Show output in terminal or copy to clipboard.
    pub output_mode: OutputMode,
    /// Sort order for menu display.
    pub sort_order: u32,
}

/// Where the command executes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExecutionMode {
    /// Run on the local machine via shell.
    Local,
    /// Run on the remote server via SSH.
    Remote,
}

/// How to handle command output.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum OutputMode {
    /// Show output in the terminal panel.
    Terminal,
    /// Copy output to clipboard.
    Clipboard,
    /// Discard output silently.
    Silent,
}

/// In-memory store for custom commands (persisted via settings export/import).
static CUSTOM_COMMANDS: std::sync::LazyLock<
    tokio::sync::RwLock<Vec<CustomCommand>>,
> = std::sync::LazyLock::new(|| tokio::sync::RwLock::new(Vec::new()));

/// Result of executing a custom command.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommandExecutionResult {
    pub command_id: String,
    pub command_expanded: String,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

// ── IPC Commands ──

/// List all user-defined custom commands.
#[tauri::command]
pub async fn list_custom_commands() -> Result<Vec<CustomCommand>, AppError> {
    let commands = CUSTOM_COMMANDS.read().await;
    Ok(commands.clone())
}

/// Create or update a custom command.
#[tauri::command]
pub async fn save_custom_command(command: CustomCommand) -> Result<CustomCommand, AppError> {
    let mut commands = CUSTOM_COMMANDS.write().await;

    // Validate template
    if command.command_template.trim().is_empty() {
        return Err(AppError::file_op(
            "Command template cannot be empty",
            "Provide a command with optional substitution patterns like {path}.",
        ));
    }
    if command.name.trim().is_empty() {
        return Err(AppError::file_op(
            "Command name cannot be empty",
            "Give the command a descriptive name.",
        ));
    }

    let mut cmd = command;
    if cmd.id.is_empty() {
        cmd.id = Uuid::new_v4().to_string();
    }

    // Replace existing or add new
    if let Some(pos) = commands.iter().position(|c| c.id == cmd.id) {
        commands[pos] = cmd.clone();
    } else {
        commands.push(cmd.clone());
    }

    Ok(cmd)
}

/// Delete a custom command by ID.
#[tauri::command]
pub async fn delete_custom_command(command_id: String) -> Result<(), AppError> {
    let mut commands = CUSTOM_COMMANDS.write().await;
    let before = commands.len();
    commands.retain(|c| c.id != command_id);
    if commands.len() == before {
        return Err(AppError::file_op(
            format!("Custom command not found: {command_id}"),
            "The command may have already been deleted.",
        ));
    }
    Ok(())
}

/// Expand a command template by substituting path patterns.
///
/// Returns the fully expanded command string ready for execution.
#[tauri::command]
pub async fn expand_custom_command(
    command_id: String,
    file_paths: Vec<String>,
    remote_base: Option<String>,
) -> Result<String, AppError> {
    let commands = CUSTOM_COMMANDS.read().await;
    let cmd = commands
        .iter()
        .find(|c| c.id == command_id)
        .ok_or_else(|| {
            AppError::file_op(
                format!("Custom command not found: {command_id}"),
                "The command may have been deleted.",
            )
        })?;

    if file_paths.is_empty() {
        return Ok(cmd.command_template.clone());
    }

    // For multi-file commands, expand once per file and join with " && "
    let expanded: Vec<String> = file_paths
        .iter()
        .map(|fp| expand_template(&cmd.command_template, fp, remote_base.as_deref()))
        .collect();

    Ok(expanded.join(" && "))
}

/// Allowlist of safe command prefixes. Commands whose first token (binary name)
/// matches one of these are permitted without further review. All other commands
/// are blocked unless the frontend passes `user_confirmed: true`, indicating the
/// user explicitly approved execution via a native confirmation dialog.
const ALLOWED_COMMANDS: &[&str] = &[
    // File inspection
    "cat", "head", "tail", "wc", "file", "stat", "du", "ls", "find",
    // Text processing
    "grep", "awk", "sed", "sort", "uniq", "cut", "tr", "diff", "jq",
    // Media / document tools
    "ffmpeg", "ffprobe", "convert", "identify", "exiftool", "mediainfo",
    // Compression
    "zip", "unzip", "tar", "gzip", "gunzip", "bzip2", "xz", "7z",
    // Checksums
    "md5sum", "sha256sum", "shasum", "xxhsum", "b2sum",
    // Code tools
    "git", "python3", "node", "npx", "cargo",
    // Editors / viewers (non-destructive)
    "open", "xdg-open", "code", "nano", "less", "bat",
];

/// Patterns that are always blocked regardless of allowlist or confirmation.
const BLOCKED_PATTERNS: &[&str] = &[
    "rm -rf /",
    "mkfs",
    "dd if=",
    "> /dev/sd",
    "chmod 777 /",
    ":(){ :|:& };:",
    "curl | sh",
    "curl | bash",
    "wget | sh",
    "wget | bash",
];

/// Check if a command template's first token is in the allowlist.
fn is_command_allowed(expanded: &str) -> bool {
    // Extract the first token (binary name) from the expanded command
    let first_token = expanded.split_whitespace().next().unwrap_or("");
    // Match against the basename (e.g. "/usr/bin/grep" → "grep")
    let basename = first_token.rsplit('/').next().unwrap_or(first_token);
    ALLOWED_COMMANDS.contains(&basename)
}

/// Check if a command contains any blocked dangerous patterns.
fn contains_blocked_pattern(expanded: &str) -> Option<&'static str> {
    let lower = expanded.to_lowercase();
    BLOCKED_PATTERNS.iter().find(|&&pat| lower.contains(pat)).copied()
}

/// Execute a custom command locally and return the result.
///
/// For remote execution, the frontend should route through the terminal panel.
///
/// Security: Commands are checked against an allowlist. Commands not on the
/// allowlist require `user_confirmed: true` (the frontend must show a native
/// OS confirmation dialog before setting this flag). Dangerous patterns are
/// always blocked.
#[tauri::command]
pub async fn execute_custom_command(
    command_id: String,
    file_paths: Vec<String>,
    remote_base: Option<String>,
    user_confirmed: Option<bool>,
) -> Result<CommandExecutionResult, AppError> {
    let commands = CUSTOM_COMMANDS.read().await;
    let cmd = commands
        .iter()
        .find(|c| c.id == command_id)
        .ok_or_else(|| {
            AppError::file_op(
                format!("Custom command not found: {command_id}"),
                "The command may have been deleted.",
            )
        })?
        .clone();
    drop(commands);

    if cmd.execution_mode == ExecutionMode::Remote {
        return Err(AppError::file_op(
            "Remote commands should be executed via the terminal panel",
            "Use expand_custom_command to get the command string, then send it to the terminal.",
        ));
    }

    // Reject file paths containing null bytes (OS command truncation attack)
    for fp in &file_paths {
        if fp.contains('\0') {
            return Err(AppError::file_op(
                "File path contains null byte",
                "Remove null bytes from the file path.",
            ));
        }
    }

    let expanded = if file_paths.is_empty() {
        cmd.command_template.clone()
    } else {
        file_paths
            .iter()
            .map(|fp| expand_template(&cmd.command_template, fp, remote_base.as_deref()))
            .collect::<Vec<_>>()
            .join(" && ")
    };

    // Block dangerous patterns unconditionally
    if let Some(pattern) = contains_blocked_pattern(&expanded) {
        return Err(AppError::file_op(
            format!("Command blocked: contains dangerous pattern '{pattern}'"),
            "This command pattern is not allowed for safety reasons.",
        ));
    }

    // Check allowlist — if not on the list, require explicit user confirmation
    if !is_command_allowed(&expanded) {
        if user_confirmed != Some(true) {
            return Err(AppError::file_op(
                format!("Command '{}' requires confirmation. The command is not on the safe allowlist.", cmd.name),
                "The frontend must show a confirmation dialog and re-call with user_confirmed: true.",
            ));
        }
        tracing::warn!(
            "Executing non-allowlisted command with user confirmation: {}",
            cmd.name
        );
    }

    // Determine if the command needs a shell (pipes, redirects, &&, etc.)
    let needs_shell = expanded.contains('|')
        || expanded.contains('>')
        || expanded.contains('<')
        || expanded.contains("&&")
        || expanded.contains("||")
        || expanded.contains(';');

    // Execute locally
    let output = if needs_shell {
        // Commands with shell operators must go through sh -c
        tokio::process::Command::new(if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "sh"
        })
        .args(if cfg!(target_os = "windows") {
            vec!["/C", &expanded]
        } else {
            vec!["-c", &expanded]
        })
        .output()
        .await
        .map_err(|e| {
            AppError::file_op(
                format!("Failed to execute command: {e}"),
                "Check that the command is valid.",
            )
        })?
    } else {
        // Simple commands: split with shlex for safe argument handling
        let parts = shlex::split(&expanded).ok_or_else(|| {
            AppError::file_op(
                "Failed to parse command",
                "Check the command template for unmatched quotes.",
            )
        })?;
        if parts.is_empty() {
            return Err(AppError::file_op(
                "Empty command after expansion",
                "Provide a valid command template.",
            ));
        }
        tokio::process::Command::new(&parts[0])
            .args(&parts[1..])
            .output()
            .await
            .map_err(|e| {
                AppError::file_op(
                    format!("Failed to execute command: {e}"),
                    "Check that the command is valid.",
                )
            })?
    };

    Ok(CommandExecutionResult {
        command_id: cmd.id,
        command_expanded: expanded,
        exit_code: output.status.code(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

// ── GUI-to-Script generation ──

/// Generate a shell script equivalent for a file operation.
///
/// Returns a string that can be copied to clipboard or saved as a script.
#[tauri::command]
pub async fn generate_script(
    operation: String,
    source_paths: Vec<String>,
    dest_path: Option<String>,
    options: Option<HashMap<String, String>>,
) -> Result<String, AppError> {
    let mut lines = vec!["#!/bin/bash".to_string(), "set -euo pipefail".to_string(), String::new()];

    match operation.as_str() {
        "copy" => {
            let dest = dest_path.as_deref().unwrap_or(".");
            for src in &source_paths {
                lines.push(format!("cp -r {} {}", shell_quote(src), shell_quote(dest)));
            }
        }
        "move" => {
            let dest = dest_path.as_deref().unwrap_or(".");
            for src in &source_paths {
                lines.push(format!("mv {} {}", shell_quote(src), shell_quote(dest)));
            }
        }
        "delete" => {
            for src in &source_paths {
                lines.push(format!("rm -rf {}", shell_quote(src)));
            }
        }
        "chmod" => {
            let mode = options
                .as_ref()
                .and_then(|o| o.get("mode"))
                .map(|s| s.as_str())
                .unwrap_or("644");
            let recursive = options
                .as_ref()
                .and_then(|o| o.get("recursive"))
                .map(|s| s == "true")
                .unwrap_or(false);
            let flag = if recursive { "-R " } else { "" };
            for src in &source_paths {
                lines.push(format!("chmod {flag}{mode} {}", shell_quote(src)));
            }
        }
        "chown" => {
            let owner = options
                .as_ref()
                .and_then(|o| o.get("owner"))
                .map(|s| s.as_str())
                .unwrap_or("root:root");
            for src in &source_paths {
                lines.push(format!("chown {owner} {}", shell_quote(src)));
            }
        }
        "sync" => {
            let dest = dest_path.as_deref().unwrap_or(".");
            let src = source_paths.first().map(|s| s.as_str()).unwrap_or(".");
            lines.push(format!(
                "rsync -avz --progress {} {}",
                shell_quote(src),
                shell_quote(dest)
            ));
        }
        "sftp_upload" => {
            let dest = dest_path.as_deref().unwrap_or("/remote/path");
            let host = options
                .as_ref()
                .and_then(|o| o.get("host"))
                .map(|s| s.as_str())
                .unwrap_or("user@host");
            for src in &source_paths {
                lines.push(format!("scp {} {}:{}", shell_quote(src), host, shell_quote(dest)));
            }
        }
        "sftp_download" => {
            let dest = dest_path.as_deref().unwrap_or(".");
            let host = options
                .as_ref()
                .and_then(|o| o.get("host"))
                .map(|s| s.as_str())
                .unwrap_or("user@host");
            for src in &source_paths {
                lines.push(format!("scp {}:{} {}", host, shell_quote(src), shell_quote(dest)));
            }
        }
        other => {
            lines.push(format!("# Unsupported operation: {other}"));
            lines.push(format!("# Source paths: {:?}", source_paths));
            if let Some(dest) = &dest_path {
                lines.push(format!("# Destination: {dest}"));
            }
        }
    }

    Ok(lines.join("\n"))
}

// ── Helpers ──

fn expand_template(template: &str, file_path: &str, remote_base: Option<&str>) -> String {
    let p = Path::new(file_path);
    let name = p
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let dir = p
        .parent()
        .map(|d| d.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| e.to_string_lossy().to_string())
        .unwrap_or_default();
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    // All substitutions are shell-quoted to prevent command injection via
    // crafted filenames (e.g. a file named `$(rm -rf /).txt`).
    template
        .replace("{path}", &shell_quote(file_path))
        .replace("{name}", &shell_quote(&name))
        .replace("{stem}", &shell_quote(&stem))
        .replace("{dir}", &shell_quote(&dir))
        .replace("{ext}", &shell_quote(&ext))
        .replace("{remote_path}", &shell_quote(remote_base.unwrap_or(file_path)))
        .replace("{local_path}", &shell_quote(file_path))
}

fn shell_quote(s: &str) -> String {
    // Always wrap in single quotes to prevent injection — even "simple" strings
    // can be crafted to exploit unquoted contexts.
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_template() {
        let result = expand_template(
            "cat {path} | grep {name}",
            "/home/user/docs/report.txt",
            None,
        );
        // All substitutions are now shell-quoted for injection safety
        assert_eq!(result, "cat '/home/user/docs/report.txt' | grep 'report.txt'");
    }

    #[test]
    fn test_expand_template_with_ext() {
        let result = expand_template(
            "convert {path} {dir}/{stem}.png",
            "/photos/image.jpg",
            None,
        );
        assert_eq!(result, "convert '/photos/image.jpg' '/photos'/'image'.png");
    }

    #[test]
    fn test_shell_quote_always_quotes() {
        // shell_quote now ALWAYS wraps in single quotes
        assert_eq!(shell_quote("simple"), "'simple'");
    }

    #[test]
    fn test_shell_quote_spaces() {
        assert_eq!(shell_quote("has spaces"), "'has spaces'");
    }

    #[tokio::test]
    async fn test_generate_script_copy() {
        let script = generate_script(
            "copy".to_string(),
            vec!["/src/file.txt".to_string()],
            Some("/dest".to_string()),
            None,
        )
        .await
        .unwrap();
        assert!(script.contains("cp -r '/src/file.txt' '/dest'"));
    }

    #[tokio::test]
    async fn test_generate_script_chmod() {
        let mut opts = HashMap::new();
        opts.insert("mode".to_string(), "755".to_string());
        opts.insert("recursive".to_string(), "true".to_string());
        let script = generate_script(
            "chmod".to_string(),
            vec!["/var/www".to_string()],
            None,
            Some(opts),
        )
        .await
        .unwrap();
        assert!(script.contains("chmod -R 755 '/var/www'"));
    }
}
