//! Archive Tools commands (T-062).
//!
//! Browse archives as virtual folders, extract, create (ZIP, TAR, TAR.GZ, 7Z),
//! password-protected archives, and large archive queue management.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Sanitize an archive path to prevent flag injection.
/// Rejects paths starting with '-' and prefixes relative paths with './' .
fn sanitize_archive_path(path: &str) -> Result<String, AppError> {
    if path.starts_with('-') {
        return Err(AppError::file_op(
            "Invalid archive path: cannot start with '-'",
            "Rename the file to remove the leading dash.",
        ));
    }
    // Prefix relative paths with ./ to prevent flag injection
    if !path.starts_with('/') && !path.starts_with("./") {
        Ok(format!("./{}", path))
    } else {
        Ok(path.to_string())
    }
}

/// Virtual folder entry when browsing an archive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveEntry {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub compressed_size: Option<u64>,
    pub is_dir: bool,
    pub modified: Option<String>,
    pub permissions: Option<String>,
}

/// Result of browsing an archive as a virtual folder.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveBrowseResult {
    pub archive_path: String,
    pub current_dir: String,
    pub entries: Vec<ArchiveEntry>,
    pub total_files: usize,
    pub total_dirs: usize,
    pub total_size: u64,
}

/// Create archive parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateArchiveParams {
    pub output_path: String,
    pub source_paths: Vec<String>,
    pub format: String,
    pub password: Option<String>,
    pub compression_level: Option<u32>,
}

/// Extract archive parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractArchiveParams {
    pub archive_path: String,
    pub dest_dir: Option<String>,
    pub password: Option<String>,
    pub selected_entries: Option<Vec<String>>,
}

/// Archive operation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveOpResult {
    pub success: bool,
    pub action: String,
    pub archive_path: String,
    pub files_processed: usize,
    pub total_size: u64,
    pub errors: Vec<String>,
    pub queue_id: Option<String>,
}

/// Large archive queue tracking.
const LARGE_ARCHIVE_THRESHOLD: u64 = 100 * 1024 * 1024; // 100 MB

/// Browse an archive as a virtual folder.
#[tauri::command]
pub async fn archive_browse(
    archive_path: String,
    sub_path: Option<String>,
) -> Result<ArchiveBrowseResult, AppError> {
    let path = Path::new(&archive_path);
    if !path.exists() {
        return Err(AppError::not_found(format!("Archive not found: {archive_path}")));
    }

    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    let current_dir = sub_path.unwrap_or_default();

    let all_entries = match ext.as_str() {
        "zip" => list_zip_entries(&archive_path)?,
        "tar" => list_tar_entries(&archive_path, false)?,
        "gz" | "tgz" => list_tar_entries(&archive_path, true)?,
        "7z" => list_7z_entries(&archive_path)?,
        _ => return Err(AppError::file_op(
            format!("Unsupported archive format: .{ext}"),
            "Supported: zip, tar, tar.gz, 7z",
        )),
    };

    // Filter to current subdirectory
    let filtered: Vec<ArchiveEntry> = if current_dir.is_empty() {
        // Root level: show only direct children
        all_entries
            .iter()
            .filter(|e| {
                let parts: Vec<&str> = e.path.trim_end_matches('/').split('/').collect();
                parts.len() == 1
            })
            .cloned()
            .collect()
    } else {
        let prefix = if current_dir.ends_with('/') {
            current_dir.clone()
        } else {
            format!("{current_dir}/")
        };

        all_entries
            .iter()
            .filter(|e| {
                if !e.path.starts_with(&prefix) {
                    return false;
                }
                let relative = &e.path[prefix.len()..];
                let parts: Vec<&str> = relative.trim_end_matches('/').split('/').collect();
                parts.len() == 1 && !parts[0].is_empty()
            })
            .cloned()
            .collect()
    };

    let total_files = filtered.iter().filter(|e| !e.is_dir).count();
    let total_dirs = filtered.iter().filter(|e| e.is_dir).count();
    let total_size: u64 = filtered.iter().map(|e| e.size).sum();

    Ok(ArchiveBrowseResult {
        archive_path,
        current_dir,
        entries: filtered,
        total_files,
        total_dirs,
        total_size,
    })
}

/// Create an archive.
#[tauri::command]
pub async fn archive_create(params: CreateArchiveParams) -> Result<ArchiveOpResult, AppError> {
    // Validate sources
    for src in &params.source_paths {
        if !Path::new(src).exists() {
            return Err(AppError::not_found(format!("Source not found: {src}")));
        }
    }

    let fmt = params.format.to_lowercase();
    if !["zip", "tar", "tar.gz", "tgz", "7z"].contains(&fmt.as_str()) {
        return Err(AppError::validation(format!(
            "Unsupported format: {fmt}. Use: zip, tar, tar.gz, 7z"
        )));
    }

    // Calculate total size for queue decision
    let total_size = params
        .source_paths
        .iter()
        .map(|p| calculate_dir_size(Path::new(p)).unwrap_or(0))
        .sum::<u64>();

    let is_large = total_size > LARGE_ARCHIVE_THRESHOLD;
    let queue_id = if is_large {
        Some(uuid::Uuid::new_v4().to_string())
    } else {
        None
    };

    // Execute archive creation. The earlier validation guarantees `fmt` is
    // in the allow-list, so the catch-all arm is dead under correct logic —
    // but returning a structured Err instead of panicking keeps the IPC
    // handler alive if the two lists ever drift apart.
    let result = match fmt.as_str() {
        "zip" => create_zip(&params.output_path, &params.source_paths, params.password.as_deref()),
        "tar" => create_tar(&params.output_path, &params.source_paths, false),
        "tar.gz" | "tgz" => create_tar(&params.output_path, &params.source_paths, true),
        "7z" => create_7z(&params.output_path, &params.source_paths, params.password.as_deref()),
        _ => Err(format!(
            "archive_create: format '{fmt}' passed allow-list but no handler matched — validation and match arms are out of sync"
        ).into()),
    };

    match result {
        Ok(count) => Ok(ArchiveOpResult {
            success: true,
            action: "create".into(),
            archive_path: params.output_path,
            files_processed: count,
            total_size,
            errors: vec![],
            queue_id,
        }),
        Err(e) => Ok(ArchiveOpResult {
            success: false,
            action: "create".into(),
            archive_path: params.output_path,
            files_processed: 0,
            total_size,
            errors: vec![e.to_string()],
            queue_id,
        }),
    }
}

/// Extract an archive.
#[tauri::command]
pub async fn archive_extract(params: ExtractArchiveParams) -> Result<ArchiveOpResult, AppError> {
    let archive_path = Path::new(&params.archive_path);
    if !archive_path.exists() {
        return Err(AppError::not_found(format!(
            "Archive not found: {}",
            params.archive_path
        )));
    }

    let dest = params.dest_dir.unwrap_or_else(|| {
        archive_path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| ".".into())
    });

    // Create destination
    std::fs::create_dir_all(&dest).map_err(|e| {
        AppError::file_op(
            format!("Cannot create destination: {e}"),
            "Check permissions",
        )
    })?;

    let ext = archive_path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();

    // Check if large archive
    let file_size = archive_path.metadata().map(|m| m.len()).unwrap_or(0);
    let queue_id = if file_size > LARGE_ARCHIVE_THRESHOLD {
        Some(uuid::Uuid::new_v4().to_string())
    } else {
        None
    };

    let result = match ext.as_str() {
        "zip" => extract_zip_archive(&params.archive_path, &dest, params.password.as_deref()),
        "tar" => extract_tar_archive(&params.archive_path, &dest, false),
        "gz" | "tgz" => extract_tar_archive(&params.archive_path, &dest, true),
        "7z" => extract_7z_archive(&params.archive_path, &dest, params.password.as_deref()),
        _ => return Err(AppError::file_op(
            format!("Unsupported format: .{ext}"),
            "Supported: zip, tar, tar.gz, 7z",
        )),
    };

    match result {
        Ok(count) => Ok(ArchiveOpResult {
            success: true,
            action: "extract".into(),
            archive_path: params.archive_path,
            files_processed: count,
            total_size: file_size,
            errors: vec![],
            queue_id,
        }),
        Err(e) => Ok(ArchiveOpResult {
            success: false,
            action: "extract".into(),
            archive_path: params.archive_path,
            files_processed: 0,
            total_size: file_size,
            errors: vec![e.to_string()],
            queue_id,
        }),
    }
}

/// Get archive info without extracting.
#[tauri::command]
pub async fn archive_info(archive_path: String) -> Result<ArchiveBrowseResult, AppError> {
    archive_browse(archive_path, None).await
}

// ── Internal helpers ──

fn list_zip_entries(path: &str) -> Result<Vec<ArchiveEntry>, AppError> {
    let safe_path = sanitize_archive_path(path)?;
    let output = std::process::Command::new("zipinfo")
        .args(["-1", &safe_path])
        .output()
        .map_err(|e| AppError::internal(format!("zipinfo failed: {e}")))?;

    let content = String::from_utf8_lossy(&output.stdout);
    Ok(content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let is_dir = line.ends_with('/');
            ArchiveEntry {
                name: line
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(line)
                    .to_string(),
                path: line.to_string(),
                size: 0,
                compressed_size: None,
                is_dir,
                modified: None,
                permissions: None,
            }
        })
        .collect())
}

fn list_tar_entries(path: &str, compressed: bool) -> Result<Vec<ArchiveEntry>, AppError> {
    let safe_path = sanitize_archive_path(path)?;
    let flag = if compressed { "-tzf" } else { "-tf" };
    let output = std::process::Command::new("tar")
        .args([flag, &safe_path])
        .output()
        .map_err(|e| AppError::internal(format!("tar list failed: {e}")))?;

    let content = String::from_utf8_lossy(&output.stdout);
    Ok(content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| {
            let is_dir = line.ends_with('/');
            ArchiveEntry {
                name: line
                    .trim_end_matches('/')
                    .rsplit('/')
                    .next()
                    .unwrap_or(line)
                    .to_string(),
                path: line.to_string(),
                size: 0,
                compressed_size: None,
                is_dir,
                modified: None,
                permissions: None,
            }
        })
        .collect())
}

fn list_7z_entries(path: &str) -> Result<Vec<ArchiveEntry>, AppError> {
    let safe_path = sanitize_archive_path(path)?;
    let output = std::process::Command::new("7z")
        .args(["l", "-slt", &safe_path])
        .output()
        .map_err(|e| AppError::internal(format!("7z list failed: {e}")))?;

    let content = String::from_utf8_lossy(&output.stdout);
    let mut entries = Vec::new();
    let mut current_path = String::new();
    let mut current_size = 0u64;
    let mut is_dir = false;

    for line in content.lines() {
        if let Some(p) = line.strip_prefix("Path = ") {
            if !current_path.is_empty() {
                entries.push(ArchiveEntry {
                    name: current_path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&current_path)
                        .to_string(),
                    path: current_path.clone(),
                    size: current_size,
                    compressed_size: None,
                    is_dir,
                    modified: None,
                    permissions: None,
                });
            }
            current_path = p.to_string();
            current_size = 0;
            is_dir = false;
        } else if let Some(s) = line.strip_prefix("Size = ") {
            current_size = s.trim().parse().unwrap_or(0);
        } else if line.starts_with("Folder = +") {
            is_dir = true;
        }
    }

    if !current_path.is_empty() {
        entries.push(ArchiveEntry {
            name: current_path
                .rsplit('/')
                .next()
                .unwrap_or(&current_path)
                .to_string(),
            path: current_path,
            size: current_size,
            compressed_size: None,
            is_dir,
            modified: None,
            permissions: None,
        });
    }

    Ok(entries)
}

fn create_zip(
    output: &str,
    sources: &[String],
    password: Option<&str>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let safe_output = sanitize_archive_path(output).map_err(|e| e.to_string())?;
    let safe_sources: Vec<String> = sources
        .iter()
        .map(|s| sanitize_archive_path(s).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(pw) = password {
        // Use 7z for password-protected archives to avoid exposing password
        // in process arguments. 7z supports stdin password via -si flag.
        // Fallback to zip -P only if 7z is unavailable.
        if which_exists("7z") {
            return create_7z_with_stdin_password(
                &safe_output, &safe_sources, pw,
            );
        }
        // zip does not support stdin passwords — -P is the only option.
        // This is a known limitation; the password is briefly visible in `ps`.
        tracing::warn!("Using zip -P (password visible in process list). Install 7z for safer password handling.");
        let mut args = vec!["-r".to_string(), "-P".to_string(), pw.to_string()];
        args.push(safe_output);
        args.extend(safe_sources.iter().cloned());

        let status = std::process::Command::new("zip")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;

        if !status.success() {
            return Err("zip creation failed".into());
        }
        return Ok(sources.len());
    }

    // No password — straightforward zip
    let mut args = vec!["-r".to_string(), safe_output];
    args.extend(safe_sources.iter().cloned());

    let status = std::process::Command::new("zip")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("zip creation failed".into());
    }
    Ok(sources.len())
}

/// Check if a command exists in PATH.
fn which_exists(cmd: &str) -> bool {
    std::process::Command::new("which")
        .arg(cmd)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Create a 7z archive with password piped via stdin (not visible in `ps`).
fn create_7z_with_stdin_password(
    output: &str,
    sources: &[String],
    password: &str,
) -> Result<usize, Box<dyn std::error::Error>> {
    use std::io::Write;
    let count = sources.len();
    let mut args = vec!["a".to_string(), format!("-p"), output.to_string()];
    args.extend(sources.iter().cloned());

    let mut child = std::process::Command::new("7z")
        .args(&args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(password.as_bytes())?;
        stdin.write_all(b"\n")?;
    }

    let status = child.wait()?;
    if !status.success() {
        return Err("7z creation with password failed".into());
    }
    Ok(count)
}

fn create_tar(
    output: &str,
    sources: &[String],
    compress: bool,
) -> Result<usize, Box<dyn std::error::Error>> {
    let safe_output = sanitize_archive_path(output).map_err(|e| e.to_string())?;
    let safe_sources: Vec<String> = sources
        .iter()
        .map(|s| sanitize_archive_path(s).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;
    let flag = if compress { "-czf" } else { "-cf" };
    let mut args = vec![flag.to_string(), safe_output];
    args.extend(safe_sources.iter().cloned());

    let status = std::process::Command::new("tar")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("tar creation failed".into());
    }
    Ok(sources.len())
}

fn create_7z(
    output: &str,
    sources: &[String],
    password: Option<&str>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let safe_output = sanitize_archive_path(output).map_err(|e| e.to_string())?;
    let safe_sources: Vec<String> = sources
        .iter()
        .map(|s| sanitize_archive_path(s).map_err(|e| e.to_string()))
        .collect::<Result<Vec<_>, _>>()?;

    if let Some(pw) = password {
        return create_7z_with_stdin_password(&safe_output, &safe_sources, pw);
    }

    let mut args = vec!["a".to_string(), safe_output];
    args.extend(safe_sources.iter().cloned());

    let status = std::process::Command::new("7z")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("7z creation failed".into());
    }
    Ok(sources.len())
}

fn extract_zip_archive(
    archive: &str,
    dest: &str,
    password: Option<&str>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let safe_archive = sanitize_archive_path(archive).map_err(|e| e.to_string())?;

    if let Some(pw) = password {
        // Prefer 7z for extraction with password (stdin-safe)
        if which_exists("7z") {
            let args = vec![
                "x".to_string(),
                safe_archive.clone(),
                format!("-o{dest}"),
                "-y".to_string(),
                "-p".to_string(),
            ];
            use std::io::Write;
            let mut child = std::process::Command::new("7z")
                .args(&args)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            if let Some(mut stdin) = child.stdin.take() {
                stdin.write_all(pw.as_bytes())?;
                stdin.write_all(b"\n")?;
            }
            let status = child.wait()?;
            if !status.success() {
                return Err("7z extraction with password failed".into());
            }
            return Ok(count_items(Path::new(dest))?);
        }
        // Fallback: unzip -P (password visible in process list)
        tracing::warn!("Using unzip -P (password visible in process list). Install 7z for safer password handling.");
        let args = vec![format!("-P{pw}"), "-o".to_string(), safe_archive, "-d".to_string(), dest.to_string()];
        let status = std::process::Command::new("unzip")
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        if !status.success() {
            return Err("unzip failed".into());
        }
        return Ok(count_items(Path::new(dest))?);
    }

    let args = vec!["-o".to_string(), safe_archive, "-d".to_string(), dest.to_string()];
    let status = std::process::Command::new("unzip")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("unzip failed".into());
    }
    Ok(count_items(Path::new(dest))?)
}

fn extract_tar_archive(
    archive: &str,
    dest: &str,
    compressed: bool,
) -> Result<usize, Box<dyn std::error::Error>> {
    let safe_archive = sanitize_archive_path(archive).map_err(|e| e.to_string())?;
    let flag = if compressed { "-xzf" } else { "-xf" };
    let status = std::process::Command::new("tar")
        .args([flag, &safe_archive, "-C", dest])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("tar extract failed".into());
    }
    Ok(count_items(Path::new(dest))?)
}

fn extract_7z_archive(
    archive: &str,
    dest: &str,
    password: Option<&str>,
) -> Result<usize, Box<dyn std::error::Error>> {
    let safe_archive = sanitize_archive_path(archive).map_err(|e| e.to_string())?;

    if let Some(pw) = password {
        // Pipe password via stdin to avoid exposure in process args
        use std::io::Write;
        let args = vec![
            "x".to_string(),
            safe_archive,
            format!("-o{dest}"),
            "-y".to_string(),
            "-p".to_string(),
        ];
        let mut child = std::process::Command::new("7z")
            .args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(pw.as_bytes())?;
            stdin.write_all(b"\n")?;
        }
        let status = child.wait()?;
        if !status.success() {
            return Err("7z extraction failed".into());
        }
        return Ok(count_items(Path::new(dest))?);
    }

    let args = vec![
        "x".to_string(),
        safe_archive,
        format!("-o{dest}"),
        "-y".to_string(),
    ];

    let status = std::process::Command::new("7z")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("7z extract failed".into());
    }
    Ok(count_items(Path::new(dest))?)
}

fn calculate_dir_size(path: &Path) -> Result<u64, std::io::Error> {
    if path.is_file() {
        return Ok(path.metadata()?.len());
    }
    let mut total = 0u64;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            total += calculate_dir_size(&entry.path())?;
        }
    }
    Ok(total)
}

fn count_items(path: &Path) -> Result<usize, std::io::Error> {
    let mut count = 0;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.path().is_dir() {
                count += count_items(&entry.path())?;
            } else {
                count += 1;
            }
        }
    }
    Ok(count)
}
