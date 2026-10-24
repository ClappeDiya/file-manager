//! Google Drive Connector (T-027).
//!
//! Features:
//! - OAuth 2.0 flow (opens browser for consent)
//! - My Drive, Shared Drives, Shared with Me browsable
//! - Google Docs/Sheets export options (PDF, DOCX)
//! - File versioning: view history, restore
//! - Auto token refresh

use crate::connectors::oauth::{OAuthClient, OAuthToken};
use crate::connectors::oauth_flow::{self, OAuthProvider};
use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry, GoogleDriveRoot, GoogleDriveSettings};
use std::sync::atomic::{AtomicBool, Ordering};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

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
    api_base: String,
    upload_base: String,
    /// Token endpoint override (tests); `None` = Google's.
    token_url: Option<String>,
    /// Rotated tokens are written back here (the app's credential store).
    credentials: Option<Arc<crate::security::CredentialStore>>,
    connected: AtomicBool,
    /// The signed-in account (e-mail), from `about`.
    account: std::sync::RwLock<Option<String>>,
    /// Which part of Drive "/" opens, export formats.
    settings: std::sync::RwLock<GoogleDriveSettings>,
}

impl GoogleDriveConnector {
    pub fn new() -> Self {
        Self {
            oauth: Arc::new(RwLock::new(None)),
            http: crate::net::client(),
            api_base: DRIVE_API_BASE.to_string(),
            upload_base: DRIVE_UPLOAD_BASE.to_string(),
            token_url: None,
            credentials: None,
            connected: AtomicBool::new(false),
            account: std::sync::RwLock::new(None),
            settings: std::sync::RwLock::new(GoogleDriveSettings::default()),
        }
    }

    /// Persist rotated tokens in `store` (under the connection's credential key).
    pub fn with_credentials(mut self, store: Arc<crate::security::CredentialStore>) -> Self {
        self.credentials = Some(store);
        self
    }

    /// Talk to `api` / `upload` / `token_url` instead of Google (tests, proxies).
    pub fn with_endpoints(mut self, api: &str, upload: &str, token_url: &str) -> Self {
        self.api_base = api.trim_end_matches('/').to_string();
        self.upload_base = upload.trim_end_matches('/').to_string();
        self.token_url = Some(token_url.to_string());
        self
    }

    /// The signed-in account, once connected.
    pub fn account(&self) -> Option<String> {
        self.account.read().ok().and_then(|a| a.clone())
    }

    /// Initialize the OAuth client with the given credentials.
    pub async fn init_oauth(
        &self,
        client_id: String,
        client_secret: Option<String>,
        callback_port: Option<u16>,
    ) {
        let port = callback_port.unwrap_or(DEFAULT_CALLBACK_PORT);
        let provider = OAuthProvider::GoogleDrive;
        let config = provider.oauth_config(client_id, client_secret, format!("http://localhost:{port}/callback"), provider.endpoints(Default::default()));

        let client = OAuthClient::new(config);
        let mut oauth = self.oauth.write().await;
        *oauth = Some(client);
    }

    /// Restore the stored grant, refresh it when expired and check it with
    /// `about` (which also names the account and gives the quota).
    async fn connect_oauth(&self, profile: &ConnectionProfile, secret: Option<String>) -> Result<(), AppError> {
        self.connected.store(false, Ordering::SeqCst);
        let client = oauth_flow::client_for_connect(OAuthProvider::GoogleDrive, profile, secret, self.token_url.as_deref(), self.credentials.clone()).await?;
        *self.oauth.write().await = Some(client);
        if let Ok(mut s) = self.settings.write() {
            *s = profile.extras.google_drive.clone().unwrap_or_default();
        }
        let about = self.about().await?;
        if let Ok(mut a) = self.account.write() {
            *a = about["user"]["emailAddress"].as_str().or(about["user"]["displayName"].as_str()).map(String::from);
        }
        self.connected.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// `GET about` (user and storage quota). A 401 with a token that has not
    /// expired (revoked, rotated elsewhere) refreshes once.
    pub async fn about(&self) -> Result<serde_json::Value, AppError> {
        let url = format!("{}/about?fields=user(displayName,emailAddress),storageQuota(limit,usage,usageInDrive,usageInDriveTrash)", self.api_base);
        for attempt in 0..2 {
            let token = self.get_token().await?;
            let resp = self.http.get(&url).bearer_auth(&token).send().await.map_err(|e| AppError::connection(format!("Google Drive is unreachable: {e}"), "Check your network connection."))?;
            let status = resp.status();
            if status.as_u16() == 401 && attempt == 0 {
                self.force_refresh().await?;
                continue;
            }
            if matches!(status.as_u16(), 401 | 403) {
                return Err(AppError::auth(format!("Google Drive refused the sign-in (HTTP {status})"), "Sign in again from Connections."));
            }
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::connection(format!("Google Drive error (HTTP {status}): {}", body.chars().take(200).collect::<String>()), "Try again later."));
            }
            return resp.json().await.map_err(|e| AppError::connection(format!("Unexpected Google Drive reply: {e}"), "Try again."));
        }
        Err(AppError::auth("Google Drive refused the renewed sign-in", "Sign in again from Connections."))
    }

    /// The grant to store for the current tokens (after a sign-in or refresh).
    pub async fn current_grant(&self) -> Option<crate::connectors::oauth::OAuthGrant> {
        self.oauth.read().await.as_ref()?.grant().await
    }

    async fn force_refresh(&self) -> Result<(), AppError> {
        let oauth = self.oauth.read().await;
        let client = oauth.as_ref().ok_or_else(|| AppError::connection("Not connected to Google Drive", "Connect first."))?;
        client.refresh_token().await.map(|_| ())
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

        let mut url = url::Url::parse(&format!("{}/files", self.api_base)).unwrap();
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

    /// Every entry at a virtual `path` (all pages). `/` opens the part of
    /// Drive chosen on the connection (My Drive, Shared with me or one
    /// shared drive).
    pub async fn list_path(&self, path: &str) -> Result<Vec<FileEntry>, AppError> {
        let loc = match drive_location(path) {
            DriveLocation::Root => {
                let s = self.settings.read().map(|s| s.clone()).unwrap_or_default();
                match (s.root, s.shared_drive_id.filter(|d| !d.is_empty())) {
                    (GoogleDriveRoot::SharedDrive, Some(d)) => DriveLocation::SharedDrive { drive_id: d, folder: None },
                    (GoogleDriveRoot::SharedWithMe, _) => DriveLocation::SharedWithMe,
                    _ => DriveLocation::MyDrive { folder: None },
                }
            }
            other => other,
        };
        match loc {
            DriveLocation::SharedDrives => self.all_shared_drives().await.map(|d| d.into_iter().map(|d| shared_drive_entry(&d.id, d.name)).collect()),
            DriveLocation::MyDrive { folder } => {
                let parent = folder.unwrap_or_else(|| "root".into());
                // Top level of My Drive lists as "/<id>"; deeper paths keep their prefix.
                let prefix = if parent == "root" { String::new() } else { path.trim_end_matches('/').to_string() };
                self.list_query(&format!("'{}' in parents and trashed = false", q_escape(&parent)), None, &prefix).await
            }
            DriveLocation::SharedWithMe => {
                let prefix = if drive_location(path) == DriveLocation::Root { "/shared-with-me".to_string() } else { path.trim_end_matches('/').to_string() };
                self.list_query("sharedWithMe = true and trashed = false", None, &prefix).await
            }
            DriveLocation::SharedDrive { drive_id, folder } => {
                let parent = folder.unwrap_or_else(|| drive_id.clone());
                let prefix = if drive_location(path) == DriveLocation::Root { format!("/shared-drives/{drive_id}") } else { path.trim_end_matches('/').to_string() };
                self.list_query(&format!("'{}' in parents and trashed = false", q_escape(&parent)), Some(&drive_id), &prefix).await
            }
            DriveLocation::Root => unreachable!("resolved above"),
        }
    }

    /// `files.list` for `query`, following every page.
    async fn list_query(&self, query: &str, drive_id: Option<&str>, prefix: &str) -> Result<Vec<FileEntry>, AppError> {
        let fields = "files(id,name,mimeType,size,modifiedTime,createdTime,parents,shared,trashed),nextPageToken";
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        for _ in 0..10_000 {
            let mut url = url::Url::parse(&format!("{}/files", self.api_base)).map_err(|e| AppError::internal(format!("Bad Drive API URL: {e}")))?;
            {
                let mut q = url.query_pairs_mut();
                q.append_pair("q", query).append_pair("fields", fields).append_pair("pageSize", "1000").append_pair("orderBy", "folder,name");
                q.append_pair("supportsAllDrives", "true").append_pair("includeItemsFromAllDrives", "true");
                match drive_id {
                    Some(d) => q.append_pair("corpora", "drive").append_pair("driveId", d),
                    None => q.append_pair("corpora", "user"),
                };
                if let Some(p) = &page {
                    q.append_pair("pageToken", p);
                }
            }
            let resp = self.http.get(url.as_str()).bearer_auth(self.get_token().await?).send().await.map_err(|e| AppError::Connection {
                message: format!("Google Drive API request failed: {e}"),
                advice: "Check your network connection.".to_string(),
            })?;
            let list: FileListResponse = drive_json(resp, "Listing").await?;
            out.extend(list.files.into_iter().map(|f| entry_at(f, prefix)));
            page = list.next_page_token;
            if page.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// Every shared drive the account can see (all pages).
    pub async fn all_shared_drives(&self) -> Result<Vec<SharedDriveInfo>, AppError> {
        let mut out = Vec::new();
        let mut page: Option<String> = None;
        for _ in 0..1_000 {
            let mut url = url::Url::parse(&format!("{}/drives", self.api_base)).map_err(|e| AppError::internal(format!("Bad Drive API URL: {e}")))?;
            url.query_pairs_mut().append_pair("pageSize", "100");
            if let Some(p) = &page {
                url.query_pairs_mut().append_pair("pageToken", p);
            }
            let resp = self.http.get(url.as_str()).bearer_auth(self.get_token().await?).send().await.map_err(|e| AppError::connection(format!("Drive API request failed: {e}"), "Check your network connection."))?;
            let list: DriveListResponse = drive_json(resp, "Listing shared drives").await?;
            out.extend(list.drives.into_iter().map(|d| SharedDriveInfo { id: d.id, name: d.name }));
            page = list.next_page_token;
            if page.is_none() {
                break;
            }
        }
        Ok(out)
    }

    /// List Shared Drives.
    async fn list_shared_drives(
        &self,
        page_token: Option<&str>,
    ) -> Result<(Vec<FileEntry>, Option<String>), AppError> {
        let token = self.get_token().await?;

        let mut url = url::Url::parse(&format!("{}/drives", self.api_base)).unwrap();
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
            "{}/files?uploadType=multipart&supportsAllDrives=true", self.upload_base
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
            "{}/files?uploadType=resumable&supportsAllDrives=true", self.upload_base
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
            "{}/files/{file_id}?alt=media&supportsAllDrives=true", self.api_base
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
            "{}/files/{file_id}/export?mimeType={}", self.api_base,
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
            "{}/files/{file_id}/revisions?fields=revisions(id,modifiedTime,originalFilename,size,keepForever)", self.api_base
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
            "{}/files/{file_id}/revisions/{revision_id}?alt=media", self.api_base
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
                "{}/files/{file_id}?supportsAllDrives=true", self.api_base
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
                "{}/files/{file_id}?supportsAllDrives=true", self.api_base
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

        let url = format!("{}/files?supportsAllDrives=true", self.api_base);

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
            "{}/files/{file_id}?supportsAllDrives=true", self.api_base
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
            "{}/files/{file_id}?addParents={new_parent_id}&supportsAllDrives=true", self.api_base
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
            Ok(())
        })
    }

    fn list_remote(
        &self,
        path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<FileEntry>, AppError>> + Send + '_>> {
        let path = path.to_string();
        Box::pin(async move { self.list_path(&path).await })
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn create_share_link(
        &self,
        remote_path: &str,
        options: &super::ShareOptions,
    ) -> Pin<Box<dyn Future<Output = Result<super::ShareLink, AppError>> + Send + '_>> {
        let (r, o) = (remote_path.to_string(), options.clone());
        Box::pin(async move {
            let permission = drive_permission_body(&o)?;
            let file_id = r.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).ok_or_else(|| AppError::validation("Choose a file or folder to share"))?.to_string();
            let token = self.get_token().await?;
            let resp = self
                .http
                .post(format!("{}/files/{file_id}/permissions?supportsAllDrives=true", self.api_base))
                .bearer_auth(&token)
                .json(&permission)
                .send()
                .await
                .map_err(|e| AppError::connection(format!("Google Drive share failed: {e}"), "Check your network connection."))?;
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                return Err(AppError::Connection {
                    message: format!("Google Drive refused the share (HTTP {status}): {}", body.chars().take(200).collect::<String>()),
                    advice: "Your organization may block sharing outside it.".into(),
                });
            }
            let meta: serde_json::Value = self
                .http
                .get(format!("{}/files/{file_id}?fields=webViewLink&supportsAllDrives=true", self.api_base))
                .bearer_auth(&token)
                .send()
                .await
                .map_err(|e| AppError::connection(format!("Google Drive link lookup failed: {e}"), "Try again."))?
                .json()
                .await
                .map_err(|e| AppError::connection(format!("Unexpected Google Drive reply: {e}"), "Try again."))?;
            let url = meta["webViewLink"].as_str().ok_or_else(|| AppError::connection("Google Drive returned no link", "Try again."))?;
            Ok(super::ShareLink {
                url: url.into(),
                expires_at: None,
                scope: super::ShareScope::Anyone,
                provider: "google_drive".into(),
                note: Some("Anyone with the link can open it until you stop sharing it in Google Drive.".into()),
            })
        })
    }
}

/// Permission for an anyone-with-the-link share. Drive can't put an
/// expiry or password on these, and domain links need the domain name, so
/// those options are refused rather than silently ignored.
pub(crate) fn drive_permission_body(o: &super::ShareOptions) -> Result<serde_json::Value, AppError> {
    if o.expires_in_secs.is_some() {
        return Err(AppError::validation("Google Drive links can't expire; leave the expiry empty or share another way"));
    }
    if o.password.as_deref().is_some_and(|p| !p.is_empty()) {
        return Err(super::share_unsupported("set a password"));
    }
    if o.scope != super::ShareScope::Anyone {
        return Err(super::share_unsupported("limit links to your organization"));
    }
    Ok(serde_json::json!({ "role": if o.allow_edit { "writer" } else { "reader" }, "type": "anyone" }))
}

/// Convert a Drive API file object to our FileEntry.
fn drive_file_to_entry(file: DriveFile, parent_path: &str) -> FileEntry {
    entry_at(file, &format!("/{parent_path}"))
}

/// The entry for `file` listed at virtual path `prefix` (its path is
/// `{prefix}/{id}`, so the folder chain and shared drive are kept).
fn entry_at(file: DriveFile, prefix: &str) -> FileEntry {
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
        path: format!("{}/{}", prefix.trim_end_matches('/'), file.id),
        name: file.name,
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

/// A shared drive (for the connection's drive picker).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedDriveInfo {
    pub id: String,
    pub name: String,
}

fn shared_drive_entry(id: &str, name: String) -> FileEntry {
    FileEntry {
        name,
        path: format!("/shared-drives/{id}"),
        is_dir: true,
        is_symlink: false,
        size: 0,
        modified: None,
        created: None,
        is_hidden: false,
        extension: None,
        permissions: None,
    }
}

/// Where a virtual Drive path points.
#[derive(Debug, Clone, PartialEq, Eq)]
enum DriveLocation {
    /// `/`: what the connection is set to open.
    Root,
    MyDrive { folder: Option<String> },
    SharedWithMe,
    SharedDrives,
    SharedDrive { drive_id: String, folder: Option<String> },
}

/// Paths are `/<folder id>/<folder id>…` (the last segment is the folder),
/// optionally under `/my-drive`, `/shared-with-me` or
/// `/shared-drives/<drive id>`.
fn drive_location(path: &str) -> DriveLocation {
    let segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    let last = |from: usize| segs.get(from..).and_then(|r| r.last()).map(|s| s.to_string());
    match segs.as_slice() {
        [] => DriveLocation::Root,
        ["my-drive", ..] => DriveLocation::MyDrive { folder: last(1).filter(|s| s != "root") },
        ["shared-with-me"] => DriveLocation::SharedWithMe,
        ["shared-with-me", ..] => DriveLocation::MyDrive { folder: last(1) },
        ["shared-drives"] => DriveLocation::SharedDrives,
        ["shared-drives", d, ..] => DriveLocation::SharedDrive { drive_id: d.to_string(), folder: last(2) },
        _ => DriveLocation::MyDrive { folder: last(0).filter(|s| s != "root") },
    }
}

/// Escape a value inside a `files.list` query string literal.
fn q_escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\'', "\\'")
}

/// The JSON body of a successful Drive reply; 401/403 are rejected sign-ins.
async fn drive_json<T: serde::de::DeserializeOwned>(resp: reqwest::Response, what: &str) -> Result<T, AppError> {
    let status = resp.status();
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        let excerpt: String = body.chars().take(300).collect();
        return Err(match status.as_u16() {
            401 => AppError::auth(format!("{what}: Google Drive refused the sign-in (HTTP {status})"), "Sign in again from Connections."),
            _ => AppError::Connection { message: format!("Google Drive API error (HTTP {status}): {excerpt}"), advice: "Check your permissions or re-authorize.".to_string() },
        });
    }
    resp.json().await.map_err(|e| AppError::Connection { message: format!("Failed to parse Drive response: {e}"), advice: "Try again.".to_string() })
}

/// Parse a virtual path into drive scope and parent folder ID.
#[cfg(test)]
fn parse_drive_path(path: &str) -> (DriveScope, Option<String>) {
    match drive_location(path) {
        DriveLocation::Root => (DriveScope::MyDrive, None),
        DriveLocation::MyDrive { folder } => (DriveScope::MyDrive, folder),
        DriveLocation::SharedWithMe => (DriveScope::SharedWithMe, None),
        DriveLocation::SharedDrives => (DriveScope::SharedDrives, None),
        DriveLocation::SharedDrive { drive_id, folder } => (DriveScope::MyDrive, Some(folder.unwrap_or(drive_id))),
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn drive_permissions() {
        use crate::connectors::ShareOptions;
        assert_eq!(super::drive_permission_body(&ShareOptions::default()).unwrap(), serde_json::json!({"role": "reader", "type": "anyone"}));
        assert_eq!(super::drive_permission_body(&ShareOptions { allow_edit: true, ..Default::default() }).unwrap()["role"], "writer");
        assert!(super::drive_permission_body(&ShareOptions { expires_in_secs: Some(60), ..Default::default() }).is_err());
    }

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
