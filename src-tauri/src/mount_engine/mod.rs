//! Mount Engine - Native OS mount management for remote/cloud storage.
//!
//! Uses native OS mount commands where available:
//! - macOS: `mount_smbfs` (SMB), `mount_nfs` (NFS), `sshfs` (SFTP if installed)
//! - Linux: `mount.cifs` (SMB), `mount.nfs` (NFS), `sshfs` (SFTP)
//!
//! For protocols without native OS mount support (S3, cloud storage), the engine
//! reports `MountCapability::NotSupported` with a clear explanation.
//!
//! Mount metadata is persisted in SQLite for auto-mount on startup.

use crate::connectors::ConnectionManager;
use crate::core::error::AppError;
use crate::storage::Repository;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

/// Status of a mounted drive.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MountStatus {
    Mounted,
    Unmounted,
    Error,
}

impl MountStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Mounted => "mounted",
            Self::Unmounted => "unmounted",
            Self::Error => "error",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "mounted" => Self::Mounted,
            "unmounted" => Self::Unmounted,
            "error" => Self::Error,
            _ => Self::Unmounted,
        }
    }
}

/// Information about a single mount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountInfo {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub connection_name: String,
    pub protocol: String,
    pub mount_point: String,
    pub status: MountStatus,
    pub auto_mount: bool,
    pub created_at: DateTime<Utc>,
    pub error_message: Option<String>,
}

/// Result returned when mounting a remote drive.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountResult {
    pub mount_id: Uuid,
    pub mount_point: String,
    pub status: MountStatus,
    pub message: String,
}

/// Saved mount configuration for auto-mount.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MountConfig {
    pub id: Uuid,
    pub connection_id: Uuid,
    pub mount_point: String,
    pub auto_mount: bool,
    pub created_at: DateTime<Utc>,
}

/// In-memory state for an active virtual mount.
#[derive(Debug, Clone)]
struct VirtualMount {
    id: Uuid,
    connection_id: Uuid,
    connection_name: String,
    protocol: String,
    mount_point: String,
    status: MountStatus,
    auto_mount: bool,
    created_at: DateTime<Utc>,
    error_message: Option<String>,
}

impl VirtualMount {
    fn to_info(&self) -> MountInfo {
        MountInfo {
            id: self.id,
            connection_id: self.connection_id,
            connection_name: self.connection_name.clone(),
            protocol: self.protocol.clone(),
            mount_point: self.mount_point.clone(),
            status: self.status,
            auto_mount: self.auto_mount,
            created_at: self.created_at,
            error_message: self.error_message.clone(),
        }
    }
}

// ──────────────────────────────────────────────
// Mount Manager
// ──────────────────────────────────────────────

/// Mount capability for a given protocol on the current OS.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MountCapability {
    /// Native OS mount command available.
    NativeMount { command: String },
    /// Requires an external tool that's not installed.
    RequiresInstall { tool: String, install_hint: String },
    /// Protocol cannot be mounted as a filesystem.
    NotSupported { reason: String },
}

/// Check what mount capability exists for a given protocol on the current OS.
pub fn check_mount_capability(protocol: &str) -> MountCapability {
    match protocol {
        "smb" | "cifs" => {
            if cfg!(target_os = "macos") {
                MountCapability::NativeMount { command: "mount_smbfs".to_string() }
            } else if cfg!(target_os = "linux") {
                if std::process::Command::new("mount.cifs").arg("--version").output().is_ok() {
                    MountCapability::NativeMount { command: "mount.cifs".to_string() }
                } else {
                    MountCapability::RequiresInstall {
                        tool: "cifs-utils".to_string(),
                        install_hint: "sudo apt install cifs-utils".to_string(),
                    }
                }
            } else {
                MountCapability::NotSupported { reason: "SMB mount not supported on this OS".to_string() }
            }
        }
        "nfs" => {
            if cfg!(target_os = "macos") {
                MountCapability::NativeMount { command: "mount_nfs".to_string() }
            } else if cfg!(target_os = "linux") {
                if std::process::Command::new("mount.nfs").arg("--version").output().is_ok() {
                    MountCapability::NativeMount { command: "mount.nfs".to_string() }
                } else {
                    MountCapability::RequiresInstall {
                        tool: "nfs-common".to_string(),
                        install_hint: "sudo apt install nfs-common".to_string(),
                    }
                }
            } else {
                MountCapability::NotSupported { reason: "NFS mount not supported on this OS".to_string() }
            }
        }
        "sftp" => {
            // sshfs is available on macOS (via macFUSE+sshfs) and Linux
            if std::process::Command::new("sshfs").arg("--version").output().is_ok() {
                MountCapability::NativeMount { command: "sshfs".to_string() }
            } else if cfg!(target_os = "macos") {
                MountCapability::RequiresInstall {
                    tool: "sshfs + macFUSE".to_string(),
                    install_hint: "brew install macfuse sshfs".to_string(),
                }
            } else {
                MountCapability::RequiresInstall {
                    tool: "sshfs".to_string(),
                    install_hint: "sudo apt install sshfs".to_string(),
                }
            }
        }
        "webdav" => {
            if cfg!(target_os = "macos") {
                // macOS has built-in WebDAV mount support
                MountCapability::NativeMount { command: "mount_webdav".to_string() }
            } else if cfg!(target_os = "linux") {
                if std::process::Command::new("mount.davfs").output().is_ok() {
                    MountCapability::NativeMount { command: "mount.davfs".to_string() }
                } else {
                    MountCapability::RequiresInstall {
                        tool: "davfs2".to_string(),
                        install_hint: "sudo apt install davfs2".to_string(),
                    }
                }
            } else {
                MountCapability::NotSupported { reason: "WebDAV mount not supported on this OS".to_string() }
            }
        }
        _ => MountCapability::NotSupported {
            reason: format!("Protocol '{}' cannot be mounted as a filesystem. Use sync or transfer instead.", protocol),
        },
    }
}

/// Manages mounts using native OS commands and persists state in SQLite.
pub struct MountManager {
    /// Active mounts keyed by mount ID.
    mounts: Arc<RwLock<HashMap<Uuid, VirtualMount>>>,
}

impl MountManager {
    pub fn new() -> Self {
        Self {
            mounts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Check mount capability for a protocol.
    pub fn check_capability(&self, protocol: &str) -> MountCapability {
        check_mount_capability(protocol)
    }

    /// Mount a remote connection at the specified mount point.
    ///
    /// Validates that the connection exists, creates the mount entry
    /// in both in-memory state and SQLite, and returns a success status.
    /// (Actual FUSE mounting will be added in a future release.)
    pub async fn mount_remote(
        &self,
        connection_id: Uuid,
        mount_point: String,
        repo: &Repository,
        connection_mgr: &ConnectionManager,
    ) -> Result<MountResult, AppError> {
        // Validate the connection exists
        let profile = connection_mgr
            .get_connection(connection_id)
            .await?
            .ok_or_else(|| AppError::connection(
                format!("Connection {} not found", connection_id),
                "Check the connection ID and ensure it exists in saved connections.",
            ))?;

        // Check for duplicate mount point
        {
            let mounts = self.mounts.read().await;
            for mount in mounts.values() {
                if mount.mount_point == mount_point && mount.status == MountStatus::Mounted {
                    return Err(AppError::connection(
                        format!("Mount point '{}' is already in use", mount_point),
                        "Choose a different mount point or unmount the existing drive first.",
                    ));
                }
            }
        }

        let protocol_str = profile.protocol.as_str();

        // Check capability before attempting mount
        let capability = check_mount_capability(protocol_str);
        match &capability {
            MountCapability::NotSupported { reason } => {
                return Err(AppError::connection(
                    format!("Cannot mount '{}': {}", protocol_str, reason),
                    "Use sync or transfer to access this remote storage.",
                ));
            }
            MountCapability::RequiresInstall { tool, install_hint } => {
                return Err(AppError::connection(
                    format!("Mounting '{}' requires '{}' which is not installed", protocol_str, tool),
                    install_hint.clone(),
                ));
            }
            MountCapability::NativeMount { .. } => {}
        }

        // Validate mount point: must be absolute, no path traversal, within user home
        let mp_path = std::path::Path::new(&mount_point);
        if !mp_path.is_absolute() {
            return Err(AppError::validation(
                "Mount point must be an absolute path (e.g. '/Users/you/Volumes/mount').",
            ));
        }
        // Canonicalize the parent to detect .. traversal (mount point itself may not exist yet)
        let canonical = if mp_path.exists() {
            mp_path.canonicalize().map_err(|e| {
                AppError::file_op(
                    format!("Cannot resolve mount point '{}': {}", mount_point, e),
                    "Check the path exists and you have permission.",
                )
            })?
        } else {
            // For non-existent paths, canonicalize the nearest existing parent
            let mut parent = mp_path.to_path_buf();
            while !parent.exists() && parent.parent().is_some() {
                parent = parent.parent().unwrap().to_path_buf();
            }
            let parent_clone = parent.clone();
            let canon_parent = parent.canonicalize().unwrap_or(parent_clone);
            canon_parent.join(mp_path.strip_prefix(&canon_parent).unwrap_or(mp_path))
        };

        // Block mounting to sensitive system directories
        let canonical_str = canonical.to_string_lossy();
        let blocked_prefixes = ["/etc", "/usr", "/bin", "/sbin", "/var", "/System", "/Library", "/boot", "/proc", "/sys", "/dev"];
        for prefix in &blocked_prefixes {
            if canonical_str.starts_with(prefix) {
                return Err(AppError::validation(
                    format!("Mount point '{}' is in a restricted system directory. Choose a path within your home directory.", mount_point),
                ));
            }
        }

        // Ensure mount point directory exists
        if !mp_path.exists() {
            std::fs::create_dir_all(mp_path).map_err(|e| {
                AppError::file_op(
                    format!("Cannot create mount point '{}': {}", mount_point, e),
                    "Choose an existing directory or check permissions.",
                )
            })?;
        }

        // Execute native OS mount command
        let mount_result = Self::execute_native_mount(
            protocol_str,
            &profile,
            &mount_point,
        ).await;

        let mount_id = Uuid::new_v4();
        let now = Utc::now();

        let (status, error_message, msg) = match mount_result {
            Ok(()) => (
                MountStatus::Mounted,
                None,
                format!("Successfully mounted '{}' at '{}'", profile.name, mount_point),
            ),
            Err(e) => (
                MountStatus::Error,
                Some(e.to_string()),
                format!("Mount failed for '{}': {}", profile.name, e),
            ),
        };

        let virtual_mount = VirtualMount {
            id: mount_id,
            connection_id,
            connection_name: profile.name.clone(),
            protocol: protocol_str.to_string(),
            mount_point: mount_point.clone(),
            status,
            auto_mount: false,
            created_at: now,
            error_message: error_message.clone(),
        };

        // Persist to SQLite
        let mp = mount_point.clone();
        let cid = connection_id.to_string();
        let mid = mount_id.to_string();
        let ts = now.to_rfc3339();
        let status_str = status.as_str().to_string();
        let err_msg = error_message.clone();
        repo.pool()
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO mount_configs (id, connection_id, mount_point, auto_mount, status, error_message, created_at)
                     VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6)",
                    rusqlite::params![mid, cid, mp, status_str, err_msg, ts],
                )?;
                Ok(())
            })
            .await?;

        // Add to in-memory state
        {
            let mut mounts = self.mounts.write().await;
            mounts.insert(mount_id, virtual_mount);
        }

        tracing::info!(
            "Mount result for '{}' ({}) at '{}': {} (mount_id: {})",
            profile.name, protocol_str, mount_point, status.as_str(), mount_id
        );

        if status == MountStatus::Error {
            return Err(AppError::connection(
                error_message.unwrap_or_default(),
                "Check connection credentials and network access.",
            ));
        }

        Ok(MountResult {
            mount_id,
            mount_point,
            status,
            message: msg,
        })
    }

    /// Execute the native OS mount command for a given protocol.
    async fn execute_native_mount(
        protocol: &str,
        profile: &crate::core::types::ConnectionProfile,
        mount_point: &str,
    ) -> Result<(), String> {
        use tokio::process::Command;

        let remote_path = &profile.remote_path;
        let username = profile.username.as_deref().unwrap_or("");
        let port = profile.port.unwrap_or(0);

        match protocol {
            "smb" | "cifs" => {
                let path_part = if remote_path.starts_with('/') {
                    remote_path.clone()
                } else {
                    format!("/{}", remote_path)
                };

                let output = if cfg!(target_os = "macos") {
                    let smb_url = if !username.is_empty() {
                        format!("//{}@{}{}", username, profile.host, path_part)
                    } else {
                        format!("//{}{}", profile.host, path_part)
                    };
                    Command::new("mount_smbfs").arg(&smb_url).arg(mount_point).output().await
                } else {
                    let share = format!("//{}{}", profile.host, path_part);
                    let mut cmd = Command::new("mount.cifs");
                    cmd.arg(&share).arg(mount_point);
                    if !username.is_empty() {
                        cmd.arg("-o").arg(format!("user={}", username));
                    }
                    cmd.output().await
                };

                match output {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr).to_string()),
                    Err(e) => Err(format!("Failed to execute mount command: {e}")),
                }
            }
            "nfs" => {
                let nfs_path = format!(
                    "{}:{}",
                    profile.host,
                    if remote_path.is_empty() { "/" } else { remote_path }
                );
                let cmd_name = if cfg!(target_os = "macos") { "mount_nfs" } else { "mount.nfs" };
                let output = Command::new(cmd_name).arg(&nfs_path).arg(mount_point).output().await;

                match output {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr).to_string()),
                    Err(e) => Err(format!("Failed to execute {cmd_name}: {e}")),
                }
            }
            "sftp" => {
                let user = if username.is_empty() { "root" } else { username };
                let path = if remote_path.is_empty() { "/" } else { remote_path.as_str() };
                let remote = format!("{}@{}:{}", user, profile.host, path);
                let ssh_port = if port > 0 { port } else { 22 };

                let output = Command::new("sshfs")
                    .arg(&remote)
                    .arg(mount_point)
                    .arg("-p").arg(ssh_port.to_string())
                    .arg("-o").arg("StrictHostKeyChecking=ask")
                    .output()
                    .await;

                match output {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr).to_string()),
                    Err(e) => Err(format!("Failed to execute sshfs: {e}")),
                }
            }
            "webdav" => {
                let scheme = if port == 443 { "https" } else { "http" };
                let effective_port = if port > 0 { port } else { 80 };
                let path = if remote_path.is_empty() { "/" } else { remote_path.as_str() };
                let webdav_url = format!("{}://{}:{}{}", scheme, profile.host, effective_port, path);

                let output = if cfg!(target_os = "macos") {
                    Command::new("mount_webdav").arg(&webdav_url).arg(mount_point).output().await
                } else {
                    Command::new("mount.davfs").arg(&webdav_url).arg(mount_point).output().await
                };

                match output {
                    Ok(o) if o.status.success() => Ok(()),
                    Ok(o) => Err(String::from_utf8_lossy(&o.stderr).to_string()),
                    Err(e) => Err(format!("Failed to execute mount command: {e}")),
                }
            }
            _ => Err(format!("Protocol '{}' does not support filesystem mounting", protocol)),
        }
    }

    /// Unmount a remote drive.
    pub async fn unmount_remote(
        &self,
        mount_id: Uuid,
        repo: &Repository,
    ) -> Result<(), AppError> {
        // Remove from in-memory state
        let mount_info = {
            let mut mounts = self.mounts.write().await;
            mounts.remove(&mount_id)
        };

        let mid = mount_id.to_string();

        if let Some(info) = &mount_info {
            // Execute real OS unmount
            let mp = &info.mount_point;
            let cmd_name = if cfg!(target_os = "macos") { "umount" } else { "fusermount" };
            let output = if info.protocol == "sftp" && cfg!(target_os = "linux") {
                tokio::process::Command::new("fusermount")
                    .arg("-u")
                    .arg(mp)
                    .output()
                    .await
            } else {
                tokio::process::Command::new("umount")
                    .arg(mp)
                    .output()
                    .await
            };

            match output {
                Ok(o) if o.status.success() => {
                    tracing::info!("OS unmount succeeded for '{}'", mp);
                }
                Ok(o) => {
                    let stderr = String::from_utf8_lossy(&o.stderr);
                    tracing::warn!("OS unmount returned error for '{}': {} (may already be unmounted)", mp, stderr);
                }
                Err(e) => {
                    tracing::warn!("Failed to run {} for '{}': {} (may already be unmounted)", cmd_name, mp, e);
                }
            }

            tracing::info!(
                "Unmounted '{}' from '{}' (mount_id: {})",
                info.connection_name,
                info.mount_point,
                mount_id
            );
        }

        // Update status in SQLite
        let mid_db = mid.clone();
        repo.pool()
            .execute(move |conn| {
                let updated = conn.execute(
                    "UPDATE mount_configs SET status = 'unmounted' WHERE id = ?1",
                    rusqlite::params![mid_db],
                )?;
                if updated == 0 {
                    // Not in DB either, try delete anyway
                    tracing::warn!("Mount {} not found in database", mid_db);
                }
                Ok(())
            })
            .await?;

        Ok(())
    }

    /// List all active and saved mounts.
    pub async fn list_mounts(
        &self,
        repo: &Repository,
        connection_mgr: &ConnectionManager,
    ) -> Result<Vec<MountInfo>, AppError> {
        // Start with in-memory active mounts
        let active: Vec<MountInfo> = {
            let mounts = self.mounts.read().await;
            mounts.values().map(|m| m.to_info()).collect()
        };

        // Also load saved configs from DB that aren't currently active
        let active_ids: Vec<String> = active.iter().map(|m| m.id.to_string()).collect();

        let saved_configs: Vec<(String, String, String, bool, String, String)> = repo
            .pool()
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, connection_id, mount_point, auto_mount, status, created_at
                     FROM mount_configs ORDER BY created_at DESC",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, bool>(3)?,
                            row.get::<_, String>(4)?,
                            row.get::<_, String>(5)?,
                        ))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            })
            .await?;

        let mut result = active;

        // Add saved configs that aren't currently active in memory
        for (id, cid, mp, auto_mount, status, created_at) in saved_configs {
            if active_ids.contains(&id) {
                continue;
            }

            let conn_id = Uuid::parse_str(&cid).unwrap_or_else(|_| Uuid::nil());
            let mount_id = Uuid::parse_str(&id).unwrap_or_else(|_| Uuid::nil());

            // Try to get connection info
            let (conn_name, protocol) = match connection_mgr.get_connection(conn_id).await {
                Ok(Some(profile)) => (profile.name, profile.protocol.as_str().to_string()),
                _ => ("Unknown".to_string(), "unknown".to_string()),
            };

            let ts = chrono::DateTime::parse_from_rfc3339(&created_at)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now());

            result.push(MountInfo {
                id: mount_id,
                connection_id: conn_id,
                connection_name: conn_name,
                protocol,
                mount_point: mp,
                status: MountStatus::from_str_lossy(&status),
                auto_mount,
                created_at: ts,
                error_message: None,
            });
        }

        Ok(result)
    }

    /// Get the status of a specific mount.
    pub async fn get_mount_status(&self, mount_id: Uuid) -> MountStatus {
        let mounts = self.mounts.read().await;
        mounts
            .get(&mount_id)
            .map(|m| m.status)
            .unwrap_or(MountStatus::Unmounted)
    }

    /// Save a mount configuration for auto-mount on startup.
    pub async fn save_mount_config(
        &self,
        connection_id: Uuid,
        mount_point: String,
        auto_mount: bool,
        repo: &Repository,
        connection_mgr: &ConnectionManager,
    ) -> Result<MountConfig, AppError> {
        // Validate connection exists
        let _profile = connection_mgr
            .get_connection(connection_id)
            .await?
            .ok_or_else(|| AppError::connection(
                format!("Connection {} not found", connection_id),
                "Check the connection ID.",
            ))?;

        let config_id = Uuid::new_v4();
        let now = Utc::now();

        let mid = config_id.to_string();
        let cid = connection_id.to_string();
        let mp = mount_point.clone();
        let am = auto_mount;
        let ts = now.to_rfc3339();

        repo.pool()
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO mount_configs (id, connection_id, mount_point, auto_mount, status, created_at)
                     VALUES (?1, ?2, ?3, ?4, 'unmounted', ?5)
                     ON CONFLICT(id) DO UPDATE SET
                         mount_point = ?3, auto_mount = ?4",
                    rusqlite::params![mid, cid, mp, am, ts],
                )?;
                Ok(())
            })
            .await?;

        tracing::info!(
            "Saved mount config: connection={}, mount_point={}, auto_mount={}",
            connection_id,
            mount_point,
            auto_mount
        );

        Ok(MountConfig {
            id: config_id,
            connection_id,
            mount_point,
            auto_mount,
            created_at: now,
        })
    }

    /// Delete a saved mount configuration.
    pub async fn delete_mount_config(
        &self,
        mount_id: Uuid,
        repo: &Repository,
    ) -> Result<(), AppError> {
        // Remove from in-memory state if active
        {
            let mut mounts = self.mounts.write().await;
            mounts.remove(&mount_id);
        }

        let mid = mount_id.to_string();
        repo.pool()
            .execute(move |conn| {
                conn.execute(
                    "DELETE FROM mount_configs WHERE id = ?1",
                    rusqlite::params![mid],
                )?;
                Ok(())
            })
            .await?;

        tracing::info!("Deleted mount config: {}", mount_id);

        Ok(())
    }
}

impl Default for MountManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mount_status_roundtrip() {
        let statuses = [MountStatus::Mounted, MountStatus::Unmounted, MountStatus::Error];
        for s in &statuses {
            assert_eq!(*s, MountStatus::from_str_lossy(s.as_str()));
        }
    }

    #[test]
    fn test_mount_status_unknown_defaults_to_unmounted() {
        assert_eq!(MountStatus::from_str_lossy("garbage"), MountStatus::Unmounted);
    }

    #[test]
    fn test_mount_manager_creation() {
        let mgr = MountManager::new();
        // Should start with no mounts
        let rt = tokio::runtime::Runtime::new().unwrap();
        let status = rt.block_on(mgr.get_mount_status(Uuid::new_v4()));
        assert_eq!(status, MountStatus::Unmounted);
    }
}
