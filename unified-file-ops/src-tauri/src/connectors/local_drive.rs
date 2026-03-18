//! Local Drive Connector and Drive-to-Drive Transfer (T-025).
//!
//! Provides local filesystem access through the Connector trait, enabling
//! drive-to-drive transfers using the same transfer queue framework.
//!
//! Features:
//! - Preflight checks: space, filesystem compatibility, duplicate risk, permissions
//! - All transfer contexts: internal-to-internal, internal-to-external, external-to-external
//! - Drive detection: name, icon, total/free space, filesystem type, eject
//! - Same transfer queue as remote transfers

use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::FileEntry;
use crate::core::types::ConnectionProfile;
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

// ──────────────────────────────────────────────
// Drive Information Types
// ──────────────────────────────────────────────

/// Filesystem type of a drive.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FilesystemType {
    Apfs,
    Hfs,
    Ntfs,
    Ext4,
    Ext3,
    Btrfs,
    Xfs,
    Zfs,
    Fat32,
    ExFat,
    Ufs,
    Unknown(String),
}

impl FilesystemType {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "apfs" => Self::Apfs,
            "hfs" | "hfs+" | "hfsplus" => Self::Hfs,
            "ntfs" | "ntfs-3g" => Self::Ntfs,
            "ext4" => Self::Ext4,
            "ext3" => Self::Ext3,
            "btrfs" => Self::Btrfs,
            "xfs" => Self::Xfs,
            "zfs" => Self::Zfs,
            "fat32" | "vfat" | "msdos" => Self::Fat32,
            "exfat" => Self::ExFat,
            "ufs" => Self::Ufs,
            other => Self::Unknown(other.to_string()),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::Apfs => "APFS",
            Self::Hfs => "HFS+",
            Self::Ntfs => "NTFS",
            Self::Ext4 => "ext4",
            Self::Ext3 => "ext3",
            Self::Btrfs => "Btrfs",
            Self::Xfs => "XFS",
            Self::Zfs => "ZFS",
            Self::Fat32 => "FAT32",
            Self::ExFat => "exFAT",
            Self::Ufs => "UFS",
            Self::Unknown(s) => s,
        }
    }

    /// Maximum single file size for this filesystem.
    pub fn max_file_size(&self) -> Option<u64> {
        match self {
            Self::Fat32 => Some(4 * 1024 * 1024 * 1024 - 1), // ~4 GB
            Self::Hfs => Some(8 * 1024 * 1024 * 1024 * 1024 * 1024), // 8 EB
            _ => None,                                                 // No practical limit
        }
    }

    /// Whether this filesystem supports Unix permissions.
    pub fn supports_unix_permissions(&self) -> bool {
        matches!(
            self,
            Self::Apfs
                | Self::Hfs
                | Self::Ext4
                | Self::Ext3
                | Self::Btrfs
                | Self::Xfs
                | Self::Zfs
                | Self::Ufs
        )
    }

    /// Whether this filesystem supports symlinks.
    pub fn supports_symlinks(&self) -> bool {
        !matches!(self, Self::Fat32)
    }

    /// Whether filenames are case-sensitive.
    pub fn is_case_sensitive(&self) -> bool {
        matches!(
            self,
            Self::Ext4 | Self::Ext3 | Self::Btrfs | Self::Xfs | Self::Zfs
        )
    }
}

/// Drive type classification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DriveType {
    /// Internal drive (SSD/HDD)
    Internal,
    /// External/removable drive (USB, Thunderbolt)
    External,
    /// Network-attached drive (mounted SMB/NFS)
    Network,
    /// Optical drive (CD/DVD/Blu-ray)
    Optical,
    /// Virtual/disk image
    Virtual,
    /// Unknown type
    Unknown,
}

/// Comprehensive information about a mounted drive/volume.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DriveInfo {
    /// Display name for the drive
    pub name: String,
    /// Mount point path
    pub mount_point: String,
    /// Drive device path (e.g., /dev/disk2s1)
    pub device: String,
    /// Total capacity in bytes
    pub total_bytes: u64,
    /// Free space in bytes
    pub free_bytes: u64,
    /// Used space in bytes
    pub used_bytes: u64,
    /// Filesystem type
    pub filesystem: FilesystemType,
    /// Drive type classification
    pub drive_type: DriveType,
    /// Whether the drive is removable/ejectable
    pub removable: bool,
    /// Whether the drive is read-only
    pub read_only: bool,
    /// Device model/label if available
    pub label: Option<String>,
    /// Volume UUID if available
    pub uuid: Option<String>,
}

// ──────────────────────────────────────────────
// Preflight Check Types
// ──────────────────────────────────────────────

/// Result of a transfer preflight check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightResult {
    /// Whether the transfer can proceed
    pub can_proceed: bool,
    /// Warnings that don't block the transfer
    pub warnings: Vec<PreflightWarning>,
    /// Errors that block the transfer
    pub errors: Vec<PreflightError>,
    /// Source drive info
    pub source_drive: Option<DriveInfo>,
    /// Destination drive info
    pub dest_drive: Option<DriveInfo>,
}

/// A non-blocking preflight warning.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightWarning {
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
}

/// A blocking preflight error.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PreflightError {
    pub code: String,
    pub message: String,
    pub detail: Option<String>,
}

/// Transfer context classification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferContext {
    /// Internal drive to internal drive
    InternalToInternal,
    /// Internal drive to external drive
    InternalToExternal,
    /// External drive to internal drive
    ExternalToInternal,
    /// External drive to external drive
    ExternalToExternal,
    /// Same drive (within same volume)
    SameDrive,
    /// Network to local or local to network
    NetworkTransfer,
}

// ──────────────────────────────────────────────
// Local Drive Connector
// ──────────────────────────────────────────────

/// Local drive connector implementing the Connector trait.
/// Enables drive-to-drive transfers through the unified transfer framework.
pub struct LocalDriveConnector {
    state: Arc<RwLock<LocalDriveState>>,
}

#[derive(Debug, Clone)]
struct LocalDriveState {
    connected: bool,
    root_path: PathBuf,
}

impl Default for LocalDriveState {
    fn default() -> Self {
        Self {
            connected: false,
            root_path: PathBuf::from("/"),
        }
    }
}

impl LocalDriveConnector {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(LocalDriveState::default())),
        }
    }

    /// Detect all mounted drives/volumes on the system.
    pub async fn detect_drives() -> Result<Vec<DriveInfo>, AppError> {
        tokio::task::spawn_blocking(|| {
            let mut drives = Vec::new();

            #[cfg(target_os = "macos")]
            {
                drives.extend(detect_drives_macos()?);
            }

            #[cfg(target_os = "linux")]
            {
                drives.extend(detect_drives_linux()?);
            }

            #[cfg(target_os = "windows")]
            {
                drives.extend(detect_drives_windows()?);
            }

            Ok(drives)
        })
        .await
        .map_err(|e| AppError::internal(format!("Drive detection task failed: {e}")))?
    }

    /// Run preflight checks before a transfer.
    pub async fn preflight_check(
        source_path: &str,
        dest_path: &str,
        total_bytes: u64,
    ) -> Result<PreflightResult, AppError> {
        let source = source_path.to_string();
        let dest = dest_path.to_string();

        tokio::task::spawn_blocking(move || {
            let mut warnings = Vec::new();
            let mut errors = Vec::new();

            // 1. Check source exists
            let source_path = Path::new(&source);
            if !source_path.exists() {
                errors.push(PreflightError {
                    code: "SOURCE_NOT_FOUND".to_string(),
                    message: "Source file or directory does not exist.".to_string(),
                    detail: Some(source.clone()),
                });
                return Ok(PreflightResult {
                    can_proceed: false,
                    warnings,
                    errors,
                    source_drive: None,
                    dest_drive: None,
                });
            }

            // 2. Check source is readable
            if !is_readable(source_path) {
                errors.push(PreflightError {
                    code: "SOURCE_NOT_READABLE".to_string(),
                    message: "Source is not readable.".to_string(),
                    detail: Some("Check file permissions.".to_string()),
                });
            }

            // 3. Get drive info for source and destination
            let dest_path = Path::new(&dest);
            let dest_parent = dest_path.parent().unwrap_or(dest_path);

            let source_drive = get_drive_for_path(source_path);
            let dest_drive = get_drive_for_path(dest_parent);

            // 4. Space check
            if let Some(ref dd) = dest_drive {
                if total_bytes > dd.free_bytes {
                    errors.push(PreflightError {
                        code: "INSUFFICIENT_SPACE".to_string(),
                        message: format!(
                            "Not enough space on destination drive. Need {} bytes, have {} bytes free.",
                            total_bytes, dd.free_bytes
                        ),
                        detail: Some(format!("Drive: {} ({})", dd.name, dd.mount_point)),
                    });
                } else if total_bytes as f64 > dd.free_bytes as f64 * 0.9 {
                    warnings.push(PreflightWarning {
                        code: "LOW_SPACE".to_string(),
                        message: "Transfer will use more than 90% of remaining free space."
                            .to_string(),
                        detail: None,
                    });
                }
            }

            // 5. Filesystem compatibility checks
            if let (Some(ref sd), Some(ref dd)) = (&source_drive, &dest_drive) {
                // Check file size limits (e.g., FAT32 4GB limit)
                if let Some(max_size) = dd.filesystem.max_file_size() {
                    if total_bytes > max_size {
                        errors.push(PreflightError {
                            code: "FILE_TOO_LARGE".to_string(),
                            message: format!(
                                "File exceeds {} max file size of {} bytes.",
                                dd.filesystem.as_str(),
                                max_size
                            ),
                            detail: None,
                        });
                    }
                }

                // Permission compatibility
                if sd.filesystem.supports_unix_permissions()
                    && !dd.filesystem.supports_unix_permissions()
                {
                    warnings.push(PreflightWarning {
                        code: "PERMISSION_LOSS".to_string(),
                        message: format!(
                            "Destination filesystem ({}) does not support Unix permissions. Permission data will be lost.",
                            dd.filesystem.as_str()
                        ),
                        detail: None,
                    });
                }

                // Symlink compatibility
                if !dd.filesystem.supports_symlinks() {
                    warnings.push(PreflightWarning {
                        code: "NO_SYMLINK_SUPPORT".to_string(),
                        message: format!(
                            "Destination filesystem ({}) does not support symbolic links.",
                            dd.filesystem.as_str()
                        ),
                        detail: None,
                    });
                }

                // Case sensitivity
                if sd.filesystem.is_case_sensitive() && !dd.filesystem.is_case_sensitive() {
                    warnings.push(PreflightWarning {
                        code: "CASE_SENSITIVITY".to_string(),
                        message: "Source filesystem is case-sensitive but destination is not. \
                                  Files with names differing only in case may conflict."
                            .to_string(),
                        detail: None,
                    });
                }
            }

            // 6. Destination writable
            if dest_parent.exists() && !is_writable(dest_parent) {
                errors.push(PreflightError {
                    code: "DEST_NOT_WRITABLE".to_string(),
                    message: "Destination directory is not writable.".to_string(),
                    detail: Some("Check folder permissions.".to_string()),
                });
            }

            if let Some(ref dd) = dest_drive {
                if dd.read_only {
                    errors.push(PreflightError {
                        code: "DEST_READ_ONLY".to_string(),
                        message: "Destination drive is read-only.".to_string(),
                        detail: Some(format!("Drive: {} ({})", dd.name, dd.mount_point)),
                    });
                }
            }

            // 7. Duplicate risk check
            if dest_path.exists() {
                warnings.push(PreflightWarning {
                    code: "DESTINATION_EXISTS".to_string(),
                    message: "A file or folder already exists at the destination.".to_string(),
                    detail: Some(dest.clone()),
                });
            }

            let can_proceed = errors.is_empty();

            Ok(PreflightResult {
                can_proceed,
                warnings,
                errors,
                source_drive,
                dest_drive,
            })
        })
        .await
        .map_err(|e| AppError::internal(format!("Preflight check failed: {e}")))?
    }

    /// Determine the transfer context for a source/destination pair.
    pub fn classify_transfer(
        source_drive: Option<&DriveInfo>,
        dest_drive: Option<&DriveInfo>,
    ) -> TransferContext {
        match (source_drive, dest_drive) {
            (Some(s), Some(d)) => {
                // Same drive?
                if s.device == d.device || s.mount_point == d.mount_point {
                    return TransferContext::SameDrive;
                }

                // Network involved?
                if s.drive_type == DriveType::Network || d.drive_type == DriveType::Network {
                    return TransferContext::NetworkTransfer;
                }

                match (s.drive_type, d.drive_type) {
                    (DriveType::Internal, DriveType::Internal) => {
                        TransferContext::InternalToInternal
                    }
                    (DriveType::Internal, DriveType::External) => {
                        TransferContext::InternalToExternal
                    }
                    (DriveType::External, DriveType::Internal) => {
                        TransferContext::ExternalToInternal
                    }
                    (DriveType::External, DriveType::External) => {
                        TransferContext::ExternalToExternal
                    }
                    _ => TransferContext::InternalToInternal,
                }
            }
            _ => TransferContext::InternalToInternal,
        }
    }

    /// Eject a removable drive.
    pub async fn eject_drive(mount_point: &str) -> Result<(), AppError> {
        let mount_point = mount_point.to_string();

        tokio::task::spawn_blocking(move || {
            #[cfg(target_os = "macos")]
            {
                let output = Command::new("diskutil")
                    .arg("eject")
                    .arg(&mount_point)
                    .output()
                    .map_err(|e| AppError::FileOperation {
                        message: format!("Failed to eject drive: {e}"),
                        advice: "Close any open files from this drive and try again.".to_string(),
                    })?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(AppError::FileOperation {
                        message: format!("Eject failed: {stderr}"),
                        advice: "Close any open files or applications using this drive.".to_string(),
                    });
                }
            }

            #[cfg(target_os = "linux")]
            {
                let output = Command::new("udisksctl")
                    .arg("unmount")
                    .arg("-b")
                    .arg(&mount_point)
                    .output()
                    .or_else(|_| {
                        Command::new("umount").arg(&mount_point).output()
                    })
                    .map_err(|e| AppError::FileOperation {
                        message: format!("Failed to eject drive: {e}"),
                        advice: "Close any open files and try again.".to_string(),
                    })?;

                if !output.status.success() {
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    return Err(AppError::FileOperation {
                        message: format!("Eject failed: {stderr}"),
                        advice: "Close any open files or applications using this drive.".to_string(),
                    });
                }
            }

            #[cfg(target_os = "windows")]
            {
                // On Windows, drive ejection is more complex.
                // For removable drives, we can try the "safely remove" approach.
                let _output = Command::new("mountvol")
                    .arg(&mount_point)
                    .arg("/D")
                    .output()
                    .map_err(|e| AppError::FileOperation {
                        message: format!("Failed to eject drive: {e}"),
                        advice: "Use the system tray to safely remove the drive.".to_string(),
                    })?;
            }

            tracing::info!("Drive ejected: {mount_point}");
            Ok(())
        })
        .await
        .map_err(|e| AppError::internal(format!("Eject task failed: {e}")))?
    }
}

impl Default for LocalDriveConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl Connector for LocalDriveConnector {
    fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let path = profile.remote_path.clone();

        Box::pin(async move {
            let root = PathBuf::from(&path);

            if !root.exists() {
                return Err(AppError::Connection {
                    message: format!("Path does not exist: {path}"),
                    advice: "Check that the drive is mounted and the path is correct.".to_string(),
                });
            }

            let mut state = self.state.write().await;
            state.connected = true;
            state.root_path = root;

            tracing::info!("Local drive connected: {path}");
            Ok(())
        })
    }

    fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            state.connected = false;
            tracing::info!("Local drive disconnected");
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
                    message: "Not connected to local drive.".to_string(),
                    advice: "Connect first.".to_string(),
                });
            }

            let full_path = if path.is_empty() || path == "/" {
                state.root_path.clone()
            } else if Path::new(&path).is_absolute() {
                PathBuf::from(&path)
            } else {
                state.root_path.join(path.trim_start_matches('/'))
            };

            list_local_directory(&full_path).await
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
// Platform-Specific Drive Detection
// ──────────────────────────────────────────────

#[cfg(target_os = "macos")]
fn detect_drives_macos() -> Result<Vec<DriveInfo>, AppError> {
    let mut drives = Vec::new();

    // Use diskutil list to get all volumes
    let output = Command::new("df")
        .arg("-Pkl")
        .output()
        .map_err(|e| AppError::internal(format!("df failed: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }

        let device = parts[0].to_string();
        let total_blocks: u64 = parts[1].parse().unwrap_or(0);
        let used_blocks: u64 = parts[2].parse().unwrap_or(0);
        let free_blocks: u64 = parts[3].parse().unwrap_or(0);
        let mount_point = parts[5..].join(" ");

        // Skip virtual filesystems
        if device.starts_with("devfs")
            || device.starts_with("map ")
            || mount_point == "/dev"
        {
            continue;
        }

        // 1K blocks
        let total_bytes = total_blocks * 1024;
        let used_bytes = used_blocks * 1024;
        let free_bytes = free_blocks * 1024;

        let name = mount_point
            .split('/')
            .next_back()
            .unwrap_or(&mount_point)
            .to_string();
        let name = if name.is_empty() {
            "Macintosh HD".to_string()
        } else {
            name
        };

        let is_external = mount_point.starts_with("/Volumes/")
            && mount_point != "/Volumes/Macintosh HD";
        let removable = is_external;

        // Detect filesystem type
        let fs_type = detect_filesystem_macos(&mount_point);

        drives.push(DriveInfo {
            name,
            mount_point,
            device,
            total_bytes,
            free_bytes,
            used_bytes,
            filesystem: fs_type,
            drive_type: if is_external {
                DriveType::External
            } else {
                DriveType::Internal
            },
            removable,
            read_only: false,
            label: None,
            uuid: None,
        });
    }

    Ok(drives)
}

#[cfg(target_os = "macos")]
fn detect_filesystem_macos(mount_point: &str) -> FilesystemType {
    let output = Command::new("mount").output();
    if let Ok(out) = output {
        let stdout = String::from_utf8_lossy(&out.stdout);
        for line in stdout.lines() {
            if line.contains(mount_point) {
                if line.contains("apfs") {
                    return FilesystemType::Apfs;
                } else if line.contains("hfs") {
                    return FilesystemType::Hfs;
                } else if line.contains("ntfs") {
                    return FilesystemType::Ntfs;
                } else if line.contains("exfat") {
                    return FilesystemType::ExFat;
                } else if line.contains("msdos") || line.contains("fat") {
                    return FilesystemType::Fat32;
                }
            }
        }
    }
    FilesystemType::Unknown("unknown".to_string())
}

#[cfg(target_os = "linux")]
fn detect_drives_linux() -> Result<Vec<DriveInfo>, AppError> {
    let mut drives = Vec::new();

    let output = Command::new("df")
        .arg("-Pkl")
        .output()
        .map_err(|e| AppError::internal(format!("df failed: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Also read /proc/mounts for filesystem types
    let mounts = std::fs::read_to_string("/proc/mounts").unwrap_or_default();

    for line in stdout.lines().skip(1) {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 6 {
            continue;
        }

        let device = parts[0].to_string();
        let total_blocks: u64 = parts[1].parse().unwrap_or(0);
        let used_blocks: u64 = parts[2].parse().unwrap_or(0);
        let free_blocks: u64 = parts[3].parse().unwrap_or(0);
        let mount_point = parts[5..].join(" ");

        // Skip virtual filesystems
        if !device.starts_with("/dev/") {
            continue;
        }
        // Skip snap mounts
        if mount_point.starts_with("/snap/") {
            continue;
        }

        let total_bytes = total_blocks * 1024;
        let used_bytes = used_blocks * 1024;
        let free_bytes = free_blocks * 1024;

        let name = mount_point
            .split('/')
            .last()
            .unwrap_or(&mount_point)
            .to_string();
        let name = if name.is_empty() {
            "Root".to_string()
        } else {
            name
        };

        // Detect filesystem from /proc/mounts
        let fs_type = mounts
            .lines()
            .find(|l| l.starts_with(&device))
            .and_then(|l| l.split_whitespace().nth(2))
            .map(FilesystemType::from_str)
            .unwrap_or(FilesystemType::Unknown("unknown".to_string()));

        // Detect if external
        let is_external = device.contains("sd")
            && !device.contains("sda")
            || mount_point.starts_with("/media/")
            || mount_point.starts_with("/mnt/");

        let removable = is_removable_linux(&device);

        drives.push(DriveInfo {
            name,
            mount_point,
            device,
            total_bytes,
            free_bytes,
            used_bytes,
            filesystem: fs_type,
            drive_type: if is_external {
                DriveType::External
            } else {
                DriveType::Internal
            },
            removable,
            read_only: false,
            label: None,
            uuid: None,
        });
    }

    Ok(drives)
}

#[cfg(target_os = "linux")]
fn is_removable_linux(device: &str) -> bool {
    // Check /sys/block/sdX/removable
    let dev_name = device
        .trim_start_matches("/dev/")
        .chars()
        .take_while(|c| c.is_alphabetic())
        .collect::<String>();
    let removable_path = format!("/sys/block/{dev_name}/removable");
    std::fs::read_to_string(&removable_path)
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

#[cfg(target_os = "windows")]
fn detect_drives_windows() -> Result<Vec<DriveInfo>, AppError> {
    let mut drives = Vec::new();

    // Use wmic to list drives
    let output = Command::new("wmic")
        .arg("logicaldisk")
        .arg("get")
        .arg("caption,drivetype,filesystem,freespace,size,volumename")
        .arg("/format:csv")
        .output()
        .map_err(|e| AppError::internal(format!("wmic failed: {e}")))?;

    let stdout = String::from_utf8_lossy(&output.stdout);

    for line in stdout.lines().skip(2) {
        let fields: Vec<&str> = line.split(',').collect();
        if fields.len() < 7 {
            continue;
        }

        let caption = fields[1].trim();
        let drive_type_code: u32 = fields[2].trim().parse().unwrap_or(0);
        let filesystem = fields[3].trim();
        let free_space: u64 = fields[4].trim().parse().unwrap_or(0);
        let total_size: u64 = fields[5].trim().parse().unwrap_or(0);
        let volume_name = fields[6].trim();

        let name = if volume_name.is_empty() {
            format!("Local Disk ({})", caption)
        } else {
            format!("{} ({})", volume_name, caption)
        };

        let drive_type = match drive_type_code {
            2 => DriveType::External,  // Removable
            3 => DriveType::Internal,  // Fixed
            4 => DriveType::Network,   // Network
            5 => DriveType::Optical,   // CD-ROM
            _ => DriveType::Unknown,
        };

        drives.push(DriveInfo {
            name,
            mount_point: caption.to_string(),
            device: caption.to_string(),
            total_bytes: total_size,
            free_bytes: free_space,
            used_bytes: total_size.saturating_sub(free_space),
            filesystem: FilesystemType::from_str(filesystem),
            drive_type,
            removable: drive_type_code == 2,
            read_only: false,
            label: if volume_name.is_empty() {
                None
            } else {
                Some(volume_name.to_string())
            },
            uuid: None,
        });
    }

    Ok(drives)
}

// Fallback for unsupported platforms
#[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
fn detect_drives_fallback() -> Result<Vec<DriveInfo>, AppError> {
    Ok(vec![DriveInfo {
        name: "Root".to_string(),
        mount_point: "/".to_string(),
        device: "/".to_string(),
        total_bytes: 0,
        free_bytes: 0,
        used_bytes: 0,
        filesystem: FilesystemType::Unknown("unknown".to_string()),
        drive_type: DriveType::Internal,
        removable: false,
        read_only: false,
        label: None,
        uuid: None,
    }])
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

/// Check if a path is readable.
fn is_readable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mode = metadata.permissions().mode();
            // Check if any read bit is set
            mode & 0o444 != 0
        } else {
            false
        }
    }

    #[cfg(not(unix))]
    {
        path.exists()
    }
}

/// Check if a path is writable.
fn is_writable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(metadata) = std::fs::metadata(path) {
            let mode = metadata.permissions().mode();
            // Check if any write bit is set
            mode & 0o222 != 0
        } else {
            false
        }
    }

    #[cfg(not(unix))]
    {
        // On Windows, try to get metadata; if successful the path exists.
        // A more thorough check would try to open the file for writing.
        path.exists() && !path.metadata().map(|m| m.permissions().readonly()).unwrap_or(true)
    }
}

/// Get drive info for the drive containing the given path.
fn get_drive_for_path(path: &Path) -> Option<DriveInfo> {
    #[cfg(target_os = "macos")]
    {
        let drives = detect_drives_macos().ok()?;
        find_best_drive_match(&drives, path)
    }

    #[cfg(target_os = "linux")]
    {
        let drives = detect_drives_linux().ok()?;
        find_best_drive_match(&drives, path)
    }

    #[cfg(target_os = "windows")]
    {
        let drives = detect_drives_windows().ok()?;
        find_best_drive_match(&drives, path)
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

/// Find the drive with the longest mount point prefix matching the path.
fn find_best_drive_match(drives: &[DriveInfo], path: &Path) -> Option<DriveInfo> {
    let path_str = path.to_string_lossy();
    drives
        .iter()
        .filter(|d| path_str.starts_with(&d.mount_point))
        .max_by_key(|d| d.mount_point.len())
        .cloned()
}

/// List entries in a local directory.
async fn list_local_directory(path: &Path) -> Result<Vec<FileEntry>, AppError> {
    let path = path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();

        let read_dir = std::fs::read_dir(&path).map_err(|e| AppError::FileOperation {
            message: format!("Failed to read directory {}: {e}", path.display()),
            advice: "Check permissions.".to_string(),
        })?;

        for entry in read_dir {
            let entry = entry.map_err(|e| AppError::FileOperation {
                message: format!("Failed to read entry: {e}"),
                advice: "Some files may not be accessible.".to_string(),
            })?;

            let metadata = match entry.metadata() {
                Ok(m) => m,
                Err(_) => continue,
            };

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

            #[cfg(unix)]
            let permissions = {
                use std::os::unix::fs::PermissionsExt;
                Some(format!("{:o}", metadata.permissions().mode() & 0o777))
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

        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(entries)
    })
    .await
    .map_err(|e| AppError::internal(format!("Directory listing task failed: {e}")))?
}

/// Format bytes to a human-readable string (e.g., "256 GB").
pub fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    const TB: u64 = 1024 * GB;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

// ──────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_filesystem_type_from_str() {
        assert_eq!(FilesystemType::from_str("apfs"), FilesystemType::Apfs);
        assert_eq!(FilesystemType::from_str("NTFS"), FilesystemType::Ntfs);
        assert_eq!(FilesystemType::from_str("ext4"), FilesystemType::Ext4);
        assert_eq!(FilesystemType::from_str("vfat"), FilesystemType::Fat32);
        assert_eq!(FilesystemType::from_str("exfat"), FilesystemType::ExFat);
        assert_eq!(FilesystemType::from_str("hfsplus"), FilesystemType::Hfs);
        assert_eq!(FilesystemType::from_str("btrfs"), FilesystemType::Btrfs);
    }

    #[test]
    fn test_filesystem_max_file_size() {
        assert!(FilesystemType::Fat32.max_file_size().is_some());
        assert!(FilesystemType::Fat32.max_file_size().unwrap() < 5 * 1024 * 1024 * 1024);
        assert!(FilesystemType::Ext4.max_file_size().is_none());
        assert!(FilesystemType::Apfs.max_file_size().is_none());
    }

    #[test]
    fn test_filesystem_capabilities() {
        assert!(FilesystemType::Ext4.supports_unix_permissions());
        assert!(FilesystemType::Apfs.supports_unix_permissions());
        assert!(!FilesystemType::Fat32.supports_unix_permissions());
        assert!(!FilesystemType::ExFat.supports_unix_permissions());

        assert!(FilesystemType::Ext4.supports_symlinks());
        assert!(!FilesystemType::Fat32.supports_symlinks());

        assert!(FilesystemType::Ext4.is_case_sensitive());
        assert!(!FilesystemType::Apfs.is_case_sensitive());
        assert!(!FilesystemType::Ntfs.is_case_sensitive());
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KB");
        assert_eq!(format_bytes(1536), "1.5 KB");
        assert_eq!(format_bytes(1024 * 1024), "1.0 MB");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0 GB");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 1024), "1.0 TB");
    }

    #[test]
    fn test_transfer_context_classification() {
        let internal_drive = DriveInfo {
            name: "Internal".to_string(),
            mount_point: "/".to_string(),
            device: "/dev/disk1".to_string(),
            total_bytes: 500_000_000_000,
            free_bytes: 200_000_000_000,
            used_bytes: 300_000_000_000,
            filesystem: FilesystemType::Apfs,
            drive_type: DriveType::Internal,
            removable: false,
            read_only: false,
            label: None,
            uuid: None,
        };

        let external_drive = DriveInfo {
            name: "USB Drive".to_string(),
            mount_point: "/Volumes/USB".to_string(),
            device: "/dev/disk2".to_string(),
            total_bytes: 128_000_000_000,
            free_bytes: 64_000_000_000,
            used_bytes: 64_000_000_000,
            filesystem: FilesystemType::ExFat,
            drive_type: DriveType::External,
            removable: true,
            read_only: false,
            label: Some("USB Drive".to_string()),
            uuid: None,
        };

        assert_eq!(
            LocalDriveConnector::classify_transfer(Some(&internal_drive), Some(&internal_drive)),
            TransferContext::SameDrive,
        );

        assert_eq!(
            LocalDriveConnector::classify_transfer(Some(&internal_drive), Some(&external_drive)),
            TransferContext::InternalToExternal,
        );

        assert_eq!(
            LocalDriveConnector::classify_transfer(Some(&external_drive), Some(&internal_drive)),
            TransferContext::ExternalToInternal,
        );

        assert_eq!(
            LocalDriveConnector::classify_transfer(Some(&external_drive), Some(&external_drive)),
            TransferContext::SameDrive,
        );
    }

    #[test]
    fn test_find_best_drive_match() {
        let drives = vec![
            DriveInfo {
                name: "Root".to_string(),
                mount_point: "/".to_string(),
                device: "/dev/disk1".to_string(),
                total_bytes: 500_000_000_000,
                free_bytes: 200_000_000_000,
                used_bytes: 300_000_000_000,
                filesystem: FilesystemType::Apfs,
                drive_type: DriveType::Internal,
                removable: false,
                read_only: false,
                label: None,
                uuid: None,
            },
            DriveInfo {
                name: "USB".to_string(),
                mount_point: "/Volumes/USB".to_string(),
                device: "/dev/disk2".to_string(),
                total_bytes: 128_000_000_000,
                free_bytes: 64_000_000_000,
                used_bytes: 64_000_000_000,
                filesystem: FilesystemType::ExFat,
                drive_type: DriveType::External,
                removable: true,
                read_only: false,
                label: None,
                uuid: None,
            },
        ];

        let result = find_best_drive_match(&drives, Path::new("/Volumes/USB/Documents/file.txt"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "USB");

        let result = find_best_drive_match(&drives, Path::new("/Users/test/file.txt"));
        assert!(result.is_some());
        assert_eq!(result.unwrap().name, "Root");
    }

    #[test]
    fn test_drive_info_serialization() {
        let drive = DriveInfo {
            name: "Test Drive".to_string(),
            mount_point: "/mnt/test".to_string(),
            device: "/dev/sdb1".to_string(),
            total_bytes: 256_000_000_000,
            free_bytes: 128_000_000_000,
            used_bytes: 128_000_000_000,
            filesystem: FilesystemType::Ext4,
            drive_type: DriveType::External,
            removable: true,
            read_only: false,
            label: Some("TestDrive".to_string()),
            uuid: Some("abc-123".to_string()),
        };

        let json = serde_json::to_string(&drive).unwrap();
        let deserialized: DriveInfo = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "Test Drive");
        assert_eq!(deserialized.filesystem, FilesystemType::Ext4);
        assert!(deserialized.removable);
    }

    #[test]
    fn test_preflight_result_serialization() {
        let result = PreflightResult {
            can_proceed: false,
            warnings: vec![PreflightWarning {
                code: "LOW_SPACE".to_string(),
                message: "Low space warning".to_string(),
                detail: None,
            }],
            errors: vec![PreflightError {
                code: "INSUFFICIENT_SPACE".to_string(),
                message: "Not enough space".to_string(),
                detail: Some("Need 50 GB more".to_string()),
            }],
            source_drive: None,
            dest_drive: None,
        };

        let json = serde_json::to_string(&result).unwrap();
        let deserialized: PreflightResult = serde_json::from_str(&json).unwrap();
        assert!(!deserialized.can_proceed);
        assert_eq!(deserialized.warnings.len(), 1);
        assert_eq!(deserialized.errors.len(), 1);
    }

    #[test]
    fn test_local_drive_connector_default() {
        let connector = LocalDriveConnector::new();
        assert!(!connector.is_connected());
    }

    #[tokio::test]
    async fn test_detect_drives() {
        // This should work on any platform (at minimum returns root/system drive)
        let drives = LocalDriveConnector::detect_drives().await;
        assert!(drives.is_ok());
        let drives = drives.unwrap();
        assert!(!drives.is_empty(), "Should detect at least one drive");

        // Every drive should have non-zero total space (except virtual)
        for drive in &drives {
            assert!(!drive.name.is_empty());
            assert!(!drive.mount_point.is_empty());
        }
    }

    #[tokio::test]
    async fn test_preflight_check_source_not_found() {
        let result = LocalDriveConnector::preflight_check(
            "/nonexistent/path/file.txt",
            "/tmp/output.txt",
            1024,
        )
        .await
        .unwrap();

        assert!(!result.can_proceed);
        assert!(result
            .errors
            .iter()
            .any(|e| e.code == "SOURCE_NOT_FOUND"));
    }
}
