//! Duplicate detection command - find duplicate files by size then hash.

use crate::output;
use crate::OutputFormat;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Serialize)]
struct DuplicateGroup {
    hash: String,
    size: u64,
    files: Vec<String>,
    count: usize,
}

#[derive(Debug, Serialize)]
struct DuplicateReport {
    total_files_scanned: usize,
    duplicate_groups: usize,
    duplicate_files: usize,
    wasted_bytes: u64,
    groups: Vec<DuplicateGroup>,
}

pub async fn execute(
    path: String,
    min_size: u64,
    action: String,
    format: &OutputFormat,
    dry_run: bool,
) -> Result<u8, Box<dyn std::error::Error>> {
    let root = Path::new(&path);
    if !root.exists() || !root.is_dir() {
        output::print_error(format, &format!("Directory not found: {path}"));
        return Ok(2);
    }

    match format {
        OutputFormat::Human => {
            println!("\n  Scanning for duplicates in: {path}");
            println!("  Minimum file size: {} bytes\n", min_size);
        }
        _ => {}
    }

    // Phase 1: Group files by size (fast pass)
    let mut size_groups: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut total_files = 0usize;

    collect_files(root, &mut size_groups, &mut total_files, min_size)?;

    // Remove size groups with only one file
    size_groups.retain(|_, files| files.len() > 1);

    match format {
        OutputFormat::Human => {
            println!("  Scanned {total_files} files");
            println!("  {} size groups with potential duplicates", size_groups.len());
        }
        _ => {}
    }

    // Phase 2: Hash files within same-size groups
    let mut duplicate_groups: Vec<DuplicateGroup> = Vec::new();
    let mut total_wasted = 0u64;

    for (size, files) in &size_groups {
        let mut hash_groups: HashMap<String, Vec<String>> = HashMap::new();

        for file in files {
            let hash = quick_hash(file)?;
            hash_groups
                .entry(hash)
                .or_default()
                .push(file.display().to_string());
        }

        for (hash, paths) in hash_groups {
            if paths.len() > 1 {
                let wasted = size * (paths.len() as u64 - 1);
                total_wasted += wasted;
                duplicate_groups.push(DuplicateGroup {
                    hash,
                    size: *size,
                    files: paths.clone(),
                    count: paths.len(),
                });
            }
        }
    }

    let total_dup_files: usize = duplicate_groups.iter().map(|g| g.count - 1).sum();

    let report = DuplicateReport {
        total_files_scanned: total_files,
        duplicate_groups: duplicate_groups.len(),
        duplicate_files: total_dup_files,
        wasted_bytes: total_wasted,
        groups: duplicate_groups,
    };

    // Output results
    match format {
        OutputFormat::Human => {
            println!();
            if report.groups.is_empty() {
                output::print_success(format, "No duplicates found");
            } else {
                println!(
                    "  Found {} duplicate group(s), {} duplicate file(s), {} wasted",
                    report.duplicate_groups,
                    report.duplicate_files,
                    format_bytes(report.wasted_bytes),
                );
                println!();

                for (i, group) in report.groups.iter().enumerate() {
                    println!(
                        "  Group {} ({}, {})",
                        i + 1,
                        format_bytes(group.size),
                        group.hash.chars().take(12).collect::<String>(),
                    );
                    for file in &group.files {
                        println!("    - {file}");
                    }
                    println!();
                }

                // Handle action
                match action.as_str() {
                    "report" => {
                        println!("  Use --action keep-newest|keep-largest|delete to take action");
                    }
                    "keep-newest" | "keep-largest" | "delete" => {
                        if dry_run {
                            println!("  [dry-run] Would apply action: {action}");
                        } else {
                            apply_action(&report.groups, &action, format)?;
                        }
                    }
                    _ => {
                        output::print_warning(
                            format,
                            &format!("Unknown action '{action}'. Use: report, keep-newest, keep-largest, delete"),
                        );
                    }
                }
            }
            println!();
        }
        _ => output::print_data(format, &report),
    }

    Ok(0)
}

fn collect_files(
    dir: &Path,
    size_groups: &mut HashMap<u64, Vec<PathBuf>>,
    total: &mut usize,
    min_size: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                collect_files(&path, size_groups, total, min_size)?;
            } else if path.is_file() {
                *total += 1;
                if let Ok(meta) = path.metadata() {
                    let size = meta.len();
                    if size >= min_size {
                        size_groups.entry(size).or_default().push(path);
                    }
                }
            }
        }
    }
    Ok(())
}

fn quick_hash(path: &Path) -> Result<String, Box<dyn std::error::Error>> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    // Read first 64KB + last 64KB for quick hash
    let meta = file.metadata()?;
    let size = meta.len();

    let mut hasher_state: u64 = 0xcbf29ce484222325; // FNV offset basis
    let mut buf = vec![0u8; 65536.min(size as usize)];

    let n = file.read(&mut buf)?;
    for &byte in &buf[..n] {
        hasher_state ^= byte as u64;
        hasher_state = hasher_state.wrapping_mul(0x100000001b3); // FNV prime
    }

    // Include file size in hash
    hasher_state ^= size;
    hasher_state = hasher_state.wrapping_mul(0x100000001b3);

    Ok(format!("{hasher_state:016x}"))
}

fn apply_action(
    groups: &[DuplicateGroup],
    action: &str,
    format: &OutputFormat,
) -> Result<(), Box<dyn std::error::Error>> {
    for group in groups {
        let mut files_with_meta: Vec<(String, std::fs::Metadata)> = group
            .files
            .iter()
            .filter_map(|f| {
                std::fs::metadata(f).ok().map(|m| (f.clone(), m))
            })
            .collect();

        if files_with_meta.len() <= 1 {
            continue;
        }

        let keep_idx = match action {
            "keep-newest" => {
                files_with_meta
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, (_, m))| m.modified().ok())
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
            "keep-largest" => {
                files_with_meta
                    .iter()
                    .enumerate()
                    .max_by_key(|(_, (_, m))| m.len())
                    .map(|(i, _)| i)
                    .unwrap_or(0)
            }
            "delete" => usize::MAX, // Delete all
            _ => 0,
        };

        for (i, (path, _)) in files_with_meta.iter().enumerate() {
            if action == "delete" || i != keep_idx {
                match std::fs::remove_file(path) {
                    Ok(()) => {
                        output::print_success(format, &format!("Deleted: {path}"));
                    }
                    Err(e) => {
                        output::print_error(format, &format!("Failed to delete {path}: {e}"));
                    }
                }
            } else {
                output::print_success(format, &format!("Keeping: {path}"));
            }
        }
    }
    Ok(())
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
