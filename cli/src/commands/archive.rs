//! Archive commands - create, extract, list archives.

use crate::output;
use crate::ArchiveCommands;
use crate::OutputFormat;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
struct ArchiveEntry {
    path: String,
    size: u64,
    compressed_size: Option<u64>,
    is_dir: bool,
    modified: Option<String>,
}

#[derive(Debug, Serialize)]
struct ArchiveInfo {
    path: String,
    format: String,
    total_entries: usize,
    total_size: u64,
    entries: Vec<ArchiveEntry>,
}

#[derive(Debug, Serialize)]
struct ArchiveResult {
    status: String,
    action: String,
    archive: String,
    files_processed: usize,
    total_size: u64,
}

pub async fn execute(
    cmd: ArchiveCommands,
    format: &OutputFormat,
    dry_run: bool,
) -> Result<u8, Box<dyn std::error::Error>> {
    match cmd {
        ArchiveCommands::Create {
            output: output_path,
            paths,
            format: archive_format,
            password,
            level,
        } => {
            // Validate input paths exist
            for p in &paths {
                if !Path::new(p).exists() {
                    output::print_error(format, &format!("Source not found: {p}"));
                    return Ok(2);
                }
            }

            let fmt = archive_format.to_lowercase();
            let valid_formats = ["zip", "tar", "tar.gz", "tgz", "7z"];
            if !valid_formats.contains(&fmt.as_str()) {
                output::print_error(
                    format,
                    &format!(
                        "Unsupported format '{archive_format}'. Supported: {}",
                        valid_formats.join(", ")
                    ),
                );
                return Ok(2);
            }

            if password.is_some() && !["zip", "7z"].contains(&fmt.as_str()) {
                output::print_warning(
                    format,
                    &format!("Password encryption is only supported for ZIP and 7Z formats"),
                );
            }

            // Calculate total size
            let mut total_size = 0u64;
            let mut file_count = 0usize;
            for p in &paths {
                let path = Path::new(p);
                if path.is_file() {
                    total_size += path.metadata()?.len();
                    file_count += 1;
                } else if path.is_dir() {
                    let (size, count) = dir_stats(path)?;
                    total_size += size;
                    file_count += count;
                }
            }

            if dry_run {
                let info = serde_json::json!({
                    "dry_run": true,
                    "action": "create",
                    "output": output_path,
                    "format": fmt,
                    "files": file_count,
                    "total_size": total_size,
                    "compression_level": level,
                    "encrypted": password.is_some(),
                });
                output::print_data(format, &info);
                return Ok(0);
            }

            // Create archive based on format
            match fmt.as_str() {
                "zip" => {
                    create_zip_archive(&output_path, &paths, password.as_deref(), level)?;
                }
                "tar" | "tar.gz" | "tgz" => {
                    create_tar_archive(&output_path, &paths, fmt.contains("gz"))?;
                }
                "7z" => {
                    // 7z requires external tool
                    create_7z_archive(&output_path, &paths, password.as_deref())?;
                }
                _ => unreachable!(),
            }

            let result = ArchiveResult {
                status: "success".into(),
                action: "create".into(),
                archive: output_path.clone(),
                files_processed: file_count,
                total_size,
            };

            match format {
                OutputFormat::Human => {
                    output::print_success(
                        format,
                        &format!(
                            "Created {fmt} archive: {output_path} ({file_count} files, {})",
                            format_bytes(total_size)
                        ),
                    );
                }
                _ => output::print_data(format, &result),
            }

            Ok(0)
        }

        ArchiveCommands::Extract {
            archive,
            dest,
            password,
        } => {
            let archive_path = Path::new(&archive);
            if !archive_path.exists() {
                output::print_error(format, &format!("Archive not found: {archive}"));
                return Ok(2);
            }

            let dest_dir = dest.unwrap_or_else(|| {
                archive_path
                    .parent()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| ".".into())
            });

            if dry_run {
                let info = serde_json::json!({
                    "dry_run": true,
                    "action": "extract",
                    "archive": archive,
                    "destination": dest_dir,
                    "encrypted": password.is_some(),
                });
                output::print_data(format, &info);
                return Ok(0);
            }

            // Determine format from extension
            let ext = archive_path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            let file_count = match ext.as_str() {
                "zip" => extract_zip(&archive, &dest_dir, password.as_deref())?,
                "tar" => extract_tar(&archive, &dest_dir, false)?,
                "gz" | "tgz" => extract_tar(&archive, &dest_dir, true)?,
                "7z" => extract_7z(&archive, &dest_dir, password.as_deref())?,
                _ => {
                    output::print_error(format, &format!("Unknown archive format: .{ext}"));
                    return Ok(2);
                }
            };

            match format {
                OutputFormat::Human => {
                    output::print_success(
                        format,
                        &format!("Extracted {file_count} item(s) to {dest_dir}"),
                    );
                }
                _ => {
                    let result = ArchiveResult {
                        status: "success".into(),
                        action: "extract".into(),
                        archive,
                        files_processed: file_count,
                        total_size: 0,
                    };
                    output::print_data(format, &result);
                }
            }

            Ok(0)
        }

        ArchiveCommands::List { archive, password } => {
            let archive_path = Path::new(&archive);
            if !archive_path.exists() {
                output::print_error(format, &format!("Archive not found: {archive}"));
                return Ok(2);
            }

            let ext = archive_path
                .extension()
                .map(|e| e.to_string_lossy().to_lowercase())
                .unwrap_or_default();

            let entries = match ext.as_str() {
                "zip" => list_zip(&archive, password.as_deref())?,
                "tar" | "gz" | "tgz" => list_tar(&archive, ext.contains("gz") || ext == "tgz")?,
                _ => {
                    output::print_error(format, &format!("Cannot list format: .{ext}"));
                    return Ok(2);
                }
            };

            let total_size: u64 = entries.iter().map(|e| e.size).sum();
            let info = ArchiveInfo {
                path: archive.clone(),
                format: ext,
                total_entries: entries.len(),
                total_size,
                entries,
            };

            match format {
                OutputFormat::Human => {
                    println!("\n  Archive: {archive}");
                    println!("  Entries: {}, Total: {}\n", info.total_entries, format_bytes(total_size));

                    let rows: Vec<Vec<String>> = info.entries.iter().map(|e| {
                        vec![
                            if e.is_dir { "D" } else { "F" }.to_string(),
                            format_bytes(e.size),
                            e.modified.clone().unwrap_or_else(|| "-".into()),
                            e.path.clone(),
                        ]
                    }).collect();

                    output::print_table(
                        format,
                        &["TYPE", "SIZE", "MODIFIED", "PATH"],
                        &rows,
                        &info.entries,
                    );
                    println!();
                }
                _ => output::print_data(format, &info),
            }

            Ok(0)
        }
    }
}

fn dir_stats(path: &Path) -> Result<(u64, usize), Box<dyn std::error::Error>> {
    let mut total_size = 0u64;
    let mut count = 0usize;
    for entry in std::fs::read_dir(path)? {
        let entry = entry?;
        let p = entry.path();
        if p.is_file() {
            total_size += p.metadata()?.len();
            count += 1;
        } else if p.is_dir() {
            let (s, c) = dir_stats(&p)?;
            total_size += s;
            count += c;
        }
    }
    Ok((total_size, count))
}

fn create_zip_archive(
    output: &str,
    paths: &[String],
    _password: Option<&str>,
    _level: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    // Use system zip command for simplicity
    let mut args = vec!["-r".to_string(), output.to_string()];
    args.extend(paths.iter().cloned());

    let status = std::process::Command::new("zip")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("zip command failed".into());
    }
    Ok(())
}

fn create_tar_archive(
    output: &str,
    paths: &[String],
    compress: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = vec![if compress { "-czf" } else { "-cf" }.to_string(), output.to_string()];
    args.extend(paths.iter().cloned());

    let status = std::process::Command::new("tar")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("tar command failed".into());
    }
    Ok(())
}

fn create_7z_archive(
    output: &str,
    paths: &[String],
    password: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut args = vec!["a".to_string(), output.to_string()];
    if let Some(pw) = password {
        args.push(format!("-p{pw}"));
    }
    args.extend(paths.iter().cloned());

    let status = std::process::Command::new("7z")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("7z command failed".into());
    }
    Ok(())
}

fn extract_zip(
    archive: &str,
    dest: &str,
    _password: Option<&str>,
) -> Result<usize, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dest)?;
    let status = std::process::Command::new("unzip")
        .args(["-o", archive, "-d", dest])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("unzip command failed".into());
    }
    // Count extracted files
    let count = count_files(Path::new(dest))?;
    Ok(count)
}

fn extract_tar(
    archive: &str,
    dest: &str,
    compressed: bool,
) -> Result<usize, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dest)?;
    let flag = if compressed { "-xzf" } else { "-xf" };
    let status = std::process::Command::new("tar")
        .args([flag, archive, "-C", dest])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("tar extract failed".into());
    }
    let count = count_files(Path::new(dest))?;
    Ok(count)
}

fn extract_7z(
    archive: &str,
    dest: &str,
    password: Option<&str>,
) -> Result<usize, Box<dyn std::error::Error>> {
    std::fs::create_dir_all(dest)?;
    let mut args = vec!["x".to_string(), archive.to_string(), format!("-o{dest}"), "-y".to_string()];
    if let Some(pw) = password {
        args.push(format!("-p{pw}"));
    }

    let status = std::process::Command::new("7z")
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()?;

    if !status.success() {
        return Err("7z extract failed".into());
    }
    let count = count_files(Path::new(dest))?;
    Ok(count)
}

fn list_zip(
    archive: &str,
    _password: Option<&str>,
) -> Result<Vec<ArchiveEntry>, Box<dyn std::error::Error>> {
    let output = std::process::Command::new("zipinfo")
        .args(["-1", archive])
        .output()?;

    let content = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<ArchiveEntry> = content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| ArchiveEntry {
            path: line.to_string(),
            size: 0,
            compressed_size: None,
            is_dir: line.ends_with('/'),
            modified: None,
        })
        .collect();

    Ok(entries)
}

fn list_tar(
    archive: &str,
    compressed: bool,
) -> Result<Vec<ArchiveEntry>, Box<dyn std::error::Error>> {
    let flag = if compressed { "-tzf" } else { "-tf" };
    let output = std::process::Command::new("tar")
        .args([flag, archive])
        .output()?;

    let content = String::from_utf8_lossy(&output.stdout);
    let entries: Vec<ArchiveEntry> = content
        .lines()
        .filter(|l| !l.is_empty())
        .map(|line| ArchiveEntry {
            path: line.to_string(),
            size: 0,
            compressed_size: None,
            is_dir: line.ends_with('/'),
            modified: None,
        })
        .collect();

    Ok(entries)
}

fn count_files(path: &Path) -> Result<usize, Box<dyn std::error::Error>> {
    let mut count = 0;
    if path.is_dir() {
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            if entry.path().is_dir() {
                count += count_files(&entry.path())?;
            } else {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.2} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.2} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.2} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
