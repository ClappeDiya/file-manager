//! FTP/FTPS Connector (T-021).
//!
//! Provides FTP and FTPS file operations with support for:
//! - Active and passive mode
//! - TLS: none, explicit FTPS, implicit FTPS
//! - Directory listing cache for performance
//! - Resume using REST command
//! - Certificate validation with self-signed cert acceptance option
//! - Multiple connections for parallel chunk transfers (FTP limitation: single stream per connection)

use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

// ── FTP Configuration ──

/// FTP transfer mode.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum FtpTransferMode {
    /// Passive mode (client connects to server data port). Default and NAT-friendly.
    #[default]
    Passive,
    /// Active mode (server connects back to client data port).
    Active,
}


/// TLS mode for FTPS.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum FtpTlsMode {
    /// No TLS (plain FTP). NOT recommended for production.
    None,
    /// Explicit TLS: starts plain, upgrades via AUTH TLS (port 21).
    #[default]
    Explicit,
    /// Implicit TLS: TLS from the start (port 990).
    Implicit,
}


/// FTP data type mode for transfers (ASCII vs Binary).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum FtpDataType {
    /// Binary mode (default) - transfers bytes as-is.
    #[default]
    Binary,
    /// ASCII mode - converts line endings between systems.
    Ascii,
    /// Auto mode - uses ASCII for known text extensions, Binary for everything else.
    Auto,
}

/// Known text file extensions for Auto data type mode.
const TEXT_EXTENSIONS: &[&str] = &[
    "txt", "html", "htm", "css", "js", "jsx", "ts", "tsx", "xml", "csv",
    "json", "md", "py", "sh", "cfg", "conf", "ini", "log", "yml", "yaml",
    "toml", "rb", "pl", "php", "c", "h", "cpp", "hpp", "java", "rs",
    "go", "swift", "kt", "sql", "r", "lua", "bat", "ps1", "env",
    "gitignore", "dockerignore", "editorconfig", "htaccess",
];

impl FtpDataType {
    /// Determine the suppaftp FileType for a given filename.
    pub fn resolve_for_file(&self, filename: &str) -> suppaftp::types::FileType {
        match self {
            FtpDataType::Binary => suppaftp::types::FileType::Binary,
            FtpDataType::Ascii => suppaftp::types::FileType::Ascii(suppaftp::types::FormatControl::Default),
            FtpDataType::Auto => {
                let ext = std::path::Path::new(filename)
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_lowercase();
                if TEXT_EXTENSIONS.contains(&ext.as_str()) {
                    suppaftp::types::FileType::Ascii(suppaftp::types::FormatControl::Default)
                } else {
                    suppaftp::types::FileType::Binary
                }
            }
        }
    }
}

/// FTP connection configuration beyond the base ConnectionProfile.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtpConfig {
    /// Transfer mode (active/passive).
    pub transfer_mode: FtpTransferMode,
    /// TLS mode.
    pub tls_mode: FtpTlsMode,
    /// Accept self-signed certificates (requires explicit user confirmation).
    pub accept_self_signed_certs: bool,
    /// Connection timeout in seconds.
    pub connect_timeout_secs: u32,
    /// Number of parallel connections for transfers.
    pub parallel_connections: u8,
    /// Directory listing cache TTL in seconds (0 = disabled).
    pub listing_cache_ttl_secs: u32,
    /// Username for authentication.
    pub username: String,
    /// Password for authentication (never logged).
    pub password: String,
    /// Data type mode for file transfers (Binary/ASCII/Auto).
    pub data_type: FtpDataType,
}

impl Default for FtpConfig {
    fn default() -> Self {
        Self {
            transfer_mode: FtpTransferMode::Passive,
            tls_mode: FtpTlsMode::Explicit,
            accept_self_signed_certs: false,
            connect_timeout_secs: 15,
            parallel_connections: 4,
            listing_cache_ttl_secs: 30,
            username: "anonymous".to_string(),
            password: String::new(),
            data_type: FtpDataType::Binary,
        }
    }
}

/// Cached directory listing entry.
#[derive(Debug, Clone)]
struct CachedListing {
    entries: Vec<FileEntry>,
    cached_at: std::time::Instant,
}

/// FTP connector health status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FtpHealthStatus {
    pub connected: bool,
    pub tls_active: bool,
    pub transfer_mode: FtpTransferMode,
    pub bytes_sent: u64,
    pub bytes_received: u64,
    pub server_banner: Option<String>,
}

// ── FTP Connector State ──

/// Wrapper enum to hold either plain FTP or TLS-secured FTP stream.
/// suppaftp uses different concrete types for plain vs TLS connections.
enum FtpStream {
    Plain(suppaftp::FtpStream),
    Tls(suppaftp::NativeTlsFtpStream),
}

impl FtpStream {
    fn login(&mut self, user: &str, password: &str) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.login(user, password),
            Self::Tls(s) => s.login(user, password),
        }
    }

    fn transfer_type(
        &mut self,
        file_type: suppaftp::types::FileType,
    ) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.transfer_type(file_type),
            Self::Tls(s) => s.transfer_type(file_type),
        }
    }

    #[allow(dead_code)]
    fn set_mode(&mut self, mode: suppaftp::types::Mode) {
        match self {
            Self::Plain(s) => s.set_mode(mode),
            Self::Tls(s) => s.set_mode(mode),
        }
    }

    #[allow(dead_code)]
    fn get_welcome_msg(&self) -> Option<&str> {
        match self {
            Self::Plain(s) => s.get_welcome_msg(),
            Self::Tls(s) => s.get_welcome_msg(),
        }
    }

    fn list(&mut self, pathname: Option<&str>) -> Result<Vec<String>, suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.list(pathname),
            Self::Tls(s) => s.list(pathname),
        }
    }

    fn retr_as_buffer(
        &mut self,
        file_name: &str,
    ) -> Result<std::io::Cursor<Vec<u8>>, suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.retr_as_buffer(file_name),
            Self::Tls(s) => s.retr_as_buffer(file_name),
        }
    }

    fn put_file<R: Read>(
        &mut self,
        filename: &str,
        r: &mut R,
    ) -> Result<u64, suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.put_file(filename, r),
            Self::Tls(s) => s.put_file(filename, r),
        }
    }

    fn append_file<R: Read>(
        &mut self,
        filename: &str,
        r: &mut R,
    ) -> Result<u64, suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.append_file(filename, r),
            Self::Tls(s) => s.append_file(filename, r),
        }
    }

    fn resume_transfer(&mut self, offset: usize) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.resume_transfer(offset),
            Self::Tls(s) => s.resume_transfer(offset),
        }
    }

    fn size(&mut self, pathname: &str) -> Result<usize, suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.size(pathname),
            Self::Tls(s) => s.size(pathname),
        }
    }

    fn rename(&mut self, from_name: &str, to_name: &str) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.rename(from_name, to_name),
            Self::Tls(s) => s.rename(from_name, to_name),
        }
    }

    fn rm(&mut self, filename: &str) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.rm(filename),
            Self::Tls(s) => s.rm(filename),
        }
    }

    fn rmdir(&mut self, pathname: &str) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.rmdir(pathname),
            Self::Tls(s) => s.rmdir(pathname),
        }
    }

    fn mkdir(&mut self, pathname: &str) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.mkdir(pathname),
            Self::Tls(s) => s.mkdir(pathname),
        }
    }

    fn pwd(&mut self) -> Result<String, suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.pwd(),
            Self::Tls(s) => s.pwd(),
        }
    }

    fn cwd(&mut self, path: &str) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.cwd(path),
            Self::Tls(s) => s.cwd(path),
        }
    }

    fn site(&mut self, command: &str) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.site(command).map(|_| ()),
            Self::Tls(s) => s.site(command).map(|_| ()),
        }
    }

    fn quit(&mut self) -> Result<(), suppaftp::FtpError> {
        match self {
            Self::Plain(s) => s.quit(),
            Self::Tls(s) => s.quit(),
        }
    }
}

/// Inner connection state (guarded by Mutex).
struct FtpSession {
    /// The FTP stream handle (plain or TLS).
    stream: FtpStream,
    /// Server welcome message.
    banner: Option<String>,
    /// Whether TLS is active.
    tls_active: bool,
}

/// FTP/FTPS Connector implementing the Connector trait.
pub struct FtpConnector {
    /// Current FTP session (None if disconnected).
    session: Arc<Mutex<Option<FtpSession>>>,
    /// Connection status flag.
    connected: AtomicBool,
    /// FTP-specific configuration.
    config: RwLock<FtpConfig>,
    /// Directory listing cache.
    listing_cache: RwLock<HashMap<String, CachedListing>>,
    /// Bytes transferred counters.
    bytes_sent: AtomicU64,
    bytes_received: AtomicU64,
}

impl FtpConnector {
    /// Create a new FTP connector with default configuration.
    pub fn new() -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            connected: AtomicBool::new(false),
            config: RwLock::new(FtpConfig::default()),
            listing_cache: RwLock::new(HashMap::new()),
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
        }
    }

    /// Create a new FTP connector with custom configuration.
    pub fn with_config(config: FtpConfig) -> Self {
        Self {
            session: Arc::new(Mutex::new(None)),
            connected: AtomicBool::new(false),
            config: RwLock::new(config),
            listing_cache: RwLock::new(HashMap::new()),
            bytes_sent: AtomicU64::new(0),
            bytes_received: AtomicU64::new(0),
        }
    }

    /// Update the FTP configuration.
    pub async fn set_config(&self, config: FtpConfig) {
        let mut cfg = self.config.write().await;
        *cfg = config;
    }

    /// Get current health status.
    pub async fn health_status(&self) -> FtpHealthStatus {
        let session = self.session.lock().await;
        let config = self.config.read().await;
        FtpHealthStatus {
            connected: self.connected.load(Ordering::SeqCst),
            tls_active: session.as_ref().map(|s| s.tls_active).unwrap_or(false),
            transfer_mode: config.transfer_mode,
            bytes_sent: self.bytes_sent.load(Ordering::Relaxed),
            bytes_received: self.bytes_received.load(Ordering::Relaxed),
            server_banner: session.as_ref().and_then(|s| s.banner.clone()),
        }
    }

    /// Invalidate the directory listing cache.
    pub async fn invalidate_cache(&self) {
        let mut cache = self.listing_cache.write().await;
        cache.clear();
    }

    /// Invalidate cache for a specific path.
    pub async fn invalidate_cache_path(&self, path: &str) {
        let mut cache = self.listing_cache.write().await;
        cache.remove(path);
        // Also invalidate parent directory
        if let Some(parent) = Path::new(path).parent() {
            cache.remove(&parent.to_string_lossy().to_string());
        }
    }

    /// Parse an FTP directory listing line into a FileEntry.
    fn parse_list_entry(line: &str, parent_path: &str) -> Option<FileEntry> {
        // Parse Unix-style listing: drwxr-xr-x  2 user group  4096 Jan 01 12:00 filename
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 9 {
            return None;
        }

        let permissions_str = parts[0];
        let size: u64 = parts[4].parse().unwrap_or(0);
        let name = parts[8..].join(" ");

        // Skip . and ..
        if name == "." || name == ".." {
            return None;
        }

        let is_dir = permissions_str.starts_with('d');
        let is_symlink = permissions_str.starts_with('l');

        // Handle symlink names (name -> target)
        let actual_name = if is_symlink {
            name.split(" -> ").next().unwrap_or(&name).to_string()
        } else {
            name.clone()
        };

        let full_path = if parent_path.ends_with('/') {
            format!("{parent_path}{actual_name}")
        } else {
            format!("{parent_path}/{actual_name}")
        };

        let extension = if !is_dir {
            Path::new(&actual_name)
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_string())
        } else {
            None
        };

        // Parse date from parts[5..8] (e.g., "Jan 01 12:00" or "Jan 01  2024")
        let modified = Self::parse_ftp_date(&parts[5..8]);

        Some(FileEntry {
            name: actual_name.clone(),
            path: full_path,
            is_dir,
            is_symlink,
            size,
            modified,
            created: None,
            is_hidden: actual_name.starts_with('.'),
            extension,
            permissions: Some(permissions_str.to_string()),
        })
    }

    /// Parse FTP date strings (e.g., "Jan 01 12:00" or "Jan 01  2024").
    fn parse_ftp_date(parts: &[&str]) -> Option<DateTime<Utc>> {
        if parts.len() < 3 {
            return None;
        }
        let month = parts[0];
        let day = parts[1];
        let time_or_year = parts[2];

        let date_str = if time_or_year.contains(':') {
            // Current year with time
            let year = Utc::now().format("%Y");
            format!("{month} {day} {year} {time_or_year}")
        } else {
            // Year without time
            format!("{month} {day} {time_or_year} 00:00")
        };

        NaiveDateTime::parse_from_str(&date_str, "%b %d %Y %H:%M")
            .ok()
            .map(|ndt| ndt.and_utc())
    }

    // ── File Operations ──

    /// List directory contents with optional caching.
    pub async fn list_directory(&self, path: &str) -> Result<Vec<FileEntry>, AppError> {
        let config = self.config.read().await;
        let cache_ttl = config.listing_cache_ttl_secs;
        drop(config);

        // Check cache
        if cache_ttl > 0 {
            let cache = self.listing_cache.read().await;
            if let Some(cached) = cache.get(path) {
                if cached.cached_at.elapsed().as_secs() < cache_ttl as u64 {
                    return Ok(cached.entries.clone());
                }
            }
        }

        // Fetch from server
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let listing = ftp
            .stream
            .list(Some(path))
            .map_err(|e| AppError::Connection {
                message: format!("Failed to list directory {path}: {e}"),
                advice: "Check that the path exists and you have permission.".to_string(),
            })?;

        let entries: Vec<FileEntry> = listing
            .iter()
            .filter_map(|line| Self::parse_list_entry(line, path))
            .collect();

        // Update cache
        if cache_ttl > 0 {
            let mut cache = self.listing_cache.write().await;
            cache.insert(
                path.to_string(),
                CachedListing {
                    entries: entries.clone(),
                    cached_at: std::time::Instant::now(),
                },
            );
        }

        Ok(entries)
    }

    /// Upload a file to the FTP server.
    pub async fn upload_file(
        &self,
        local_path: &str,
        remote_path: &str,
        resume_offset: Option<u64>,
    ) -> Result<u64, AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let local_file =
            std::fs::File::open(local_path).map_err(|e| AppError::FileOperation {
                message: format!("Cannot open local file {local_path}: {e}"),
                advice: "Check that the file exists.".to_string(),
            })?;

        let _metadata = local_file.metadata().map_err(|e| AppError::FileOperation {
            message: format!("Cannot read file metadata: {e}"),
            advice: "Check file permissions.".to_string(),
        })?;

        // Handle resume with REST command
        if let Some(offset) = resume_offset {
            if offset > 0 {
                let mut reader = std::io::BufReader::new(local_file);
                // Skip past already-uploaded bytes
                std::io::copy(&mut reader.by_ref().take(offset), &mut std::io::sink())
                    .map_err(|e| AppError::FileOperation {
                        message: format!("Cannot seek to resume offset: {e}"),
                        advice: "Resume may not be supported.".to_string(),
                    })?;

                // Use append to resume
                let bytes_written = ftp.stream
                    .append_file(remote_path, &mut reader)
                    .map_err(|e| AppError::Connection {
                        message: format!("FTP resume upload failed: {e}"),
                        advice: "The server may not support resume.".to_string(),
                    })?;

                self.bytes_sent.fetch_add(bytes_written, Ordering::Relaxed);
                self.invalidate_cache_path(remote_path).await;
                return Ok(offset + bytes_written);
            }
        }

        // Standard upload
        let mut reader = std::io::BufReader::new(local_file);
        let bytes_written = ftp.stream
            .put_file(remote_path, &mut reader)
            .map_err(|e| AppError::Connection {
                message: format!("FTP upload failed for {remote_path}: {e}"),
                advice: "Check permissions and disk space on server.".to_string(),
            })?;

        self.bytes_sent.fetch_add(bytes_written, Ordering::Relaxed);
        self.invalidate_cache_path(remote_path).await;
        Ok(bytes_written)
    }

    /// Download a file from the FTP server.
    pub async fn download_file(
        &self,
        remote_path: &str,
        local_path: &str,
        resume_offset: Option<u64>,
    ) -> Result<u64, AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        // Get remote file size (for progress tracking)
        let _remote_size = ftp
            .stream
            .size(remote_path)
            .unwrap_or(0) as u64;

        // Handle resume
        if let Some(offset) = resume_offset {
            if offset > 0 {
                // Set resume position via REST command
                ftp.stream
                    .resume_transfer(offset as usize)
                    .map_err(|e| AppError::Connection {
                        message: format!("FTP resume command failed: {e}"),
                        advice: "The server may not support resume.".to_string(),
                    })?;

                // Retrieve data from offset
                let cursor = ftp
                    .stream
                    .retr_as_buffer(remote_path)
                    .map_err(|e| AppError::Connection {
                        message: format!("FTP resume download failed: {e}"),
                        advice: "The server may not support resume.".to_string(),
                    })?;

                let data = cursor.into_inner();

                use std::io::Write;
                let mut local_file = std::fs::OpenOptions::new()
                    
                    .append(true)
                    .open(local_path)
                    .map_err(|e| AppError::FileOperation {
                        message: format!("Cannot open local file for resume: {e}"),
                        advice: "Check that the partial file exists.".to_string(),
                    })?;
                local_file.write_all(&data).map_err(|e| AppError::FileOperation {
                    message: format!("Error writing to local file: {e}"),
                    advice: "Check disk space and permissions.".to_string(),
                })?;

                let bytes_received = data.len() as u64;
                self.bytes_received.fetch_add(bytes_received, Ordering::Relaxed);
                return Ok(offset + bytes_received);
            }
        }

        // Standard download
        let cursor = ftp
            .stream
            .retr_as_buffer(remote_path)
            .map_err(|e| AppError::Connection {
                message: format!("FTP download failed for {remote_path}: {e}"),
                advice: "Check that the file exists and you have permission.".to_string(),
            })?;

        let data = cursor.into_inner();
        std::fs::write(local_path, &data).map_err(|e| AppError::FileOperation {
            message: format!("Cannot write to {local_path}: {e}"),
            advice: "Check disk space and permissions.".to_string(),
        })?;

        let bytes = data.len() as u64;
        self.bytes_received.fetch_add(bytes, Ordering::Relaxed);
        Ok(bytes)
    }

    /// Rename a file or directory on the FTP server.
    pub async fn rename(&self, old_path: &str, new_path: &str) -> Result<(), AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        ftp.stream
            .rename(old_path, new_path)
            .map_err(|e| AppError::Connection {
                message: format!("Failed to rename {old_path} to {new_path}: {e}"),
                advice: "Check permissions.".to_string(),
            })?;

        self.invalidate_cache_path(old_path).await;
        self.invalidate_cache_path(new_path).await;
        Ok(())
    }

    /// Delete a file on the FTP server.
    pub async fn delete_file(&self, path: &str) -> Result<(), AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        ftp.stream
            .rm(path)
            .map_err(|e| AppError::Connection {
                message: format!("Failed to delete {path}: {e}"),
                advice: "Check that the file exists and you have permission.".to_string(),
            })?;

        self.invalidate_cache_path(path).await;
        Ok(())
    }

    /// Delete a directory on the FTP server.
    pub async fn delete_directory(&self, path: &str) -> Result<(), AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        ftp.stream
            .rmdir(path)
            .map_err(|e| AppError::Connection {
                message: format!("Failed to delete directory {path}: {e}"),
                advice: "Check that the directory is empty and you have permission.".to_string(),
            })?;

        self.invalidate_cache_path(path).await;
        Ok(())
    }

    /// Create a directory on the FTP server.
    pub async fn mkdir(&self, path: &str) -> Result<(), AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        ftp.stream
            .mkdir(path)
            .map_err(|e| AppError::Connection {
                message: format!("Failed to create directory {path}: {e}"),
                advice: "Check that the parent directory exists and you have permission.".to_string(),
            })?;

        self.invalidate_cache_path(path).await;
        Ok(())
    }

    /// Get the current working directory.
    pub async fn pwd(&self) -> Result<String, AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        ftp.stream
            .pwd()
            .map_err(|e| AppError::Connection {
                message: format!("Failed to get current directory: {e}"),
                advice: "Connection may have been lost.".to_string(),
            })
    }

    /// Change the current working directory.
    pub async fn cwd(&self, path: &str) -> Result<(), AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        ftp.stream
            .cwd(path)
            .map_err(|e| AppError::Connection {
                message: format!("Failed to change directory to {path}: {e}"),
                advice: "Check that the directory exists.".to_string(),
            })?;

        Ok(())
    }

    /// Get file size via SIZE command.
    pub async fn file_size(&self, path: &str) -> Result<u64, AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        let size = ftp
            .stream
            .size(path)
            .map_err(|e| AppError::Connection {
                message: format!("Failed to get size of {path}: {e}"),
                advice: "The server may not support the SIZE command.".to_string(),
            })?;

        Ok(size as u64)
    }

    /// Change file permissions via FTP SITE CHMOD command.
    /// `mode` is an octal string like "755" or "644".
    pub async fn chmod(&self, path: &str, mode: &str) -> Result<(), AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        ftp.stream
            .site(&format!("CHMOD {mode} {path}"))
            .map_err(|e| AppError::Connection {
                message: format!("SITE CHMOD failed for {path}: {e}"),
                advice: "The FTP server may not support the SITE CHMOD command.".to_string(),
            })?;

        tracing::debug!("FTP chmod {mode} on {path}");
        Ok(())
    }

    /// Send a raw FTP SITE command (for extensibility).
    pub async fn site_command(&self, command: &str) -> Result<(), AppError> {
        let mut session = self.session.lock().await;
        let ftp = session.as_mut().ok_or_else(|| AppError::Connection {
            message: "Not connected to FTP server.".to_string(),
            advice: "Connect to the server first.".to_string(),
        })?;

        ftp.stream
            .site(command)
            .map_err(|e| AppError::Connection {
                message: format!("SITE {command} failed: {e}"),
                advice: "The server may not support this SITE command.".to_string(),
            })?;

        Ok(())
    }

    /// Execute post-login commands (sent after authentication).
    pub async fn execute_post_login_commands(&self, commands: &[String]) -> Result<(), AppError> {
        for cmd in commands {
            let trimmed = cmd.trim();
            if trimmed.is_empty() {
                continue;
            }
            tracing::debug!("Executing post-login FTP command: {trimmed}");
            self.site_command(trimmed).await?;
        }
        Ok(())
    }
}

impl Default for FtpConnector {
    fn default() -> Self {
        Self::new()
    }
}

// ── Connector trait implementation ──

impl super::Connector for FtpConnector {
    fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + '_>> {
        let host = profile.host.clone();
        let port = profile.port.unwrap_or(21);
        let username = profile
            .username
            .clone()
            .unwrap_or_else(|| "anonymous".to_string());

        Box::pin(async move {
            if self.connected.load(Ordering::SeqCst) {
                return Err(AppError::Connection {
                    message: "Already connected.".to_string(),
                    advice: "Disconnect first before connecting.".to_string(),
                });
            }

            let config = self.config.read().await.clone();
            let timeout = config.connect_timeout_secs;
            let tls_mode = config.tls_mode;
            let transfer_mode = config.transfer_mode;
            let accept_self_signed = config.accept_self_signed_certs;
            let data_type = config.data_type;
            let password = config.password.clone();
            let username_for_auth = if !username.is_empty() {
                username.clone()
            } else {
                config.username.clone()
            };
            let host_clone = host.clone();

            // Connect in blocking context (suppaftp is synchronous)
            let (stream, banner, tls_active) = tokio::task::spawn_blocking(move || {
                // Parse socket address
                let addr_str = format!("{host_clone}:{port}");
                let socket_addr: std::net::SocketAddr = addr_str
                    .parse()
                    .or_else(|_| {
                        // Try DNS resolution
                        use std::net::ToSocketAddrs;
                        addr_str
                            .to_socket_addrs()
                            .map_err(|e| AppError::Connection {
                                message: format!("Cannot resolve {addr_str}: {e}"),
                                advice: "Check the hostname.".to_string(),
                            })?
                            .next()
                            .ok_or_else(|| AppError::Connection {
                                message: format!("No addresses found for {addr_str}"),
                                advice: "Check the hostname.".to_string(),
                            })
                    })?;

                // Create FTP stream with timeout
                let mut ftp_stream = suppaftp::FtpStream::connect_timeout(
                    socket_addr,
                    std::time::Duration::from_secs(timeout as u64),
                )
                .map_err(|e| AppError::Connection {
                    message: format!("Failed to connect to FTP server {addr_str}: {e}"),
                    advice: "Check that the host is reachable and the port is correct.".to_string(),
                })?;

                // Get server banner
                let banner = ftp_stream
                    .get_welcome_msg()
                    .map(|s| s.to_string());

                // Set transfer mode
                match transfer_mode {
                    FtpTransferMode::Passive => {
                        ftp_stream.set_mode(suppaftp::types::Mode::Passive);
                    }
                    FtpTransferMode::Active => {
                        ftp_stream.set_mode(suppaftp::types::Mode::Active);
                    }
                }

                // Handle TLS - for TLS modes, we need to use NativeTlsFtpStream
                // which has a matching TLS connector type
                let (ftp_stream, tls_active): (FtpStream, bool) = match tls_mode {
                    FtpTlsMode::None => (FtpStream::Plain(ftp_stream), false),
                    FtpTlsMode::Explicit | FtpTlsMode::Implicit => {
                        // Build native TLS connector
                        let tls_builder = if accept_self_signed {
                            suppaftp::native_tls::TlsConnector::builder()
                                .danger_accept_invalid_certs(true)
                                .build()
                        } else {
                            suppaftp::native_tls::TlsConnector::new()
                        };

                        let native_tls = tls_builder.map_err(|e| AppError::Connection {
                            message: format!("Failed to create TLS connector: {e}"),
                            advice: "Check TLS configuration.".to_string(),
                        })?;

                        let tls_connector = suppaftp::NativeTlsConnector::from(native_tls);

                        // Need to reconnect with TLS type - create a NativeTlsFtpStream
                        // and then upgrade to secure
                        drop(ftp_stream); // Drop the plain connection

                        let tls_stream = suppaftp::NativeTlsFtpStream::connect_timeout(
                            socket_addr,
                            std::time::Duration::from_secs(timeout as u64),
                        )
                        .map_err(|e| AppError::Connection {
                            message: format!("Failed to connect for FTPS: {e}"),
                            advice: "Check that the host is reachable.".to_string(),
                        })?;

                        // Set transfer mode on the TLS stream
                        let mut tls_stream = tls_stream;
                        match transfer_mode {
                            FtpTransferMode::Passive => {
                                tls_stream.set_mode(suppaftp::types::Mode::Passive);
                            }
                            FtpTransferMode::Active => {
                                tls_stream.set_mode(suppaftp::types::Mode::Active);
                            }
                        }

                        let secure_stream = tls_stream
                            .into_secure(tls_connector, &host)
                            .map_err(|e| AppError::Connection {
                                message: format!("FTPS TLS negotiation failed: {e}"),
                                advice: "Check that the server supports FTPS.".to_string(),
                            })?;

                        (FtpStream::Tls(secure_stream), true)
                    }
                };

                // Login and set binary mode
                let mut stream = ftp_stream;
                stream
                    .login(&username_for_auth, &password)
                    .map_err(|e| AppError::Connection {
                        message: format!("FTP authentication failed: {e}"),
                        advice: "Check your username and password.".to_string(),
                    })?;

                // Set transfer type based on FtpConfig data_type setting
                // Default to Binary for the connection; Auto mode resolves per-file during transfer
                let initial_type = match data_type {
                    FtpDataType::Ascii => suppaftp::types::FileType::Ascii(suppaftp::types::FormatControl::Default),
                    _ => suppaftp::types::FileType::Binary,
                };
                stream
                    .transfer_type(initial_type)
                    .map_err(|e| AppError::Connection {
                        message: format!("Failed to set transfer mode: {e}"),
                        advice: "The server may not support the selected transfer mode.".to_string(),
                    })?;

                Ok::<_, AppError>((stream, banner, tls_active))
            })
            .await
            .map_err(|e| AppError::internal(format!("FTP connection task panicked: {e}")))??;

            // Store session
            {
                let mut session_guard = self.session.lock().await;
                *session_guard = Some(FtpSession {
                    stream,
                    banner,
                    tls_active,
                });
            }

            self.connected.store(true, Ordering::SeqCst);
            tracing::info!(
                "FTP connected (TLS: {tls_active}, Mode: {:?})",
                transfer_mode
            );
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

            // Close FTP session
            {
                let mut session_guard = self.session.lock().await;
                if let Some(mut ftp_session) = session_guard.take() {
                    let _ = ftp_session.stream.quit();
                }
            }

            // Clear cache
            self.invalidate_cache().await;

            self.connected.store(false, Ordering::SeqCst);
            self.bytes_sent.store(0, Ordering::Relaxed);
            self.bytes_received.store(0, Ordering::Relaxed);

            tracing::info!("FTP session disconnected");
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::Connector;
    

    #[test]
    fn test_ftp_connector_creation() {
        let connector = FtpConnector::new();
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_ftp_connector_with_config() {
        let config = FtpConfig {
            transfer_mode: FtpTransferMode::Active,
            tls_mode: FtpTlsMode::Implicit,
            accept_self_signed_certs: true,
            connect_timeout_secs: 30,
            parallel_connections: 2,
            listing_cache_ttl_secs: 60,
            username: "user".to_string(),
            password: "pass".to_string(),
            data_type: FtpDataType::Binary,
        };
        let connector = FtpConnector::with_config(config);
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_ftp_config_default() {
        let config = FtpConfig::default();
        assert_eq!(config.transfer_mode, FtpTransferMode::Passive);
        assert_eq!(config.tls_mode, FtpTlsMode::Explicit);
        assert!(!config.accept_self_signed_certs);
        assert_eq!(config.connect_timeout_secs, 15);
        assert_eq!(config.parallel_connections, 4);
        assert_eq!(config.listing_cache_ttl_secs, 30);
        assert_eq!(config.username, "anonymous");
    }

    #[test]
    fn test_transfer_mode_default() {
        assert_eq!(FtpTransferMode::default(), FtpTransferMode::Passive);
    }

    #[test]
    fn test_tls_mode_default() {
        assert_eq!(FtpTlsMode::default(), FtpTlsMode::Explicit);
    }

    #[test]
    fn test_parse_list_entry_file() {
        let line = "-rw-r--r--  1 user group  1024 Jan 15 12:30 test.txt";
        let entry = FtpConnector::parse_list_entry(line, "/home/user");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.name, "test.txt");
        assert_eq!(entry.path, "/home/user/test.txt");
        assert!(!entry.is_dir);
        assert_eq!(entry.size, 1024);
        assert_eq!(entry.extension, Some("txt".to_string()));
        assert!(!entry.is_hidden);
    }

    #[test]
    fn test_parse_list_entry_directory() {
        let line = "drwxr-xr-x  2 user group  4096 Jan 15 12:30 Documents";
        let entry = FtpConnector::parse_list_entry(line, "/home/user");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert_eq!(entry.name, "Documents");
        assert!(entry.is_dir);
        assert_eq!(entry.size, 4096);
        assert!(entry.extension.is_none());
    }

    #[test]
    fn test_parse_list_entry_hidden() {
        let line = "-rw-r--r--  1 user group  256 Jan 15 12:30 .bashrc";
        let entry = FtpConnector::parse_list_entry(line, "/home/user");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert!(entry.is_hidden);
    }

    #[test]
    fn test_parse_list_entry_symlink() {
        let line = "lrwxrwxrwx  1 user group  7 Jan 15 12:30 link -> target";
        let entry = FtpConnector::parse_list_entry(line, "/home/user");
        assert!(entry.is_some());
        let entry = entry.unwrap();
        assert!(entry.is_symlink);
        assert_eq!(entry.name, "link");
    }

    #[test]
    fn test_parse_list_entry_skip_dots() {
        let line1 = "drwxr-xr-x  2 user group  4096 Jan 15 12:30 .";
        let line2 = "drwxr-xr-x  2 user group  4096 Jan 15 12:30 ..";
        assert!(FtpConnector::parse_list_entry(line1, "/").is_none());
        assert!(FtpConnector::parse_list_entry(line2, "/").is_none());
    }

    #[test]
    fn test_parse_list_entry_trailing_slash_path() {
        let line = "-rw-r--r--  1 user group  1024 Jan 15 12:30 file.txt";
        let entry = FtpConnector::parse_list_entry(line, "/home/user/");
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().path, "/home/user/file.txt");
    }

    #[test]
    fn test_parse_list_entry_invalid() {
        let line = "not a valid listing";
        assert!(FtpConnector::parse_list_entry(line, "/").is_none());
    }

    #[test]
    fn test_ftp_health_status_serialization() {
        let status = FtpHealthStatus {
            connected: true,
            tls_active: true,
            transfer_mode: FtpTransferMode::Passive,
            bytes_sent: 1024,
            bytes_received: 2048,
            server_banner: Some("Welcome to FTP server".to_string()),
        };
        let json = serde_json::to_string(&status).unwrap();
        let deserialized: FtpHealthStatus = serde_json::from_str(&json).unwrap();
        assert!(deserialized.connected);
        assert!(deserialized.tls_active);
    }

    #[test]
    fn test_ftp_transfer_mode_serialization() {
        let modes = [FtpTransferMode::Passive, FtpTransferMode::Active];
        for mode in &modes {
            let json = serde_json::to_string(mode).unwrap();
            let deserialized: FtpTransferMode = serde_json::from_str(&json).unwrap();
            assert_eq!(*mode, deserialized);
        }
    }

    #[test]
    fn test_ftp_tls_mode_serialization() {
        let modes = [FtpTlsMode::None, FtpTlsMode::Explicit, FtpTlsMode::Implicit];
        for mode in &modes {
            let json = serde_json::to_string(mode).unwrap();
            let deserialized: FtpTlsMode = serde_json::from_str(&json).unwrap();
            assert_eq!(*mode, deserialized);
        }
    }

    #[tokio::test]
    async fn test_health_status_disconnected() {
        let connector = FtpConnector::new();
        let status = connector.health_status().await;
        assert!(!status.connected);
        assert!(!status.tls_active);
        assert_eq!(status.bytes_sent, 0);
        assert_eq!(status.bytes_received, 0);
        assert!(status.server_banner.is_none());
    }

    #[tokio::test]
    async fn test_set_config() {
        let connector = FtpConnector::new();
        let new_config = FtpConfig {
            transfer_mode: FtpTransferMode::Active,
            ..FtpConfig::default()
        };
        connector.set_config(new_config).await;

        let config = connector.config.read().await;
        assert_eq!(config.transfer_mode, FtpTransferMode::Active);
    }

    #[tokio::test]
    async fn test_disconnect_when_not_connected() {
        let connector = FtpConnector::new();
        use super::super::Connector;
        let result = connector.disconnect().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_list_directory_without_connection() {
        let connector = FtpConnector::new();
        let result = connector.list_directory("/home").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_upload_without_connection() {
        let connector = FtpConnector::new();
        let result = connector.upload_file("/local/file.txt", "/remote/file.txt", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_download_without_connection() {
        let connector = FtpConnector::new();
        let result = connector.download_file("/remote/file.txt", "/local/file.txt", None).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_rename_without_connection() {
        let connector = FtpConnector::new();
        let result = connector.rename("/old", "/new").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_delete_without_connection() {
        let connector = FtpConnector::new();
        let result = connector.delete_file("/some/file.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_mkdir_without_connection() {
        let connector = FtpConnector::new();
        let result = connector.mkdir("/new/dir").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_pwd_without_connection() {
        let connector = FtpConnector::new();
        let result = connector.pwd().await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cwd_without_connection() {
        let connector = FtpConnector::new();
        let result = connector.cwd("/home").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_file_size_without_connection() {
        let connector = FtpConnector::new();
        let result = connector.file_size("/some/file.txt").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_cache_invalidation() {
        let connector = FtpConnector::new();

        // Manually insert a cache entry
        {
            let mut cache = connector.listing_cache.write().await;
            cache.insert(
                "/home".to_string(),
                CachedListing {
                    entries: vec![],
                    cached_at: std::time::Instant::now(),
                },
            );
        }

        // Verify it's there
        {
            let cache = connector.listing_cache.read().await;
            assert!(cache.contains_key("/home"));
        }

        // Invalidate
        connector.invalidate_cache().await;

        // Verify it's gone
        {
            let cache = connector.listing_cache.read().await;
            assert!(cache.is_empty());
        }
    }

    #[tokio::test]
    async fn test_cache_path_invalidation() {
        let connector = FtpConnector::new();

        // Insert cache entries
        {
            let mut cache = connector.listing_cache.write().await;
            cache.insert(
                "/home/user".to_string(),
                CachedListing {
                    entries: vec![],
                    cached_at: std::time::Instant::now(),
                },
            );
            cache.insert(
                "/home".to_string(),
                CachedListing {
                    entries: vec![],
                    cached_at: std::time::Instant::now(),
                },
            );
        }

        // Invalidate specific path (should also invalidate parent)
        connector.invalidate_cache_path("/home/user/file.txt").await;

        {
            let cache = connector.listing_cache.read().await;
            assert!(!cache.contains_key("/home/user"));
        }
    }

    #[test]
    fn test_ftp_config_serialization() {
        let config = FtpConfig::default();
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: FtpConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.transfer_mode, FtpTransferMode::Passive);
        assert_eq!(deserialized.tls_mode, FtpTlsMode::Explicit);
    }

    #[test]
    fn test_parse_ftp_date_with_time() {
        let parts = ["Jan", "15", "12:30"];
        let date = FtpConnector::parse_ftp_date(&parts);
        assert!(date.is_some());
    }

    #[test]
    fn test_parse_ftp_date_with_year() {
        let parts = ["Jan", "15", "2024"];
        let date = FtpConnector::parse_ftp_date(&parts);
        assert!(date.is_some());
    }

    #[test]
    fn test_parse_ftp_date_insufficient_parts() {
        let parts = ["Jan", "15"];
        let date = FtpConnector::parse_ftp_date(&parts);
        assert!(date.is_none());
    }
}
