//! Batch rename command - rename files using patterns and tokens.

use crate::output;
use crate::OutputFormat;
use serde::Serialize;
use std::path::Path;
use ufop_core::naming::{plan_rename, RenameSpec};

#[derive(Debug, Clone, Serialize)]
struct RenamePreview {
    original: String,
    renamed: String,
    warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
struct RenameReport {
    total_files: usize,
    renamed: usize,
    skipped: usize,
    previews: Vec<RenamePreview>,
}

#[allow(clippy::too_many_arguments)]
pub async fn execute(
    path: String,
    pattern: Option<String>,
    find: Option<String>,
    replace: Option<String>,
    use_regex: bool,
    case: Option<String>,
    start: u32,
    step: u32,
    pad: usize,
    format: &OutputFormat,
    dry_run: bool,
) -> Result<u8, Box<dyn std::error::Error>> {
    let root = Path::new(&path);

    // Collect files to rename
    let files: Vec<std::path::PathBuf> = if root.is_dir() {
        let mut entries: Vec<_> = std::fs::read_dir(root)?
            .filter_map(|e| e.ok())
            .filter(|e| e.path().is_file())
            .map(|e| e.path())
            .collect();
        entries.sort();
        entries
    } else if root.is_file() {
        vec![root.to_path_buf()]
    } else {
        output::print_error(format, &format!("Path not found: {path}"));
        return Ok(2);
    };

    if files.is_empty() {
        output::print_warning(format, "No files found to rename");
        return Ok(0);
    }

    // Plan with the shared pipeline (same as the desktop batch rename).
    let spec = RenameSpec {
        pattern,
        find,
        replace,
        use_regex,
        case_transform: case,
        counter_start: start as u64,
        counter_step: step as u64,
        counter_pad: pad,
    };
    let plan = match plan_rename(&files, &spec) {
        Ok(p) => p,
        Err(e) => {
            output::print_error(format, &e.to_string());
            return Ok(2);
        }
    };
    let previews: Vec<RenamePreview> = plan
        .iter()
        .map(|e| RenamePreview { original: e.original_name.clone(), renamed: e.new_name.clone(), warnings: e.warnings.clone() })
        .collect();

    let report = RenameReport {
        total_files: files.len(),
        renamed: previews.iter().filter(|p| p.original != p.renamed).count(),
        skipped: previews.iter().filter(|p| p.original == p.renamed).count(),
        previews: previews.clone(),
    };

    // Output preview
    match format {
        OutputFormat::Human => {
            println!("\n  Batch Rename Preview ({} files):\n", report.total_files);

            for preview in &report.previews {
                if preview.original == preview.renamed {
                    println!("  \x1b[90m{}\x1b[0m (unchanged)", preview.original);
                } else {
                    println!(
                        "  {} -> \x1b[32m{}\x1b[0m",
                        preview.original, preview.renamed
                    );
                }
                for warning in &preview.warnings {
                    println!("    \x1b[33m! {warning}\x1b[0m");
                }
            }

            println!(
                "\n  {} will be renamed, {} unchanged",
                report.renamed, report.skipped
            );
        }
        _ => output::print_data(format, &report),
    }

    // Apply renames unless dry-run: refuses collisions with other entries or
    // existing files, and renames in two phases so chains and swaps work.
    if !dry_run && report.renamed > 0 {
        match ufop_core::naming::rename::apply(&plan) {
            Ok(outcome) => {
                if let OutputFormat::Human = format {
                    println!();
                    output::print_success(format, &format!("{} file(s) renamed", outcome.renamed.len()));
                }
            }
            Err(reason) => {
                output::print_error(format, &format!("Nothing was renamed: {reason}"));
                return Ok(1);
            }
        }
    } else if dry_run {
        if let OutputFormat::Human = format {
            println!("\n  [dry-run] No files were renamed\n");
        }
    }

    Ok(0)
}
