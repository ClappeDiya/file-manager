//! Output formatting for human, JSON, and YAML modes.

use crate::OutputFormat;
use serde::Serialize;

/// Print a success message in the appropriate format.
pub fn print_success(format: &OutputFormat, message: &str) {
    match format {
        OutputFormat::Human => {
            println!("\x1b[32m✓\x1b[0m {message}");
        }
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "status": "success",
                "message": message
            });
            println!("{}", serde_json::to_string_pretty(&obj).unwrap());
        }
        OutputFormat::Yaml => {
            let obj = serde_json::json!({
                "status": "success",
                "message": message
            });
            println!("{}", serde_yaml::to_string(&obj).unwrap());
        }
    }
}

/// Print an error message in the appropriate format.
pub fn print_error(format: &OutputFormat, message: &str) {
    match format {
        OutputFormat::Human => {
            eprintln!("\x1b[31m✗\x1b[0m {message}");
        }
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "status": "error",
                "message": message
            });
            eprintln!("{}", serde_json::to_string_pretty(&obj).unwrap());
        }
        OutputFormat::Yaml => {
            let obj = serde_json::json!({
                "status": "error",
                "message": message
            });
            eprintln!("{}", serde_yaml::to_string(&obj).unwrap());
        }
    }
}

/// Print a warning message.
pub fn print_warning(format: &OutputFormat, message: &str) {
    match format {
        OutputFormat::Human => {
            eprintln!("\x1b[33m!\x1b[0m {message}");
        }
        OutputFormat::Json => {
            let obj = serde_json::json!({
                "status": "warning",
                "message": message
            });
            eprintln!("{}", serde_json::to_string_pretty(&obj).unwrap());
        }
        OutputFormat::Yaml => {
            let obj = serde_json::json!({
                "status": "warning",
                "message": message
            });
            eprintln!("{}", serde_yaml::to_string(&obj).unwrap());
        }
    }
}

/// Print structured data in the appropriate format.
pub fn print_data<T: Serialize>(format: &OutputFormat, data: &T) {
    match format {
        OutputFormat::Human => {
            // For human mode, caller should handle formatting
            println!("{}", serde_json::to_string_pretty(data).unwrap());
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(data).unwrap());
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(data).unwrap());
        }
    }
}

/// Print a table in human mode, structured data otherwise.
pub fn print_table<T: Serialize>(format: &OutputFormat, headers: &[&str], rows: &[Vec<String>], data: &T) {
    match format {
        OutputFormat::Human => {
            if rows.is_empty() {
                println!("  (no results)");
                return;
            }
            // Calculate column widths
            let mut widths: Vec<usize> = headers.iter().map(|h| h.len()).collect();
            for row in rows {
                for (i, cell) in row.iter().enumerate() {
                    if i < widths.len() {
                        widths[i] = widths[i].max(cell.len());
                    }
                }
            }

            // Print header
            let header_line: Vec<String> = headers
                .iter()
                .zip(widths.iter())
                .map(|(h, w)| format!("{:<width$}", h, width = w))
                .collect();
            println!("  {}", header_line.join("  "));
            let sep: Vec<String> = widths.iter().map(|w| "-".repeat(*w)).collect();
            println!("  {}", sep.join("  "));

            // Print rows
            for row in rows {
                let cells: Vec<String> = row
                    .iter()
                    .zip(widths.iter())
                    .map(|(c, w)| format!("{:<width$}", c, width = w))
                    .collect();
                println!("  {}", cells.join("  "));
            }
        }
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(data).unwrap());
        }
        OutputFormat::Yaml => {
            println!("{}", serde_yaml::to_string(data).unwrap());
        }
    }
}

/// JSON event emitter for --json mode progress.
#[derive(Debug, Serialize)]
pub struct ProgressEvent {
    pub event: String,
    pub progress: f64,
    pub bytes_transferred: u64,
    pub bytes_total: u64,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
    pub current_file: Option<String>,
}

impl ProgressEvent {
    pub fn emit(&self) {
        println!("{}", serde_json::to_string(self).unwrap());
    }
}
