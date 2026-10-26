//! Dropbox Connector (T-028).
//!
//! Features:
//! - OAuth 2.0 flow
//! - Root, team folders, shared folders browsable
//! - File operations: upload, download, rename, move, delete, mkdir
//! - Selective sync support
//! - Drag-and-drop between Dropbox and local panes

use crate::connectors::oauth::{OAuthClient, OAuthToken};
use crate::connectors::oauth_flow::{self, OAuthProvider};
use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, DropboxSettings, FileEntry};
use std::sync::atomic::{AtomicBool, Ordering};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    api_base: String,
    content_base: String,
    /// Token endpoint override (tests); `None` = Dropbox's.
    token_url: Option<String>,
    /// Rotated tokens are written back here (the app's credential store).
    credentials: Option<Arc<crate::security::CredentialStore>>,
    connected: AtomicBool,
    /// The saved connection the current sign-in belongs to.
    profile_id: std::sync::RwLock<Option<uuid::Uuid>>,
    account: std::sync::RwLock<Option<String>>,
    /// Team space / team folder namespace used as the root.
    settings: std::sync::RwLock<DropboxSettings>,
}

impl DropboxConnector {
    pub fn new() -> Self {
        Self {
            oauth: Arc::new(RwLock::new(None)),
            http: crate::net::client(),
            api_base: DROPBOX_API_BASE.to_string(),
            content_base: DROPBOX_CONTENT_BASE.to_string(),
            token_url: None,
            credentials: None,
            connected: AtomicBool::new(false),
            profile_id: std::sync::RwLock::new(None),
            account: std::sync::RwLock::new(None),
            settings: std::sync::RwLock::new(DropboxSettings::default()),
        }
    }

    /// A new, disconnected instance with this one's endpoints and credential
    /// store (an action for another account runs on it).
    pub fn fresh(&self) -> Self {
        Self {
            http: self.http.clone(),
            api_base: self.api_base.clone(),
            content_base: self.content_base.clone(),
            token_url: self.token_url.clone(),
            credentials: self.credentials.clone(),
            ..Self::new()
        }
    }

    /// Persist rotated tokens in `store` (under the connection's credential key).
    pub fn with_credentials(mut self, store: Arc<crate::security::CredentialStore>) -> Self {
        self.credentials = Some(store);
        self
    }

    /// Talk to `api` / `content` / `token_url` instead of Dropbox (tests).
    pub fn with_endpoints(mut self, api: &str, content: &str, token_url: &str) -> Self {
        self.api_base = api.trim_end_matches('/').to_string();
        self.content_base = content.trim_end_matches('/').to_string();
        self.token_url = Some(token_url.to_string());
        self
    }

    pub fn account(&self) -> Option<String> {
        self.account.read().ok().and_then(|a| a.clone())
    }

    /// `Dropbox-API-Path-Root` for the chosen team space / team folder.
    fn path_root_headers(&self) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        let ns = self.settings.read().ok().and_then(|s| s.root_namespace_id.clone()).filter(|n| !n.is_empty());
        if let Some(ns) = ns {
            let v = serde_json::json!({ ".tag": "namespace_id", "namespace_id": ns }).to_string();
            if let Ok(v) = reqwest::header::HeaderValue::from_str(&v) {
                h.insert("Dropbox-API-Path-Root", v);
            }
        }
        h
    }

    /// Initialize the OAuth client.
    pub async fn init_oauth(
        &self,
        client_id: String,
        client_secret: Option<String>,
        callback_port: Option<u16>,
    ) {
        let port = callback_port.unwrap_or(DEFAULT_CALLBACK_PORT);
        let provider = OAuthProvider::Dropbox;
        let config = provider.oauth_config(client_id, client_secret, format!("http://localhost:{port}/callback"), provider.endpoints(Default::default()));

        let client = OAuthClient::new(config);
        let mut oauth = self.oauth.write().await;
        *oauth = Some(client);
    }

    /// Restore the stored grant, refresh it when expired and check it with
    /// `users/get_current_account`.
    async fn connect_oauth(&self, profile: &ConnectionProfile, secret: Option<String>) -> Result<(), AppError> {
        self.connected.store(false, Ordering::SeqCst);
        self.set_profile_id(None);
        let client = oauth_flow::client_for_connect(OAuthProvider::Dropbox, profile, secret, self.token_url.as_deref(), self.credentials.clone()).await?;
        *self.oauth.write().await = Some(client);
        if let Ok(mut s) = self.settings.write() {
            *s = profile.extras.dropbox.clone().unwrap_or_default();
        }
        let me = self.current_account().await?;
        if let Ok(mut a) = self.account.write() {
            *a = me["email"].as_str().or(me["name"]["display_name"].as_str()).map(String::from);
        }
        self.set_profile_id(Some(profile.id));
        self.connected.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn set_profile_id(&self, id: Option<uuid::Uuid>) {
        if let Ok(mut p) = self.profile_id.write() {
            *p = id;
        }
    }

    /// `users/get_current_account`; a 401 with an unexpired token refreshes once.
    pub async fn current_account(&self) -> Result<serde_json::Value, AppError> {
        self.rpc_json("users/get_current_account", None).await
    }

    /// `users/get_space_usage` (used and allocated bytes).
    pub async fn space_usage(&self) -> Result<serde_json::Value, AppError> {
        self.rpc_json("users/get_space_usage", None).await
    }

    /// A Dropbox RPC call (`body` = JSON arguments, `None` = no arguments)
    /// that refreshes once on 401 and treats a second 401 as rejected.
    async fn rpc_json(&self, endpoint: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, AppError> {
        let url = format!("{}/{endpoint}", self.api_base);
        for attempt in 0..2 {
            let token = self.get_token().await?;
            let req = self.http.post(&url).bearer_auth(&token).headers(self.path_root_headers());
            let req = match &body {
                Some(b) => req.json(b),
                None => req.header("Content-Type", "application/json").body("null"),
            };
            let resp = req.send().await.map_err(|e| AppError::connection(format!("Dropbox is unreachable: {e}"), "Check your network connection."))?;
            let status = resp.status();
            if status.as_u16() == 401 && attempt == 0 {
                self.force_refresh().await?;
                continue;
            }
            if status.as_u16() == 401 {
                return Err(AppError::auth(format!("Dropbox refused the sign-in (HTTP {status})"), "Sign in again from Connections."));
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::connection(format!("Dropbox {endpoint} failed (HTTP {status}): {}", body.chars().take(200).collect::<String>()), "Try again later."));
            }
            return resp.json().await.map_err(|e| AppError::connection(format!("Unexpected Dropbox reply: {e}"), "Try again."));
        }
        Err(AppError::auth("Dropbox refused the renewed sign-in", "Sign in again from Connections."))
    }

    /// The grant to store for the current tokens (after a sign-in or refresh).
    pub async fn current_grant(&self) -> Option<crate::connectors::oauth::OAuthGrant> {
        self.oauth.read().await.as_ref()?.grant().await
    }

    async fn force_refresh(&self) -> Result<(), AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::connection("Not connected to Dropbox", "Connect first."))?;
        client.refresh_token().await.map(|_| ())
    }

    /// Roots the connection can open: the member's own folder, the team
    /// space (team accounts) and team folders.
    pub async fn namespaces(&self) -> Result<Vec<DropboxNamespace>, AppError> {
        let me = self.current_account().await?;
        let mut out = vec![DropboxNamespace { namespace_id: None, label: "My Dropbox".into(), kind: "home".into() }];
        let root = &me["root_info"];
        if root[".tag"].as_str() == Some("team") {
            if let Some(ns) = root["root_namespace_id"].as_str() {
                out.push(DropboxNamespace { namespace_id: Some(ns.into()), label: "Team space".into(), kind: "team_space".into() });
            }
        }
        // A failure is reported, not shown as "no team folders".
        let folders = self.list_shared_folders().await.map_err(|e| match e {
            AppError::Authentication { message, advice } => AppError::Authentication { message: format!("Cannot list the team folders: {message}"), advice },
            e => AppError::connection(format!("Cannot list the team folders: {}", e.user_message()), "Try again; if it keeps failing, check the account's sharing permissions."),
        })?;
        for f in folders {
            if f.is_team_folder {
                out.push(DropboxNamespace { namespace_id: Some(f.shared_folder_id), label: f.name, kind: "team_folder".into() });
            }
        }
        Ok(out)
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
                    .post(format!("{}/files/list_folder/continue", self.api_base))
                    .bearer_auth(&token)
            .headers(self.path_root_headers())
                    .json(&serde_json::json!({ "cursor": cur }))
                    .send()
                    .await
            } else {
                // Initial listing
                self.http
                    .post(format!("{}/files/list_folder", self.api_base))
                    .bearer_auth(&token)
            .headers(self.path_root_headers())
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
                if status.as_u16() == 401 {
                    return Err(AppError::auth(format!("Dropbox refused the sign-in (HTTP {status})"), "Sign in again from Connections."));
                }
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
            .post(format!("{}/sharing/list_folders", self.api_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/upload", self.content_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/upload_session/start", self.content_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
                .post(format!("{}/files/upload_session/append_v2", self.content_base))
                .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/upload_session/finish", self.content_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/download", self.content_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/delete_v2", self.api_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/permanently_delete", self.api_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/create_folder_v2", self.api_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/move_v2", self.api_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/copy_v2", self.api_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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
            .post(format!("{}/files/get_metadata", self.api_base))
            .bearer_auth(&token)
            .headers(self.path_root_headers())
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

/// `get_space_usage` → quota. A team allocation is shared: free space is
/// what the team has left.
fn dropbox_quota(v: &serde_json::Value) -> super::RemoteQuota {
    let used = super::json_u64(&v["used"]);
    let a = &v["allocation"];
    let total = super::json_u64(&a["allocated"]);
    let pool_used = if a[".tag"].as_str() == Some("team") { super::json_u64(&a["used"]).or(used) } else { used };
    super::RemoteQuota { total, used, free: total.zip(pool_used).map(|(t, u)| t.saturating_sub(u)), trashed: None }
}

/// A root a Dropbox connection can open (for the namespace picker).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DropboxNamespace {
    /// `None` = the member's own folder (no path-root header).
    pub namespace_id: Option<String>,
    pub label: String,
    /// `home`, `team_space` or `team_folder`.
    pub kind: String,
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
    fn rename_entry(&self, from: &str, to: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::core::error::AppError>> + Send + '_>> {
        let (from, to) = (from.to_string(), to.to_string());
        Box::pin(async move { self.move_entry(&from, &to).await })
    }

    fn server_copy(&self, from: &str, to: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, crate::core::error::AppError>> + Send + '_>> {
        let (from, to) = (from.to_string(), to.to_string());
        Box::pin(async move { self.copy_entry(&from, &to).await.map(|()| true) })
    }


    fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        self.connect_with_secret(profile, None)
    }

    /// The secret is the stored OAuth grant (see `oauth_flow`).
    fn connect_with_secret(
        &self,
        profile: &ConnectionProfile,
        secret: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let profile = profile.clone();
        Box::pin(async move { self.connect_oauth(&profile, secret).await })
    }

    fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            let oauth = self.oauth.read().await;
            if let Some(client) = oauth.as_ref() {
                client.clear_token().await;
            }
            self.connected.store(false, Ordering::SeqCst);
            self.set_profile_id(None);
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
        self.connected.load(Ordering::SeqCst)
    }

    fn connected_profile_id(&self) -> Option<uuid::Uuid> {
        self.profile_id.read().ok().and_then(|p| *p).filter(|_| self.is_connected())
    }

    fn detached(&self) -> Option<Arc<dyn Connector>> {
        Some(Arc::new(self.fresh()))
    }

    fn upload(&self, local_path: &str, remote_path: &str) -> Pin<Box<dyn Future<Output = Result<u64, AppError>> + Send + '_>> {
        let (l, r) = (local_path.to_string(), remote_path.to_string());
        Box::pin(async move {
            let data = tokio::fs::read(&l).await.map_err(|e| AppError::file_op(format!("Cannot read {l}: {e}"), "Check the file."))?;
            let n = data.len() as u64;
            self.upload_file(&r, data, "overwrite").await?;
            Ok(n)
        })
    }

    fn download(&self, remote_path: &str, local_path: &str) -> Pin<Box<dyn Future<Output = Result<u64, AppError>> + Send + '_>> {
        let (r, l) = (remote_path.to_string(), local_path.to_string());
        Box::pin(async move {
            let data = self.download_file(&r).await?;
            tokio::fs::write(&l, &data).await.map_err(|e| AppError::file_op(format!("Cannot write {l}: {e}"), "Check the destination folder."))?;
            Ok(data.len() as u64)
        })
    }

    fn make_directory(&self, remote_path: &str) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let r = remote_path.to_string();
        Box::pin(async move { self.create_folder(&r).await.map(|_| ()) })
    }

    /// Soft delete (recoverable from Dropbox's deleted files).
    fn remove(&self, remote_path: &str, _is_dir: bool) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let r = remote_path.to_string();
        Box::pin(async move { self.delete(&r).await })
    }

    /// `users/get_space_usage`: the member's use against the individual or
    /// team allocation.
    fn quota(&self) -> Pin<Box<dyn Future<Output = Result<Option<super::RemoteQuota>, AppError>> + Send + '_>> {
        Box::pin(async move { Ok(Some(dropbox_quota(&self.space_usage().await?))) })
    }

    fn create_share_link(
        &self,
        remote_path: &str,
        options: &super::ShareOptions,
    ) -> Pin<Box<dyn Future<Output = Result<super::ShareLink, AppError>> + Send + '_>> {
        let (r, o) = (remote_path.to_string(), options.clone());
        Box::pin(async move {
            let token = self.get_token().await?;
            let resp = self
                .http
                .post(format!("{}/sharing/create_shared_link_with_settings", self.api_base))
                .bearer_auth(&token)
            .headers(self.path_root_headers())
                .json(&dropbox_link_body(&r, &o, chrono::Utc::now()))
                .send()
                .await
                .map_err(|e| AppError::connection(format!("Dropbox share failed: {e}"), "Check your network connection."))?;
            let status = resp.status().as_u16();
            let body = resp.text().await.unwrap_or_default();
            parse_dropbox_link(status, &body, o.scope)
        })
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

/// `create_shared_link_with_settings` request body.
pub(crate) fn dropbox_link_body(path: &str, o: &super::ShareOptions, now: chrono::DateTime<chrono::Utc>) -> serde_json::Value {
    let mut settings = serde_json::json!({
        "audience": { ".tag": match o.scope { super::ShareScope::Anyone => "public", super::ShareScope::Organization => "team" } },
        "access": { ".tag": if o.allow_edit { "editor" } else { "viewer" } },
    });
    if let Some(s) = o.expires_in_secs {
        settings["expires"] = (now + chrono::Duration::seconds(s as i64)).to_rfc3339_opts(chrono::SecondsFormat::Secs, true).into();
    }
    if let Some(p) = o.password.as_deref().filter(|p| !p.is_empty()) {
        settings["require_password"] = true.into();
        settings["link_password"] = p.into();
    }
    serde_json::json!({ "path": path, "settings": settings })
}

/// Link from the reply; an already-existing link is reused (Dropbox keeps
/// one link per file and won't change its settings here).
pub(crate) fn parse_dropbox_link(status: u16, body: &str, scope: super::ShareScope) -> Result<super::ShareLink, AppError> {
    let v: serde_json::Value = serde_json::from_str(body).unwrap_or_default();
    let expires = |m: &serde_json::Value| m["expires"].as_str().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&chrono::Utc));
    if status == 200 {
        let url = v["url"].as_str().ok_or_else(|| AppError::connection("Dropbox returned no link", "Try again."))?;
        return Ok(super::ShareLink { url: url.into(), expires_at: expires(&v), scope, provider: "dropbox".into(), note: None });
    }
    let existing = &v["error"]["shared_link_already_exists"]["metadata"];
    if let Some(url) = existing["url"].as_str() {
        return Ok(super::ShareLink {
            url: url.into(),
            expires_at: expires(existing),
            scope,
            provider: "dropbox".into(),
            note: Some("This file already had a link; Dropbox reused it with its existing settings.".into()),
        });
    }
    let summary = v["error_summary"].as_str().unwrap_or(body);
    Err(AppError::Connection {
        message: format!("Dropbox refused the share link: {summary}"),
        advice: "Expiry dates, passwords and team-only links need a Dropbox plan that allows them.".into(),
    })
}

#[cfg(test)]
mod namespace_tests {
    use super::*;
    use crate::connectors::mock_http::{json, serve, status};
    use crate::core::types::ConnectionProtocol;

    /// Scopes (item 25): a team space / team folder chosen on the profile
    /// is sent as `Dropbox-API-Path-Root`; the picker lists both; quota.
    #[tokio::test(flavor = "multi_thread")]
    async fn team_namespace_root_and_quota() {
        let mock = serve(|r| match r.path.as_str() {
            "/2/users/get_current_account" => json(serde_json::json!({ "email": "m@corp.example", "root_info": { ".tag": "team", "root_namespace_id": "100", "home_namespace_id": "200" } })),
            "/2/sharing/list_folders" => json(serde_json::json!({ "entries": [
                { "shared_folder_id": "300", "name": "Marketing", "is_team_folder": true },
                { "shared_folder_id": "400", "name": "Shared with Bob", "is_team_folder": false }
            ] })),
            "/2/users/get_space_usage" => json(serde_json::json!({ "used": 10, "allocation": { ".tag": "team", "used": 600, "allocated": 1000 } })),
            "/2/files/list_folder" => json(serde_json::json!({ "entries": [], "cursor": "c", "has_more": false })),
            _ => status(404, "no"),
        })
        .await;
        let b = &mock.base;
        let c = DropboxConnector::new().with_endpoints(&format!("{b}/2"), &format!("{b}/c"), &format!("{b}/token"));
        let mut p = crate::connectors::live_tests::profile(ConnectionProtocol::Dropbox, "", None, "", "/");
        c.connect_with_secret(&p, Some("tok".into())).await.unwrap();
        let ns = c.namespaces().await.unwrap();
        assert_eq!(ns.iter().map(|n| (n.namespace_id.as_deref(), n.kind.as_str())).collect::<Vec<_>>(), vec![(None, "home"), (Some("100"), "team_space"), (Some("300"), "team_folder")]);
        c.list_remote("/").await.unwrap();
        assert!(mock.requests("/2/files/list_folder")[0].header("dropbox-api-path-root").is_none());
        p.extras.dropbox = Some(DropboxSettings { root_namespace_id: Some("300".into()), root_label: Some("Marketing".into()) });
        c.connect_with_secret(&p, Some("tok".into())).await.unwrap();
        c.list_remote("/").await.unwrap();
        let header = mock.requests("/2/files/list_folder")[1].header("dropbox-api-path-root").unwrap();
        assert_eq!(serde_json::from_str::<serde_json::Value>(&header).unwrap(), serde_json::json!({ ".tag": "namespace_id", "namespace_id": "300" }));
        assert_eq!(c.quota().await.unwrap().unwrap(), crate::connectors::RemoteQuota { total: Some(1000), used: Some(10), free: Some(400), trashed: None });
    }

    /// Regression: a failure listing the team folders was swallowed, so the
    /// picker showed no team folders instead of saying why.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_team_folder_listing_failure_is_reported() {
        let mock = serve(|r| match r.path.as_str() {
            "/2/users/get_current_account" => json(serde_json::json!({ "email": "m@corp.example", "root_info": { ".tag": "team", "root_namespace_id": "100", "home_namespace_id": "200" } })),
            "/2/sharing/list_folders" => status(500, "boom"),
            _ => status(404, "no"),
        })
        .await;
        let b = &mock.base;
        let c = DropboxConnector::new().with_endpoints(&format!("{b}/2"), &format!("{b}/c"), &format!("{b}/token"));
        let p = crate::connectors::live_tests::profile(ConnectionProtocol::Dropbox, "", None, "", "/");
        c.connect_with_secret(&p, Some("tok".into())).await.unwrap();
        let err = c.namespaces().await.expect_err("the failure must reach the picker");
        assert!(err.user_message().contains("team folders"), "{}", err.user_message());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn share_link_body_and_replies() {
        use crate::connectors::{ShareOptions, ShareScope};
        let now = chrono::DateTime::parse_from_rfc3339("2026-10-22T10:00:00Z").unwrap().with_timezone(&chrono::Utc);
        let b = super::dropbox_link_body("/a.pdf", &ShareOptions { expires_in_secs: Some(60), password: Some("pw".into()), ..Default::default() }, now);
        assert_eq!(b["settings"]["expires"], "2026-10-22T10:01:00Z");
        assert_eq!(b["settings"]["audience"][".tag"], "public");
        assert_eq!((b["settings"]["require_password"].as_bool(), b["settings"]["link_password"].as_str()), (Some(true), Some("pw")));
        let ok = super::parse_dropbox_link(200, r#"{"url":"https://www.dropbox.com/s/x/a.pdf?dl=0","expires":"2026-10-22T10:01:00Z"}"#, ShareScope::Anyone).unwrap();
        assert_eq!(ok.url, "https://www.dropbox.com/s/x/a.pdf?dl=0");
        assert!(ok.expires_at.is_some() && ok.note.is_none());
        let reused = super::parse_dropbox_link(409, r#"{"error_summary":"shared_link_already_exists/metadata/..","error":{".tag":"shared_link_already_exists","shared_link_already_exists":{".tag":"metadata","metadata":{"url":"https://db.tt/old"}}}}"#, ShareScope::Anyone).unwrap();
        assert_eq!(reused.url, "https://db.tt/old");
        assert!(reused.note.is_some());
        let err = super::parse_dropbox_link(409, r#"{"error_summary":"settings_error/not_authorized/.."}"#, ShareScope::Anyone).unwrap_err();
        assert!(err.to_string().contains("settings_error"));
    }

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
