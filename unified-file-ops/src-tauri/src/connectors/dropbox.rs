//! Dropbox Connector (T-028).
//!
//! Features:
//! - OAuth 2.0 flow
//! - Root, team folders, shared folders browsable
//! - File operations: upload, download, rename, move, delete, mkdir
//! - Selective sync support
//! - Drag-and-drop between Dropbox and local panes

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

const DROPBOX_AUTH_URL: &str = "https://www.dropbox.com/oauth2/authorize";
const DROPBOX_TOKEN_URL: &str = "https://api.dropboxapi.com/oauth2/token";
const DROPBOX_API_BASE: &str = "https://api.dropboxapi.com/2";
const DROPBOX_CONTENT_BASE: &str = "https://content.dropboxapi.com/2";
const DEFAULT_CALLBACK_PORT: u16 = 9877;

// Upload limit for simple upload (150MB for Dropbox)
const SIMPLE_UPLOAD_LIMIT: u64 = 150 * 1024 * 1024;
// Session upload chunk size: 8MB
const SESSION_CHUNK_SIZE: usize = 8 * 1024 * 1024;

/// Dropbox file metadata from the API.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct DropboxEntry {
    #[serde(rename = ".tag")]
    tag: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    path_display: Option<String>,
    #[serde(default)]
    path_lower: Option<String>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    size: Option<u64>,
    #[serde(default)]
    server_modified: Option<String>,
    #[serde(default)]
    client_modified: Option<String>,
    #[serde(default)]
    content_hash: Option<String>,
    #[serde(default)]
    is_downloadable: Option<bool>,
    #[serde(default)]
    sharing_info: Option<serde_json::Value>,
}

/// Response from files/list_folder endpoint.
#[derive(Debug, Deserialize)]
struct ListFolderResponse {
    entries: Vec<DropboxEntry>,
    cursor: String,
    has_more: bool,
}

/// Response from files/upload_session/start.
#[derive(Debug, Deserialize)]
struct UploadSessionStart {
    session_id: String,
}

/// Shared folder metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharedFolderInfo {
    pub shared_folder_id: String,
    pub name: String,
    pub path_lower: Option<String>,
    pub is_team_folder: bool,
}

/// Response from sharing/list_folders.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct ListSharedFoldersResponse {
    entries: Vec<SharedFolderEntry>,
    cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SharedFolderEntry {
    #[serde(default)]
    shared_folder_id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    path_lower: Option<String>,
    #[serde(default)]
    is_team_folder: Option<bool>,
}

/// Dropbox connector.
pub struct DropboxConnector {
    oauth: Arc<RwLock<Option<OAuthClient>>>,
    http: reqwest::Client,
}

impl DropboxConnector {
    pub fn new() -> Self {
        Self {
            oauth: Arc::new(RwLock::new(None)),
            http: reqwest::Client::new(),
        }
    }

    /// Initialize the OAuth client.
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
            auth_url: DROPBOX_AUTH_URL.to_string(),
            token_url: DROPBOX_TOKEN_URL.to_string(),
            redirect_uri: format!("http://localhost:{port}/callback"),
            scopes: vec![], // Dropbox doesn't use scopes in the same way
            extra_params: vec![
                ("token_access_type".to_string(), "offline".to_string()),
            ],
        };

        let client = OAuthClient::new(config);
        let mut oauth = self.oauth.write().await;
        *oauth = Some(client);
    }

    /// Get the authorization URL.
    pub async fn get_auth_url(&self) -> Result<(String, String), AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::Connection {
            message: "OAuth not initialized".to_string(),
            advice: "Call init_oauth first.".to_string(),
        })?;
        Ok(client.authorization_url().await)
    }

    /// Complete OAuth flow with authorization code.
    pub async fn complete_auth(&self, code: &str) -> Result<OAuthToken, AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::Connection {
            message: "OAuth not initialized".to_string(),
            advice: "Call init_oauth first.".to_string(),
        })?;
        client.exchange_code(code).await
    }

    /// Restore a saved token.
    pub async fn restore_token(&self, token_json: &str) -> Result<(), AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::Connection {
            message: "OAuth not initialized".to_string(),
            advice: "Call init_oauth first.".to_string(),
        })?;
        client.restore_token(token_json).await
    }

    /// Get a valid access token (auto-refreshes if expired).
    async fn get_token(&self) -> Result<String, AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to Dropbox".to_string(),
            advice: "Connect first.".to_string(),
        })?;
        client.get_valid_token().await
    }

    /// List files and folders in a Dropbox path.
    pub async fn list_folder(
        &self,
        path: &str,
    ) -> Result<Vec<FileEntry>, AppError> {
        let token = self.get_token().await?;

        // Dropbox uses empty string for root
        let dbx_path = if path.is_empty() || path == "/" {
            "".to_string()
        } else {
            path.to_string()
        };

        let mut all_entries = Vec::new();
        let mut has_more = true;
        let mut cursor: Option<String> = None;

        while has_more {
            let resp = if let Some(ref cur) = cursor {
                // Continue listing
                self.http
                    .post(format!("{DROPBOX_API_BASE}/files/list_folder/continue"))
                    .bearer_auth(&token)
                    .json(&serde_json::json!({ "cursor": cur }))
                    .send()
                    .await
            } else {
                // Initial listing
                self.http
                    .post(format!("{DROPBOX_API_BASE}/files/list_folder"))
                    .bearer_auth(&token)
                    .json(&serde_json::json!({
                        "path": dbx_path,
                        "recursive": false,
                        "include_media_info": false,
                        "include_deleted": false,
                        "include_mounted_folders": true,
                        "include_non_downloadable_files": true,
                        "limit": 2000,
                    }))
                    .send()
                    .await
            };

            let resp = resp.map_err(|e| AppError::Connection {
                message: format!("Dropbox API request failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Connection {
                    message: format!("Dropbox list folder failed (HTTP {status}): {body}"),
                    advice: "Check the path and your permissions.".to_string(),
                });
            }

            let list: ListFolderResponse = resp.json().await.map_err(|e| AppError::Connection {
                message: format!("Failed to parse Dropbox response: {e}"),
                advice: "Try again.".to_string(),
            })?;

            for entry in list.entries {
                all_entries.push(dropbox_entry_to_file_entry(entry));
            }

            has_more = list.has_more;
            cursor = Some(list.cursor);
        }

        Ok(all_entries)
    }

    /// List shared folders.
    pub async fn list_shared_folders(&self) -> Result<Vec<SharedFolderInfo>, AppError> {
        let token = self.get_token().await?;

        let resp = self.http
            .post(format!("{DROPBOX_API_BASE}/sharing/list_folders"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "limit": 100 }))
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("Dropbox API request failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("List shared folders failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        let list: ListSharedFoldersResponse = resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(list
            .entries
            .into_iter()
            .map(|e| SharedFolderInfo {
                shared_folder_id: e.shared_folder_id,
                name: e.name,
                path_lower: e.path_lower,
                is_team_folder: e.is_team_folder.unwrap_or(false),
            })
            .collect())
    }

    /// Upload a file to Dropbox.
    pub async fn upload_file(
        &self,
        path: &str,
        data: Vec<u8>,
        mode: &str,
    ) -> Result<DropboxFileMetadata, AppError> {
        if data.len() as u64 <= SIMPLE_UPLOAD_LIMIT {
            self.simple_upload(path, &data, mode).await
        } else {
            self.session_upload(path, &data, mode).await
        }
    }

    /// Simple upload for files <= 150MB.
    async fn simple_upload(
        &self,
        path: &str,
        data: &[u8],
        mode: &str,
    ) -> Result<DropboxFileMetadata, AppError> {
        let token = self.get_token().await?;

        let api_arg = serde_json::json!({
            "path": path,
            "mode": mode,
            "autorename": true,
            "mute": false,
        });

        let resp = self.http
            .post(format!("{DROPBOX_CONTENT_BASE}/files/upload"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", api_arg.to_string())
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Dropbox upload failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Dropbox upload failed (HTTP {status}): {body}"),
                advice: "Check your storage quota and permissions.".to_string(),
            });
        }

        let meta: DropboxFileMetadata = resp.json().await.map_err(|e| AppError::Transfer {
            message: format!("Failed to parse upload response: {e}"),
            advice: "The file may have been uploaded. Check Dropbox.".to_string(),
        })?;

        tracing::info!("Uploaded to Dropbox: {}", path);
        Ok(meta)
    }

    /// Session upload for large files (> 150MB).
    async fn session_upload(
        &self,
        path: &str,
        data: &[u8],
        mode: &str,
    ) -> Result<DropboxFileMetadata, AppError> {
        let token = self.get_token().await?;

        // Step 1: Start upload session
        let first_chunk = &data[..std::cmp::min(SESSION_CHUNK_SIZE, data.len())];

        let resp = self.http
            .post(format!("{DROPBOX_CONTENT_BASE}/files/upload_session/start"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", serde_json::json!({"close": false}).to_string())
            .header("Content-Type", "application/octet-stream")
            .body(first_chunk.to_vec())
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Upload session start failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Upload session start failed (HTTP {status}): {body}"),
                advice: "Try again.".to_string(),
            });
        }

        let session: UploadSessionStart = resp.json().await.map_err(|e| AppError::Transfer {
            message: format!("Failed to parse session response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        // Step 2: Upload remaining chunks
        let mut offset = first_chunk.len();

        while offset < data.len() {
            let end = std::cmp::min(offset + SESSION_CHUNK_SIZE, data.len());
            let chunk = &data[offset..end];

            let api_arg = serde_json::json!({
                "cursor": {
                    "session_id": session.session_id,
                    "offset": offset,
                },
                "close": false,
            });

            let resp = self.http
                .post(format!("{DROPBOX_CONTENT_BASE}/files/upload_session/append_v2"))
                .bearer_auth(&token)
                .header("Dropbox-API-Arg", api_arg.to_string())
                .header("Content-Type", "application/octet-stream")
                .body(chunk.to_vec())
                .send()
                .await
                .map_err(|e| AppError::Transfer {
                    message: format!("Upload chunk failed: {e}"),
                    advice: "Upload will be retried.".to_string(),
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Transfer {
                    message: format!("Upload chunk failed (HTTP {status}): {body}"),
                    advice: "Try again.".to_string(),
                });
            }

            offset = end;
        }

        // Step 3: Finish upload session
        let api_arg = serde_json::json!({
            "cursor": {
                "session_id": session.session_id,
                "offset": data.len(),
            },
            "commit": {
                "path": path,
                "mode": mode,
                "autorename": true,
                "mute": false,
            },
        });

        let resp = self.http
            .post(format!("{DROPBOX_CONTENT_BASE}/files/upload_session/finish"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", api_arg.to_string())
            .header("Content-Type", "application/octet-stream")
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Upload session finish failed: {e}"),
                advice: "The file may have been partially uploaded.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Upload session finish failed (HTTP {status}): {body}"),
                advice: "Try again.".to_string(),
            });
        }

        let meta: DropboxFileMetadata = resp.json().await.map_err(|e| AppError::Transfer {
            message: format!("Failed to parse response: {e}"),
            advice: "Check Dropbox for the file.".to_string(),
        })?;

        tracing::info!("Session upload to Dropbox complete: {}", path);
        Ok(meta)
    }

    /// Download a file from Dropbox.
    pub async fn download_file(&self, path: &str) -> Result<Vec<u8>, AppError> {
        let token = self.get_token().await?;

        let api_arg = serde_json::json!({ "path": path });

        let resp = self.http
            .post(format!("{DROPBOX_CONTENT_BASE}/files/download"))
            .bearer_auth(&token)
            .header("Dropbox-API-Arg", api_arg.to_string())
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Dropbox download failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Download failed (HTTP {status}): {body}"),
                advice: "Check the path and your permissions.".to_string(),
            });
        }

        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| AppError::Transfer {
            message: format!("Failed to read download data: {e}"),
            advice: "Try again.".to_string(),
        })
    }

    /// Delete a file or folder (soft delete).
    ///
    /// Uses the `files/delete_v2` endpoint which removes the item but keeps
    /// it recoverable via Dropbox's version history for the retention period
    /// (30 days for Basic, 180 days for Professional/Business).
    pub async fn delete(&self, path: &str) -> Result<(), AppError> {
        let token = self.get_token().await?;

        let resp = self.http
            .post(format!("{DROPBOX_API_BASE}/files/delete_v2"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "path": path }))
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Dropbox delete failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Delete failed (HTTP {status}): {body}"),
                advice: "Check the path and your permissions.".to_string(),
            });
        }

        Ok(())
    }

    /// Permanently delete a file or folder.
    ///
    /// Uses the `files/permanently_delete` endpoint. Once called, the item
    /// cannot be recovered. This requires the file to exist (not already
    /// deleted via `delete`). For items already soft-deleted, Dropbox
    /// handles permanent removal after the retention period.
    ///
    /// Note: This endpoint may require specific permissions on Dropbox
    /// Business accounts (team admin approval for permanent deletion).
    pub async fn permanently_delete(&self, path: &str) -> Result<(), AppError> {
        let token = self.get_token().await?;

        let resp = self.http
            .post(format!("{DROPBOX_API_BASE}/files/permanently_delete"))
            .bearer_auth(&token)
            .json(&serde_json::json!({ "path": path }))
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Dropbox permanent delete failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Permanent delete failed (HTTP {status}): {body}"),
                advice: "Check the path and your permissions. Permanent deletion may require admin approval on Business accounts.".to_string(),
            });
        }

        tracing::info!("Permanently deleted from Dropbox: {}", path);
        Ok(())
    }

    /// Create a folder.
    pub async fn create_folder(&self, path: &str) -> Result<String, AppError> {
        let token = self.get_token().await?;

        let resp = self.http
            .post(format!("{DROPBOX_API_BASE}/files/create_folder_v2"))
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "path": path,
                "autorename": false,
            }))
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
                advice: "Check the path and your permissions.".to_string(),
            });
        }

        Ok(path.to_string())
    }

    /// Rename (move) a file or folder.
    pub async fn rename(&self, from_path: &str, to_path: &str) -> Result<(), AppError> {
        self.move_entry(from_path, to_path).await
    }

    /// Move a file or folder.
    pub async fn move_entry(&self, from_path: &str, to_path: &str) -> Result<(), AppError> {
        let token = self.get_token().await?;

        let resp = self.http
            .post(format!("{DROPBOX_API_BASE}/files/move_v2"))
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "from_path": from_path,
                "to_path": to_path,
                "autorename": false,
                "allow_ownership_transfer": false,
            }))
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Dropbox move failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Move failed (HTTP {status}): {body}"),
                advice: "Check paths and permissions.".to_string(),
            });
        }

        Ok(())
    }

    /// Copy a file or folder.
    pub async fn copy_entry(&self, from_path: &str, to_path: &str) -> Result<(), AppError> {
        let token = self.get_token().await?;

        let resp = self.http
            .post(format!("{DROPBOX_API_BASE}/files/copy_v2"))
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "from_path": from_path,
                "to_path": to_path,
                "autorename": false,
            }))
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Dropbox copy failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Copy failed (HTTP {status}): {body}"),
                advice: "Check paths and permissions.".to_string(),
            });
        }

        Ok(())
    }

    /// Get file metadata.
    pub async fn get_metadata(&self, path: &str) -> Result<DropboxFileMetadata, AppError> {
        let token = self.get_token().await?;

        let resp = self.http
            .post(format!("{DROPBOX_API_BASE}/files/get_metadata"))
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "path": path,
                "include_media_info": false,
                "include_deleted": false,
            }))
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("Get metadata failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("Get metadata failed (HTTP {status}): {body}"),
                advice: "Check the path.".to_string(),
            });
        }

        resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse metadata: {e}"),
            advice: "Try again.".to_string(),
        })
    }
}

impl Default for DropboxConnector {
    fn default() -> Self {
        Self::new()
    }
}

/// Public file metadata structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DropboxFileMetadata {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub path_display: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub size: Option<u64>,
    #[serde(default)]
    pub server_modified: Option<String>,
    #[serde(default)]
    pub content_hash: Option<String>,
}

impl Connector for DropboxConnector {
    fn connect(
        &self,
        _profile: &ConnectionProfile,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            let oauth = self.oauth.read().await;
            if oauth.is_none() {
                return Err(AppError::Connection {
                    message: "Dropbox OAuth not initialized".to_string(),
                    advice: "Initialize OAuth with your app key before connecting.".to_string(),
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
            self.list_folder(&path).await
        })
    }

    fn is_connected(&self) -> bool {
        false // Conservative default
    }
}

/// Convert a Dropbox API entry to our FileEntry.
fn dropbox_entry_to_file_entry(entry: DropboxEntry) -> FileEntry {
    let is_dir = entry.tag == "folder";
    let path = entry
        .path_display
        .clone()
        .unwrap_or_else(|| format!("/{}", entry.name));

    let modified = entry
        .server_modified
        .as_deref()
        .and_then(|d| d.parse::<DateTime<Utc>>().ok());

    let extension = if !is_dir {
        entry
            .name
            .rsplit('.')
            .next()
            .filter(|ext| ext.len() < 10)
            .map(|s| s.to_string())
    } else {
        None
    };

    FileEntry {
        name: entry.name,
        path,
        is_dir,
        is_symlink: false,
        size: entry.size.unwrap_or(0),
        modified,
        created: None,
        is_hidden: false,
        extension,
        permissions: if entry.sharing_info.is_some() {
            Some("shared".to_string())
        } else {
            None
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_connector_creation() {
        let connector = DropboxConnector::new();
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_dropbox_entry_to_file_entry_folder() {
        let entry = DropboxEntry {
            tag: "folder".to_string(),
            name: "Documents".to_string(),
            path_display: Some("/Documents".to_string()),
            path_lower: Some("/documents".to_string()),
            id: Some("id:abc123".to_string()),
            size: None,
            server_modified: None,
            client_modified: None,
            content_hash: None,
            is_downloadable: None,
            sharing_info: None,
        };

        let fe = dropbox_entry_to_file_entry(entry);
        assert_eq!(fe.name, "Documents");
        assert_eq!(fe.path, "/Documents");
        assert!(fe.is_dir);
        assert_eq!(fe.size, 0);
    }

    #[test]
    fn test_dropbox_entry_to_file_entry_file() {
        let entry = DropboxEntry {
            tag: "file".to_string(),
            name: "report.pdf".to_string(),
            path_display: Some("/Documents/report.pdf".to_string()),
            path_lower: Some("/documents/report.pdf".to_string()),
            id: Some("id:file123".to_string()),
            size: Some(12345),
            server_modified: Some("2024-03-15T10:00:00Z".to_string()),
            client_modified: None,
            content_hash: Some("abcdef0123456789".to_string()),
            is_downloadable: Some(true),
            sharing_info: Some(serde_json::json!({"shared_folder_id": "sf123"})),
        };

        let fe = dropbox_entry_to_file_entry(entry);
        assert_eq!(fe.name, "report.pdf");
        assert!(!fe.is_dir);
        assert_eq!(fe.size, 12345);
        assert_eq!(fe.extension.as_deref(), Some("pdf"));
        assert_eq!(fe.permissions.as_deref(), Some("shared"));
        assert!(fe.modified.is_some());
    }

    #[tokio::test]
    async fn test_oauth_initialization() {
        let connector = DropboxConnector::new();
        connector.init_oauth("test_key".to_string(), Some("test_secret".to_string()), None).await;

        let result = connector.get_auth_url().await;
        assert!(result.is_ok());
        let (url, state) = result.unwrap();
        assert!(url.contains("dropbox.com"));
        assert!(url.contains("test_key"));
        assert!(!state.is_empty());
    }

    #[test]
    fn test_constants() {
        assert_eq!(SIMPLE_UPLOAD_LIMIT, 150 * 1024 * 1024);
        assert_eq!(SESSION_CHUNK_SIZE, 8 * 1024 * 1024);
    }
}
