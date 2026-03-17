//! Tauri IPC commands for file version browsing and restoration across cloud providers.
//!
//! Provides frontend-accessible commands for:
//! - Listing file versions (Google Drive revisions, S3 object versions, OneDrive versions)
//! - Restoring a file to a specific version
//!
//! Google Drive uses `GoogleDriveConnector::list_revisions` / `download_revision`.
//! S3 uses `S3Connector::list_object_versions` / `restore_object_version`.
//! OneDrive uses the Microsoft Graph API versions endpoint.

use crate::connectors::google_drive::GoogleDriveConnector;
use crate::connectors::onedrive::OneDriveConnector;
use crate::connectors::s3::S3Connector;
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

/// A provider-agnostic file version entry returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileVersion {
    pub version_id: String,
    pub provider: String,
    pub modified_time: Option<String>,
    pub size: Option<u64>,
    pub filename: Option<String>,
    pub is_latest: bool,
    pub keep_forever: Option<bool>,
}

// ──────────────────────────────────────────────
// IPC Command: list_file_versions
// ──────────────────────────────────────────────

/// List all available versions of a file across cloud providers.
///
/// # Parameters
/// - `protocol`: One of `"google_drive"`, `"s3"`, `"onedrive"`.
/// - `file_id`: Provider-specific file identifier (Drive file ID, S3 `bucket/key`, OneDrive item ID).
/// - `connection_id`: Identifies which saved connection/account to use.
#[tauri::command]
pub async fn list_file_versions(
    protocol: String,
    file_id: String,
    connection_id: String,
    gdrive: State<'_, Arc<GoogleDriveConnector>>,
    s3: State<'_, Arc<S3Connector>>,
    onedrive: State<'_, Arc<OneDriveConnector>>,
) -> Result<Vec<FileVersion>, AppError> {
    match protocol.as_str() {
        "google_drive" => list_versions_google_drive(&file_id, &gdrive).await,
        "s3" => list_versions_s3(&file_id, &connection_id, &s3).await,
        "onedrive" => list_versions_onedrive(&file_id, &onedrive).await,
        other => Err(AppError::Connection {
            message: format!("Unsupported protocol for versioning: {other}"),
            advice: "Use one of: google_drive, s3, onedrive.".to_string(),
        }),
    }
}

// ──────────────────────────────────────────────
// IPC Command: restore_file_version
// ──────────────────────────────────────────────

/// Restore a file to a specific version.
#[tauri::command]
pub async fn restore_file_version(
    protocol: String,
    file_id: String,
    version_id: String,
    connection_id: String,
    gdrive: State<'_, Arc<GoogleDriveConnector>>,
    s3: State<'_, Arc<S3Connector>>,
    onedrive: State<'_, Arc<OneDriveConnector>>,
) -> Result<(), AppError> {
    match protocol.as_str() {
        "google_drive" => restore_version_google_drive(&file_id, &version_id, &gdrive).await,
        "s3" => restore_version_s3(&file_id, &version_id, &connection_id, &s3).await,
        "onedrive" => restore_version_onedrive(&file_id, &version_id, &onedrive).await,
        other => Err(AppError::Connection {
            message: format!("Unsupported protocol for version restore: {other}"),
            advice: "Use one of: google_drive, s3, onedrive.".to_string(),
        }),
    }
}

// ──────────────────────────────────────────────
// Google Drive implementation
// ──────────────────────────────────────────────

async fn list_versions_google_drive(
    file_id: &str,
    gdrive: &Arc<GoogleDriveConnector>,
) -> Result<Vec<FileVersion>, AppError> {
    let revisions = gdrive.list_revisions(file_id).await?;
    let total = revisions.len();
    let versions: Vec<FileVersion> = revisions
        .into_iter()
        .enumerate()
        .map(|(idx, rev)| {
            let size = rev.size.as_deref().and_then(|s| s.parse::<u64>().ok());
            FileVersion {
                version_id: rev.id,
                provider: "google_drive".to_string(),
                modified_time: rev.modified_time,
                size,
                filename: rev.original_filename,
                is_latest: idx == total - 1,
                keep_forever: rev.keep_forever,
            }
        })
        .collect();
    Ok(versions)
}

async fn restore_version_google_drive(
    file_id: &str,
    revision_id: &str,
    gdrive: &Arc<GoogleDriveConnector>,
) -> Result<(), AppError> {
    let revision_data = gdrive.download_revision(file_id, revision_id).await?;
    let token = gdrive.get_token().await?;
    let url = format!(
        "https://www.googleapis.com/upload/drive/v3/files/{file_id}?uploadType=media&supportsAllDrives=true"
    );

    let resp = reqwest::Client::new()
        .patch(&url)
        .bearer_auth(&token)
        .header("Content-Type", "application/octet-stream")
        .body(revision_data)
        .send()
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("Failed to re-upload revision content: {e}"),
            advice: "Check your network connection and try again.".to_string(),
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Transfer {
            message: format!("Failed to restore Google Drive revision (HTTP {status}): {body}"),
            advice: "Check file permissions or try again.".to_string(),
        });
    }

    tracing::info!("Restored Google Drive file {file_id} to revision {revision_id}");
    Ok(())
}

// ──────────────────────────────────────────────
// S3 implementation
// ──────────────────────────────────────────────

async fn list_versions_s3(
    file_id: &str,
    _connection_id: &str,
    s3: &Arc<S3Connector>,
) -> Result<Vec<FileVersion>, AppError> {
    let (bucket, key) = file_id.split_once('/').ok_or_else(|| AppError::Connection {
        message: format!("Invalid S3 file_id format: {file_id}"),
        advice: r#"Use the format "bucket/key" (e.g. "my-bucket/path/to/file.txt")."#.to_string(),
    })?;

    let s3_versions = s3.list_object_versions(bucket, key).await?;

    let versions: Vec<FileVersion> = s3_versions
        .into_iter()
        .filter(|v| !v.is_delete_marker)
        .map(|v| FileVersion {
            version_id: v.version_id,
            provider: "s3".to_string(),
            modified_time: v.last_modified,
            size: v.size,
            filename: Some(v.key),
            is_latest: v.is_latest,
            keep_forever: None,
        })
        .collect();

    Ok(versions)
}

async fn restore_version_s3(
    file_id: &str,
    version_id: &str,
    _connection_id: &str,
    s3: &Arc<S3Connector>,
) -> Result<(), AppError> {
    let (bucket, key) = file_id.split_once('/').ok_or_else(|| AppError::Connection {
        message: format!("Invalid S3 file_id format: {file_id}"),
        advice: r#"Use the format "bucket/key"."#.to_string(),
    })?;

    s3.restore_object_version(bucket, key, version_id).await
}

// ──────────────────────────────────────────────
// OneDrive implementation
// ──────────────────────────────────────────────

/// OneDrive version from Graph API.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphVersionEntry {
    id: String,
    #[serde(default)]
    last_modified_date_time: Option<String>,
    #[serde(default)]
    size: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct GraphVersionList {
    value: Vec<GraphVersionEntry>,
}

async fn list_versions_onedrive(
    file_id: &str,
    onedrive: &Arc<OneDriveConnector>,
) -> Result<Vec<FileVersion>, AppError> {
    let token = onedrive.get_token().await?;

    let url = format!(
        "https://graph.microsoft.com/v1.0/me/drive/items/{file_id}/versions"
    );

    let resp = reqwest::Client::new()
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| AppError::Connection {
            message: format!("OneDrive version listing failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Connection {
            message: format!("OneDrive version listing failed (HTTP {status}): {body}"),
            advice: "Check your permissions.".to_string(),
        });
    }

    let list: GraphVersionList = resp.json().await.map_err(|e| AppError::Connection {
        message: format!("Failed to parse OneDrive versions response: {e}"),
        advice: "Try again.".to_string(),
    })?;

    let versions: Vec<FileVersion> = list
        .value
        .into_iter()
        .enumerate()
        .map(|(idx, v)| FileVersion {
            version_id: v.id,
            provider: "onedrive".to_string(),
            modified_time: v.last_modified_date_time,
            size: v.size,
            filename: None,
            is_latest: idx == 0, // Graph API returns latest first
            keep_forever: None,
        })
        .collect();

    Ok(versions)
}

async fn restore_version_onedrive(
    file_id: &str,
    version_id: &str,
    onedrive: &Arc<OneDriveConnector>,
) -> Result<(), AppError> {
    let token = onedrive.get_token().await?;

    let url = format!(
        "https://graph.microsoft.com/v1.0/me/drive/items/{file_id}/versions/{version_id}/restoreVersion"
    );

    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|e| AppError::Transfer {
            message: format!("OneDrive version restore failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

    if !resp.status().is_success() && resp.status().as_u16() != 204 {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(AppError::Transfer {
            message: format!("OneDrive version restore failed (HTTP {status}): {body}"),
            advice: "Check your permissions.".to_string(),
        });
    }

    tracing::info!("Restored OneDrive file {file_id} to version {version_id}");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_file_version_serialization() {
        let version = FileVersion {
            version_id: "rev-123".to_string(),
            provider: "google_drive".to_string(),
            modified_time: Some("2025-06-15T10:30:00Z".to_string()),
            size: Some(1024),
            filename: Some("report.pdf".to_string()),
            is_latest: true,
            keep_forever: Some(false),
        };

        let json = serde_json::to_string(&version).unwrap();
        assert!(json.contains("rev-123"));

        let deserialized: FileVersion = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.version_id, "rev-123");
        assert!(deserialized.is_latest);
    }

    #[test]
    fn test_s3_file_id_parsing() {
        let file_id = "my-bucket/path/to/file.txt";
        let (bucket, key) = file_id.split_once('/').unwrap();
        assert_eq!(bucket, "my-bucket");
        assert_eq!(key, "path/to/file.txt");

        let invalid = "just-a-bucket";
        assert!(invalid.split_once('/').is_none());
    }
}
