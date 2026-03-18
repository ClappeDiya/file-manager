//! Google Drive Connector (T-027).
//!
//! Features:
//! - OAuth 2.0 flow (opens browser for consent)
//! - My Drive, Shared Drives, Shared with Me browsable
//! - Google Docs/Sheets export options (PDF, DOCX)
//! - File versioning: view history, restore
//! - Auto token refresh

use crate::connectors::oauth::{OAuthClient, OAuthConfig, OAuthToken};
use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const DRIVE_API_BASE: &str = "https://www.googleapis.com/drive/v3";
const DRIVE_UPLOAD_BASE: &str = "https://www.googleapis.com/upload/drive/v3";
const DEFAULT_CALLBACK_PORT: u16 = 9876;

/// Google Drive file MIME types for export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoogleDocExportFormat {
    Pdf,
    Docx,
    Xlsx,
    Pptx,
    Csv,
    Txt,
    Html,
    Epub,
}

impl GoogleDocExportFormat {
    pub fn mime_type(&self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::Docx => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Pptx => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
            Self::Csv => "text/csv",
            Self::Txt => "text/plain",
            Self::Html => "text/html",
            Self::Epub => "application/epub+zip",
        }
    }

    pub fn extension(&self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::Docx => "docx",
            Self::Xlsx => "xlsx",
            Self::Pptx => "pptx",
            Self::Csv => "csv",
            Self::Txt => "txt",
            Self::Html => "html",
            Self::Epub => "epub",
        }
    }
}

/// Google Drive file metadata from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveFile {
    id: String,
    name: String,
    mime_type: String,
    #[serde(default)]
    size: Option<String>,
    modified_time: Option<String>,
    created_time: Option<String>,
    #[serde(default)]
    parents: Option<Vec<String>>,
    #[serde(default)]
    shared: Option<bool>,
    #[serde(default)]
    trashed: Option<bool>,
}

/// Response from files.list endpoint.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileListResponse {
    #[serde(default)]
    files: Vec<DriveFile>,
    next_page_token: Option<String>,
}

/// Response from drives.list endpoint (Shared Drives).
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DriveListResponse {
    #[serde(default)]
    drives: Vec<SharedDrive>,
    next_page_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharedDrive {
    id: String,
    name: String,
}

/// File revision from the revisions.list endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileRevision {
    pub id: String,
    pub modified_time: Option<String>,
    #[serde(default)]
    pub original_filename: Option<String>,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub keep_forever: Option<bool>,
}

#[derive(Debug, Deserialize)]
struct RevisionListResponse {
    #[serde(default)]
    revisions: Vec<FileRevision>,
}

/// Drive browse mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriveScope {
    MyDrive,
    SharedDrives,
    SharedWithMe,
}

/// Google Drive connector.
pub struct GoogleDriveConnector {
    oauth: Arc<RwLock<Option<OAuthClient>>>,
    http: reqwest::Client,
}

impl GoogleDriveConnector {
    pub fn new() -> Self {
        Self {
            oauth: Arc::new(RwLock::new(None)),
            http: reqwest::Client::new(),
        }
    }

    /// Initialize the OAuth client with the given credentials.
    pub async fn init_oauth(
        &self,
        client_id: String,
        client_secret: Option<String>,
        callback_port: Option<u16>,
    ) {
        let port = callback_port.unwrap_or(DEFAULT_CALLBACK_PORT);
        let config = OAuthConfig {
            client_id,
            client_secret,
            auth_url: GOOGLE_AUTH_URL.to_string(),
            token_url: GOOGLE_TOKEN_URL.to_string(),
            redirect_uri: format!("http://localhost:{port}/callback"),
            scopes: vec![
                "https://www.googleapis.com/auth/drive".to_string(),
                "https://www.googleapis.com/auth/drive.file".to_string(),
            ],
            extra_params: vec![
                ("access_type".to_string(), "offline".to_string()),
                ("prompt".to_string(), "consent".to_string()),
            ],
        };

        let client = OAuthClient::new(config);
        let mut oauth = self.oauth.write().await;
        *oauth = Some(client);
    }

    /// Get the authorization URL (user opens this in browser).
    pub async fn get_auth_url(&self) -> Result<(String, String), AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::Connection {
            message: "OAuth not initialized".to_string(),
            advice: "Call init_oauth first with your client ID.".to_string(),
        })?;
        Ok(client.authorization_url().await)
    }

    /// Complete the OAuth flow with the authorization code.
    pub async fn complete_auth(&self, code: &str) -> Result<OAuthToken, AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::Connection {
            message: "OAuth not initialized".to_string(),
            advice: "Call init_oauth first.".to_string(),
        })?;
        client.exchange_code(code).await
    }

    /// Restore a previously saved token (e.g., from credential store).
    pub async fn restore_token(&self, token_json: &str) -> Result<(), AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::Connection {
            message: "OAuth not initialized".to_string(),
            advice: "Call init_oauth first.".to_string(),
        })?;
        client.restore_token(token_json).await
    }

    /// Get a valid access token (auto-refreshes if expired).
    pub async fn get_token(&self) -> Result<String, AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to Google Drive".to_string(),
            advice: "Connect first.".to_string(),
        })?;
        client.get_valid_token().await
    }

    /// List files in a specific folder.
    pub async fn list_files(
        &self,
        parent_id: Option<&str>,
        scope: DriveScope,
        page_token: Option<&str>,
    ) -> Result<(Vec<FileEntry>, Option<String>), AppError> {
        let token = self.get_token().await?;

        let query = match scope {
            DriveScope::MyDrive => {
                if let Some(pid) = parent_id {
                    format!("'{}' in parents and trashed = false", pid)
                } else {
                    "'root' in parents and trashed = false".to_string()
                }
            }
            DriveScope::SharedWithMe => {
                "sharedWithMe = true and trashed = false".to_string()
            }
            DriveScope::SharedDrives => {
                // This lists drives themselves, not files
                return self.list_shared_drives(page_token).await;
            }
        };

        let fields = "files(id,name,mimeType,size,modifiedTime,createdTime,parents,shared,trashed),nextPageToken";

        let mut url = url::Url::parse(&format!("{DRIVE_API_BASE}/files")).unwrap();
        {
            let mut params = url.query_pairs_mut();
            params.append_pair("q", &query);
            params.append_pair("fields", fields);
            params.append_pair("pageSize", "100");
            params.append_pair("orderBy", "folder,name");
            if let Some(pt) = page_token {
                params.append_pair("pageToken", pt);
            }
            // Support Shared Drives in results
            params.append_pair("supportsAllDrives", "true");
            params.append_pair("includeItemsFromAllDrives", "true");
        }

        let resp = self.http
            .get(url.as_str())
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("Google Drive API request failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("Google Drive API error (HTTP {status}): {body}"),
                advice: "Check your permissions or re-authorize.".to_string(),
            });
        }

        let list: FileListResponse = resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse Drive response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        let entries: Vec<FileEntry> = list
            .files
            .into_iter()
            .map(|f| drive_file_to_entry(f, parent_id.unwrap_or("root")))
            .collect();

        Ok((entries, list.next_page_token))
    }

    /// List Shared Drives.
    async fn list_shared_drives(
        &self,
        page_token: Option<&str>,
    ) -> Result<(Vec<FileEntry>, Option<String>), AppError> {
        let token = self.get_token().await?;

        let mut url = url::Url::parse(&format!("{DRIVE_API_BASE}/drives")).unwrap();
        {
            let mut params = url.query_pairs_mut();
            params.append_pair("pageSize", "100");
            if let Some(pt) = page_token {
                params.append_pair("pageToken", pt);
            }
        }

        let resp = self.http
            .get(url.as_str())
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("Drive API request failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("Drive API error (HTTP {status}): {body}"),
                advice: "You may not have Shared Drives access.".to_string(),
            });
        }

        let list: DriveListResponse = resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse Drive response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        let entries: Vec<FileEntry> = list
            .drives
            .into_iter()
            .map(|d| FileEntry {
                name: d.name,
                path: format!("/shared-drives/{}", d.id),
                is_dir: true,
                is_symlink: false,
                size: 0,
                modified: None,
                created: None,
                is_hidden: false,
                extension: None,
                permissions: None,
            })
            .collect();

        Ok((entries, list.next_page_token))
    }

    /// Upload a file to Google Drive.
    pub async fn upload_file(
        &self,
        name: &str,
        parent_id: Option<&str>,
        data: Vec<u8>,
        mime_type: Option<&str>,
    ) -> Result<String, AppError> {
        let token = self.get_token().await?;

        let file_size = data.len();
        let content_type = mime_type.unwrap_or("application/octet-stream");

        // Use simple upload for small files, resumable for large
        if file_size < 5 * 1024 * 1024 {
            self.simple_upload(&token, name, parent_id, &data, content_type).await
        } else {
            self.resumable_upload(&token, name, parent_id, &data, content_type).await
        }
    }

    /// Simple upload for small files (< 5MB).
    async fn simple_upload(
        &self,
        token: &str,
        name: &str,
        parent_id: Option<&str>,
        data: &[u8],
        content_type: &str,
    ) -> Result<String, AppError> {
        let mut metadata = serde_json::json!({ "name": name });
        if let Some(pid) = parent_id {
            metadata["parents"] = serde_json::json!([pid]);
        }

        let boundary = "===ufop_boundary===";
        let mut body = Vec::new();

        // Metadata part
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
        body.extend_from_slice(metadata.to_string().as_bytes());
        body.extend_from_slice(b"\r\n");

        // Content part
        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(format!("Content-Type: {content_type}\r\n\r\n").as_bytes());
        body.extend_from_slice(data);
        body.extend_from_slice(format!("\r\n--{boundary}--").as_bytes());

        let url = format!(
            "{DRIVE_UPLOAD_BASE}/files?uploadType=multipart&supportsAllDrives=true"
        );

        let resp = self.http
            .post(&url)
            .bearer_auth(token)
            .header(
                "Content-Type",
                format!("multipart/related; boundary={boundary}"),
            )
            .body(body)
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Google Drive upload failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Google Drive upload failed (HTTP {status}): {body}"),
                advice: "Check your storage quota and permissions.".to_string(),
            });
        }

        let file: DriveFile = resp.json().await.map_err(|e| AppError::Transfer {
            message: format!("Failed to parse upload response: {e}"),
            advice: "The file may have been uploaded. Check Drive.".to_string(),
        })?;

        tracing::info!("Uploaded to Google Drive: {} (ID: {})", name, file.id);
        Ok(file.id)
    }

    /// Resumable upload for large files (> 5MB).
    async fn resumable_upload(
        &self,
        token: &str,
        name: &str,
        parent_id: Option<&str>,
        data: &[u8],
        content_type: &str,
    ) -> Result<String, AppError> {
        let mut metadata = serde_json::json!({ "name": name });
        if let Some(pid) = parent_id {
            metadata["parents"] = serde_json::json!([pid]);
        }

        // Step 1: Initiate resumable upload
        let url = format!(
            "{DRIVE_UPLOAD_BASE}/files?uploadType=resumable&supportsAllDrives=true"
        );

        let resp = self.http
            .post(&url)
            .bearer_auth(token)
            .header("Content-Type", "application/json")
            .header("X-Upload-Content-Type", content_type)
            .header("X-Upload-Content-Length", data.len().to_string())
            .body(metadata.to_string())
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Failed to initiate resumable upload: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Resumable upload init failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        let upload_url = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| AppError::Transfer {
                message: "No upload URL in resumable upload response".to_string(),
                advice: "Try again.".to_string(),
            })?
            .to_string();

        // Step 2: Upload data in chunks (256KB aligned for Google)
        let chunk_size = 256 * 1024 * 4; // 1MB chunks
        let total = data.len();
        let mut offset = 0;

        while offset < total {
            let end = std::cmp::min(offset + chunk_size, total);
            let chunk = &data[offset..end];
            let is_last = end == total;

            let range = format!(
                "bytes {}-{}/{}",
                offset,
                end - 1,
                if is_last { total.to_string() } else { "*".to_string() }
            );

            let resp = self.http
                .put(&upload_url)
                .header("Content-Range", &range)
                .header("Content-Length", chunk.len().to_string())
                .body(chunk.to_vec())
                .send()
                .await
                .map_err(|e| AppError::Transfer {
                    message: format!("Chunk upload failed: {e}"),
                    advice: "Upload will be retried.".to_string(),
                })?;

            let status = resp.status().as_u16();
            if status == 200 || status == 201 {
                // Upload complete
                let file: DriveFile = resp.json().await.map_err(|e| AppError::Transfer {
                    message: format!("Failed to parse upload response: {e}"),
                    advice: "The file may have been uploaded.".to_string(),
                })?;
                tracing::info!("Uploaded to Google Drive: {} (ID: {})", name, file.id);
                return Ok(file.id);
            } else if status == 308 {
                // Resume incomplete - continue
                offset = end;
            } else {
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Transfer {
                    message: format!("Upload failed (HTTP {status}): {body}"),
                    advice: "Try again.".to_string(),
                });
            }
        }

        Err(AppError::Transfer {
            message: "Upload completed without a file ID response".to_string(),
            advice: "Check Google Drive for the uploaded file.".to_string(),
        })
    }

    /// Download a file from Google Drive.
    pub async fn download_file(&self, file_id: &str) -> Result<Vec<u8>, AppError> {
        let token = self.get_token().await?;

        let url = format!(
            "{DRIVE_API_BASE}/files/{file_id}?alt=media&supportsAllDrives=true"
        );

        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Google Drive download failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Download failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| AppError::Transfer {
            message: format!("Failed to read download data: {e}"),
            advice: "Try again.".to_string(),
        })
    }

    /// Export a Google Docs/Sheets/Slides file to a different format.
    pub async fn export_file(
        &self,
        file_id: &str,
        format: GoogleDocExportFormat,
    ) -> Result<Vec<u8>, AppError> {
        let token = self.get_token().await?;

        let url = format!(
            "{DRIVE_API_BASE}/files/{file_id}/export?mimeType={}",
            percent_encoding::percent_encode(
                format.mime_type().as_bytes(),
                percent_encoding::NON_ALPHANUMERIC,
            )
        );

        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Google Drive export failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Export failed (HTTP {status}): {body}"),
                advice: "This file type may not support the requested export format.".to_string(),
            });
        }

        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| AppError::Transfer {
            message: format!("Failed to read export data: {e}"),
            advice: "Try again.".to_string(),
        })
    }

    /// List file version history.
    pub async fn list_revisions(&self, file_id: &str) -> Result<Vec<FileRevision>, AppError> {
        let token = self.get_token().await?;

        let url = format!(
            "{DRIVE_API_BASE}/files/{file_id}/revisions?fields=revisions(id,modifiedTime,originalFilename,size,keepForever)"
        );

        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("Failed to list revisions: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("List revisions failed (HTTP {status}): {body}"),
                advice: "This file may not support versioning.".to_string(),
            });
        }

        let list: RevisionListResponse = resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse revisions response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(list.revisions)
    }

    /// Download a specific revision of a file.
    pub async fn download_revision(
        &self,
        file_id: &str,
        revision_id: &str,
    ) -> Result<Vec<u8>, AppError> {
        let token = self.get_token().await?;

        let url = format!(
            "{DRIVE_API_BASE}/files/{file_id}/revisions/{revision_id}?alt=media"
        );

        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Revision download failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Revision download failed (HTTP {status}): {body}"),
                advice: "This revision may no longer be available.".to_string(),
            });
        }

        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| AppError::Transfer {
            message: format!("Failed to read revision data: {e}"),
            advice: "Try again.".to_string(),
        })
    }

    /// Delete a file or move it to trash.
    pub async fn delete_file(&self, file_id: &str, permanent: bool) -> Result<(), AppError> {
        let token = self.get_token().await?;

        if permanent {
            let url = format!(
                "{DRIVE_API_BASE}/files/{file_id}?supportsAllDrives=true"
            );
            let resp = self.http
                .delete(&url)
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| AppError::Transfer {
                    message: format!("Delete failed: {e}"),
                    advice: "Check your network connection.".to_string(),
                })?;

            if !resp.status().is_success() && resp.status().as_u16() != 204 {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Transfer {
                    message: format!("Delete failed (HTTP {status}): {body}"),
                    advice: "Check your permissions.".to_string(),
                });
            }
        } else {
            // Move to trash
            let url = format!(
                "{DRIVE_API_BASE}/files/{file_id}?supportsAllDrives=true"
            );
            let resp = self.http
                .patch(&url)
                .bearer_auth(&token)
                .json(&serde_json::json!({ "trashed": true }))
                .send()
                .await
                .map_err(|e| AppError::Transfer {
                    message: format!("Trash failed: {e}"),
                    advice: "Check your network connection.".to_string(),
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Transfer {
                    message: format!("Trash failed (HTTP {status}): {body}"),
                    advice: "Check your permissions.".to_string(),
                });
            }
        }

        Ok(())
    }

    /// Create a folder.
    pub async fn create_folder(
        &self,
        name: &str,
        parent_id: Option<&str>,
    ) -> Result<String, AppError> {
        let token = self.get_token().await?;

        let mut metadata = serde_json::json!({
            "name": name,
            "mimeType": "application/vnd.google-apps.folder",
        });
        if let Some(pid) = parent_id {
            metadata["parents"] = serde_json::json!([pid]);
        }

        let url = format!("{DRIVE_API_BASE}/files?supportsAllDrives=true");

        let resp = self.http
            .post(&url)
            .bearer_auth(&token)
            .json(&metadata)
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Create folder failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Create folder failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        let file: DriveFile = resp.json().await.map_err(|e| AppError::Transfer {
            message: format!("Failed to parse response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(file.id)
    }

    /// Rename a file or folder.
    pub async fn rename_file(&self, file_id: &str, new_name: &str) -> Result<(), AppError> {
        let token = self.get_token().await?;

        let url = format!(
            "{DRIVE_API_BASE}/files/{file_id}?supportsAllDrives=true"
        );

        let resp = self.http
            .patch(&url)
            .bearer_auth(&token)
            .json(&serde_json::json!({ "name": new_name }))
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Rename failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Rename failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        Ok(())
    }

    /// Move a file to a different folder.
    pub async fn move_file(
        &self,
        file_id: &str,
        new_parent_id: &str,
        old_parent_id: Option<&str>,
    ) -> Result<(), AppError> {
        let token = self.get_token().await?;

        let mut url = format!(
            "{DRIVE_API_BASE}/files/{file_id}?addParents={new_parent_id}&supportsAllDrives=true"
        );
        if let Some(old) = old_parent_id {
            url.push_str(&format!("&removeParents={old}"));
        }

        let resp = self.http
            .patch(&url)
            .bearer_auth(&token)
            .json(&serde_json::json!({}))
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Move failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Move failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        Ok(())
    }
}

impl Default for GoogleDriveConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl Connector for GoogleDriveConnector {
    fn connect(
        &self,
        _profile: &ConnectionProfile,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            // For Google Drive, connection is established via OAuth flow
            // The profile should have an OAuth token stored in credential_ref
            let oauth = self.oauth.read().await;
            if oauth.is_none() {
                return Err(AppError::Connection {
                    message: "Google Drive OAuth not initialized".to_string(),
                    advice: "Initialize OAuth with your client ID before connecting.".to_string(),
                });
            }
            Ok(())
        })
    }

    fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            let oauth = self.oauth.read().await;
            if let Some(client) = oauth.as_ref() {
                client.clear_token().await;
            }
            Ok(())
        })
    }

    fn list_remote(
        &self,
        path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<FileEntry>, AppError>> + Send + '_>> {
        let path = path.to_string();
        Box::pin(async move {
            let (scope, parent_id) = parse_drive_path(&path);
            let (entries, _) = self.list_files(parent_id.as_deref(), scope, None).await?;
            Ok(entries)
        })
    }

    fn is_connected(&self) -> bool {
        false // Conservative; actual check needs async
    }
}

/// Convert a Drive API file object to our FileEntry.
fn drive_file_to_entry(file: DriveFile, parent_path: &str) -> FileEntry {
    let is_folder = file.mime_type == "application/vnd.google-apps.folder";
    let is_google_doc = file.mime_type.starts_with("application/vnd.google-apps.");

    let size = file
        .size
        .as_deref()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);

    let modified = file
        .modified_time
        .as_deref()
        .and_then(|d| d.parse::<DateTime<Utc>>().ok());

    let created = file
        .created_time
        .as_deref()
        .and_then(|d| d.parse::<DateTime<Utc>>().ok());

    let extension = if is_google_doc {
        // Google Docs show their native type
        match file.mime_type.as_str() {
            "application/vnd.google-apps.document" => Some("gdoc".to_string()),
            "application/vnd.google-apps.spreadsheet" => Some("gsheet".to_string()),
            "application/vnd.google-apps.presentation" => Some("gslides".to_string()),
            _ => None,
        }
    } else {
        file.name
            .rsplit('.')
            .next()
            .filter(|ext| ext.len() < 10)
            .map(|s| s.to_string())
    };

    FileEntry {
        name: file.name,
        path: format!("/{}/{}", parent_path, file.id),
        is_dir: is_folder,
        is_symlink: false,
        size,
        modified,
        created,
        is_hidden: false,
        extension,
        permissions: if file.shared.unwrap_or(false) {
            Some("shared".to_string())
        } else {
            None
        },
    }
}

/// Parse a virtual path into drive scope and parent folder ID.
fn parse_drive_path(path: &str) -> (DriveScope, Option<String>) {
    let path = path.trim_start_matches('/');

    if path.is_empty() || path == "my-drive" {
        return (DriveScope::MyDrive, None);
    }

    if path == "shared-with-me" {
        return (DriveScope::SharedWithMe, None);
    }

    if path == "shared-drives" {
        return (DriveScope::SharedDrives, None);
    }

    if let Some(rest) = path.strip_prefix("shared-drives/") {
        if rest.contains('/') {
            let parts: Vec<&str> = rest.splitn(2, '/').collect();
            return (DriveScope::MyDrive, Some(parts.last().unwrap_or(&"").to_string()));
        }
        return (DriveScope::MyDrive, Some(rest.to_string()));
    }

    if let Some(rest) = path.strip_prefix("my-drive/") {
        return (DriveScope::MyDrive, Some(rest.to_string()));
    }

    // Default: treat as folder ID
    (DriveScope::MyDrive, Some(path.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connector_creation() {
        let connector = GoogleDriveConnector::new();
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_parse_drive_path() {
        let (scope, parent) = parse_drive_path("/");
        assert_eq!(scope, DriveScope::MyDrive);
        assert!(parent.is_none());

        let (scope, parent) = parse_drive_path("/my-drive");
        assert_eq!(scope, DriveScope::MyDrive);
        assert!(parent.is_none());

        let (scope, _) = parse_drive_path("/shared-with-me");
        assert_eq!(scope, DriveScope::SharedWithMe);

        let (scope, _) = parse_drive_path("/shared-drives");
        assert_eq!(scope, DriveScope::SharedDrives);

        let (scope, parent) = parse_drive_path("/my-drive/folder123");
        assert_eq!(scope, DriveScope::MyDrive);
        assert_eq!(parent.as_deref(), Some("folder123"));
    }

    #[test]
    fn test_drive_file_to_entry_folder() {
        let file = DriveFile {
            id: "folder1".to_string(),
            name: "My Folder".to_string(),
            mime_type: "application/vnd.google-apps.folder".to_string(),
            size: None,
            modified_time: Some("2024-01-15T10:00:00.000Z".to_string()),
            created_time: None,
            parents: Some(vec!["root".to_string()]),
            shared: Some(false),
            trashed: Some(false),
        };

        let entry = drive_file_to_entry(file, "root");
        assert_eq!(entry.name, "My Folder");
        assert!(entry.is_dir);
        assert_eq!(entry.size, 0);
    }

    #[test]
    fn test_drive_file_to_entry_google_doc() {
        let file = DriveFile {
            id: "doc1".to_string(),
            name: "My Document".to_string(),
            mime_type: "application/vnd.google-apps.document".to_string(),
            size: None,
            modified_time: None,
            created_time: None,
            parents: None,
            shared: Some(true),
            trashed: Some(false),
        };

        let entry = drive_file_to_entry(file, "root");
        assert_eq!(entry.extension.as_deref(), Some("gdoc"));
        assert_eq!(entry.permissions.as_deref(), Some("shared"));
    }

    #[test]
    fn test_drive_file_to_entry_regular_file() {
        let file = DriveFile {
            id: "file1".to_string(),
            name: "report.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            size: Some("1024".to_string()),
            modified_time: Some("2024-06-01T12:00:00.000Z".to_string()),
            created_time: Some("2024-05-01T12:00:00.000Z".to_string()),
            parents: Some(vec!["folder1".to_string()]),
            shared: Some(false),
            trashed: Some(false),
        };

        let entry = drive_file_to_entry(file, "folder1");
        assert_eq!(entry.name, "report.pdf");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 1024);
        assert_eq!(entry.extension.as_deref(), Some("pdf"));
        assert!(entry.modified.is_some());
        assert!(entry.created.is_some());
    }

    #[test]
    fn test_export_format_mime_types() {
        assert_eq!(GoogleDocExportFormat::Pdf.mime_type(), "application/pdf");
        assert_eq!(GoogleDocExportFormat::Pdf.extension(), "pdf");
        assert_eq!(GoogleDocExportFormat::Csv.mime_type(), "text/csv");
        assert!(GoogleDocExportFormat::Docx.mime_type().contains("wordprocessingml"));
    }

    #[tokio::test]
    async fn test_oauth_initialization() {
        let connector = GoogleDriveConnector::new();
        connector.init_oauth("test_client".to_string(), Some("test_secret".to_string()), None).await;

        let result = connector.get_auth_url().await;
        assert!(result.is_ok());
        let (url, state) = result.unwrap();
        assert!(url.contains("accounts.google.com"));
        assert!(url.contains("test_client"));
        assert!(!state.is_empty());
    }
}
