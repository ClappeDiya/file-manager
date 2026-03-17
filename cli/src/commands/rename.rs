//! Batch rename command - rename files using patterns and tokens.

use crate::output;
use crate::OutputFormat;
use serde::Serialize;
use std::path::Path;

#[derive(Debug, Serialize)]
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

    // Generate previews
    let mut previews: Vec<RenamePreview> = Vec::new();
    let mut counter = start;

    for file in &files {
        let filename = file
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        let stem = file
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();

        let ext = file
            .extension()
            .map(|e| e.to_string_lossy().to_string())
            .unwrap_or_default();

        let parent_name = file
            .parent()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();

        let date_str = file
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .map(|t| {
                let dt: chrono::DateTime<chrono::Utc> = t.into();
                dt.format("%Y%m%d").to_string()
            })
            .unwrap_or_else(|| "unknown".into());

        let mut new_name = filename.clone();
        let mut warnings: Vec<String> = Vec::new();

        // Apply pattern with token substitution
        if let Some(ref pat) = pattern {
            let counter_str = if pad > 0 {
                format!("{:0>width$}", counter, width = pad)
            } else {
                counter.to_string()
            };

            new_name = pat
                .replace("{name}", &stem)
                .replace("{ext}", &ext)
                .replace("{num}", &counter_str)
                .replace("{date}", &date_str)
                .replace("{parent}", &parent_name)
                .replace("{counter}", &counter_str);

            // Add extension if pattern doesn't include it and original had one
            if !new_name.contains('.') && !ext.is_empty() {
                new_name = format!("{new_name}.{ext}");
            }
        }

        // Apply find/replace
        if let (Some(ref find_str), Some(ref replace_str)) = (&find, &replace) {
            if use_regex {
                match regex::Regex::new(find_str) {
                    Ok(re) => {
                        new_name = re.replace_all(&new_name, replace_str.as_str()).to_string();
                    }
                    Err(e) => {
                        output::print_error(format, &format!("Invalid regex: {e}"));
                        return Ok(2);
                    }
                }
            } else {
                new_name = new_name.replace(find_str.as_str(), replace_str.as_str());
            }
        } else if let Some(ref find_str) = find {
            // Find without replace = delete matches
            if use_regex {
                match regex::Regex::new(find_str) {
                    Ok(re) => {
                        new_name = re.replace_all(&new_name, "").to_string();
                    }
                    Err(e) => {
                        output::print_error(format, &format!("Invalid regex: {e}"));
                        return Ok(2);
                    }
                }
            } else {
                new_name = new_name.replace(find_str.as_str(), "");
            }
        }

        // Apply case transformation
        if let Some(ref case_type) = case {
            new_name = match case_type.to_lowercase().as_str() {
                "upper" => new_name.to_uppercase(),
                "lower" => new_name.to_lowercase(),
                "title" => title_case(&new_name),
                "sentence" => sentence_case(&new_name),
                _ => {
                    output::print_error(
                        format,
                        &format!("Unknown case type '{case_type}'. Use: upper, lower, title, sentence"),
                    );
                    return Ok(2);
                }
            };
        }

        // Check for compatibility issues (basic checks)
        let invalid_chars = ['<', '>', ':', '"', '|', '?', '*'];
        for ch in &invalid_chars {
            if new_name.contains(*ch) {
                warnings.push(format!("Contains '{ch}' which is invalid on Windows"));
            }
        }

        if new_name.is_empty() {
            warnings.push("Resulting filename would be empty!".into());
        }

        previews.push(RenamePreview {
            original: filename,
            renamed: new_name,
            warnings,
        });

        counter += step;
    }

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

    // Apply renames unless dry-run
    if !dry_run && report.renamed > 0 {
        let mut success_count = 0;
        let mut fail_count = 0;

        for (i, preview) in report.previews.iter().enumerate() {
            if preview.original == preview.renamed {
                continue;
            }

            let old_path = &files[i];
            let new_path = old_path.parent().unwrap_or(Path::new(".")).join(&preview.renamed);

            match std::fs::rename(old_path, &new_path) {
                Ok(()) => success_count += 1,
                Err(e) => {
                    fail_count += 1;
                    output::print_error(format, &format!("Failed to rename '{}': {e}", preview.original));
                }
            }
        }

        match format {
            OutputFormat::Human => {
                println!();
                output::print_success(
                    format,
                    &format!("{success_count} file(s) renamed, {fail_count} failed"),
                );
            }
            _ => {}
        }

        if fail_count > 0 {
            return Ok(1); // Partial failure
        }
    } else if dry_run {
        match format {
            OutputFormat::Human => {
                println!("\n  [dry-run] No files were renamed\n");
            }
            _ => {}
        }
    }

    Ok(0)
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => {
                    let upper: String = first.to_uppercase().collect();
                    let rest: String = chars.collect::<String>().to_lowercase();
                    format!("{upper}{rest}")
                }
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn sentence_case(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut chars = lower.chars();
    match chars.next() {
        Some(first) => {
            let upper: String = first.to_uppercase().collect();
            format!("{upper}{}", chars.collect::<String>())
        }
        None => String::new(),
    }
}
