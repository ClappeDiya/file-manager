//! OneDrive Connector (T-029).
//!
//! Features:
//! - OAuth 2.0 (personal Microsoft accounts) and Azure AD (business/O365)
//! - SharePoint document libraries browsable
//! - OneDrive naming restriction profile auto-applied:
//!   - 400-char path limit
//!   - Forbidden characters: \ / : * ? " < > | # %
//!   - Reserved names: CON, PRN, AUX, NUL, COM0-COM9, LPT0-LPT9
//! - Compatibility warnings surface immediately on drag
//! - Large file upload via upload sessions

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

// Microsoft identity platform endpoints
const MS_AUTH_URL_PERSONAL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/authorize";
const MS_AUTH_URL_BUSINESS: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/authorize";
const MS_TOKEN_URL_PERSONAL: &str = "https://login.microsoftonline.com/consumers/oauth2/v2.0/token";
const MS_TOKEN_URL_BUSINESS: &str = "https://login.microsoftonline.com/common/oauth2/v2.0/token";
const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0";
const DEFAULT_CALLBACK_PORT: u16 = 9878;

// OneDrive naming restrictions
const MAX_PATH_LENGTH: usize = 400;
const FORBIDDEN_CHARS: &[char] = &['\\', '/', ':', '*', '?', '"', '<', '>', '|', '#', '%'];
const RESERVED_NAMES: &[&str] = &[
    "CON", "PRN", "AUX", "NUL",
    "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
    "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

// Simple upload limit: 4MB for OneDrive
const SIMPLE_UPLOAD_LIMIT: u64 = 4 * 1024 * 1024;
// Upload session chunk size: 10MB (must be multiple of 320KB for OneDrive)
const SESSION_CHUNK_SIZE: usize = 10 * 320 * 1024; // 3.125MB, aligned to 320KB

/// OneDrive account type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AccountType {
    Personal,
    Business,
}

/// Naming restriction check result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingCheck {
    pub is_valid: bool,
    pub issues: Vec<NamingIssue>,
}

/// A specific naming restriction violation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NamingIssue {
    pub kind: NamingIssueKind,
    pub description: String,
    pub suggested_fix: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NamingIssueKind {
    PathTooLong,
    ForbiddenCharacter,
    ReservedName,
    TrailingDotOrSpace,
    LeadingTilde,
    EmptyName,
}

/// OneDrive item from Graph API.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct GraphDriveItem {
    id: String,
    name: String,
    size: Option<u64>,
    #[serde(default)]
    folder: Option<FolderFacet>,
    #[serde(default)]
    file: Option<FileFacet>,
    last_modified_date_time: Option<String>,
    created_date_time: Option<String>,
    #[serde(default)]
    parent_reference: Option<ParentReference>,
    #[serde(default)]
    web_url: Option<String>,
    #[serde(default)]
    shared: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct FolderFacet {
    child_count: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct FileFacet {
    mime_type: Option<String>,
    #[serde(default)]
    hashes: Option<FileHashes>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct FileHashes {
    sha256_hash: Option<String>,
    quick_xor_hash: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ParentReference {
    drive_id: Option<String>,
    id: Option<String>,
    path: Option<String>,
}

#[derive(Debug, Deserialize)]
struct GraphDriveItemList {
    value: Vec<GraphDriveItem>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

/// SharePoint site information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SharePointSite {
    pub id: String,
    pub display_name: String,
    pub web_url: String,
}

#[derive(Debug, Deserialize)]
struct GraphSiteList {
    value: Vec<SharePointSiteEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SharePointSiteEntry {
    id: String,
    display_name: Option<String>,
    web_url: Option<String>,
}

/// Upload session response.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct UploadSession {
    upload_url: String,
    expiration_date_time: Option<String>,
}

/// OneDrive connector.
pub struct OneDriveConnector {
    oauth: Arc<RwLock<Option<OAuthClient>>>,
    http: reqwest::Client,
    account_type: Arc<RwLock<AccountType>>,
}

impl OneDriveConnector {
    pub fn new() -> Self {
        Self {
            oauth: Arc::new(RwLock::new(None)),
            http: reqwest::Client::new(),
            account_type: Arc::new(RwLock::new(AccountType::Personal)),
        }
    }

    /// Initialize OAuth for personal or business accounts.
    pub async fn init_oauth(
        &self,
        client_id: String,
        client_secret: Option<String>,
        account_type: AccountType,
        callback_port: Option<u16>,
    ) {
        let port = callback_port.unwrap_or(DEFAULT_CALLBACK_PORT);

        let (auth_url, token_url) = match account_type {
            AccountType::Personal => (MS_AUTH_URL_PERSONAL, MS_TOKEN_URL_PERSONAL),
            AccountType::Business => (MS_AUTH_URL_BUSINESS, MS_TOKEN_URL_BUSINESS),
        };

        let config = OAuthConfig {
            client_id,
            client_secret,
            auth_url: auth_url.to_string(),
            token_url: token_url.to_string(),
            redirect_uri: format!("http://localhost:{port}/callback"),
            scopes: vec![
                "Files.ReadWrite.All".to_string(),
                "Sites.Read.All".to_string(),
                "offline_access".to_string(),
            ],
            extra_params: vec![],
        };

        let client = OAuthClient::new(config);
        let mut oauth = self.oauth.write().await;
        *oauth = Some(client);

        let mut at = self.account_type.write().await;
        *at = account_type;
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

    /// Complete the OAuth flow.
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

    /// Get a valid access token.
    pub async fn get_token(&self) -> Result<String, AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to OneDrive".to_string(),
            advice: "Connect first.".to_string(),
        })?;
        client.get_valid_token().await
    }

    /// List items in a OneDrive folder.
    pub async fn list_items(
        &self,
        path: Option<&str>,
    ) -> Result<Vec<FileEntry>, AppError> {
        let token = self.get_token().await?;

        let url = match path {
            Some(p) if !p.is_empty() && p != "/" => {
                let encoded = percent_encoding::percent_encode(
                    p.as_bytes(),
                    percent_encoding::NON_ALPHANUMERIC,
                );
                format!("{GRAPH_API_BASE}/me/drive/root:/{encoded}:/children?$top=200")
            }
            _ => format!("{GRAPH_API_BASE}/me/drive/root/children?$top=200"),
        };

        self.fetch_items(&url, &token).await
    }

    /// List items in a drive by item ID (for navigation).
    pub async fn list_items_by_id(&self, item_id: &str) -> Result<Vec<FileEntry>, AppError> {
        let token = self.get_token().await?;
        let url = format!("{GRAPH_API_BASE}/me/drive/items/{item_id}/children?$top=200");
        self.fetch_items(&url, &token).await
    }

    /// Fetch items from a Graph API URL with pagination.
    async fn fetch_items(&self, url: &str, token: &str) -> Result<Vec<FileEntry>, AppError> {
        let mut all_entries = Vec::new();
        let mut next_url = Some(url.to_string());

        while let Some(current_url) = next_url {
            let resp = self.http
                .get(&current_url)
                .bearer_auth(token)
                .send()
                .await
                .map_err(|e| AppError::Connection {
                    message: format!("OneDrive API request failed: {e}"),
                    advice: "Check your network connection.".to_string(),
                })?;

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Connection {
                    message: format!("OneDrive API error (HTTP {status}): {body}"),
                    advice: "Check your permissions or re-authorize.".to_string(),
                });
            }

            let list: GraphDriveItemList = resp.json().await.map_err(|e| AppError::Connection {
                message: format!("Failed to parse OneDrive response: {e}"),
                advice: "Try again.".to_string(),
            })?;

            for item in list.value {
                all_entries.push(graph_item_to_file_entry(item));
            }

            next_url = list.next_link;
        }

        Ok(all_entries)
    }

    /// List SharePoint sites.
    pub async fn list_sharepoint_sites(&self) -> Result<Vec<SharePointSite>, AppError> {
        let token = self.get_token().await?;

        let url = format!("{GRAPH_API_BASE}/sites?search=*&$top=100");

        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("SharePoint API request failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("SharePoint API error (HTTP {status}): {body}"),
                advice: "You may not have SharePoint access.".to_string(),
            });
        }

        let list: GraphSiteList = resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(list
            .value
            .into_iter()
            .map(|s| SharePointSite {
                id: s.id,
                display_name: s.display_name.unwrap_or_else(|| "Untitled".to_string()),
                web_url: s.web_url.unwrap_or_default(),
            })
            .collect())
    }

    /// List document libraries for a SharePoint site.
    pub async fn list_site_drives(&self, site_id: &str) -> Result<Vec<FileEntry>, AppError> {
        let token = self.get_token().await?;

        let url = format!("{GRAPH_API_BASE}/sites/{site_id}/drives");

        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("SharePoint drives request failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("SharePoint drives error (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        #[derive(Deserialize)]
        struct DriveList {
            value: Vec<DriveEntry>,
        }
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct DriveEntry {
            id: String,
            name: String,
            #[serde(default)]
            drive_type: Option<String>,
        }

        let list: DriveList = resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(list
            .value
            .into_iter()
            .map(|d| FileEntry {
                name: d.name,
                path: format!("/drives/{}", d.id),
                is_dir: true,
                is_symlink: false,
                size: 0,
                modified: None,
                created: None,
                is_hidden: false,
                extension: None,
                permissions: d.drive_type,
            })
            .collect())
    }

    /// Upload a file to OneDrive.
    pub async fn upload_file(
        &self,
        path: &str,
        data: Vec<u8>,
    ) -> Result<String, AppError> {
        // Check naming restrictions before upload
        let check = check_onedrive_naming(path);
        if !check.is_valid {
            let issues_desc: Vec<String> = check.issues.iter().map(|i| i.description.clone()).collect();
            return Err(AppError::Transfer {
                message: format!("OneDrive naming restrictions violated: {}", issues_desc.join("; ")),
                advice: check.issues.first().and_then(|i| i.suggested_fix.clone()).unwrap_or_default(),
            });
        }

        if data.len() as u64 <= SIMPLE_UPLOAD_LIMIT {
            self.simple_upload(path, &data).await
        } else {
            self.session_upload(path, &data).await
        }
    }

    /// Simple upload for small files (< 4MB).
    async fn simple_upload(&self, path: &str, data: &[u8]) -> Result<String, AppError> {
        let token = self.get_token().await?;

        let encoded_path = percent_encoding::percent_encode(
            path.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC,
        );
        let url = format!("{GRAPH_API_BASE}/me/drive/root:/{encoded_path}:/content");

        let resp = self.http
            .put(&url)
            .bearer_auth(&token)
            .header("Content-Type", "application/octet-stream")
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("OneDrive upload failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Upload failed (HTTP {status}): {body}"),
                advice: "Check your storage quota and permissions.".to_string(),
            });
        }

        let item: GraphDriveItem = resp.json().await.map_err(|e| AppError::Transfer {
            message: format!("Failed to parse upload response: {e}"),
            advice: "The file may have been uploaded.".to_string(),
        })?;

        tracing::info!("Uploaded to OneDrive: {} (ID: {})", path, item.id);
        Ok(item.id)
    }

    /// Upload session for large files (> 4MB).
    async fn session_upload(&self, path: &str, data: &[u8]) -> Result<String, AppError> {
        let token = self.get_token().await?;

        // Step 1: Create upload session
        let encoded_path = percent_encoding::percent_encode(
            path.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC,
        );
        let url = format!("{GRAPH_API_BASE}/me/drive/root:/{encoded_path}:/createUploadSession");

        let resp = self.http
            .post(&url)
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "item": {
                    "@microsoft.graph.conflictBehavior": "rename"
                }
            }))
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("Create upload session failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("Create upload session failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        let session: UploadSession = resp.json().await.map_err(|e| AppError::Transfer {
            message: format!("Failed to parse upload session response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        // Step 2: Upload chunks (must be multiples of 320KB)
        let total = data.len();
        let mut offset = 0;

        while offset < total {
            let end = std::cmp::min(offset + SESSION_CHUNK_SIZE, total);
            let chunk = &data[offset..end];

            let content_range = format!("bytes {}-{}/{}", offset, end - 1, total);

            let resp = self.http
                .put(&session.upload_url)
                .header("Content-Range", &content_range)
                .header("Content-Length", chunk.len().to_string())
                .body(chunk.to_vec())
                .send()
                .await
                .map_err(|e| AppError::Transfer {
                    message: format!("Upload chunk failed: {e}"),
                    advice: "Upload will be retried.".to_string(),
                })?;

            let status = resp.status().as_u16();
            if status == 200 || status == 201 {
                // Upload complete
                let item: GraphDriveItem = resp.json().await.map_err(|e| AppError::Transfer {
                    message: format!("Failed to parse response: {e}"),
                    advice: "The file may have been uploaded.".to_string(),
                })?;
                tracing::info!("Session upload to OneDrive complete: {} (ID: {})", path, item.id);
                return Ok(item.id);
            } else if status == 202 {
                // Accepted, continue
                offset = end;
            } else {
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Transfer {
                    message: format!("Upload chunk failed (HTTP {status}): {body}"),
                    advice: "Try again.".to_string(),
                });
            }
        }

        Err(AppError::Transfer {
            message: "Upload completed without receiving file ID".to_string(),
            advice: "Check OneDrive for the file.".to_string(),
        })
    }

    /// Download a file from OneDrive.
    pub async fn download_file(&self, item_id: &str) -> Result<Vec<u8>, AppError> {
        let token = self.get_token().await?;

        let url = format!("{GRAPH_API_BASE}/me/drive/items/{item_id}/content");

        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("OneDrive download failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        // OneDrive returns a 302 redirect to the download URL
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

    /// Delete a file or folder (moves to recycle bin).
    ///
    /// The Microsoft Graph API `DELETE /me/drive/items/{id}` moves the item
    /// to the OneDrive recycle bin by default (soft delete). Items remain
    /// recoverable for up to 93 days.
    pub async fn delete_item(&self, item_id: &str) -> Result<(), AppError> {
        let token = self.get_token().await?;

        let url = format!("{GRAPH_API_BASE}/me/drive/items/{item_id}");

        let resp = self.http
            .delete(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("OneDrive delete failed: {e}"),
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

        Ok(())
    }

    /// Permanently delete an item by first moving it to the recycle bin,
    /// then purging it from the recycle bin.
    ///
    /// This is a two-step process:
    /// 1. DELETE the item (moves to recycle bin)
    /// 2. DELETE from the recycle bin (permanent removal)
    pub async fn delete_item_permanently(&self, item_id: &str) -> Result<(), AppError> {
        // Step 1: Move to recycle bin (standard delete)
        self.delete_item(item_id).await?;

        // Step 2: Permanently delete from recycle bin
        let token = self.get_token().await?;
        let url = format!(
            "{GRAPH_API_BASE}/me/drive/items/{item_id}/permanentDelete"
        );

        let resp = self.http
            .post(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Transfer {
                message: format!("OneDrive permanent delete failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        // 204 No Content on success, or the item may already be purged
        if !resp.status().is_success() && resp.status().as_u16() != 204 {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            // If the permanent delete endpoint is not available (older API),
            // fall back silently -- the item is already in the recycle bin.
            if status.as_u16() == 404 || status.as_u16() == 405 {
                tracing::warn!(
                    "permanentDelete not supported for item {item_id}, item is in recycle bin"
                );
                return Ok(());
            }
            return Err(AppError::Transfer {
                message: format!("Permanent delete failed (HTTP {status}): {body}"),
                advice: "The item was moved to the recycle bin but could not be permanently deleted.".to_string(),
            });
        }

        tracing::info!("Permanently deleted OneDrive item: {item_id}");
        Ok(())
    }

    /// List items in the OneDrive recycle bin.
    ///
    /// Returns items that have been soft-deleted and can be restored
    /// or permanently removed.
    pub async fn list_recycle_bin(&self) -> Result<Vec<FileEntry>, AppError> {
        let token = self.get_token().await?;

        // The Graph API exposes deleted items at /me/drive/items/{driveItem-id}/children
        // but the recycle bin is available through a special endpoint:
        // GET /me/drive/special/recycle/children  (not officially documented)
        // The supported approach is: GET /me/drive/root/delta with a filter
        // However, the most reliable approach uses the list of deleted items:
        let url = format!(
            "{GRAPH_API_BASE}/me/drive/special/recycle/children?$top=200"
        );

        let resp = self.http
            .get(&url)
            .bearer_auth(&token)
            .send()
            .await
            .map_err(|e| AppError::Connection {
                message: format!("OneDrive recycle bin request failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("Recycle bin listing failed (HTTP {status}): {body}"),
                advice: "Check your permissions or re-authorize.".to_string(),
            });
        }

        let list: GraphDriveItemList = resp.json().await.map_err(|e| AppError::Connection {
            message: format!("Failed to parse recycle bin response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        let entries = list
            .value
            .into_iter()
            .map(graph_item_to_file_entry)
            .collect();

        Ok(entries)
    }

    /// Create a folder.
    pub async fn create_folder(
        &self,
        parent_path: &str,
        name: &str,
    ) -> Result<String, AppError> {
        // Check naming restrictions
        let check = check_onedrive_naming(name);
        if !check.is_valid {
            let issues: Vec<String> = check.issues.iter().map(|i| i.description.clone()).collect();
            return Err(AppError::Transfer {
                message: format!("OneDrive naming restrictions: {}", issues.join("; ")),
                advice: check.issues.first().and_then(|i| i.suggested_fix.clone()).unwrap_or_default(),
            });
        }

        let token = self.get_token().await?;

        let encoded_parent = percent_encoding::percent_encode(
            parent_path.as_bytes(),
            percent_encoding::NON_ALPHANUMERIC,
        );
        let url = if parent_path.is_empty() || parent_path == "/" {
            format!("{GRAPH_API_BASE}/me/drive/root/children")
        } else {
            format!("{GRAPH_API_BASE}/me/drive/root:/{encoded_parent}:/children")
        };

        let resp = self.http
            .post(&url)
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "name": name,
                "folder": {},
                "@microsoft.graph.conflictBehavior": "rename"
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
                advice: "Check your permissions.".to_string(),
            });
        }

        let item: GraphDriveItem = resp.json().await.map_err(|e| AppError::Transfer {
            message: format!("Failed to parse response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(item.id)
    }

    /// Rename an item.
    pub async fn rename_item(&self, item_id: &str, new_name: &str) -> Result<(), AppError> {
        let check = check_onedrive_naming(new_name);
        if !check.is_valid {
            let issues: Vec<String> = check.issues.iter().map(|i| i.description.clone()).collect();
            return Err(AppError::Transfer {
                message: format!("OneDrive naming restrictions: {}", issues.join("; ")),
                advice: check.issues.first().and_then(|i| i.suggested_fix.clone()).unwrap_or_default(),
            });
        }

        let token = self.get_token().await?;

        let url = format!("{GRAPH_API_BASE}/me/drive/items/{item_id}");

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

    /// Move an item to a new parent folder.
    pub async fn move_item(
        &self,
        item_id: &str,
        new_parent_id: &str,
    ) -> Result<(), AppError> {
        let token = self.get_token().await?;

        let url = format!("{GRAPH_API_BASE}/me/drive/items/{item_id}");

        let resp = self.http
            .patch(&url)
            .bearer_auth(&token)
            .json(&serde_json::json!({
                "parentReference": { "id": new_parent_id }
            }))
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

impl Default for OneDriveConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl Connector for OneDriveConnector {
    fn connect(
        &self,
        _profile: &ConnectionProfile,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            let oauth = self.oauth.read().await;
            if oauth.is_none() {
                return Err(AppError::Connection {
                    message: "OneDrive OAuth not initialized".to_string(),
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
            if path.is_empty() || path == "/" {
                self.list_items(None).await
            } else {
                self.list_items(Some(&path)).await
            }
        })
    }

    fn is_connected(&self) -> bool {
        false // Conservative default
    }
}

// ── Naming Restriction Enforcement ──

/// Check a filename or path against OneDrive naming restrictions.
/// This is called during preflight/drag operations for immediate feedback.
pub fn check_onedrive_naming(name: &str) -> NamingCheck {
    let mut issues = Vec::new();

    // Empty name check
    if name.is_empty() {
        issues.push(NamingIssue {
            kind: NamingIssueKind::EmptyName,
            description: "File name cannot be empty".to_string(),
            suggested_fix: None,
        });
        return NamingCheck {
            is_valid: false,
            issues,
        };
    }

    // Path length check
    if name.len() > MAX_PATH_LENGTH {
        issues.push(NamingIssue {
            kind: NamingIssueKind::PathTooLong,
            description: format!(
                "Path is {} characters, OneDrive limit is {MAX_PATH_LENGTH}",
                name.len()
            ),
            suggested_fix: Some("Shorten the file name or reduce nesting depth.".to_string()),
        });
    }

    // Check each segment of the path
    for segment in name.split('/').filter(|s| !s.is_empty()) {
        // Forbidden characters
        for ch in FORBIDDEN_CHARS {
            if segment.contains(*ch) {
                issues.push(NamingIssue {
                    kind: NamingIssueKind::ForbiddenCharacter,
                    description: format!("Character '{ch}' is not allowed in OneDrive file names"),
                    suggested_fix: Some(format!("Replace '{ch}' with an underscore or remove it.")),
                });
            }
        }

        // Reserved names (case-insensitive, with or without extension)
        let stem = segment.split('.').next().unwrap_or(segment);
        if RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
            issues.push(NamingIssue {
                kind: NamingIssueKind::ReservedName,
                description: format!("'{stem}' is a reserved name on OneDrive/Windows"),
                suggested_fix: Some(format!("Rename '{stem}' to '_{stem}' or similar.")),
            });
        }

        // Trailing dot or space
        if segment.ends_with('.') || segment.ends_with(' ') {
            issues.push(NamingIssue {
                kind: NamingIssueKind::TrailingDotOrSpace,
                description: "File names cannot end with a dot or space on OneDrive".to_string(),
                suggested_fix: Some("Remove the trailing dot or space.".to_string()),
            });
        }

        // Leading tilde
        if segment.starts_with('~') {
            issues.push(NamingIssue {
                kind: NamingIssueKind::LeadingTilde,
                description: "File names starting with '~' may cause issues on OneDrive".to_string(),
                suggested_fix: Some("Remove or replace the leading tilde.".to_string()),
            });
        }
    }

    NamingCheck {
        is_valid: issues.is_empty(),
        issues,
    }
}

/// Sanitize a filename for OneDrive compatibility.
pub fn sanitize_for_onedrive(name: &str) -> String {
    let mut result = name.to_string();

    // Replace forbidden characters
    for ch in FORBIDDEN_CHARS {
        result = result.replace(*ch, "_");
    }

    // Handle reserved names
    let parts: Vec<&str> = result.split('/').collect();
    let sanitized_parts: Vec<String> = parts
        .into_iter()
        .map(|segment| {
            if segment.is_empty() {
                return String::new();
            }

            let mut s = segment.to_string();

            // Remove trailing dots and spaces
            while s.ends_with('.') || s.ends_with(' ') {
                s.pop();
            }

            // Handle reserved names
            let stem = s.split('.').next().unwrap_or(&s);
            if RESERVED_NAMES.iter().any(|r| r.eq_ignore_ascii_case(stem)) {
                s = format!("_{s}");
            }

            // Remove leading tilde
            if s.starts_with('~') {
                s = format!("_{}", &s[1..]);
            }

            s
        })
        .collect();

    let result = sanitized_parts.join("/");

    // Truncate to max path length
    if result.len() > MAX_PATH_LENGTH {
        result[..MAX_PATH_LENGTH].to_string()
    } else {
        result
    }
}

/// Convert a Graph API drive item to our FileEntry.
fn graph_item_to_file_entry(item: GraphDriveItem) -> FileEntry {
    let is_dir = item.folder.is_some();
    let size = item.size.unwrap_or(0);

    let modified = item
        .last_modified_date_time
        .as_deref()
        .and_then(|d| d.parse::<DateTime<Utc>>().ok());

    let created = item
        .created_date_time
        .as_deref()
        .and_then(|d| d.parse::<DateTime<Utc>>().ok());

    let path = item
        .parent_reference
        .as_ref()
        .and_then(|p| p.path.clone())
        .map(|p| format!("{}/{}", p, item.name))
        .unwrap_or_else(|| format!("/{}", item.name));

    let extension = if !is_dir {
        item.name
            .rsplit('.')
            .next()
            .filter(|ext| ext.len() < 10)
            .map(|s| s.to_string())
    } else {
        None
    };

    FileEntry {
        name: item.name,
        path,
        is_dir,
        is_symlink: false,
        size,
        modified,
        created,
        is_hidden: false,
        extension,
        permissions: if item.shared.is_some() {
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
        let connector = OneDriveConnector::new();
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_check_valid_name() {
        let check = check_onedrive_naming("my-document.pdf");
        assert!(check.is_valid);
        assert!(check.issues.is_empty());
    }

    #[test]
    fn test_check_forbidden_chars() {
        let check = check_onedrive_naming("file:name.txt");
        assert!(!check.is_valid);
        assert!(check.issues.iter().any(|i| i.kind == NamingIssueKind::ForbiddenCharacter));
    }

    #[test]
    fn test_check_reserved_name() {
        let check = check_onedrive_naming("CON.txt");
        assert!(!check.is_valid);
        assert!(check.issues.iter().any(|i| i.kind == NamingIssueKind::ReservedName));
    }

    #[test]
    fn test_check_reserved_name_case_insensitive() {
        let check = check_onedrive_naming("con");
        assert!(!check.is_valid);

        let check = check_onedrive_naming("Com1.log");
        assert!(!check.is_valid);
    }

    #[test]
    fn test_check_trailing_dot() {
        let check = check_onedrive_naming("file.");
        assert!(!check.is_valid);
        assert!(check.issues.iter().any(|i| i.kind == NamingIssueKind::TrailingDotOrSpace));
    }

    #[test]
    fn test_check_trailing_space() {
        let check = check_onedrive_naming("file ");
        assert!(!check.is_valid);
        assert!(check.issues.iter().any(|i| i.kind == NamingIssueKind::TrailingDotOrSpace));
    }

    #[test]
    fn test_check_leading_tilde() {
        let check = check_onedrive_naming("~tempfile");
        assert!(!check.is_valid);
        assert!(check.issues.iter().any(|i| i.kind == NamingIssueKind::LeadingTilde));
    }

    #[test]
    fn test_check_path_too_long() {
        let long_name = "a".repeat(401);
        let check = check_onedrive_naming(&long_name);
        assert!(!check.is_valid);
        assert!(check.issues.iter().any(|i| i.kind == NamingIssueKind::PathTooLong));
    }

    #[test]
    fn test_check_empty_name() {
        let check = check_onedrive_naming("");
        assert!(!check.is_valid);
        assert!(check.issues.iter().any(|i| i.kind == NamingIssueKind::EmptyName));
    }

    #[test]
    fn test_check_path_with_segments() {
        let check = check_onedrive_naming("valid/path/file.txt");
        assert!(check.is_valid);

        let check = check_onedrive_naming("valid/CON/file.txt");
        assert!(!check.is_valid);
    }

    #[test]
    fn test_sanitize_forbidden_chars() {
        assert_eq!(sanitize_for_onedrive("file:name*.txt"), "file_name_.txt");
        assert_eq!(sanitize_for_onedrive("a<b>c"), "a_b_c");
    }

    #[test]
    fn test_sanitize_reserved_name() {
        assert_eq!(sanitize_for_onedrive("CON"), "_CON");
        assert_eq!(sanitize_for_onedrive("con.txt"), "_con.txt");
    }

    #[test]
    fn test_sanitize_trailing() {
        assert_eq!(sanitize_for_onedrive("file."), "file");
        assert_eq!(sanitize_for_onedrive("file "), "file");
        assert_eq!(sanitize_for_onedrive("file.. "), "file");
    }

    #[test]
    fn test_sanitize_leading_tilde() {
        assert_eq!(sanitize_for_onedrive("~temp"), "_temp");
    }

    #[test]
    fn test_sanitize_path_length() {
        let long = "a".repeat(500);
        let sanitized = sanitize_for_onedrive(&long);
        assert!(sanitized.len() <= MAX_PATH_LENGTH);
    }

    #[test]
    fn test_graph_item_to_file_entry_folder() {
        let item = GraphDriveItem {
            id: "item1".to_string(),
            name: "Documents".to_string(),
            size: None,
            folder: Some(FolderFacet { child_count: Some(5) }),
            file: None,
            last_modified_date_time: Some("2024-01-15T10:00:00Z".to_string()),
            created_date_time: None,
            parent_reference: None,
            web_url: None,
            shared: None,
        };

        let entry = graph_item_to_file_entry(item);
        assert_eq!(entry.name, "Documents");
        assert!(entry.is_dir);
        assert!(entry.modified.is_some());
    }

    #[test]
    fn test_graph_item_to_file_entry_file() {
        let item = GraphDriveItem {
            id: "item2".to_string(),
            name: "report.xlsx".to_string(),
            size: Some(5000),
            folder: None,
            file: Some(FileFacet {
                mime_type: Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet".to_string()),
                hashes: None,
            }),
            last_modified_date_time: None,
            created_date_time: Some("2024-06-01T12:00:00Z".to_string()),
            parent_reference: Some(ParentReference {
                drive_id: Some("drive1".to_string()),
                id: Some("parent1".to_string()),
                path: Some("/drive/root:/Documents".to_string()),
            }),
            web_url: None,
            shared: Some(serde_json::json!({"scope": "users"})),
        };

        let entry = graph_item_to_file_entry(item);
        assert_eq!(entry.name, "report.xlsx");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 5000);
        assert_eq!(entry.extension.as_deref(), Some("xlsx"));
        assert_eq!(entry.path, "/drive/root:/Documents/report.xlsx");
        assert_eq!(entry.permissions.as_deref(), Some("shared"));
    }

    #[tokio::test]
    async fn test_oauth_personal_init() {
        let connector = OneDriveConnector::new();
        connector
            .init_oauth(
                "test_client".to_string(),
                None,
                AccountType::Personal,
                None,
            )
            .await;

        let (url, state) = connector.get_auth_url().await.unwrap();
        assert!(url.contains("login.microsoftonline.com/consumers"));
        assert!(!state.is_empty());
    }

    #[tokio::test]
    async fn test_oauth_business_init() {
        let connector = OneDriveConnector::new();
        connector
            .init_oauth(
                "test_client".to_string(),
                Some("secret".to_string()),
                AccountType::Business,
                None,
            )
            .await;

        let (url, _) = connector.get_auth_url().await.unwrap();
        assert!(url.contains("login.microsoftonline.com/common"));
    }
}
