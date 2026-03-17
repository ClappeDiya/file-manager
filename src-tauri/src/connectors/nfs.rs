//! NFS Connector (T-024).
//!
//! Provides NFSv3 and NFSv4 connectivity using system mount commands.
//!
//! Features:
//! - Connection: hostname, export path, NFS version (v3/v4)
//! - Unix permission display (rwxrwxrwx)
//! - Appears as mount point in sidebar under Network
//! - Cross-platform (macOS, Linux; read-only stub on Windows)

use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

// ──────────────────────────────────────────────
// NFS Configuration
// ──────────────────────────────────────────────

/// NFS protocol version.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum NfsVersion {
    /// NFSv3
    V3,
    /// NFSv4
    #[default]
    V4,
    /// NFSv4.1
    V41,
    /// NFSv4.2
    V42,
}

impl NfsVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::V3 => "3",
            Self::V4 => "4",
            Self::V41 => "4.1",
            Self::V42 => "4.2",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "3" | "v3" | "nfs3" => Self::V3,
            "4" | "v4" | "nfs4" => Self::V4,
            "4.1" | "v4.1" | "nfs4.1" => Self::V41,
            "4.2" | "v4.2" | "nfs4.2" => Self::V42,
            _ => Self::V4, // Default to NFSv4
        }
    }

    /// The NFS type string for mount commands.
    pub fn mount_type(&self) -> &'static str {
        match self {
            Self::V3 => "nfs",
            Self::V4 | Self::V41 | Self::V42 => "nfs4",
        }
    }
}


/// NFS mount configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NfsConfig {
    /// NFS version to use.
    pub version: NfsVersion,
    /// Read size in bytes (for performance tuning).
    pub rsize: u32,
    /// Write size in bytes (for performance tuning).
    pub wsize: u32,
    /// Soft mount (returns errors on timeout) vs hard mount (retries indefinitely).
    pub soft: bool,
    /// Timeout in deciseconds (tenths of a second).
    pub timeo: u32,
    /// Number of retries before returning an error.
    pub retrans: u32,
    /// Use TCP (recommended) vs UDP.
    pub tcp: bool,
    /// Mount read-only.
    pub read_only: bool,
    /// Lock support (disable for some NAS devices).
    pub nolock: bool,
    /// Additional mount options.
    pub extra_options: Vec<String>,
}

impl Default for NfsConfig {
    fn default() -> Self {
        Self {
            version: NfsVersion::V4,
            rsize: 1048576,  // 1 MB
            wsize: 1048576,  // 1 MB
            soft: true,
            timeo: 600,      // 60 seconds
            retrans: 2,
            tcp: true,
            read_only: false,
            nolock: false,
            extra_options: Vec::new(),
        }
    }
}

/// Information about an NFS export discovered from a server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NfsExport {
    /// Export path on the server.
    pub path: String,
    /// Allowed clients (e.g., "*.example.com", "192.168.1.0/24").
    pub allowed_clients: Vec<String>,
}

// ──────────────────────────────────────────────
// NFS Connector
// ──────────────────────────────────────────────

/// NFS connector using system mount commands.
pub struct NfsConnector {
    state: Arc<RwLock<NfsConnectionState>>,
}

#[derive(Debug, Clone)]
#[derive(Default)]
struct NfsConnectionState {
    connected: bool,
    mount_point: Option<PathBuf>,
    host: String,
    export_path: String,
    config: NfsConfig,
}


impl NfsConnector {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(NfsConnectionState::default())),
        }
    }

    /// Create a connector with specific NFS configuration.
    pub fn with_config(config: NfsConfig) -> Self {
        let state = NfsConnectionState {
            config,
            ..Default::default()
        };
        Self {
            state: Arc::new(RwLock::new(state)),
        }
    }

    /// Get the mount point path for a given host and export.
    fn mount_point_path(host: &str, export_path: &str) -> PathBuf {
        let base = std::env::temp_dir().join("ufop_nfs_mounts");
        let sanitized_host = host.replace(['/', '\\', ':', ' '], "_");
        let sanitized_export = export_path
            .trim_start_matches('/')
            .replace(['/', '\\', ':', ' '], "_");
        base.join(format!("{sanitized_host}_{sanitized_export}"))
    }

    /// Build platform-specific NFS mount command.
    fn build_mount_command(
        host: &str,
        export_path: &str,
        mount_point: &Path,
        config: &NfsConfig,
    ) -> Result<Command, AppError> {
        #[cfg(target_os = "macos")]
        {
            let mut cmd = Command::new("mount");
            cmd.arg("-t").arg("nfs");

            let mut opts = Vec::new();
            opts.push(format!("vers={}", config.version.as_str()));
            opts.push(format!("rsize={}", config.rsize));
            opts.push(format!("wsize={}", config.wsize));

            if config.soft {
                opts.push("soft".to_string());
            } else {
                opts.push("hard".to_string());
            }

            opts.push(format!("timeo={}", config.timeo));
            opts.push(format!("retrans={}", config.retrans));

            if config.tcp {
                opts.push("tcp".to_string());
            }

            if config.read_only {
                opts.push("ro".to_string());
            }

            if config.nolock {
                opts.push("nolocks".to_string());
            }

            for opt in &config.extra_options {
                opts.push(opt.clone());
            }

            cmd.arg("-o").arg(opts.join(","));
            cmd.arg(format!("{host}:{export_path}"));
            cmd.arg(mount_point);

            Ok(cmd)
        }

        #[cfg(target_os = "linux")]
        {
            let mut cmd = Command::new("mount");
            cmd.arg("-t").arg(config.version.mount_type());

            let mut opts = Vec::new();

            // NFSv3 needs explicit version; NFSv4 uses nfs4 type
            if matches!(config.version, NfsVersion::V3) {
                opts.push("nfsvers=3".to_string());
            } else if matches!(config.version, NfsVersion::V41) {
                opts.push("minorversion=1".to_string());
            } else if matches!(config.version, NfsVersion::V42) {
                opts.push("minorversion=2".to_string());
            }

            opts.push(format!("rsize={}", config.rsize));
            opts.push(format!("wsize={}", config.wsize));

            if config.soft {
                opts.push("soft".to_string());
            } else {
                opts.push("hard".to_string());
            }

            opts.push(format!("timeo={}", config.timeo));
            opts.push(format!("retrans={}", config.retrans));

            if config.tcp {
                opts.push("proto=tcp".to_string());
            }

            if config.read_only {
                opts.push("ro".to_string());
            }

            if config.nolock {
                opts.push("nolock".to_string());
            }

            for opt in &config.extra_options {
                opts.push(opt.clone());
            }

            cmd.arg("-o").arg(opts.join(","));
            cmd.arg(format!("{host}:{export_path}"));
            cmd.arg(mount_point);

            Ok(cmd)
        }

        #[cfg(target_os = "windows")]
        {
            // Windows NFS client (if enabled)
            let mut cmd = Command::new("mount");
            cmd.arg("-o")
                .arg(format!("anon,nolock,rsize={},wsize={}", config.rsize, config.wsize));
            cmd.arg(format!("\\\\{host}{}", export_path.replace('/', "\\")));
            cmd.arg(mount_point);

            Ok(cmd)
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            Err(AppError::Connection {
                message: "NFS is not supported on this platform.".to_string(),
                advice: "Use macOS, Linux, or Windows with NFS client enabled.".to_string(),
            })
        }
    }

    /// Build platform-specific unmount command.
    fn build_unmount_command(mount_point: &Path) -> Command {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let mut cmd = Command::new("umount");
            cmd.arg(mount_point);
            cmd
        }

        #[cfg(target_os = "windows")]
        {
            let mut cmd = Command::new("umount");
            cmd.arg(mount_point);
            cmd
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            let mut cmd = Command::new("echo");
            cmd.arg("Unmount not supported");
            cmd
        }
    }

    /// List NFS exports available on a server.
    pub async fn list_exports(host: &str) -> Result<Vec<NfsExport>, AppError> {
        let host = host.to_string();

        tokio::task::spawn_blocking(move || {
            let output = Command::new("showmount")
                .arg("-e")
                .arg(&host)
                .output()
                .map_err(|e| AppError::Connection {
                    message: format!("Failed to query NFS exports from {host}: {e}"),
                    advice: "Ensure showmount is installed and the NFS server is reachable."
                        .to_string(),
                })?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(AppError::Connection {
                    message: format!("showmount failed: {stderr}"),
                    advice: "Check that the NFS server is running and allows exports.".to_string(),
                });
            }

            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut exports = Vec::new();

            for line in stdout.lines().skip(1) {
                // Skip header "Export list for..."
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
                let path = parts.first().unwrap_or(&"").to_string();
                let clients = parts
                    .get(1)
                    .map(|s| {
                        s.trim()
                            .split(',')
                            .map(|c| c.trim().to_string())
                            .collect()
                    })
                    .unwrap_or_default();

                if !path.is_empty() {
                    exports.push(NfsExport {
                        path,
                        allowed_clients: clients,
                    });
                }
            }

            Ok(exports)
        })
        .await
        .map_err(|e| AppError::Connection {
            message: format!("Export listing task failed: {e}"),
            advice: "Try again.".to_string(),
        })?
    }

    /// Get the current mount point path if connected.
    pub async fn mount_point(&self) -> Option<PathBuf> {
        self.state.read().await.mount_point.clone()
    }
}

impl Default for NfsConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl Connector for NfsConnector {
    fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let host = profile.host.clone();
        let export_path = profile.remote_path.clone();

        Box::pin(async move {
            let mut state = self.state.write().await;

            if export_path.is_empty() || export_path == "/" {
                return Err(AppError::Connection {
                    message: "No NFS export path specified.".to_string(),
                    advice: "Set the remote path to the NFS export (e.g., /exports/data)."
                        .to_string(),
                });
            }

            // Create mount point directory
            let mount_point = Self::mount_point_path(&host, &export_path);
            tokio::fs::create_dir_all(&mount_point).await.map_err(|e| {
                AppError::Connection {
                    message: format!("Failed to create mount point: {e}"),
                    advice: "Check file permissions.".to_string(),
                }
            })?;

            let mount_point_clone = mount_point.clone();
            let host_clone = host.clone();
            let export_clone = export_path.clone();
            let config_clone = state.config.clone();

            let result = tokio::task::spawn_blocking(move || {
                let mut cmd = Self::build_mount_command(
                    &host_clone,
                    &export_clone,
                    &mount_point_clone,
                    &config_clone,
                )?;

                let output = cmd.output().map_err(|e| AppError::Connection {
                    message: format!("Failed to execute NFS mount command: {e}"),
                    advice: "Ensure NFS client is installed on your system.".to_string(),
                })?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(AppError::Connection {
                        message: format!("NFS mount failed: {stderr}"),
                        advice: "Check the hostname, export path, and NFS version.".to_string(),
                    });
                }

                Ok(())
            })
            .await
            .map_err(|e| AppError::Connection {
                message: format!("Mount task failed: {e}"),
                advice: "Try again.".to_string(),
            })?;

            result?;

            state.connected = true;
            state.mount_point = Some(mount_point);
            state.host = host;
            state.export_path = export_path;

            tracing::info!(
                "NFS connected: {}:{}",
                state.host,
                state.export_path
            );

            Ok(())
        })
    }

    fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            if !state.connected {
                return Ok(());
            }

            if let Some(mount_point) = &state.mount_point {
                let mp = mount_point.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let mut cmd = Self::build_unmount_command(&mp);
                    let output = cmd.output().map_err(|e| AppError::Connection {
                        message: format!("Failed to unmount NFS: {e}"),
                        advice: "Close any files from this mount and try again.".to_string(),
                    })?;

                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(AppError::Connection {
                            message: format!("NFS unmount failed: {stderr}"),
                            advice: "Close any open files and try again.".to_string(),
                        });
                    }

                    Ok(())
                })
                .await
                .map_err(|e| AppError::Connection {
                    message: format!("Unmount task failed: {e}"),
                    advice: "Try again.".to_string(),
                })?;

                result?;

                // Clean up mount point directory
                let _ = tokio::fs::remove_dir(mount_point).await;
            }

            tracing::info!(
                "NFS disconnected: {}:{}",
                state.host,
                state.export_path
            );

            state.connected = false;
            state.mount_point = None;

            Ok(())
        })
    }

    fn list_remote(
        &self,
        path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<FileEntry>, AppError>> + Send + '_>> {
        let path = path.to_string();

        Box::pin(async move {
            let state = self.state.read().await;

            if !state.connected {
                return Err(AppError::Connection {
                    message: "Not connected to NFS export.".to_string(),
                    advice: "Connect first before browsing files.".to_string(),
                });
            }

            let mount_point = state.mount_point.as_ref().ok_or_else(|| {
                AppError::Connection {
                    message: "No mount point available.".to_string(),
                    advice: "Reconnect to the NFS export.".to_string(),
                }
            })?;

            let full_path = if path.is_empty() || path == "/" {
                mount_point.clone()
            } else {
                mount_point.join(path.trim_start_matches('/'))
            };

            list_nfs_directory(&full_path).await
        })
    }

    fn is_connected(&self) -> bool {
        self.state
            .try_read()
            .map(|s| s.connected)
            .unwrap_or(false)
    }
}

// ──────────────────────────────────────────────
// Unix Permission Formatting
// ──────────────────────────────────────────────

/// Convert a Unix permission mode to rwxrwxrwx string.
pub fn format_unix_permissions(mode: u32) -> String {
    let mut result = String::with_capacity(9);

    // Owner
    result.push(if mode & 0o400 != 0 { 'r' } else { '-' });
    result.push(if mode & 0o200 != 0 { 'w' } else { '-' });
    result.push(if mode & 0o100 != 0 { 'x' } else { '-' });

    // Group
    result.push(if mode & 0o040 != 0 { 'r' } else { '-' });
    result.push(if mode & 0o020 != 0 { 'w' } else { '-' });
    result.push(if mode & 0o010 != 0 { 'x' } else { '-' });

    // Others
    result.push(if mode & 0o004 != 0 { 'r' } else { '-' });
    result.push(if mode & 0o002 != 0 { 'w' } else { '-' });
    result.push(if mode & 0o001 != 0 { 'x' } else { '-' });

    result
}

/// Parse a permission string like "755" into rwxrwxrwx format.
pub fn octal_to_rwx(octal_str: &str) -> String {
    if let Ok(mode) = u32::from_str_radix(octal_str, 8) {
        format_unix_permissions(mode)
    } else {
        octal_str.to_string()
    }
}

// ──────────────────────────────────────────────
// Directory Listing (with Unix permissions)
// ──────────────────────────────────────────────

/// List directory entries from a mounted NFS path, including Unix permissions.
async fn list_nfs_directory(path: &Path) -> Result<Vec<FileEntry>, AppError> {
    let path = path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();

        let read_dir = std::fs::read_dir(&path).map_err(|e| AppError::Connection {
            message: format!("Failed to read NFS directory {}: {e}", path.display()),
            advice: "Check that the NFS export is mounted and accessible.".to_string(),
        })?;

        for entry in read_dir {
            let entry = entry.map_err(|e| AppError::Connection {
                message: format!("Failed to read entry: {e}"),
                advice: "Some files may not be accessible.".to_string(),
            })?;

            let metadata = entry.metadata().map_err(|e| AppError::Connection {
                message: format!("Failed to read metadata: {e}"),
                advice: "Some files may not be accessible due to NFS permissions.".to_string(),
            })?;

            let name = entry.file_name().to_string_lossy().to_string();
            let file_path = entry.path().to_string_lossy().to_string();
            let is_hidden = name.starts_with('.');

            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .and_then(|d| Utc.timestamp_opt(d.as_secs() as i64, 0).single())
                });

            let created = metadata
                .created()
                .ok()
                .and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .and_then(|d| Utc.timestamp_opt(d.as_secs() as i64, 0).single())
                });

            let extension = if metadata.is_file() {
                Path::new(&name)
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
            } else {
                None
            };

            // Get Unix permissions in rwxrwxrwx format for NFS
            #[cfg(unix)]
            let permissions = {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode() & 0o777;
                Some(format_unix_permissions(mode))
            };
            #[cfg(not(unix))]
            let permissions = None;

            entries.push(FileEntry {
                name,
                path: file_path,
                is_dir: metadata.is_dir(),
                is_symlink: metadata.file_type().is_symlink(),
                size: metadata.len(),
                modified,
                created,
                is_hidden,
                extension,
                permissions,
            });
        }

        // Sort: directories first, then alphabetical
        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(entries)
    })
    .await
    .map_err(|e| AppError::Connection {
        message: format!("Directory listing task failed: {e}"),
        advice: "Try again.".to_string(),
    })?
}

// ──────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nfs_version_str() {
        assert_eq!(NfsVersion::V3.as_str(), "3");
        assert_eq!(NfsVersion::V4.as_str(), "4");
        assert_eq!(NfsVersion::V41.as_str(), "4.1");
        assert_eq!(NfsVersion::V42.as_str(), "4.2");
    }

    #[test]
    fn test_nfs_version_from_str() {
        assert_eq!(NfsVersion::from_str_lossy("3"), NfsVersion::V3);
        assert_eq!(NfsVersion::from_str_lossy("v3"), NfsVersion::V3);
        assert_eq!(NfsVersion::from_str_lossy("4"), NfsVersion::V4);
        assert_eq!(NfsVersion::from_str_lossy("4.1"), NfsVersion::V41);
        assert_eq!(NfsVersion::from_str_lossy("unknown"), NfsVersion::V4);
    }

    #[test]
    fn test_nfs_mount_type() {
        assert_eq!(NfsVersion::V3.mount_type(), "nfs");
        assert_eq!(NfsVersion::V4.mount_type(), "nfs4");
        assert_eq!(NfsVersion::V41.mount_type(), "nfs4");
        assert_eq!(NfsVersion::V42.mount_type(), "nfs4");
    }

    #[test]
    fn test_format_unix_permissions() {
        assert_eq!(format_unix_permissions(0o755), "rwxr-xr-x");
        assert_eq!(format_unix_permissions(0o644), "rw-r--r--");
        assert_eq!(format_unix_permissions(0o777), "rwxrwxrwx");
        assert_eq!(format_unix_permissions(0o000), "---------");
        assert_eq!(format_unix_permissions(0o700), "rwx------");
        assert_eq!(format_unix_permissions(0o600), "rw-------");
        assert_eq!(format_unix_permissions(0o444), "r--r--r--");
    }

    #[test]
    fn test_octal_to_rwx() {
        assert_eq!(octal_to_rwx("755"), "rwxr-xr-x");
        assert_eq!(octal_to_rwx("644"), "rw-r--r--");
        assert_eq!(octal_to_rwx("777"), "rwxrwxrwx");
        assert_eq!(octal_to_rwx("invalid"), "invalid");
    }

    #[test]
    fn test_nfs_config_defaults() {
        let config = NfsConfig::default();
        assert_eq!(config.version, NfsVersion::V4);
        assert_eq!(config.rsize, 1048576);
        assert_eq!(config.wsize, 1048576);
        assert!(config.soft);
        assert!(config.tcp);
        assert!(!config.read_only);
        assert!(!config.nolock);
    }

    #[test]
    fn test_mount_point_path() {
        let mp = NfsConnector::mount_point_path("192.168.1.100", "/exports/data");
        let mp_str = mp.to_string_lossy();
        assert!(mp_str.contains("ufop_nfs_mounts"));
        assert!(mp_str.contains("192.168.1.100_exports_data"));
    }

    #[test]
    fn test_nfs_connector_default() {
        let connector = NfsConnector::new();
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_nfs_export_serialization() {
        let export = NfsExport {
            path: "/exports/data".to_string(),
            allowed_clients: vec!["192.168.1.0/24".to_string(), "*.example.com".to_string()],
        };
        let json = serde_json::to_string(&export).unwrap();
        let deserialized: NfsExport = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.path, "/exports/data");
        assert_eq!(deserialized.allowed_clients.len(), 2);
    }

    #[test]
    fn test_nfs_config_serialization() {
        let config = NfsConfig {
            version: NfsVersion::V41,
            rsize: 524288,
            wsize: 524288,
            soft: false,
            timeo: 300,
            retrans: 3,
            tcp: true,
            read_only: true,
            nolock: true,
            extra_options: vec!["ac".to_string()],
        };
        let json = serde_json::to_string(&config).unwrap();
        let deserialized: NfsConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.version, NfsVersion::V41);
        assert!(deserialized.read_only);
        assert!(deserialized.nolock);
    }
}
