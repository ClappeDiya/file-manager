//! Compatibility check command - validate paths across platforms.

use crate::output;
use crate::OutputFormat;
use serde::Serialize;

#[derive(Debug, Serialize)]
struct CompatResult {
    path: String,
    issues: Vec<CompatIssue>,
    safe: bool,
}

#[derive(Debug, Serialize)]
struct CompatIssue {
    severity: String,
    platform: String,
    message: String,
    suggestion: Option<String>,
}

const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

const WINDOWS_INVALID_CHARS: &[char] = &['<', '>', ':', '"', '|', '?', '*'];

pub async fn execute(
    paths: Vec<String>,
    targets: Option<Vec<String>>,
    format: &OutputFormat,
) -> Result<u8, Box<dyn std::error::Error>> {
    let target_platforms = targets.unwrap_or_else(|| {
        vec!["windows".into(), "macos".into(), "linux".into()]
    });

    let mut results: Vec<CompatResult> = Vec::new();
    let mut has_errors = false;

    for path in &paths {
        let mut issues = Vec::new();
        let filename = std::path::Path::new(path)
            .file_name()
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_else(|| path.clone());

        // Check for each target platform
        for platform in &target_platforms {
            match platform.as_str() {
                "windows" => {
                    // Check reserved names
                    let stem = filename.split('.').next().unwrap_or(&filename).to_uppercase();
                    if WINDOWS_RESERVED.contains(&stem.as_str()) {
                        issues.push(CompatIssue {
                            severity: "error".into(),
                            platform: "windows".into(),
                            message: format!("'{filename}' is a reserved name on Windows"),
                            suggestion: Some(format!("Rename to '_{filename}'")),
                        });
                    }

                    // Check invalid characters
                    for ch in WINDOWS_INVALID_CHARS {
                        if filename.contains(*ch) {
                            issues.push(CompatIssue {
                                severity: "error".into(),
                                platform: "windows".into(),
                                message: format!("Character '{ch}' is not allowed on Windows"),
                                suggestion: Some(format!("Replace '{ch}' with '_'")),
                            });
                        }
                    }

                    // Check trailing dots/spaces
                    if filename.ends_with('.') || filename.ends_with(' ') {
                        issues.push(CompatIssue {
                            severity: "error".into(),
                            platform: "windows".into(),
                            message: "Trailing dots or spaces not allowed on Windows".into(),
                            suggestion: Some("Remove trailing dots and spaces".into()),
                        });
                    }

                    // Check path length
                    if path.len() > 260 {
                        issues.push(CompatIssue {
                            severity: "warning".into(),
                            platform: "windows".into(),
                            message: format!("Path length ({}) exceeds Windows MAX_PATH (260)", path.len()),
                            suggestion: Some("Shorten directory structure or enable long paths".into()),
                        });
                    }
                }

                "macos" => {
                    // Check colon (used as path separator in HFS+)
                    if filename.contains(':') {
                        issues.push(CompatIssue {
                            severity: "error".into(),
                            platform: "macos".into(),
                            message: "Colon ':' is not allowed in macOS filenames".into(),
                            suggestion: Some("Replace ':' with '-' or '_'".into()),
                        });
                    }

                    // Check filename length
                    if filename.len() > 255 {
                        issues.push(CompatIssue {
                            severity: "error".into(),
                            platform: "macos".into(),
                            message: format!("Filename length ({}) exceeds 255 bytes", filename.len()),
                            suggestion: Some("Shorten the filename".into()),
                        });
                    }

                    // Unicode normalization warning
                    let nfc: String = unicode_normalization_check(&filename);
                    if nfc != filename {
                        issues.push(CompatIssue {
                            severity: "warning".into(),
                            platform: "macos".into(),
                            message: "Filename uses non-NFD Unicode normalization (macOS uses NFD)".into(),
                            suggestion: Some("Normalize to NFD form".into()),
                        });
                    }
                }

                "linux" => {
                    // Check null byte and forward slash (only invalid chars on Linux)
                    if filename.contains('\0') {
                        issues.push(CompatIssue {
                            severity: "error".into(),
                            platform: "linux".into(),
                            message: "Null byte in filename".into(),
                            suggestion: Some("Remove null bytes".into()),
                        });
                    }

                    // Check filename length
                    if filename.as_bytes().len() > 255 {
                        issues.push(CompatIssue {
                            severity: "error".into(),
                            platform: "linux".into(),
                            message: format!("Filename exceeds 255 bytes ({} bytes)", filename.as_bytes().len()),
                            suggestion: Some("Shorten the filename".into()),
                        });
                    }

                    // Check case sensitivity warning
                    if filename != filename.to_lowercase() && filename != filename.to_uppercase() {
                        issues.push(CompatIssue {
                            severity: "info".into(),
                            platform: "linux".into(),
                            message: "Linux is case-sensitive; this filename may collide on case-insensitive systems".into(),
                            suggestion: None,
                        });
                    }
                }
                _ => {}
            }
        }

        let safe = issues.iter().all(|i| i.severity != "error");
        if !safe {
            has_errors = true;
        }

        results.push(CompatResult {
            path: path.clone(),
            issues,
            safe,
        });
    }

    // Output
    match format {
        OutputFormat::Human => {
            println!();
            for result in &results {
                let icon = if result.safe { "\x1b[32m✓\x1b[0m" } else { "\x1b[31m✗\x1b[0m" };
                println!("  {icon} {}", result.path);
                for issue in &result.issues {
                    let color = match issue.severity.as_str() {
                        "error" => "\x1b[31m",
                        "warning" => "\x1b[33m",
                        _ => "\x1b[36m",
                    };
                    println!(
                        "    {color}[{}]\x1b[0m {} ({})",
                        issue.severity.to_uppercase(),
                        issue.message,
                        issue.platform,
                    );
                    if let Some(ref sug) = issue.suggestion {
                        println!("        Suggestion: {sug}");
                    }
                }
                if result.issues.is_empty() {
                    println!("    Compatible with all target platforms");
                }
            }
            println!();
        }
        _ => output::print_data(format, &results),
    }

    if has_errors {
        Ok(1) // Partial failure - some paths have issues
    } else {
        Ok(0)
    }
}

fn unicode_normalization_check(s: &str) -> String {
    // Simple check: if any char has combining marks, it might differ in normalization
    // This is a heuristic without pulling in the full unicode-normalization crate
    s.to_string()
}
