//! WebDAV Connector (T-022).
//!
//! Provides WebDAV file operations with support for:
//! - Auth: basic, digest, OAuth (Bearer token)
//! - HTTPS enforced by default (HTTP requires explicit opt-in with warning)
//! - WebDAV locking where server supports it
//! - Chunked upload for large files
//! - Content-Range PUT for partial uploads
//! - Compatible with Nextcloud, ownCloud, and other WebDAV servers

use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

// ── WebDAV Configuration ──

/// WebDAV authentication method.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WebDavAuthMethod {
    /// HTTP Basic authentication.
    Basic {
        username: String,
        password: String,
    },
    /// HTTP Digest authentication.
    Digest {
        username: String,
        password: String,
    },
    /// OAuth2 Bearer token.
    OAuth {
        access_token: String,
        refresh_token: Option<String>,
        token_endpoint: Option<String>,
    },
    /// No authentication.
    None,
}

/// WebDAV connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDavConfig {
    /// Authentication method.
    pub auth_method: WebDavAuthMethod,
    /// Base URL of the WebDAV server (e.g., "https://cloud.example.com/remote.php/dav/files/user/").
    pub base_url: String,
    /// Allow HTTP (insecure) connections. Default: false (HTTPS required).
    pub allow_insecure_http: bool,
    /// Accept self-signed or invalid SSL certificates.
    pub accept_invalid_certs: bool,
    /// Connection timeout in seconds.
    pub connect_timeout_secs: u32,
    /// Request timeout in seconds.
    pub request_timeout_secs: u32,
    /// Chunk size for large file uploads (bytes). Default: 5MB.
    pub chunk_upload_size: u64,
    /// Minimum file size to trigger chunked upload (bytes). Default: 10MB.
    pub chunk_upload_threshold: u64,
    /// Enable WebDAV locking for file operations.
    pub enable_locking: bool,
    /// Lock timeout in seconds.
    pub lock_timeout_secs: u32,
    /// User-Agent header.
    pub user_agent: String,
    /// Server compatibility mode.
    pub compat_mode: WebDavCompat,
}

impl Default for WebDavConfig {
    fn default() -> Self {
        Self {
            auth_method: WebDavAuthMethod::None,
            base_url: String::new(),
            allow_insecure_http: false,
            accept_invalid_certs: false,
            connect_timeout_secs: 15,
            request_timeout_secs: 300,
            chunk_upload_size: 5 * 1024 * 1024,          // 5 MB
            chunk_upload_threshold: 10 * 1024 * 1024,     // 10 MB
            enable_locking: false,
            lock_timeout_secs: 300,
            user_agent: "UnifiedFileOps/0.1".to_string(),
            compat_mode: WebDavCompat::Standard,
        }
    }
}

/// WebDAV server compatibility mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WebDavCompat {
    /// Standard WebDAV.
    Standard,
    /// Nextcloud-specific adaptations (chunked upload v2, etc.).
    Nextcloud,
    /// ownCloud-specific adaptations.
    OwnCloud,
}

/// WebDAV lock token.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDavLock {
    pub token: String,
    pub path: String,
    pub owner: String,
    pub timeout_secs: u32,
    pub created_at: DateTime<Utc>,
}

/// WebDAV health status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebDavHealthStatus {
    pub connected: bool,
    pub https_active: bool,
    pub server_type: Option<String>,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub base_url: String,
    pub compat_mode: WebDavCompat,
}

/// A parsed WebDAV PROPFIND response entry.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct DavEntry {
    href: String,
    is_collection: bool,
    content_length: u64,
    last_modified: Option<DateTime<Utc>>,
    created: Option<DateTime<Utc>>,
    etag: Option<String>,
    content_type: Option<String>,
}

// ── WebDAV Connector State ──

/// HTTP client session for WebDAV.
struct WebDavSession {
    client: reqwest::Client,
    base_url: String,
    auth_header: Option<(String, String)>,
}

/// WebDAV Connector implementing the Connector trait.
pub struct WebDavConnector {
    /// HTTP session (None if disconnected).
    session: Arc<Mutex<Option<WebDavSession>>>,
    /// Connection status flag.
    connected: AtomicBool,
    /// WebDAV-specific configuration.
    config: RwLock<WebDavConfig>,
    /// Active locks.
    active_locks: RwLock<HashMap<String, WebDavLock>>,
    /// Bytes transferred counters.
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
    /// Detected server type.
    server_type: RwLock<Option<String>>,
}

impl WebDavConnector {
    /// Create a new WebDAV connector with default configuration.
    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            connected: AtomicBool::new(false),
            config: RwLock::new(WebDavConfig::default()),
            active_locks: RwLock::new(HashMap::new()),
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            server_type: RwLock::new(None),
        }
    }

    /// Create a new WebDAV connector with custom configuration.
    pub fn with_config(config: WebDavConfig) -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            connected: AtomicBool::new(false),
            config: RwLock::new(config),
            active_locks: RwLock::new(HashMap::new()),
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
            server_type: RwLock::new(None),
        }
    }

    /// Update the WebDAV configuration.
    pub async fn set_config(&self, config: WebDavConfig) {
        let mut cfg = self.config.write().await;
        *cfg = config;
    }

    /// Get current health status.
    pub async fn health_status(&self) -> WebDavHealthStatus {
        let config = self.config.read().await;
        let server_type = self.server_type.read().await;

        WebDavHealthStatus {
            connected: self.connected.load(Ordering::SeqCst),
            https_active: config.base_url.starts_with("https://"),
            server_type: server_type.clone(),
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            base_url: config.base_url.clone(),
            compat_mode: config.compat_mode,
        }
    }

    /// Validate the base URL for security.
    fn validate_url(url: &str, allow_http: bool) -> Result<String, AppError> {
        let url = url.trim_end_matches('/').to_string();

        if url.starts_with("https://") {
            Ok(url)
        } else if url.starts_with("http://") {
            if allow_http {
                tracing::warn!(
                    "WebDAV connection using insecure HTTP. Credentials may be transmitted in plaintext."
                );
                Ok(url)
            } else {
                Err(AppError::Connection {
                    message: "HTTP connections are not allowed for WebDAV.".to_string(),
                    advice: "Use HTTPS or enable allow_insecure_http in configuration.".to_string(),
                })
            }
        } else {
            Err(AppError::Connection {
                message: format!("Invalid WebDAV URL: {url}"),
                advice: "URL must start with https:// (or http:// if explicitly allowed).".to_string(),
            })
        }
    }

    /// Build the auth header for requests.
    fn build_auth_header(auth: &WebDavAuthMethod) -> Option<(String, String)> {
        match auth {
            WebDavAuthMethod::Basic { username, password } => {
                let credentials = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    format!("{username}:{password}"),
                );
                Some(("Authorization".to_string(), format!("Basic {credentials}")))
            }
            WebDavAuthMethod::Digest { username, password } => {
                // Digest auth is typically handled by the HTTP client during the request,
                // not as a pre-computed header. For initial connection, we'll use basic
                // and the server will challenge us for digest if needed.
                let credentials = base64::Engine::encode(
                    &base64::engine::general_purpose::STANDARD,
                    format!("{username}:{password}"),
                );
                Some(("Authorization".to_string(), format!("Basic {credentials}")))
            }
            WebDavAuthMethod::OAuth { access_token, .. } => {
                Some(("Authorization".to_string(), format!("Bearer {access_token}")))
            }
            WebDavAuthMethod::None => None,
        }
    }

    /// Build the full URL for a path.
    fn build_url(base: &str, path: &str) -> String {
        let base = base.trim_end_matches('/');
        let path = path.trim_start_matches('/');
        if path.is_empty() {
            format!("{base}/")
        } else {
            format!("{base}/{path}")
        }
    }

    /// Parse a PROPFIND XML response into DavEntry objects.
    fn parse_propfind_response(xml: &str, _base_url: &str) -> Vec<DavEntry> {
        use quick_xml::events::Event;
        use quick_xml::Reader;

        // Servers disagree on namespace prefixes (d:, D:, lp1:, none) and put
        // attributes on <response>, so match on local names only.
        let mut reader = Reader::from_str(xml);
        let mut entries = Vec::new();
        let mut current: Option<DavEntry> = None;
        let mut field: Option<String> = None;

        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) => {
                    let name = String::from_utf8_lossy(e.local_name().as_ref()).to_lowercase();
                    match name.as_str() {
                        "response" => {
                            current = Some(DavEntry {
                                href: String::new(),
                                is_collection: false,
                                content_length: 0,
                                last_modified: None,
                                created: None,
                                etag: None,
                                content_type: None,
                            });
                        }
                        "collection" => {
                            if let Some(c) = current.as_mut() {
                                c.is_collection = true;
                            }
                        }
                        "href" | "getcontentlength" | "getlastmodified" | "creationdate" | "getetag"
                        | "getcontenttype" => field = Some(name),
                        _ => {}
                    }
                }
                Ok(Event::Empty(e)) => {
                    if e.local_name().as_ref().eq_ignore_ascii_case(b"collection") {
                        if let Some(c) = current.as_mut() {
                            c.is_collection = true;
                        }
                    }
                }
                Ok(Event::Text(t)) => {
                    if let (Some(c), Some(f)) = (current.as_mut(), field.as_deref()) {
                        let text = t.decode().map(|c| c.to_string()).unwrap_or_default();
                        let text = text.trim();
                        match f {
                            "href" => c.href = text.to_string(),
                            "getcontentlength" => c.content_length = text.parse().unwrap_or(0),
                            "getlastmodified" => {
                                c.last_modified = DateTime::parse_from_rfc2822(text).ok().map(|d| d.with_timezone(&Utc))
                            }
                            "creationdate" => {
                                c.created = DateTime::parse_from_rfc3339(text).ok().map(|d| d.with_timezone(&Utc))
                            }
                            "getetag" => c.etag = Some(text.to_string()),
                            "getcontenttype" => c.content_type = Some(text.to_string()),
                            _ => {}
                        }
                    }
                }
                Ok(Event::End(e)) => {
                    field = None;
                    if e.local_name().as_ref().eq_ignore_ascii_case(b"response") {
                        if let Some(c) = current.take() {
                            entries.push(c);
                        }
                    }
                }
                Ok(Event::Eof) | Err(_) => break,
                _ => {}
            }
        }

        entries
    }

    /// Text of the first element whose local name is `element_name`, ignoring namespace prefixes.
    fn extract_xml_value(xml: &str, element_name: &str) -> Option<String> {
        use quick_xml::events::Event;
        use quick_xml::Reader;

        let mut reader = Reader::from_str(xml);
        let mut inside = false;
        loop {
            match reader.read_event() {
                Ok(Event::Start(e)) if e.local_name().as_ref().eq_ignore_ascii_case(element_name.as_bytes()) => {
                    inside = true;
                }
                Ok(Event::Text(t)) if inside => {
                    return t.decode().ok().map(|c| c.trim().to_string());
                }
                Ok(Event::End(_)) if inside => return None,
                Ok(Event::Eof) | Err(_) => return None,
                _ => {}
            }
        }
    }

    /// Convert a DavEntry to a FileEntry.
    fn dav_entry_to_file_entry(entry: &DavEntry, _base_url: &str) -> FileEntry {
        // Extract filename from href
        let href_decoded = urlencoding::decode(&entry.href).unwrap_or_default().to_string();
        let href_trimmed = href_decoded.trim_end_matches('/');
        let name = href_trimmed
            .rsplit('/')
            .next()
            .unwrap_or(href_trimmed)
            .to_string();

        let path = href_decoded.trim_end_matches('/').to_string();

        let extension = if !entry.is_collection {
            Path::new(&name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_string())
        } else {
            None
        };

        FileEntry {
            name: name.clone(),
            path,
            is_dir: entry.is_collection,
            is_symlink: false,
            size: entry.content_length,
            modified: entry.last_modified,
            created: entry.created,
            is_hidden: name.starts_with('.'),
            extension,
            permissions: None,
        }
    }

    // ── File Operations ──

    /// List directory contents via PROPFIND.
    pub async fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>, AppError> {
        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        // A collection URL must end in "/", or servers answer 301 (redirects are off).
        let mut url = Self::build_url(&dav.base_url, path);
        if !url.ends_with('/') {
            url.push('/');
        }

        let propfind_body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:resourcetype/>
    <d:getcontentlength/>
    <d:getlastmodified/>
    <d:creationdate/>
    <d:getetag/>
    <d:getcontenttype/>
  </d:prop>
</d:propfind>"#;

        let mut request = dav
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &url)
            .header("Depth", "1")
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(propfind_body);

        if let Some((ref key, ref value)) = dav.auth_header {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| AppError::Connection {
            message: format!("WebDAV PROPFIND failed for {url}: {e}"),
            advice: "Check network connection and server availability.".to_string(),
        })?;

        if !response.status().is_success() && response.status().as_u16() != 207 {
            return Err(AppError::Connection {
                message: format!(
                    "WebDAV PROPFIND returned status {}: {}",
                    response.status(),
                    response
                        .status()
                        .canonical_reason()
                        .unwrap_or("Unknown")
                ),
                advice: "Check the path and your permissions.".to_string(),
            });
        }

        let body = response.text().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read PROPFIND response: {e}"),
            advice: "Server may have returned invalid data.".to_string(),
        })?;

        let dav_entries = Self::parse_propfind_response(&body, &dav.base_url);

        // Convert to FileEntry, skipping the first entry (the directory itself)
        let entries: Vec<FileEntry> = dav_entries
            .iter()
            .skip(1) // First entry is the directory itself
            .map(|e| Self::dav_entry_to_file_entry(e, &dav.base_url))
            .filter(|e| !e.name.is_empty())
            .collect();

        Ok(entries)
    }

    /// Upload a file to the WebDAV server.
    pub async fn upload_file(
        &self,
        local_path: &str,
        remote_path: &str,
        resume_offset: Option<u64>,
    ) -> Result<u64, AppError> {
        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let file_data = std::fs::read(local_path).map_err(|e| AppError::FileOperation {
            message: format!("Cannot read local file {local_path}: {e}"),
            advice: "Check that the file exists and you have permission.".to_string(),
        })?;

        let total_size = file_data.len() as u64;
        let config = self.config.read().await;

        // Check if chunked upload should be used
        if total_size >= config.chunk_upload_threshold && resume_offset.is_none() {
            drop(config);
            drop(session);
            return self.chunked_upload(local_path, remote_path, total_size).await;
        }

        let url = Self::build_url(&dav.base_url, remote_path);

        // Handle partial upload with Content-Range
        let mut request = if let Some(offset) = resume_offset {
            if offset > 0 {
                let data_slice = &file_data[offset as usize..];
                dav.client
                    .put(&url)
                    .header(
                        "Content-Range",
                        format!("bytes {}-{}/{}", offset, total_size - 1, total_size),
                    )
                    .body(data_slice.to_vec())
            } else {
                dav.client.put(&url).body(file_data.clone())
            }
        } else {
            dav.client.put(&url).body(file_data.clone())
        };

        if let Some((ref key, ref value)) = dav.auth_header {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| AppError::Connection {
            message: format!("WebDAV PUT failed for {url}: {e}"),
            advice: "Check network connection and server availability.".to_string(),
        })?;

        if !response.status().is_success() {
            return Err(AppError::Connection {
                message: format!(
                    "WebDAV upload failed with status {}: {}",
                    response.status(),
                    response
                        .status()
                        .canonical_reason()
                        .unwrap_or("Unknown")
                ),
                advice: "Check permissions and available space on the server.".to_string(),
            });
        }

        self.bytes_sent.fetch_add(total_size, Ordering::Relaxed);
        Ok(total_size)
    }

    /// Perform chunked upload for large files (Nextcloud/ownCloud compatible).
    async fn chunked_upload(
        &self,
        local_path: &str,
        remote_path: &str,
        total_size: u64,
    ) -> Result<u64, AppError> {
        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let config = self.config.read().await;
        let chunk_size = config.chunk_upload_size;
        let compat = config.compat_mode;
        drop(config);

        let file_data = std::fs::read(local_path).map_err(|e| AppError::FileOperation {
            message: format!("Cannot read local file {local_path}: {e}"),
            advice: "Check that the file exists.".to_string(),
        })?;

        let upload_id = uuid::Uuid::new_v4().to_string();

        match compat {
            WebDavCompat::Nextcloud | WebDavCompat::OwnCloud => {
                // Nextcloud chunked upload v2:
                // 1. MKCOL upload directory
                // 2. PUT each chunk
                // 3. MOVE to final destination

                let chunk_dir_url = Self::build_url(
                    &dav.base_url,
                    &format!("uploads/{upload_id}"),
                );

                // Create chunk upload directory
                let mut request = dav.client
                    .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &chunk_dir_url);
                if let Some((ref key, ref value)) = dav.auth_header {
                    request = request.header(key.as_str(), value.as_str());
                }
                let _ = request.send().await;

                // Upload chunks
                let mut offset: u64 = 0;
                let mut chunk_index = 0u32;
                while offset < total_size {
                    let end = std::cmp::min(offset + chunk_size, total_size);
                    let chunk_data = &file_data[offset as usize..end as usize];

                    let chunk_url = Self::build_url(
                        &dav.base_url,
                        &format!("uploads/{upload_id}/{:08}", chunk_index),
                    );

                    let mut request = dav.client
                        .put(&chunk_url)
                        .body(chunk_data.to_vec());
                    if let Some((ref key, ref value)) = dav.auth_header {
                        request = request.header(key.as_str(), value.as_str());
                    }

                    let response = request.send().await.map_err(|e| AppError::Connection {
                        message: format!("Chunk upload {chunk_index} failed: {e}"),
                        advice: "Check network connection.".to_string(),
                    })?;

                    if !response.status().is_success() {
                        return Err(AppError::Connection {
                            message: format!("Chunk upload failed with status {}", response.status()),
                            advice: "The server may not support chunked uploads.".to_string(),
                        });
                    }

                    self.bytes_sent.fetch_add(chunk_data.len() as u64, Ordering::Relaxed);
                    offset = end;
                    chunk_index += 1;
                }

                // Move assembled file to final destination
                let final_url = Self::build_url(&dav.base_url, remote_path);
                let last_chunk_url = Self::build_url(
                    &dav.base_url,
                    &format!("uploads/{upload_id}/.file"),
                );

                let mut request = dav.client
                    .request(reqwest::Method::from_bytes(b"MOVE").unwrap(), &last_chunk_url)
                    .header("Destination", &final_url);
                if let Some((ref key, ref value)) = dav.auth_header {
                    request = request.header(key.as_str(), value.as_str());
                }
                let _ = request.send().await;
            }
            WebDavCompat::Standard => {
                // Standard WebDAV: just PUT the whole thing
                // (already handled by the caller for smaller files)
                let url = Self::build_url(&dav.base_url, remote_path);
                let mut request = dav.client.put(&url).body(file_data);
                if let Some((ref key, ref value)) = dav.auth_header {
                    request = request.header(key.as_str(), value.as_str());
                }
                let response = request.send().await.map_err(|e| AppError::Connection {
                    message: format!("WebDAV upload failed: {e}"),
                    advice: "Check network connection.".to_string(),
                })?;
                if !response.status().is_success() {
                    return Err(AppError::Connection {
                        message: format!("Upload failed with status {}", response.status()),
                        advice: "Check permissions and available space.".to_string(),
                    });
                }
                self.bytes_sent.fetch_add(total_size, Ordering::Relaxed);
            }
        }

        Ok(total_size)
    }

    /// Download a file from the WebDAV server.
    pub async fn download_file(
        &self,
        remote_path: &str,
        local_path: &str,
        resume_offset: Option<u64>,
    ) -> Result<u64, AppError> {
        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let url = Self::build_url(&dav.base_url, remote_path);

        let mut request = dav.client.get(&url);

        // Handle resume with Range header
        if let Some(offset) = resume_offset {
            if offset > 0 {
                request = request.header("Range", format!("bytes={}-", offset));
            }
        }

        if let Some((ref key, ref value)) = dav.auth_header {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| AppError::Connection {
            message: format!("WebDAV GET failed for {url}: {e}"),
            advice: "Check network connection and server availability.".to_string(),
        })?;

        if !response.status().is_success() {
            return Err(AppError::Connection {
                message: format!(
                    "WebDAV download failed with status {}: {}",
                    response.status(),
                    response.status().canonical_reason().unwrap_or("Unknown")
                ),
                advice: "Check that the file exists and you have permission.".to_string(),
            });
        }

        let data = response.bytes().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read response body: {e}"),
            advice: "Connection may have been interrupted.".to_string(),
        })?;

        // Write to local file
        if let Some(offset) = resume_offset {
            if offset > 0 {
                use std::io::Write;
                let mut file = std::fs::OpenOptions::new()
                    
                    .append(true)
                    .open(local_path)
                    .map_err(|e| AppError::FileOperation {
                        message: format!("Cannot open local file for resume: {e}"),
                        advice: "Check that the partial file exists.".to_string(),
                    })?;
                file.write_all(&data).map_err(|e| AppError::FileOperation {
                    message: format!("Error writing to local file: {e}"),
                    advice: "Check disk space and permissions.".to_string(),
                })?;

                let total = offset + data.len() as u64;
                self.bytes_received.fetch_add(data.len() as u64, Ordering::Relaxed);
                return Ok(total);
            }
        }

        std::fs::write(local_path, &data).map_err(|e| AppError::FileOperation {
            message: format!("Cannot write to {local_path}: {e}"),
            advice: "Check disk space and permissions.".to_string(),
        })?;

        let bytes = data.len() as u64;
        self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
        Ok(bytes)
    }

    /// Create a directory (MKCOL) on the WebDAV server.
    pub async fn mkdir(&self, path: &str) -> Result<(), AppError> {
        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let url = Self::build_url(&dav.base_url, path);

        let mut request = dav
            .client
            .request(reqwest::Method::from_bytes(b"MKCOL").unwrap(), &url);

        if let Some((ref key, ref value)) = dav.auth_header {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| AppError::Connection {
            message: format!("WebDAV MKCOL failed for {url}: {e}"),
            advice: "Check network connection.".to_string(),
        })?;

        if !response.status().is_success() {
            return Err(AppError::Connection {
                message: format!(
                    "Failed to create directory, status {}: {}",
                    response.status(),
                    response.status().canonical_reason().unwrap_or("Unknown")
                ),
                advice: "Check that the parent directory exists and you have permission.".to_string(),
            });
        }

        Ok(())
    }

    /// Delete a file or directory on the WebDAV server.
    pub async fn delete(&self, path: &str) -> Result<(), AppError> {
        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let url = Self::build_url(&dav.base_url, path);

        let mut request = dav.client.delete(&url);

        if let Some((ref key, ref value)) = dav.auth_header {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| AppError::Connection {
            message: format!("WebDAV DELETE failed for {url}: {e}"),
            advice: "Check network connection.".to_string(),
        })?;

        if !response.status().is_success() {
            return Err(AppError::Connection {
                message: format!(
                    "Failed to delete, status {}: {}",
                    response.status(),
                    response.status().canonical_reason().unwrap_or("Unknown")
                ),
                advice: "Check that the resource exists and you have permission.".to_string(),
            });
        }

        Ok(())
    }

    /// Rename/move a resource on the WebDAV server (MOVE).
    pub async fn rename(&self, old_path: &str, new_path: &str) -> Result<(), AppError> {
        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let source_url = Self::build_url(&dav.base_url, old_path);
        let dest_url = Self::build_url(&dav.base_url, new_path);

        let mut request = dav
            .client
            .request(reqwest::Method::from_bytes(b"MOVE").unwrap(), &source_url)
            .header("Destination", &dest_url)
            .header("Overwrite", "F");

        if let Some((ref key, ref value)) = dav.auth_header {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| AppError::Connection {
            message: format!("WebDAV MOVE failed: {e}"),
            advice: "Check network connection.".to_string(),
        })?;

        if !response.status().is_success() {
            return Err(AppError::Connection {
                message: format!(
                    "Failed to move/rename, status {}: {}",
                    response.status(),
                    response.status().canonical_reason().unwrap_or("Unknown")
                ),
                advice: "Check permissions and that both paths are valid.".to_string(),
            });
        }

        Ok(())
    }

    /// Copy a resource on the WebDAV server (COPY).
    pub async fn copy(&self, source_path: &str, dest_path: &str) -> Result<(), AppError> {
        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let source_url = Self::build_url(&dav.base_url, source_path);
        let dest_url = Self::build_url(&dav.base_url, dest_path);

        let mut request = dav
            .client
            .request(reqwest::Method::from_bytes(b"COPY").unwrap(), &source_url)
            .header("Destination", &dest_url)
            .header("Overwrite", "F");

        if let Some((ref key, ref value)) = dav.auth_header {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| AppError::Connection {
            message: format!("WebDAV COPY failed: {e}"),
            advice: "Check network connection.".to_string(),
        })?;

        if !response.status().is_success() {
            return Err(AppError::Connection {
                message: format!(
                    "Failed to copy, status {}: {}",
                    response.status(),
                    response.status().canonical_reason().unwrap_or("Unknown")
                ),
                advice: "Check permissions and available space.".to_string(),
            });
        }

        Ok(())
    }

    /// Lock a resource (WebDAV LOCK).
    pub async fn lock(&self, path: &str) -> Result<WebDavLock, AppError> {
        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let config = self.config.read().await;
        let timeout = config.lock_timeout_secs;
        drop(config);

        let url = Self::build_url(&dav.base_url, path);

        let lock_body = r#"<?xml version="1.0" encoding="utf-8"?>
<d:lockinfo xmlns:d="DAV:">
  <d:lockscope><d:exclusive/></d:lockscope>
  <d:locktype><d:write/></d:locktype>
  <d:owner><d:href>UnifiedFileOps</d:href></d:owner>
</d:lockinfo>"#.to_string();

        let mut request = dav
            .client
            .request(reqwest::Method::from_bytes(b"LOCK").unwrap(), &url)
            .header("Timeout", format!("Second-{timeout}"))
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(lock_body);

        if let Some((ref key, ref value)) = dav.auth_header {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| AppError::Connection {
            message: format!("WebDAV LOCK failed for {url}: {e}"),
            advice: "The server may not support locking.".to_string(),
        })?;

        if !response.status().is_success() {
            return Err(AppError::Connection {
                message: format!("LOCK failed with status {}", response.status()),
                advice: "The resource may already be locked or locking is not supported.".to_string(),
            });
        }

        // Extract lock token from response
        let body = response.text().await.unwrap_or_default();
        let lock_token = Self::extract_xml_value(&body, "locktoken")
            .or_else(|| Self::extract_xml_value(&body, "href"))
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let lock = WebDavLock {
            token: lock_token,
            path: path.to_string(),
            owner: "UnifiedFileOps".to_string(),
            timeout_secs: timeout,
            created_at: Utc::now(),
        };

        // Store lock
        let mut locks = self.active_locks.write().await;
        locks.insert(path.to_string(), lock.clone());

        Ok(lock)
    }

    /// Unlock a resource (WebDAV UNLOCK).
    pub async fn unlock(&self, path: &str) -> Result<(), AppError> {
        let lock = {
            let locks = self.active_locks.read().await;
            locks.get(path).cloned()
        };

        let lock = lock.ok_or_else(|| AppError::Connection {
            message: format!("No active lock found for {path}"),
            advice: "The resource may not have been locked.".to_string(),
        })?;

        let session = self.session.lock().await;
        let dav = session.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to WebDAV server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let url = Self::build_url(&dav.base_url, path);

        let mut request = dav
            .client
            .request(reqwest::Method::from_bytes(b"UNLOCK").unwrap(), &url)
            .header("Lock-Token", format!("<{}>", lock.token));

        if let Some((ref key, ref value)) = dav.auth_header {
            request = request.header(key.as_str(), value.as_str());
        }

        let response = request.send().await.map_err(|e| AppError::Connection {
            message: format!("WebDAV UNLOCK failed for {url}: {e}"),
            advice: "Check network connection.".to_string(),
        })?;

        if !response.status().is_success() && response.status().as_u16() != 204 {
            return Err(AppError::Connection {
                message: format!("UNLOCK failed with status {}", response.status()),
                advice: "The lock may have already expired.".to_string(),
            });
        }

        // Remove lock from tracking
        let mut locks = self.active_locks.write().await;
        locks.remove(path);

        Ok(())
    }

    /// Get active locks.
    pub async fn active_locks(&self) -> Vec<WebDavLock> {
        let locks = self.active_locks.read().await;
        locks.values().cloned().collect()
    }
}

impl Default for WebDavConnector {
    fn default() -> Self {
        Self::new()
    }
}

// ── Connector trait implementation ──

impl super::Connector for WebDavConnector {
    fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + '_>> {
        let host = profile.host.clone();
        let port = profile.port.unwrap_or(443);
        let remote_path = profile.remote_path.clone();

        Box::pin(async move {
            if self.connected.load(Ordering::SeqCst) {
                return Err(AppError::Connection {
                    message: "Already connected.".to_string(),
                    advice: "Disconnect first before connecting.".to_string(),
                });
            }

            let config = self.config.read().await.clone();

            // Build base URL
            let base_url = if !config.base_url.is_empty() {
                config.base_url.clone()
            } else {
                let scheme = if port == 80 || config.allow_insecure_http {
                    "http"
                } else {
                    "https"
                };
                if port == 443 || port == 80 {
                    format!("{scheme}://{host}{remote_path}")
                } else {
                    format!("{scheme}://{host}:{port}{remote_path}")
                }
            };

            // Validate URL security
            let validated_url = Self::validate_url(&base_url, config.allow_insecure_http)?;

            // Build HTTP client
            let accept_invalid_certs = config.accept_invalid_certs;
            let connect_timeout = config.connect_timeout_secs;
            let request_timeout = config.request_timeout_secs;
            let user_agent = config.user_agent.clone();
            let auth_method = config.auth_method.clone();

            let client = {
                let builder = reqwest::Client::builder()
                    .timeout(std::time::Duration::from_secs(request_timeout as u64))
                    .connect_timeout(std::time::Duration::from_secs(connect_timeout as u64))
                    .user_agent(&user_agent)
                    .danger_accept_invalid_certs(accept_invalid_certs)
                    // Disable redirect following for WebDAV
                    .redirect(reqwest::redirect::Policy::none());
                builder.build().map_err(|e| AppError::Connection {
                    message: format!("Failed to create HTTP client: {e}"),
                    advice: "Check TLS/SSL configuration.".to_string(),
                })?
            };

            // Build auth header
            let auth_header = Self::build_auth_header(&auth_method);

            // Test connection with OPTIONS request
            let validated_url_clone = validated_url.clone();
            let auth_header_clone = auth_header.clone();
            let client_clone = client.clone();

            let server_info = async move {
                let mut request = client_clone
                    .request(reqwest::Method::OPTIONS, &validated_url_clone);

                if let Some((ref key, ref value)) = auth_header_clone {
                    request = request.header(key.as_str(), value.as_str());
                }

                let response = request.send().await.map_err(|e| AppError::Connection {
                    message: format!("Cannot reach WebDAV server at {validated_url_clone}: {e}"),
                    advice: "Check the URL and network connectivity.".to_string(),
                })?;

                if response.status().as_u16() == 401 {
                    return Err(AppError::Connection {
                        message: "Authentication failed.".to_string(),
                        advice: "Check your username, password, or token.".to_string(),
                    });
                }

                // Check for DAV header
                let dav_header = response
                    .headers()
                    .get("dav")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());

                let server_header = response
                    .headers()
                    .get("server")
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());

                Ok::<_, AppError>((dav_header, server_header))
            }
            .await?;

            // Detect server type
            let (dav_header, server_header) = server_info;
            if let Some(ref server) = server_header {
                let mut st = self.server_type.write().await;
                *st = Some(server.clone());
                tracing::info!("WebDAV server: {server}");
            }
            if let Some(ref dav) = dav_header {
                tracing::info!("WebDAV capabilities: {dav}");
            }

            // Store session
            {
                let mut session = self.session.lock().await;
                *session = Some(WebDavSession {
                    client,
                    base_url: validated_url.clone(),
                    auth_header,
                });
            }

            self.connected.store(true, Ordering::SeqCst);
            tracing::info!("WebDAV connected to {validated_url}");
            Ok(())
        })
    }

    fn disconnect(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + '_>>
    {
        Box::pin(async move {
            if !self.connected.load(Ordering::SeqCst) {
                return Ok(());
            }

            // Release all locks
            let lock_paths: Vec<String> = {
                let locks = self.active_locks.read().await;
                locks.keys().cloned().collect()
            };
            for path in lock_paths {
                if let Err(e) = self.unlock(&path).await {
                    tracing::warn!("Failed to release lock on {path}: {e}");
                }
            }

            // Drop session. A `reqwest::Client` owns its own runtime, and
            // dropping that inside an async task panics, so drop it on a blocking thread.
            let old_session = self.session.lock().await.take();
            let _ = tokio::task::spawn_blocking(move || drop(old_session)).await;

            self.connected.store(false, Ordering::SeqCst);
            self.bytes_sent.store(0, Ordering::Relaxed);
            self.bytes_received.store(0, Ordering::Relaxed);

            tracing::info!("WebDAV session disconnected");
            Ok(())
        })
    }

    fn list_remote(
        &self,
        path: &str,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<Vec<FileEntry>, AppError>> + Send + '_>,
    > {
        let path = path.to_string();
        Box::pin(async move { self.list_directory(&path).await })
    }

    fn is_connected(&self) -> bool {
        self.connected.load(Ordering::SeqCst)
    }

    fn connect_with_secret(&self, profile: &ConnectionProfile, secret: Option<String>) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + '_>> {
        let mut profile = profile.clone();
        Box::pin(async move {
            {
                let mut cfg = self.config.write().await;
                cfg.auth_method = match (profile.username.clone().filter(|u| !u.is_empty()), secret) {
                    (Some(username), Some(password)) => WebDavAuthMethod::Basic { username, password },
                    _ => WebDavAuthMethod::None,
                };
                // The host may be a full URL ("http://host:8080/dav"); its scheme
                // decides TLS, since the form has no separate field for it.
                let host = profile.host.clone();
                if let Some(rest) = host.strip_prefix("http://").or_else(|| host.strip_prefix("https://")) {
                    cfg.allow_insecure_http = host.starts_with("http://");
                    let rest = rest.trim_end_matches('/');
                    let (authority, path) = rest.split_once('/').map(|(a, p)| (a, format!("/{p}"))).unwrap_or((rest, String::new()));
                    let base_path = if profile.remote_path == "/" { path } else { format!("{path}{}", profile.remote_path) };
                    cfg.base_url = format!("{}://{authority}{base_path}", if host.starts_with("http://") { "http" } else { "https" });
                    profile.host = authority.to_string();
                } else {
                    cfg.base_url = String::new();
                }
            }
            super::Connector::connect(self, &profile).await
        })
    }

    fn upload(&self, local_path: &str, remote_path: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u64, AppError>> + Send + '_>> {
        let (l, r) = (local_path.to_string(), remote_path.to_string());
        Box::pin(async move {
            super::staged_upload(
                &r,
                |temp| async move { self.upload_file(&l, &temp, None).await },
                |from, to| async move { self.rename(&from, &to).await },
                |p| async move { self.delete(&p).await },
            )
            .await
        })
    }

    fn download(&self, remote_path: &str, local_path: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<u64, AppError>> + Send + '_>> {
        let (r, l) = (remote_path.to_string(), local_path.to_string());
        Box::pin(async move { self.download_file(&r, &l, None).await })
    }

    fn make_directory(&self, remote_path: &str) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + '_>> {
        let r = remote_path.to_string();
        Box::pin(async move { self.mkdir(&r).await })
    }

    fn remove(&self, remote_path: &str, _is_dir: bool) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + '_>> {
        let r = remote_path.to_string();
        Box::pin(async move { self.delete(&r).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::Connector;

    #[test]
    fn test_webdav_connector_creation() {
        let connector = WebDavConnector::new();
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_webdav_connector_with_config() {
        let config = WebDavConfig {
            auth_method: WebDavAuthMethod::Basic {
                username: "user".to_string(),
                password: "pass".to_string(),
            },
            base_url: "https://cloud.example.com/remote.php/dav/".to_string(),
            allow_insecure_http: false,
            accept_invalid_certs: false,
            connect_timeout_secs: 30,
            request_timeout_secs: 600,
            chunk_upload_size: 10 * 1024 * 1024,
            chunk_upload_threshold: 50 * 1024 * 1024,
            enable_locking: true,
            lock_timeout_secs: 600,
            user_agent: "TestClient/1.0".to_string(),
            compat_mode: WebDavCompat::Nextcloud,
        };
        let connector = WebDavConnector::with_config(config);
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_webdav_config_default() {
        let config = WebDavConfig::default();
        assert!(!config.allow_insecure_http);
        assert!(!config.accept_invalid_certs);
        assert_eq!(config.connect_timeout_secs, 15);
        assert_eq!(config.request_timeout_secs, 300);
        assert_eq!(config.chunk_upload_size, 5 * 1024 * 1024);
        assert_eq!(config.chunk_upload_threshold, 10 * 1024 * 1024);
        assert!(!config.enable_locking);
        assert_eq!(config.compat_mode, WebDavCompat::Standard);
    }

    #[test]
    fn test_validate_url_https() {
        let result = WebDavConnector::validate_url("https://example.com/dav/", false);
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "https://example.com/dav");
    }

    #[test]
    fn test_validate_url_http_blocked() {
        let result = WebDavConnector::validate_url("http://example.com/dav/", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_url_http_allowed() {
        let result = WebDavConnector::validate_url("http://example.com/dav/", true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_url_invalid() {
        let result = WebDavConnector::validate_url("ftp://example.com/", false);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_url() {
        assert_eq!(
            WebDavConnector::build_url("https://example.com/dav", "files/test.txt"),
            "https://example.com/dav/files/test.txt"
        );
        assert_eq!(
            WebDavConnector::build_url("https://example.com/dav/", "/files/test.txt"),
            "https://example.com/dav/files/test.txt"
        );
        assert_eq!(
            WebDavConnector::build_url("https://example.com/dav", ""),
            "https://example.com/dav/"
        );
    }

    #[test]
    fn test_build_auth_header_basic() {
        let auth = WebDavAuthMethod::Basic {
            username: "user".to_string(),
            password: "pass".to_string(),
        };
        let header = WebDavConnector::build_auth_header(&auth);
        assert!(header.is_some());
        let (key, value) = header.unwrap();
        assert_eq!(key, "Authorization");
        assert!(value.starts_with("Basic "));
    }

    #[test]
    fn test_build_auth_header_oauth() {
        let auth = WebDavAuthMethod::OAuth {
            access_token: "my_token_123".to_string(),
            refresh_token: None,
            token_endpoint: None,
        };
        let header = WebDavConnector::build_auth_header(&auth);
        assert!(header.is_some());
        let (key, value) = header.unwrap();
        assert_eq!(key, "Authorization");
        assert_eq!(value, "Bearer my_token_123");
    }

    #[test]
    fn test_build_auth_header_none() {
        let auth = WebDavAuthMethod::None;
        let header = WebDavConnector::build_auth_header(&auth);
        assert!(header.is_none());
    }

    #[test]
    fn test_extract_xml_value() {
        let xml = r#"<d:response><d:href>/dav/file.txt</d:href><d:getcontentlength>1024</d:getcontentlength></d:response>"#;
        assert_eq!(
            WebDavConnector::extract_xml_value(xml, "href"),
            Some("/dav/file.txt".to_string())
        );
        assert_eq!(
            WebDavConnector::extract_xml_value(xml, "getcontentlength"),
            Some("1024".to_string())
        );
    }

    #[test]
    fn test_extract_xml_value_not_found() {
        let xml = "<d:response></d:response>";
        assert_eq!(WebDavConnector::extract_xml_value(xml, "href"), None);
    }

    #[test]
    fn test_parse_propfind_apache_style_with_attributes_and_custom_prefixes() {
        // Apache mod_dav: attributes on <response>, props under an `lp1:` prefix.
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<D:multistatus xmlns:D="DAV:">
<D:response xmlns:lp1="DAV:" xmlns:lp2="http://apache.org/dav/props/">
<D:href>/dir/</D:href>
<D:propstat><D:prop><lp1:resourcetype><D:collection/></lp1:resourcetype>
<lp1:getlastmodified>Mon, 12 Oct 2026 00:04:52 GMT</lp1:getlastmodified></D:prop></D:propstat>
</D:response>
<D:response xmlns:lp1="DAV:">
<D:href>/dir/a%20b.bin</D:href>
<D:propstat><D:prop><lp1:resourcetype/><lp1:getcontentlength>3000000</lp1:getcontentlength>
<lp1:getetag>"x"</lp1:getetag></D:prop></D:propstat>
</D:response>
</D:multistatus>"#;
        let entries = WebDavConnector::parse_propfind_response(xml, "http://h/dir");
        assert_eq!(entries.len(), 2);
        assert!(entries[0].is_collection);
        assert!(entries[0].last_modified.is_some());
        assert!(!entries[1].is_collection);
        assert_eq!(entries[1].content_length, 3_000_000);
        assert_eq!(entries[1].href, "/dir/a%20b.bin");
    }

    #[test]
    fn test_parse_propfind_response() {
        let xml = r#"<?xml version="1.0" encoding="utf-8"?>
<d:multistatus xmlns:d="DAV:">
  <d:response>
    <d:href>/dav/</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype><d:collection/></d:resourcetype>
      </d:prop>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/file.txt</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype/>
        <d:getcontentlength>1024</d:getcontentlength>
      </d:prop>
    </d:propstat>
  </d:response>
  <d:response>
    <d:href>/dav/subdir/</d:href>
    <d:propstat>
      <d:prop>
        <d:resourcetype><d:collection/></d:resourcetype>
      </d:prop>
    </d:propstat>
  </d:response>
</d:multistatus>"#;

        let entries = WebDavConnector::parse_propfind_response(xml, "https://example.com/dav");
        assert_eq!(entries.len(), 3);

        // First entry is the directory itself
        assert!(entries[0].is_collection);
        assert_eq!(entries[0].href, "/dav/");

        // Second is a file
        assert!(!entries[1].is_collection);
        assert_eq!(entries[1].content_length, 1024);

        // Third is a subdirectory
        assert!(entries[2].is_collection);
    }

    #[test]
    fn test_dav_entry_to_file_entry() {
        let entry = DavEntry {
            href: "/dav/files/document.pdf".to_string(),
            is_collection: false,
            content_length: 2048,
            last_modified: Some(Utc::now()),
            created: None,
            etag: Some("\"abc123\"".to_string()),
            content_type: Some("application/pdf".to_string()),
        };

        let file_entry = WebDavConnector::dav_entry_to_file_entry(&entry, "https://example.com/dav");
        assert_eq!(file_entry.name, "document.pdf");
        assert!(!file_entry.is_dir);
        assert_eq!(file_entry.size, 2048);
        assert_eq!(file_entry.extension, Some("pdf".to_string()));
    }

    #[test]
    fn test_dav_entry_to_file_entry_directory() {
        let entry = DavEntry {
            href: "/dav/files/Documents/".to_string(),
            is_collection: true,
            content_length: 0,
            last_modified: None,
            created: None,
            etag: None,
            content_type: None,
        };

        let file_entry = WebDavConnector::dav_entry_to_file_entry(&entry, "https://example.com/dav");
        assert_eq!(file_entry.name, "Documents");
        assert!(file_entry.is_dir);
        assert!(file_entry.extension.is_none());
    }

    #[test]
    fn test_dav_entry_to_file_entry_hidden() {
        let entry = DavEntry {
            href: "/dav/.hidden_file".to_string(),
            is_collection: false,
            content_length: 100,
            last_modified: None,
            created: None,
            etag: None,
            content_type: None,
        };

        let file_entry = WebDavConnector::dav_entry_to_file_entry(&entry, "https://example.com/dav");
        assert!(file_entry.is_hidden);
    }

    #[test]
    fn test_webdav_auth_method_serialization() {
        let methods: Vec<WebDavAuthMethod> = vec![
            WebDavAuthMethod::Basic {
                username: "user".to_string(),
                password: "pass".to_string(),
            },
            WebDavAuthMethod::Digest {
                username: "user".to_string(),
                password: "pass".to_string(),
            },
            WebDavAuthMethod::OAuth {
                access_token: "token".to_string(),
                refresh_token: Some("refresh".to_string()),
                token_endpoint: Some("https://auth.example.com/token".to_string()),
            },
            WebDavAuthMethod::None,
        ];
        for method in &methods {
            let json = serde_json::to_string(method).unwrap();
            let _: WebDavAuthMethod = serde_json::from_str(&json).unwrap();
        }
    }

    #[test]
    fn test_webdav_compat_serialization() {
        let modes = [
            WebDavCompat::Standard,
            WebDavCompat::Nextcloud,
            WebDavCompat::OwnCloud,
        ];
        for mode in &modes {
            let json = serde_json::to_string(mode).unwrap();
            let deserialized: WebDavCompat = serde_json::from_str(&json).unwrap();
            assert_eq!(*mode, deserialized);
        }
    }

    #[test]
    fn test_webdav_lock_serialization() {
        let lock = WebDavLock {
            token: "opaquelocktoken:abc123".to_string(),
            path: "/files/document.txt".to_string(),
            owner: "user".to_string(),
            timeout_secs: 300,
            created_at: Utc::now(),
        };
        let json = serde_json::to_string(&lock).unwrap();
        let deserialized: WebDavLock = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.token, lock.token);
        assert_eq!(deserialized.path, lock.path);
    }

    #[test]
    fn test_webdav_health_status_serialization() {
        let status = WebDavHealthStatus {
            connected: true,
            https_active: true,
            server_type: Some("Apache/2.4".to_string()),
            bytes_sent: 1024,
            bytes_received: 2048,
            base_url: "https://example.com/dav".to_string(),
            compat_mode: WebDavCompat::Nextcloud,
        };
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: WebDavHealthStatus = serde_json::from_str(&json).unwrap();
        assert!(deserialized.connected);
        assert_eq!(deserialized.compat_mode, WebDavCompat::Nextcloud);
    }

    #[tokio::test]
    async fn test_health_status_disconnected() {
        let connector = WebDavConnector::new();
        let status = connector.health_status().await;
        assert!(!status.connected);
        assert_eq!(status.bytes_sent, 0);
        assert_eq!(status.bytes_received, 0);
    }

    #[tokio::test]
    async fn test_set_config() {
        let connector = WebDavConnector::new();
        let new_config = WebDavConfig {
            compat_mode: WebDavCompat::Nextcloud,
            ..WebDavConfig::default()
        };
        connector.set_config(new_config).await;

        let config = connector.config.read().await;
        assert_eq!(config.compat_mode, WebDavCompat::Nextcloud);
    }

    #[tokio::test]
    async fn test_disconnect_when_not_connected() {
        let connector = WebDavConnector::new();
        use super::super::Connector;
        let result = connector.disconnect().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_directory_without_connection() {
        let connector = WebDavConnector::new();
        let result = connector.list_directory("/files").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_upload_without_connection() {
        let connector = WebDavConnector::new();
        let result = connector
            .upload_file("/local/file.txt", "/remote/file.txt", None)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_download_without_connection() {
        let connector = WebDavConnector::new();
        let result = connector
            .download_file("/remote/file.txt", "/local/file.txt", None)
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mkdir_without_connection() {
        let connector = WebDavConnector::new();
        let result = connector.mkdir("/new/dir").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_without_connection() {
        let connector = WebDavConnector::new();
        let result = connector.delete("/some/file.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rename_without_connection() {
        let connector = WebDavConnector::new();
        let result = connector.rename("/old", "/new").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_copy_without_connection() {
        let connector = WebDavConnector::new();
        let result = connector.copy("/source", "/dest").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_lock_without_connection() {
        let connector = WebDavConnector::new();
        let result = connector.lock("/some/file.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_unlock_without_lock() {
        let connector = WebDavConnector::new();
        let result = connector.unlock("/some/file.txt").await;
        assert!(result.is_err()); // No active lock
    }

    #[tokio::test]
    async fn test_active_locks_empty() {
        let connector = WebDavConnector::new();
        let locks = connector.active_locks().await;
        assert!(locks.is_empty());
    }

    #[test]
    fn test_webdav_config_serialization() {
        let config = WebDavConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: WebDavConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.compat_mode, WebDavCompat::Standard);
        assert!(!deserialized.allow_insecure_http);
    }
}
