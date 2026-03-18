//! Sync reporting - per-run reports with health indicators.
//!
//! Generates detailed reports for each sync run, computes health
//! indicators, and supports CSV/JSON export.

use crate::core::error::AppError;
use crate::core::types::*;
use chrono::Utc;
// uuid used in tests
#[cfg(test)]
use uuid::Uuid;

/// Compute the health indicator for a sync pair based on recent reports.
pub fn compute_health(reports: &[SyncReport], pair_enabled: bool) -> SyncHealth {
    if !pair_enabled {
        return SyncHealth::Gray;
    }

    if reports.is_empty() {
        return SyncHealth::Gray;
    }

    // Look at the most recent report
    let latest = &reports[0];

    match latest.status {
        SyncRunStatus::Success => {
            // Check if stale (no sync in 24+ hours for watch-based pairs)
            let age = Utc::now() - latest.ended_at;
            if age.num_hours() > 48 {
                SyncHealth::Yellow
            } else {
                SyncHealth::Green
            }
        }
        SyncRunStatus::PartialSuccess => SyncHealth::Yellow,
        SyncRunStatus::Failed => SyncHealth::Red,
        SyncRunStatus::Running => SyncHealth::Green,
        SyncRunStatus::Cancelled => SyncHealth::Yellow,
    }
}

/// Export a sync report as CSV.
pub fn export_report_csv(report: &SyncReport) -> Result<String, AppError> {
    let mut wtr = csv::Writer::from_writer(Vec::new());

    wtr.write_record([
        "Field", "Value",
    ])
    .map_err(csv_err)?;

    wtr.write_record(["Pair ID", &report.pair_id.to_string()]).map_err(csv_err)?;
    wtr.write_record(["Pair Name", &report.pair_name]).map_err(csv_err)?;
    wtr.write_record(["Started At", &report.started_at.to_rfc3339()]).map_err(csv_err)?;
    wtr.write_record(["Ended At", &report.ended_at.to_rfc3339()]).map_err(csv_err)?;
    wtr.write_record(["Duration (ms)", &report.duration_ms.to_string()]).map_err(csv_err)?;
    wtr.write_record(["Files Added", &report.files_added.to_string()]).map_err(csv_err)?;
    wtr.write_record(["Files Modified", &report.files_modified.to_string()]).map_err(csv_err)?;
    wtr.write_record(["Files Deleted", &report.files_deleted.to_string()]).map_err(csv_err)?;
    wtr.write_record(["Files Skipped", &report.files_skipped.to_string()]).map_err(csv_err)?;
    wtr.write_record(["Conflicts Resolved", &report.conflicts_resolved.to_string()]).map_err(csv_err)?;
    wtr.write_record(["Errors", &report.errors.to_string()]).map_err(csv_err)?;
    wtr.write_record(["Bytes Transferred", &report.bytes_transferred.to_string()]).map_err(csv_err)?;
    wtr.write_record(["Status", &format!("{:?}", report.status)]).map_err(csv_err)?;
    wtr.write_record(["Health", &format!("{:?}", report.health)]).map_err(csv_err)?;
    wtr.write_record(["Resumed", &report.resumed.to_string()]).map_err(csv_err)?;

    if !report.error_messages.is_empty() {
        wtr.write_record(["Errors Detail", &report.error_messages.join("; ")]).map_err(csv_err)?;
    }

    let data = wtr.into_inner().map_err(|e| AppError::Sync {
        message: format!("CSV export error: {}", e),
        advice: "Try again.".to_string(),
    })?;

    String::from_utf8(data).map_err(|e| AppError::Sync {
        message: format!("CSV encoding error: {}", e),
        advice: "Try again.".to_string(),
    })
}

/// Export a sync report as JSON.
pub fn export_report_json(report: &SyncReport) -> Result<String, AppError> {
    serde_json::to_string_pretty(report).map_err(|e| AppError::Sync {
        message: format!("JSON export error: {}", e),
        advice: "Try again.".to_string(),
    })
}

fn csv_err(e: csv::Error) -> AppError {
    AppError::Sync {
        message: format!("CSV write error: {}", e),
        advice: "Try again.".to_string(),
    }
}

/// Format bytes for human-readable display.
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format duration for human-readable display.
pub fn format_duration(ms: u64) -> String {
    if ms < 1000 {
        format!("{}ms", ms)
    } else if ms < 60_000 {
        format!("{:.1}s", ms as f64 / 1000.0)
    } else if ms < 3_600_000 {
        let mins = ms / 60_000;
        let secs = (ms % 60_000) / 1000;
        format!("{}m {}s", mins, secs)
    } else {
        let hours = ms / 3_600_000;
        let mins = (ms % 3_600_000) / 60_000;
        format!("{}h {}m", hours, mins)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_report(status: SyncRunStatus) -> SyncReport {
        SyncReport {
            id: Uuid::new_v4(),
            pair_id: Uuid::new_v4(),
            pair_name: "Test Pair".to_string(),
            started_at: Utc::now() - chrono::Duration::minutes(5),
            ended_at: Utc::now(),
            duration_ms: 5000,
            files_added: 10,
            files_modified: 5,
            files_deleted: 2,
            files_skipped: 3,
            conflicts_resolved: 1,
            errors: 0,
            bytes_transferred: 1024 * 1024 * 50,
            status,
            health: SyncHealth::Green,
            error_messages: vec![],
            resumed: false,
        }
    }

    #[test]
    fn test_compute_health_green() {
        let reports = vec![make_report(SyncRunStatus::Success)];
        assert_eq!(compute_health(&reports, true), SyncHealth::Green);
    }

    #[test]
    fn test_compute_health_yellow_partial() {
        let reports = vec![make_report(SyncRunStatus::PartialSuccess)];
        assert_eq!(compute_health(&reports, true), SyncHealth::Yellow);
    }

    #[test]
    fn test_compute_health_red_failed() {
        let reports = vec![make_report(SyncRunStatus::Failed)];
        assert_eq!(compute_health(&reports, true), SyncHealth::Red);
    }

    #[test]
    fn test_compute_health_gray_disabled() {
        let reports = vec![make_report(SyncRunStatus::Success)];
        assert_eq!(compute_health(&reports, false), SyncHealth::Gray);
    }

    #[test]
    fn test_compute_health_gray_no_reports() {
        assert_eq!(compute_health(&[], true), SyncHealth::Gray);
    }

    #[test]
    fn test_export_report_csv() {
        let report = make_report(SyncRunStatus::Success);
        let csv = export_report_csv(&report).unwrap();
        assert!(csv.contains("Test Pair"));
        assert!(csv.contains("Files Added"));
        assert!(csv.contains("10"));
    }

    #[test]
    fn test_export_report_json() {
        let report = make_report(SyncRunStatus::Success);
        let json = export_report_json(&report).unwrap();
        assert!(json.contains("Test Pair"));
        assert!(json.contains("files_added"));
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(500), "500ms");
        assert_eq!(format_duration(5000), "5.0s");
        assert_eq!(format_duration(90_000), "1m 30s");
        assert_eq!(format_duration(3_700_000), "1h 1m");
    }

    #[test]
    fn test_export_report_with_errors() {
        let mut report = make_report(SyncRunStatus::PartialSuccess);
        report.errors = 2;
        report.error_messages = vec![
            "file1.txt: permission denied".to_string(),
            "file2.txt: not found".to_string(),
        ];

        let csv = export_report_csv(&report).unwrap();
        assert!(csv.contains("permission denied"));

        let json = export_report_json(&report).unwrap();
        assert!(json.contains("not found"));
    }
}
