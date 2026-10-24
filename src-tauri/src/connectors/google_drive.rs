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
const GOOGLE_APPS_MIME: &str = "application/vnd.google-apps.";
/// Smaller files go up in one multipart request; larger ones are streamed.
const SIMPLE_UPLOAD_LIMIT: u64 = 5 * 1024 * 1024;

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
    /// `pdf`, `docx`… (the extension).
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.trim().to_ascii_lowercase().as_str() {
            "pdf" => Self::Pdf,
            "docx" => Self::Docx,
            "xlsx" => Self::Xlsx,
            "pptx" => Self::Pptx,
            "csv" => Self::Csv,
            "txt" => Self::Txt,
            "html" => Self::Html,
            "epub" => Self::Epub,
            _ => return None,
        })
    }

    /// Formats Google can export a native file of `mime_type` to, the
    /// default first; `None` for files that are not Google Docs editors files.
    pub fn choices(mime_type: &str) -> Option<&'static [GoogleDocExportFormat]> {
        use GoogleDocExportFormat::*;
        Some(match mime_type {
            "application/vnd.google-apps.document" => &[Docx, Pdf, Txt, Html, Epub],
            "application/vnd.google-apps.spreadsheet" => &[Xlsx, Pdf, Csv, Html],
            "application/vnd.google-apps.presentation" => &[Pptx, Pdf, Txt],
            "application/vnd.google-apps.drawing" => &[Pdf],
            _ => return None,
        })
    }

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
    /// The saved connection the current sign-in belongs to.
    profile_id: std::sync::RwLock<Option<uuid::Uuid>>,
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
            profile_id: std::sync::RwLock::new(None),
            account: std::sync::RwLock::new(None),
            settings: std::sync::RwLock::new(GoogleDriveSettings::default()),
        }
    }

    /// A new, disconnected instance with this one's endpoints and credential
    /// store (an action for another account runs on it).
    pub fn fresh(&self) -> Self {
        Self {
            http: self.http.clone(),
            api_base: self.api_base.clone(),
            upload_base: self.upload_base.clone(),
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
        self.set_profile_id(None);
        let client = oauth_flow::client_for_connect(OAuthProvider::GoogleDrive, profile, secret, self.token_url.as_deref(), self.credentials.clone()).await?;
        *self.oauth.write().await = Some(client);
        if let Ok(mut s) = self.settings.write() {
            *s = profile.extras.google_drive.clone().unwrap_or_default();
        }
        let about = self.about().await?;
        if let Ok(mut a) = self.account.write() {
            *a = about["user"]["emailAddress"].as_str().or(about["user"]["displayName"].as_str()).map(String::from);
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
            if status.as_u16() == 401 {
                return Err(AppError::auth(format!("Google Drive refused the sign-in (HTTP {status})"), "Sign in again from Connections."));
            }
            if status.as_u16() == 403 {
                return Err(about_forbidden(&resp.text().await.unwrap_or_default()));
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

    /// The folder ID new items under virtual folder `dir` go into.
    fn folder_for(&self, dir: &str) -> Result<String, AppError> {
        let settings = self.settings.read().map(|s| s.clone()).unwrap_or_default();
        match drive_location(dir) {
            DriveLocation::Root => match (settings.root, settings.shared_drive_id.filter(|d| !d.is_empty())) {
                (GoogleDriveRoot::SharedDrive, Some(d)) => Ok(d),
                (GoogleDriveRoot::SharedWithMe, _) => Err(AppError::validation("Open a folder inside Shared with me to add files there")),
                _ => Ok("root".into()),
            },
            DriveLocation::MyDrive { folder } => Ok(folder.unwrap_or_else(|| "root".into())),
            DriveLocation::SharedDrive { drive_id, folder } => Ok(folder.unwrap_or(drive_id)),
            DriveLocation::SharedWithMe | DriveLocation::SharedDrives => Err(AppError::validation("Open a folder to add files there")),
        }
    }

    /// The export format for a native Google file (`None` = not exportable).
    fn export_format_for(&self, mime_type: &str) -> Option<GoogleDocExportFormat> {
        let choices = GoogleDocExportFormat::choices(mime_type)?;
        let f = self.settings.read().map(|s| s.export_formats.clone()).unwrap_or_default();
        let wanted = match mime_type {
            "application/vnd.google-apps.document" => f.document,
            "application/vnd.google-apps.spreadsheet" => f.spreadsheet,
            "application/vnd.google-apps.presentation" => f.presentation,
            _ => f.drawing,
        };
        Some(wanted.as_deref().and_then(GoogleDocExportFormat::parse).filter(|w| choices.contains(w)).unwrap_or(choices[0]))
    }

    /// Every entry at a virtual `path` (all pages). `/` opens the part of
    /// Drive chosen on the connection (My Drive, Shared with me or one
    /// shared drive).
    pub async fn list_path(&self, path: &str) -> Result<Vec<FileEntry>, AppError> {
        let loc = self.resolved_location(path);
        let prefix = listing_prefix(path, &loc);
        match loc {
            DriveLocation::SharedDrives => self.all_shared_drives().await.map(|d| d.into_iter().map(|d| shared_drive_entry(&d.id, d.name)).collect()),
            DriveLocation::MyDrive { folder } => {
                let parent = folder.unwrap_or_else(|| "root".into());
                self.list_query(&format!("'{}' in parents and trashed = false", q_escape(&parent)), None, &prefix).await
            }
            DriveLocation::SharedWithMe => self.list_query("sharedWithMe = true and trashed = false", None, &prefix).await,
            DriveLocation::SharedDrive { drive_id, folder } => {
                let parent = folder.unwrap_or_else(|| drive_id.clone());
                self.list_query(&format!("'{}' in parents and trashed = false", q_escape(&parent)), Some(&drive_id), &prefix).await
            }
            DriveLocation::Root => unreachable!("resolved above"),
        }
    }

    /// Where virtual `path` points, with `/` resolved to the part of Drive
    /// chosen on the connection.
    fn resolved_location(&self, path: &str) -> DriveLocation {
        match drive_location(path) {
            DriveLocation::Root => {
                let s = self.settings.read().map(|s| s.clone()).unwrap_or_default();
                match (s.root, s.shared_drive_id.filter(|d| !d.is_empty())) {
                    (GoogleDriveRoot::SharedDrive, Some(d)) => DriveLocation::SharedDrive { drive_id: d, folder: None },
                    (GoogleDriveRoot::SharedWithMe, _) => DriveLocation::SharedWithMe,
                    _ => DriveLocation::MyDrive { folder: None },
                }
            }
            other => other,
        }
    }

    /// The virtual path of item `id` directly in folder `dir`, as
    /// [`Self::list_path`] lists it.
    fn child_path(&self, dir: &str, id: &str) -> String {
        format!("{}/{id}", listing_prefix(dir, &self.resolved_location(dir)))
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

    /// The shared drive new items under virtual folder `dir` belong to.
    fn drive_id_for(&self, dir: &str) -> Option<String> {
        match drive_location(dir) {
            DriveLocation::Root => {
                let s = self.settings.read().map(|s| s.clone()).unwrap_or_default();
                s.shared_drive_id.filter(|d| !d.is_empty() && s.root == GoogleDriveRoot::SharedDrive)
            }
            DriveLocation::SharedDrive { drive_id, .. } => Some(drive_id),
            _ => None,
        }
    }

    /// The ID of a (non-folder) file called `name` directly in `parent`.
    async fn find_file(&self, parent: &str, name: &str, drive_id: Option<&str>) -> Result<Option<String>, AppError> {
        // Not folders, nor Google Docs (their content cannot be replaced by a file).
        let q = format!("name = '{}' and '{}' in parents and trashed = false and not mimeType contains '{GOOGLE_APPS_MIME}'", q_escape(name), q_escape(parent));
        let found = self.list_query(&q, drive_id, "").await?;
        Ok(found.into_iter().find(|e| e.name == name && !e.is_dir).map(|e| e.path.trim_start_matches('/').to_string()))
    }

    /// Resumable upload streamed from `local` (`size` bytes): `POST` creates
    /// a file with `metadata`, `PATCH` on `…/files/{id}` replaces its content.
    async fn upload_streamed(&self, method: reqwest::Method, init_url: &str, metadata: serde_json::Value, local: &str, size: u64) -> Result<String, AppError> {
        let unreachable = |e: reqwest::Error| AppError::transfer(format!("Google Drive upload failed: {e}"), "Check your network connection.");
        let resp = self
            .http
            .request(method, init_url)
            .bearer_auth(self.get_token().await?)
            .header("Content-Type", "application/json; charset=UTF-8")
            .header("X-Upload-Content-Length", size.to_string())
            .body(metadata.to_string())
            .send()
            .await
            .map_err(unreachable)?;
        if !resp.status().is_success() {
            let status = resp.status();
            let body: String = resp.text().await.unwrap_or_default().chars().take(300).collect();
            let message = format!("Google Drive upload failed (HTTP {status}): {body}");
            return Err(if status.as_u16() == 401 { AppError::auth(message, "Sign in again from Connections.") } else { AppError::transfer(message, "Check your storage quota and permissions.") });
        }
        let session = resp
            .headers()
            .get("location")
            .and_then(|v| v.to_str().ok())
            .map(String::from)
            .ok_or_else(|| AppError::transfer("Google Drive gave no upload address", "Try again."))?;
        let file = tokio::fs::File::open(local).await.map_err(|e| AppError::file_op(format!("Cannot read {local}: {e}"), "Check the file."))?;
        let resp = self
            .http
            .put(&session)
            .header(reqwest::header::CONTENT_LENGTH, size)
            .body(reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::new(file)))
            .send()
            .await
            .map_err(unreachable)?;
        let done: DriveFile = drive_json(resp, "Uploading").await?;
        tracing::info!("Uploaded to Google Drive: {} (ID: {})", local, done.id);
        Ok(done.id)
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
        if (file_size as u64) < SIMPLE_UPLOAD_LIMIT {
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
    /// Paths are folder-id chains ending in the item's id; the last segment
    /// of `to` is the new name. A different parent folder moves it first.
    fn rename_entry(&self, from: &str, to: &str) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let (from, to) = (from.to_string(), to.to_string());
        Box::pin(async move {
            let id = last_segment(&from)?;
            let (from_dir, _) = split_parent(&from)?;
            let (to_dir, name) = split_parent(&to)?;
            if from_dir != to_dir {
                let parent = self.folder_for(&to_dir)?;
                let old = self.folder_for(&from_dir).ok();
                self.move_file(&id, &parent, old.as_deref()).await?;
            }
            self.rename_file(&id, &name).await
        })
    }

    /// The item keeps its id: it is listed under `to`'s folder by that id.
    fn renamed_path(&self, from: &str, to: &str) -> String {
        match (last_segment(from), split_parent(to)) {
            (Ok(id), Ok((dir, _))) => self.child_path(&dir, &id),
            _ => to.to_string(),
        }
    }

    /// A native Google document downloads as an export: the connection's
    /// format's extension is added to its name.
    fn download_name(&self, entry: &FileEntry) -> String {
        let mime = match entry.extension.as_deref() {
            Some("gdoc") => "application/vnd.google-apps.document",
            Some("gsheet") => "application/vnd.google-apps.spreadsheet",
            Some("gslides") => "application/vnd.google-apps.presentation",
            Some("gdraw") => "application/vnd.google-apps.drawing",
            _ => return entry.name.clone(),
        };
        match self.export_format_for(mime) {
            Some(f) if !entry.name.to_ascii_lowercase().ends_with(&format!(".{}", f.extension())) => format!("{}.{}", entry.name, f.extension()),
            _ => entry.name.clone(),
        }
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
        Box::pin(async move { self.list_path(&path).await })
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
            let (dir, name) = split_parent(&r)?;
            let parent = self.folder_for(&dir)?;
            let size = tokio::fs::metadata(&l).await.map_err(|e| AppError::file_op(format!("Cannot read {l}: {e}"), "Check the file."))?.len();
            // Drive allows several files with one name: replace the one
            // already there (as Dropbox and OneDrive do) instead of adding
            // a duplicate on every re-run.
            match self.find_file(&parent, &name, self.drive_id_for(&dir).as_deref()).await? {
                Some(id) => {
                    let url = format!("{}/files/{}?uploadType=resumable&supportsAllDrives=true", self.upload_base, percent_encoding::utf8_percent_encode(&id, percent_encoding::NON_ALPHANUMERIC));
                    self.upload_streamed(reqwest::Method::PATCH, &url, serde_json::json!({}), &l, size).await?;
                }
                None if size < SIMPLE_UPLOAD_LIMIT => {
                    let data = tokio::fs::read(&l).await.map_err(|e| AppError::file_op(format!("Cannot read {l}: {e}"), "Check the file."))?;
                    self.upload_file(&name, Some(&parent), data, None).await?;
                }
                None => {
                    let url = format!("{}/files?uploadType=resumable&supportsAllDrives=true", self.upload_base);
                    self.upload_streamed(reqwest::Method::POST, &url, serde_json::json!({ "name": name, "parents": [parent] }), &l, size).await?;
                }
            }
            Ok(size)
        })
    }

    /// Downloads the file; Google Docs, Sheets, Slides and Drawings are
    /// exported in the connection's format (docx/xlsx/pptx/pdf by default),
    /// and the local name gets that extension.
    fn download(&self, remote_path: &str, local_path: &str) -> Pin<Box<dyn Future<Output = Result<u64, AppError>> + Send + '_>> {
        let (r, l) = (remote_path.to_string(), local_path.to_string());
        Box::pin(async move {
            let id = last_segment(&r)?;
            let meta: DriveFile = drive_json(
                self.http.get(format!("{}/files/{id}?fields=id,name,mimeType&supportsAllDrives=true", self.api_base)).bearer_auth(self.get_token().await?).send().await.map_err(|e| AppError::transfer(format!("Google Drive is unreachable: {e}"), "Check your network connection."))?,
                "Reading the file",
            )
            .await?;
            // A native Google file is exported (the connection's format);
            // it is written to `local_path` as given: the caller picks the
            // name (the panel adds the format's extension).
            let data = match self.export_format_for(&meta.mime_type) {
                Some(format) => self.export_file(&id, format).await?,
                None if meta.mime_type.starts_with("application/vnd.google-apps.") => {
                    return Err(AppError::validation(format!("'{}' is a Google {} and cannot be downloaded", meta.name, meta.mime_type.trim_start_matches("application/vnd.google-apps."))));
                }
                None => self.download_file(&id).await?,
            };
            tokio::fs::write(&l, &data).await.map_err(|e| AppError::file_op(format!("Cannot write {l}: {e}"), "Check the destination folder."))?;
            Ok(data.len() as u64)
        })
    }

    fn make_directory(&self, remote_path: &str) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let r = remote_path.to_string();
        Box::pin(async move { self.make_directory_path(&r).await.map(|_| ()) })
    }

    /// `remote_path` is the parent folder's path plus the new name; the
    /// folder is listed (and its contents go) under its new id.
    fn make_directory_path(&self, remote_path: &str) -> Pin<Box<dyn Future<Output = Result<String, AppError>> + Send + '_>> {
        let r = remote_path.to_string();
        Box::pin(async move {
            let (dir, name) = split_parent(&r)?;
            let parent = self.folder_for(&dir)?;
            let id = self.create_folder(&name, Some(&parent)).await?;
            Ok(self.child_path(&dir, &id))
        })
    }

    /// Moves the file or folder to the Drive trash (recoverable).
    fn remove(&self, remote_path: &str, _is_dir: bool) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let r = remote_path.to_string();
        Box::pin(async move { self.delete_file(&last_segment(&r)?, false).await })
    }

    /// `about.storageQuota`; no `limit` means unlimited.
    fn quota(&self) -> Pin<Box<dyn Future<Output = Result<Option<super::RemoteQuota>, AppError>> + Send + '_>> {
        Box::pin(async move { Ok(Some(drive_quota(&self.about().await?["storageQuota"]))) })
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
            "application/vnd.google-apps.drawing" => Some("gdraw".to_string()),
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

/// `storageQuota` → quota (`limit` absent = unlimited).
fn drive_quota(q: &serde_json::Value) -> super::RemoteQuota {
    let (total, used) = (super::json_u64(&q["limit"]), super::json_u64(&q["usage"]));
    super::RemoteQuota { total, used, free: total.zip(used).map(|(t, u)| t.saturating_sub(u)), trashed: super::json_u64(&q["usageInDriveTrash"]) }
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

/// What [`GoogleDriveConnector::list_path`] puts before each listed id in
/// `path` (`resolved` is where it points): nothing at the top of My Drive,
/// the scope at "/" of Shared with me or a shared drive, else `path`.
fn listing_prefix(path: &str, resolved: &DriveLocation) -> String {
    let at_root = drive_location(path) == DriveLocation::Root;
    match resolved {
        DriveLocation::MyDrive { folder: None } => String::new(),
        DriveLocation::SharedWithMe if at_root => "/shared-with-me".to_string(),
        DriveLocation::SharedDrive { drive_id, .. } if at_root => format!("/shared-drives/{drive_id}"),
        _ => path.trim_end_matches('/').to_string(),
    }
}

/// The last segment of a virtual path (the file or folder ID).
fn last_segment(path: &str) -> Result<String, AppError> {
    path.trim_end_matches('/').rsplit('/').next().filter(|s| !s.is_empty()).map(String::from).ok_or_else(|| AppError::validation("Choose a file or folder"))
}

/// `"/a/b/name"` → (`"/a/b"`, `"name"`).
fn split_parent(path: &str) -> Result<(String, String), AppError> {
    let p = path.trim_end_matches('/');
    let (dir, name) = p.rsplit_once('/').unwrap_or(("", p));
    if name.is_empty() {
        return Err(AppError::validation("Enter a name"));
    }
    Ok((if dir.is_empty() { "/".into() } else { dir.to_string() }, name.to_string()))
}

/// Escape a value inside a `files.list` query string literal.
fn q_escape(v: &str) -> String {
    v.replace('\\', "\\\\").replace('\'', "\\'")
}

/// A 403 from `about` (every connect). Google also answers 403 for rate
/// limits, quotas and an API not enabled for the app, so only authorization
/// reasons are a rejected sign-in; limits are retried later.
fn about_forbidden(body: &str) -> AppError {
    use crate::connectors::google_errors::{classify_403, Forbidden};
    let (kind, reason) = classify_403(body);
    let why = reason.as_deref().unwrap_or("no reason given");
    match kind {
        Forbidden::Throttled => AppError::connection(format!("Google Drive is limiting requests (HTTP 403: {why})"), "UFOP tries again later; wait a few minutes."),
        Forbidden::ApiDisabled => AppError::Configuration {
            message: format!("The Google Drive API is not enabled for this OAuth app (HTTP 403: {why})"),
            advice: "Enable the Google Drive API in the Google Cloud project of your OAuth client, or sign in with the built-in app.".to_string(),
        },
        Forbidden::Authorization => AppError::auth(format!("Google Drive refused the sign-in (HTTP 403: {why})"), "Sign in again from Connections."),
        Forbidden::Unknown => AppError::connection(format!("Google Drive refused the request (HTTP 403: {why})"), "Try again later; if it keeps failing, sign in again from Connections."),
    }
}

/// The JSON body of a successful Drive reply; a 401 is a rejected sign-in
/// (a 403 is about the item or a rate limit, not the sign-in).
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
    fn locations_keep_folders_and_shared_drives() {
        assert_eq!(drive_location("/"), DriveLocation::Root);
        assert_eq!(drive_location("/root/f1/f2"), DriveLocation::MyDrive { folder: Some("f2".into()) });
        assert_eq!(drive_location("/shared-drives/D/f1"), DriveLocation::SharedDrive { drive_id: "D".into(), folder: Some("f1".into()) });
        assert_eq!(drive_location("/shared-drives/D"), DriveLocation::SharedDrive { drive_id: "D".into(), folder: None });
        assert_eq!(drive_location("/shared-with-me/x"), DriveLocation::MyDrive { folder: Some("x".into()) });
        assert_eq!(split_parent("/f1/a b.txt").unwrap(), ("/f1".to_string(), "a b.txt".to_string()));
        assert_eq!(split_parent("/a.txt").unwrap(), ("/".to_string(), "a.txt".to_string()));
        assert_eq!(q_escape("it's"), "it\\'s");
    }

    /// Regression: uploading a name that already exists in the folder made
    /// a duplicate (Drive allows equal names; an automation re-run piled
    /// them up), where Dropbox and OneDrive replace the file. The existing
    /// file's content is now replaced, streamed from disk.
    #[tokio::test(flavor = "multi_thread")]
    async fn uploading_an_existing_name_replaces_that_file() {
        use crate::connectors::mock_http::{json, serve, status};
        use axum::response::IntoResponse;
        let exists = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let e2 = exists.clone();
        let mock = serve(move |r| {
            let base = format!("http://{}", r.header("host").unwrap_or_default());
            let file = |id: &str| json(serde_json::json!({ "id": id, "name": "it's.txt", "mimeType": "text/plain" }));
            match r.path.as_str() {
                "/api/about" => json(serde_json::json!({ "user": { "emailAddress": "t@example.com" } })),
                "/api/files" if e2.load(std::sync::atomic::Ordering::SeqCst) => json(serde_json::json!({ "files": [{ "id": "old", "name": "it's.txt", "mimeType": "text/plain" }] })),
                "/api/files" => json(serde_json::json!({ "files": [] })),
                "/upload/files" if r.query_param("uploadType").as_deref() == Some("resumable") => ([("location", format!("{base}/session/big"))], "").into_response(),
                "/upload/files" => file("new"),
                "/upload/files/old" => ([("location", format!("{base}/session/old"))], "").into_response(),
                "/session/old" => file("old"),
                "/session/big" => file("big"),
                _ => status(404, "no"),
            }
        })
        .await;
        let b = &mock.base;
        let c = GoogleDriveConnector::new().with_endpoints(&format!("{b}/api"), &format!("{b}/upload"), &format!("{b}/token"));
        let p = crate::connectors::live_tests::profile(crate::core::types::ConnectionProtocol::GoogleDrive, "", None, "", "/");
        c.connect_with_secret(&p, Some(r#"{"access_token":"t"}"#.into())).await.unwrap();
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("it's.txt");
        std::fs::write(&f, "v1").unwrap();
        // New name: created.
        assert_eq!(c.upload(f.to_str().unwrap(), "/it's.txt").await.unwrap(), 2);
        assert_eq!(mock.requests("/upload/files").len(), 1);
        let lookup = &mock.requests("/api/files")[0];
        assert_eq!(lookup.query_param("q").as_deref(), Some("name = 'it\\'s.txt' and 'root' in parents and trashed = false and not mimeType contains 'application/vnd.google-apps.'"));
        // Same name again: that file's content is replaced, no second file.
        exists.store(true, std::sync::atomic::Ordering::SeqCst);
        std::fs::write(&f, "version 2").unwrap();
        assert_eq!(c.upload(f.to_str().unwrap(), "/it's.txt").await.unwrap(), 9);
        assert_eq!(mock.requests("/upload/files").len(), 1, "no duplicate is created");
        let init = &mock.requests("/upload/files/old")[0];
        assert_eq!((init.method.as_str(), init.query_param("uploadType").as_deref()), ("PATCH", Some("resumable")));
        let put = &mock.requests("/session/old")[0];
        assert_eq!((put.method.as_str(), &put.body[..]), ("PUT", &b"version 2"[..]));
        // A large new file is streamed through a resumable session.
        exists.store(false, std::sync::atomic::Ordering::SeqCst);
        let big = dir.path().join("big.bin");
        std::fs::write(&big, vec![7u8; SIMPLE_UPLOAD_LIMIT as usize + 1]).unwrap();
        assert_eq!(c.upload(big.to_str().unwrap(), "/big.bin").await.unwrap(), SIMPLE_UPLOAD_LIMIT + 1);
        let init = mock.requests("/upload/files").into_iter().find(|r| r.query_param("uploadType").as_deref() == Some("resumable")).unwrap();
        assert_eq!((init.method.as_str(), init.json()["name"].as_str(), init.header("x-upload-content-length")), ("POST", Some("big.bin"), Some((SIMPLE_UPLOAD_LIMIT + 1).to_string())));
        assert_eq!(mock.requests("/session/big")[0].body.len() as u64, SIMPLE_UPLOAD_LIMIT + 1);
    }

    /// Regression: every 403 from `about` (run on each connect) was a
    /// rejected sign-in, so a rate-limited background reconnect flagged the
    /// connection and paused its automation. Only authorization reasons are.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_rate_limited_connect_is_not_a_rejected_sign_in() {
        use crate::connectors::mock_http::{serve, status};
        let reason = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let r2 = reason.clone();
        let mock = serve(move |_| {
            let reason = r2.lock().unwrap().clone();
            status(403, &serde_json::json!({ "error": { "code": 403, "message": "m", "errors": [{ "domain": "usageLimits", "reason": reason, "message": "m" }] } }).to_string())
        })
        .await;
        let b = &mock.base;
        let p = crate::connectors::live_tests::profile(crate::core::types::ConnectionProtocol::GoogleDrive, "", None, "", "/");
        for (why, auth) in [("rateLimitExceeded", false), ("userRateLimitExceeded", false), ("dailyLimitExceeded", false), ("accessNotConfigured", false), ("insufficientPermissions", true), ("authError", true)] {
            *reason.lock().unwrap() = why.to_string();
            let c = GoogleDriveConnector::new().with_endpoints(&format!("{b}/api"), &format!("{b}/upload"), &format!("{b}/token"));
            let err = c.connect_with_secret(&p, Some(r#"{"access_token":"t"}"#.into())).await.unwrap_err();
            assert_eq!(err.is_auth_failure(), auth, "{why}: {err}");
            if why.contains("RateLimit") || why.starts_with("rate") {
                assert!(ufop_core::retry::is_retryable(&err), "{why} is retried: {err}");
            }
        }
    }

    /// Scopes (item 25): "/" opens the chosen shared drive; Docs export in
    /// the chosen format; uploads go to the right folder.
    #[tokio::test(flavor = "multi_thread")]
    async fn shared_drive_root_and_export_formats() {
        use crate::connectors::mock_http::{json, serve, status};
        use crate::core::types::{GoogleDriveSettings, GoogleExportFormats};
        let mock = serve(|r| match r.path.as_str() {
            "/api/about" => json(serde_json::json!({ "user": { "emailAddress": "t@example.com" }, "storageQuota": { "usage": "7" } })),
            "/api/files" => json(serde_json::json!({ "files": [{ "id": "x1", "name": "Plan", "mimeType": "application/vnd.google-apps.document" }] })),
            "/api/drives" => json(serde_json::json!({ "drives": [{ "id": "D1", "name": "Team" }] })),
            "/api/files/x1" => json(serde_json::json!({ "id": "x1", "name": "Plan", "mimeType": "application/vnd.google-apps.document" })),
            "/api/files/x1/export" => status(200, "exported"),
            "/upload/files" => json(serde_json::json!({ "id": "new", "name": "n.txt", "mimeType": "text/plain" })),
            _ => status(404, "no"),
        })
        .await;
        let b = &mock.base;
        let c = GoogleDriveConnector::new().with_endpoints(&format!("{b}/api"), &format!("{b}/upload"), &format!("{b}/token"));
        let mut p = crate::connectors::live_tests::profile(crate::core::types::ConnectionProtocol::GoogleDrive, "", None, "", "/");
        p.extras.google_drive = Some(GoogleDriveSettings {
            root: GoogleDriveRoot::SharedDrive,
            shared_drive_id: Some("D1".into()),
            shared_drive_name: Some("Team".into()),
            export_formats: GoogleExportFormats { document: Some("pdf".into()), ..Default::default() },
        });
        c.connect_with_secret(&p, Some(r#"{"access_token":"t"}"#.into())).await.unwrap();
        assert_eq!(c.quota().await.unwrap().unwrap().used, Some(7));
        assert_eq!(c.all_shared_drives().await.unwrap(), vec![SharedDriveInfo { id: "D1".into(), name: "Team".into() }]);
        let list = c.list_remote("/").await.unwrap();
        assert_eq!(list[0].path, "/shared-drives/D1/x1");
        let q = &mock.requests("/api/files")[0];
        assert_eq!((q.query_param("corpora").as_deref(), q.query_param("driveId").as_deref()), (Some("drive"), Some("D1")));
        assert_eq!(q.query_param("q").as_deref(), Some("'D1' in parents and trashed = false"));
        // Export as PDF (chosen), written exactly where the caller asked
        // (regression: it went to `{local}.pdf`, and callers that used the
        // path they passed found no file). The caller picks the name.
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join("Plan");
        assert_eq!(c.download(&list[0].path, local.to_str().unwrap()).await.unwrap(), 8);
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "exported");
        assert!(!dir.path().join("Plan.pdf").exists());
        assert_eq!(mock.requests("/api/files/x1/export")[0].query_param("mimeType").as_deref(), Some("application/pdf"));
        // An unsupported choice falls back to the default (docx).
        assert_eq!(c.export_format_for("application/vnd.google-apps.spreadsheet"), Some(GoogleDocExportFormat::Xlsx));
        assert_eq!(c.export_format_for("text/plain"), None);
        // Uploading to "/" goes into the shared drive.
        let f = dir.path().join("n.txt");
        std::fs::write(&f, "hi").unwrap();
        c.upload(f.to_str().unwrap(), "/n.txt").await.unwrap();
        let body = String::from_utf8_lossy(&mock.requests("/upload/files")[0].body).to_string();
        assert!(body.contains(r#""parents":["D1"]"#), "{body}");
    }

    /// New folders and renamed items are reported at the path the folder
    /// listing gives them (ids, not names), so callers can put things in
    /// them and find them again.
    #[tokio::test(flavor = "multi_thread")]
    async fn created_and_renamed_items_are_addressed_by_their_listed_id_path() {
        let drive = super::fake_drive::start(&[("f1", "Folder 1", "root", true), ("x9", "a.txt", "f1", false)]).await;
        let c = drive.connector.as_ref();
        let sub = c.make_directory_path("/f1/new").await.unwrap();
        let top = c.make_directory_path("/top").await.unwrap();
        assert!(c.list_remote("/f1").await.unwrap().iter().any(|e| e.path == sub && e.name == "new"), "{sub}");
        assert!(c.list_remote("/").await.unwrap().iter().any(|e| e.path == top && e.name == "top"), "{top}");
        assert_eq!(c.renamed_path("/f1/x9", "/f1/b.txt"), "/f1/x9", "a rename keeps the id path");
        assert_eq!(c.renamed_path("/f1/x9", &format!("{sub}/a.txt")), format!("{sub}/x9"));
        assert_eq!(crate::commands::connector_commands::rename_at(c, "/f1/x9", &format!("{sub}/b.txt")).await.unwrap(), format!("{sub}/x9"));
        assert!(c.list_remote(&sub).await.unwrap().iter().any(|e| e.path == format!("{sub}/x9") && e.name == "b.txt"));
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

/// A small in-process Google Drive for tests of callers that address Drive
/// by folder-id paths (pane transfers, typed addresses): folders and files
/// live under parent ids, and every request naming an unknown parent id
/// answers 404 exactly as Drive does.
#[cfg(test)]
pub(crate) mod fake_drive {
    use super::GoogleDriveConnector;
    use crate::connectors::mock_http::{json, serve, status, Mock, Seen};
    use crate::connectors::Connector;
    use std::collections::BTreeMap;
    use std::sync::{Arc, Mutex};

    /// One Drive item: (name, parent id, is folder).
    pub type Item = (String, String, bool);

    pub struct FakeDrive {
        pub mock: Mock,
        pub items: Arc<Mutex<BTreeMap<String, Item>>>,
        pub connector: Arc<GoogleDriveConnector>,
    }

    impl FakeDrive {
        /// The id of the item called `name` in folder `parent`.
        pub fn id_of(&self, name: &str, parent: &str) -> Option<String> {
            self.items.lock().unwrap().iter().find(|(_, (n, p, _))| n == name && p == parent).map(|(id, _)| id.clone())
        }
    }

    const FOLDER: &str = "application/vnd.google-apps.folder";

    fn file_json(id: &str, (name, _, folder): &Item) -> serde_json::Value {
        serde_json::json!({ "id": id, "name": name, "mimeType": if *folder { FOLDER } else { "text/plain" } })
    }

    /// The parent id of a `files.list` query (`'<id>' in parents`).
    fn query_parent(r: &Seen) -> Option<String> {
        let q = r.query_param("q")?;
        let end = q.find("' in parents")?;
        let start = q[..end].rfind('\'')? + 1;
        Some(q[start..end].to_string())
    }

    /// The metadata JSON of a multipart upload body.
    fn multipart_metadata(body: &[u8]) -> serde_json::Value {
        let text = String::from_utf8_lossy(body);
        let json = text.split("\r\n\r\n").nth(1).and_then(|p| p.split("\r\n").next()).unwrap_or("{}");
        serde_json::from_str(json).unwrap_or(serde_json::Value::Null)
    }

    /// Serve a Drive holding `seed` (id → item; `root` is always there) and
    /// sign a connector in to it.
    pub async fn start(seed: &[(&str, &str, &str, bool)]) -> FakeDrive {
        let items: Arc<Mutex<BTreeMap<String, Item>>> = Arc::default();
        for (id, name, parent, folder) in seed {
            items.lock().unwrap().insert(id.to_string(), (name.to_string(), parent.to_string(), *folder));
        }
        let state = items.clone();
        let mock = serve(move |r| {
            let mut items = state.lock().unwrap();
            let is_folder = |items: &BTreeMap<String, Item>, id: &str| id == "root" || items.get(id).is_some_and(|i| i.2);
            let not_found = || status(404, r#"{"error":{"code":404,"message":"File not found"}}"#);
            let create = |items: &mut BTreeMap<String, Item>, name: String, parent: String, folder: bool| {
                if !is_folder(items, &parent) {
                    return not_found();
                }
                let id = format!("id{}", items.len() + 1);
                let item = (name, parent, folder);
                let body = file_json(&id, &item);
                items.insert(id, item);
                json(body)
            };
            match (r.method.as_str(), r.path.as_str()) {
                (_, "/api/about") => json(serde_json::json!({ "user": { "emailAddress": "t@example.com" } })),
                ("GET", "/api/files") => match query_parent(r) {
                    Some(p) if is_folder(&items, &p) => json(serde_json::json!({ "files": items.iter().filter(|(_, i)| i.1 == p).map(|(id, i)| file_json(id, i)).collect::<Vec<_>>() })),
                    _ => not_found(),
                },
                ("POST", "/api/files") => {
                    let m = r.json();
                    create(&mut items, m["name"].as_str().unwrap_or_default().to_string(), m["parents"][0].as_str().unwrap_or("root").to_string(), m["mimeType"] == FOLDER)
                }
                ("POST", "/upload/files") => {
                    let m = multipart_metadata(&r.body);
                    create(&mut items, m["name"].as_str().unwrap_or_default().to_string(), m["parents"][0].as_str().unwrap_or("root").to_string(), false)
                }
                ("PATCH", p) if p.starts_with("/api/files/") => {
                    let id = p.trim_start_matches("/api/files/").to_string();
                    let new_parent = r.query_param("addParents");
                    if new_parent.as_deref().is_some_and(|np| !is_folder(&items, np)) {
                        return not_found();
                    }
                    let name = r.json()["name"].as_str().map(String::from);
                    match items.get_mut(&id) {
                        Some(item) => {
                            if let Some(np) = new_parent {
                                item.1 = np;
                            }
                            if let Some(n) = name {
                                item.0 = n;
                            }
                            json(file_json(&id, item))
                        }
                        None => not_found(),
                    }
                }
                _ => not_found(),
            }
        })
        .await;
        let b = &mock.base;
        let connector = GoogleDriveConnector::new().with_endpoints(&format!("{b}/api"), &format!("{b}/upload"), &format!("{b}/token"));
        let p = crate::connectors::live_tests::profile(crate::core::types::ConnectionProtocol::GoogleDrive, "", None, "", "/");
        connector.connect_with_secret(&p, Some(r#"{"access_token":"t"}"#.into())).await.unwrap();
        FakeDrive { mock, items, connector: Arc::new(connector) }
    }
}
