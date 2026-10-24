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
//!
//! rclone mounts take cache options (up to full offline caching, with
//! "make available offline" pinning) and follow the organization's mount
//! profile (`policy`).

pub mod policy;
pub mod rclone_mount;
pub mod webdav_mount;

pub use policy::{CacheMode, MountOptions};

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
    /// Cache and read-only options in force (after the mount profile).
    #[serde(default)]
    pub options: MountOptions,
    /// What the organization's mount profile changed.
    #[serde(default)]
    pub policy_notes: Vec<String>,
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
    options: MountOptions,
    policy_notes: Vec<String>,
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
            options: self.options.clone(),
            policy_notes: self.policy_notes.clone(),
        }
    }
}

/// Statements the engine writes mounts with (checked against the schema in tests).
const INSERT_MOUNT_SQL: &str = "INSERT INTO mount_configs (id, connection_id, mount_point, auto_mount, status, error_message, created_at, options_json) VALUES (?1, ?2, ?3, 0, ?4, ?5, ?6, ?7)";
const UPDATE_MOUNT_STATUS_SQL: &str = "UPDATE mount_configs SET status = ?2, error_message = ?3 WHERE id = ?1";

fn dir_size(p: &std::path::Path) -> u64 {
    std::fs::read_dir(p)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .map(|e| match e.file_type() {
                    Ok(t) if t.is_dir() => dir_size(&e.path()),
                    Ok(t) if t.is_file() => e.metadata().map(|m| m.len()).unwrap_or(0),
                    _ => 0,
                })
                .sum()
        })
        .unwrap_or(0)
}

/// A saved mount configuration as stored.
#[derive(Debug, Clone)]
struct SavedConfig {
    connection_id: Uuid,
    mount_point: String,
    auto_mount: bool,
    options: MountOptions,
    created_at: DateTime<Utc>,
}

async fn load_saved(repo: &Repository, id: Uuid) -> Result<SavedConfig, AppError> {
    let key = id.to_string();
    let row: Option<(String, String, bool, String, String)> = repo
        .pool()
        .execute(move |conn| {
            Ok(rusqlite::OptionalExtension::optional(conn.query_row(
                "SELECT connection_id, mount_point, auto_mount, options_json, created_at FROM mount_configs WHERE id = ?1",
                rusqlite::params![key],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            ))?)
        })
        .await?;
    let (cid, mount_point, auto_mount, options_json, created_at) = row.ok_or_else(|| AppError::validation("That saved mount no longer exists"))?;
    Ok(SavedConfig {
        connection_id: Uuid::parse_str(&cid).map_err(|_| AppError::validation("The saved mount refers to an invalid connection"))?,
        mount_point,
        auto_mount,
        options: serde_json::from_str(&options_json).unwrap_or_default(),
        created_at: DateTime::parse_from_rfc3339(&created_at).map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now()),
    })
}

/// Result of "unmount all".
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct UnmountAllReport {
    pub unmounted: usize,
    /// `name: reason` for each drive that could not be unmounted.
    pub failed: Vec<String>,
}

/// Result of "make available offline".
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PinReport {
    pub files: u64,
    pub bytes: u64,
    /// Files that could not be read (offline already, permissions…).
    pub failed: u64,
    /// Stopped at the cache size limit.
    pub truncated: bool,
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

/// Expand a leading `~` to the home directory.
pub fn expand_home(p: &str) -> String {
    let home = || directories::UserDirs::new().map(|d| d.home_dir().to_path_buf());
    match p.strip_prefix("~") {
        Some(rest) if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') => match home() {
            Some(h) => format!("{}{}", h.display(), rest),
            None => p.to_string(),
        },
        _ => p.to_string(),
    }
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
        // Cloud storage mounts through rclone + FUSE.
        "rclone" | "s3" => {
            if !rclone_mount::rclone_available() {
                MountCapability::RequiresInstall { tool: "rclone".to_string(), install_hint: "Install rclone from https://rclone.org/install/".to_string() }
            } else {
                match rclone_mount::fuse_available() {
                    Ok(()) => MountCapability::NativeMount { command: "rclone mount".to_string() },
                    Err((tool, install_hint)) => MountCapability::RequiresInstall { tool, install_hint },
                }
            }
        }
        "google_drive" | "dropbox" | "onedrive" | "azure_blob" | "adls_gen2" | "azure_files" | "b2" | "swift" => MountCapability::NotSupported {
            reason: "Add this account as an rclone remote (`rclone config`), create an rclone connection for it, and mount that.".to_string(),
        },
        _ => MountCapability::NotSupported {
            reason: format!("Protocol '{}' cannot be mounted as a filesystem. Use sync or transfer instead.", protocol),
        },
    }
}

/// Manages mounts using native OS commands and persists state in SQLite.
///
/// `Clone` is cheap and shares state — the only field is an `Arc`. Cloning is
/// what lets startup hand a handle to the background auto-mount task before the
/// manager is moved into Tauri's managed state.
#[derive(Clone)]
pub struct MountManager {
    /// Active mounts keyed by mount ID.
    mounts: Arc<RwLock<HashMap<Uuid, VirtualMount>>>,
    /// rclone processes serving cloud mounts.
    rclone: rclone_mount::RcloneProcesses,
    /// The organization's mount profile (managed settings).
    managed: Arc<RwLock<Option<ufop_core::agent::ManagedMounts>>>,
    /// rclone VFS caches, one folder per mount.
    cache_root: std::path::PathBuf,
    /// Bumped whenever a mount's state or a saved configuration changes (tray, UI refresh).
    changed: Arc<tokio::sync::watch::Sender<u64>>,
}

impl MountManager {
    pub fn new() -> Self {
        let cache_root = crate::paths::cache_dir().join("mounts");
        Self::with_cache_root(cache_root)
    }

    pub fn with_cache_root(cache_root: impl Into<std::path::PathBuf>) -> Self {
        Self {
            mounts: Arc::new(RwLock::new(HashMap::new())),
            rclone: rclone_mount::RcloneProcesses::default(),
            managed: Arc::new(RwLock::new(None)),
            cache_root: cache_root.into(),
            changed: Arc::new(tokio::sync::watch::channel(0).0),
        }
    }

    /// A receiver that fires when any mount changes state or a saved mount is
    /// added, edited or removed.
    pub fn subscribe(&self) -> tokio::sync::watch::Receiver<u64> {
        self.changed.subscribe()
    }

    fn notify(&self) {
        self.changed.send_modify(|n| *n = n.wrapping_add(1));
    }

    /// Apply (or release) the organization's mount profile; it governs new mounts.
    pub async fn set_managed(&self, profile: Option<ufop_core::agent::ManagedMounts>) {
        *self.managed.write().await = profile;
    }

    pub async fn managed(&self) -> Option<ufop_core::agent::ManagedMounts> {
        self.managed.read().await.clone()
    }

    fn cache_dir(&self, mount_id: Uuid) -> std::path::PathBuf {
        self.cache_root.join(mount_id.to_string())
    }

    /// The cache folder of a mount: its own folder under the chosen cache
    /// folder, or under the app's cache folder. Always a folder this mount
    /// owns, so clearing it never touches anything else.
    fn cache_dir_for(&self, mount_id: Uuid, o: &MountOptions) -> std::path::PathBuf {
        match o.cache_dir.as_deref() {
            Some(custom) => std::path::PathBuf::from(expand_home(custom)).join(mount_id.to_string()),
            None => self.cache_dir(mount_id),
        }
    }

    /// The cache folder of a mount, from its options (running or saved).
    async fn resolve_cache_dir(&self, mount_id: Uuid, repo: &Repository) -> std::path::PathBuf {
        let options = match self.mounts.read().await.get(&mount_id) {
            Some(m) => Some(m.options.clone()),
            None => load_saved(repo, mount_id).await.ok().map(|c| c.options),
        };
        self.cache_dir_for(mount_id, &options.unwrap_or_default())
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
        self.mount_remote_with(connection_id, mount_point, MountOptions::default(), None, repo, connection_mgr).await
    }

    /// Mount with cache/read-only `options` (subject to the mount profile).
    /// `config_id` mounts a saved configuration in place (its row is updated,
    /// not duplicated).
    pub async fn mount_remote_with(
        &self,
        connection_id: Uuid,
        mount_point: String,
        options: MountOptions,
        config_id: Option<Uuid>,
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

        // On Windows a chosen drive letter is the mount point ("X:"); there is no folder to check.
        let os = policy::Os::current();
        let drive = policy::drive_mount_point(&options, os);
        // "~/x" (what the mount dialog suggests) means the home folder.
        let mount_point = drive.clone().unwrap_or_else(|| expand_home(&mount_point));
        // Validate mount point: must be absolute, no path traversal, within user home
        let mp_path = std::path::Path::new(&mount_point);
        if drive.is_none() && !mp_path.is_absolute() {
            return Err(AppError::validation(
                "Mount point must be an absolute path (e.g. '/Users/you/Volumes/mount').",
            ));
        }
        // Canonicalize the parent to detect .. traversal (mount point itself may not exist yet)
        let canonical = if drive.is_some() {
            mp_path.to_path_buf()
        } else if mp_path.exists() {
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

        // The organization's mount profile: allowed protocols, mount root, forced options.
        let is_rclone = matches!(protocol_str, "rclone" | "s3");
        let (options, policy_notes) = policy::effective_on(&options, protocol_str, &canonical, self.managed().await.as_ref(), os)?;
        // A subfolder mounts that part of the connection (rclone remote path or native share path).
        let mut profile = profile;
        profile.remote_path = policy::join_subfolder(&profile.remote_path, options.subfolder.as_deref());
        if !is_rclone && options.read_only {
            return Err(AppError::PermissionDenied {
                message: format!("{protocol_str} drives can't be mounted read-only here, and read-only is required"),
                advice: "Mount it through an rclone connection, or ask your administrator.".into(),
            });
        }

        // Ensure mount point directory exists
        if drive.is_none() && !mp_path.exists() {
            std::fs::create_dir_all(mp_path).map_err(|e| {
                AppError::file_op(
                    format!("Cannot create mount point '{}': {}", mount_point, e),
                    "Choose an existing directory or check permissions.",
                )
            })?;
        }

        let mount_id = config_id.unwrap_or_else(Uuid::new_v4);
        let mount_result = if is_rclone {
            let secret = connection_mgr.get_password(connection_id).await.ok().flatten();
            match rclone_mount::spec_for(&profile, secret.as_deref()) {
                Ok(spec) => {
                    let flags = policy::rclone_flags_on(&options, &self.cache_dir_for(mount_id, &options), os);
                    self.rclone.mount(mount_id, &spec, &mount_point, &flags).await
                }
                Err(e) => Err(e),
            }
        } else {
            Self::execute_native_mount(protocol_str, &profile, &mount_point).await
        };
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

        // A saved configuration keeps the auto-mount choice it was saved with.
        let auto_mount = match config_id {
            Some(id) => load_saved(repo, id).await.map(|c| c.auto_mount).unwrap_or(false),
            None => false,
        };
        let virtual_mount = VirtualMount {
            id: mount_id,
            connection_id,
            connection_name: profile.name.clone(),
            protocol: protocol_str.to_string(),
            mount_point: mount_point.clone(),
            status,
            auto_mount,
            created_at: now,
            error_message: error_message.clone(),
            options: options.clone(),
            policy_notes: policy_notes.clone(),
        };

        // Persist to SQLite: a saved configuration keeps its row.
        let mp = mount_point.clone();
        let cid = connection_id.to_string();
        let mid = mount_id.to_string();
        let ts = now.to_rfc3339();
        let status_str = status.as_str().to_string();
        let err_msg = error_message.clone();
        let opts = serde_json::to_string(&options)?;
        let saved = config_id.is_some();
        repo.pool()
            .execute(move |conn| {
                if saved {
                    conn.execute(UPDATE_MOUNT_STATUS_SQL, rusqlite::params![mid, status_str, err_msg])?;
                } else {
                    conn.execute(INSERT_MOUNT_SQL, rusqlite::params![mid, cid, mp, status_str, err_msg, ts, opts])?;
                }
                Ok(())
            })
            .await?;

        // Add to in-memory state
        {
            let mut mounts = self.mounts.write().await;
            mounts.insert(mount_id, virtual_mount);
        }
        self.notify();

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
            message: if policy_notes.is_empty() { msg } else { format!("{msg} ({})", policy_notes.join(", ")) },
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
            if self.rclone.unmount(mount_id, &info.mount_point).await {
                tracing::info!("Unmounted rclone mount '{}' (mount_id: {})", info.mount_point, mount_id);
            }
        }
        if let Some(info) = mount_info.as_ref().filter(|i| !matches!(i.protocol.as_str(), "rclone" | "s3")) {
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

        self.notify();
        Ok(())
    }

    /// Re-mount every saved config flagged `auto_mount`, honouring the promise
    /// in this module's header ("persisted in SQLite for auto-mount on startup")
    /// and the "Auto-mount" checkbox in the mount dialog.
    ///
    /// Until this existed there was no restore path at all: `save_mount_config`
    /// wrote the flag and `list_mounts` read it back purely to render "Auto",
    /// so the checkbox was decorative — the user ticked it, restarted, and
    /// nothing mounted.
    ///
    /// Returns `(mounted, failed)` counts. Never returns `Err`: a mount that
    /// fails (server down, credentials expired, mount point gone) is logged and
    /// skipped, because one unreachable server must not cost the user the rest.
    /// Callers must run this OFF the startup path — `mount_remote` shells out to
    /// `mount_smbfs`/`sshfs`, which can block for a long time on an unreachable
    /// host, and app boot must never wait on a network mount.
    pub async fn auto_mount_saved(
        &self,
        repo: &Repository,
        connection_mgr: &ConnectionManager,
    ) -> (usize, usize) {
        let configs: Vec<(String, String, String, String)> = match repo
            .pool()
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT connection_id, mount_point, id, options_json FROM mount_configs \
                     WHERE auto_mount = 1 ORDER BY created_at ASC",
                )?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            })
            .await
        {
            Ok(rows) => rows,
            Err(e) => {
                tracing::warn!("Auto-mount: could not read saved mounts: {e}");
                return (0, 0);
            }
        };

        let (mut mounted, mut failed) = (0usize, 0usize);
        for (connection_id, mount_point, config_id, options_json) in configs {
            let Ok(cid) = Uuid::parse_str(&connection_id) else {
                tracing::warn!("Auto-mount: skipping config with invalid connection id {connection_id}");
                failed += 1;
                continue;
            };
            // `mount_remote` is idempotent enough for this: it rejects a
            // duplicate mount point that is already Mounted, so a race with a
            // manual mount degrades to a logged skip rather than a double mount.
            let options: MountOptions = serde_json::from_str(&options_json).unwrap_or_default();
            let config_id = Uuid::parse_str(&config_id).ok();
            match self.mount_remote_with(cid, mount_point.clone(), options, config_id, repo, connection_mgr).await {
                Ok(_) => {
                    mounted += 1;
                    tracing::info!("Auto-mounted {mount_point}");
                }
                Err(e) => {
                    failed += 1;
                    tracing::warn!("Auto-mount of {mount_point} failed (non-fatal): {e}");
                }
            }
        }

        if mounted > 0 || failed > 0 {
            tracing::info!("Auto-mount complete: {mounted} mounted, {failed} failed");
        }
        (mounted, failed)
    }

    /// Mount a saved configuration with its own options (its row is kept).
    pub async fn mount_saved(&self, id: Uuid, repo: &Repository, connection_mgr: &ConnectionManager) -> Result<MountResult, AppError> {
        if self.mounts.read().await.get(&id).is_some_and(|m| m.status == MountStatus::Mounted) {
            return Err(AppError::validation("That drive is already mounted"));
        }
        let c = load_saved(repo, id).await?;
        self.mount_remote_with(c.connection_id, c.mount_point, c.options, Some(id), repo, connection_mgr).await
    }

    /// The saved configuration `id` as the Mounts list shows it (and the options to mount it with).
    pub async fn saved_options(&self, id: Uuid, repo: &Repository) -> Result<MountOptions, AppError> {
        Ok(load_saved(repo, id).await?.options)
    }

    /// Change a saved mount (connection, mount point, auto-mount, options).
    /// The drive must be unmounted; the new settings apply to the next mount.
    #[allow(clippy::too_many_arguments)]
    pub async fn update_mount_config(
        &self,
        id: Uuid,
        connection_id: Option<Uuid>,
        mount_point: String,
        auto_mount: bool,
        options: MountOptions,
        repo: &Repository,
        connection_mgr: &ConnectionManager,
    ) -> Result<MountConfig, AppError> {
        if self.mounts.read().await.get(&id).is_some_and(|m| m.status == MountStatus::Mounted) {
            return Err(AppError::validation("Unmount the drive before editing it"));
        }
        let existing = load_saved(repo, id).await?;
        let connection_id = connection_id.unwrap_or(existing.connection_id);
        let profile = connection_mgr.get_connection(connection_id).await?.ok_or_else(|| {
            AppError::connection(format!("Connection {connection_id} not found"), "Check the connection ID and ensure it exists in saved connections.")
        })?;
        let os = policy::Os::current();
        let mount_point = mount_point.trim().to_string();
        if mount_point.is_empty() && policy::drive_mount_point(&options, os).is_none() {
            return Err(AppError::validation("Mount point cannot be empty"));
        }
        let mp = policy::drive_mount_point(&options, os).unwrap_or_else(|| expand_home(&mount_point));
        let options = policy::validate_on(&options, profile.protocol.as_str(), std::path::Path::new(&mp), os)?;
        let (cid, mid, mpc, opts) = (connection_id.to_string(), id.to_string(), mp.clone(), serde_json::to_string(&options)?);
        repo.pool()
            .execute(move |conn| {
                let clash: Option<String> = rusqlite::OptionalExtension::optional(conn.query_row(
                    "SELECT id FROM mount_configs WHERE connection_id = ?1 AND mount_point = ?2 AND id != ?3",
                    rusqlite::params![cid, mpc, mid],
                    |r| r.get(0),
                ))?;
                if clash.is_some() {
                    return Ok(false);
                }
                conn.execute(
                    "UPDATE mount_configs SET connection_id = ?2, mount_point = ?3, auto_mount = ?4, options_json = ?5, status = 'unmounted', error_message = NULL WHERE id = ?1",
                    rusqlite::params![mid, cid, mpc, auto_mount, opts],
                )?;
                Ok(true)
            })
            .await?
            .then_some(())
            .ok_or_else(|| AppError::validation("Another saved mount already uses that connection and mount point"))?;
        // A failed earlier attempt is not the state of the edited mount.
        self.mounts.write().await.remove(&id);
        self.notify();
        Ok(MountConfig { id, connection_id, mount_point: mp, auto_mount, created_at: existing.created_at })
    }

    /// Unmount every mounted drive; drives that refuse are reported, not fatal.
    pub async fn unmount_all(&self, repo: &Repository) -> UnmountAllReport {
        let targets: Vec<(Uuid, String)> =
            self.mounts.read().await.values().filter(|m| m.status == MountStatus::Mounted).map(|m| (m.id, m.connection_name.clone())).collect();
        let mut report = UnmountAllReport::default();
        for (id, name) in targets {
            match self.unmount_remote(id, repo).await {
                Ok(()) => report.unmounted += 1,
                Err(e) => report.failed.push(format!("{name}: {e}")),
            }
        }
        report
    }

    /// Mounts that were Mounted but are gone (rclone stopped, unmounted outside
    /// the app) become errors instead of staying "mounted".
    async fn reconcile(&self, repo: &Repository) {
        #[cfg(unix)]
        {
            let mounted: Vec<(Uuid, String)> =
                self.mounts.read().await.values().filter(|m| m.status == MountStatus::Mounted).map(|m| (m.id, m.mount_point.clone())).collect();
            if mounted.is_empty() {
                return;
            }
            let gone: Vec<(Uuid, String)> = tokio::task::spawn_blocking(move || {
                mounted.into_iter().filter(|(_, p)| !rclone_mount::is_mounted(std::path::Path::new(p))).collect()
            })
            .await
            .unwrap_or_default();
            for (id, mount_point) in gone {
                let message = "The drive is no longer mounted (it was disconnected or unmounted outside UFOP)".to_string();
                if let Some(m) = self.mounts.write().await.get_mut(&id) {
                    m.status = MountStatus::Error;
                    m.error_message = Some(message.clone());
                }
                // Stop what is left of the process and clear a stale mount.
                self.rclone.unmount(id, &mount_point).await;
                let (mid, msg) = (id.to_string(), message);
                if let Err(e) = repo.pool().execute(move |c| Ok(c.execute(UPDATE_MOUNT_STATUS_SQL, rusqlite::params![mid, MountStatus::Error.as_str(), msg])?)).await {
                    tracing::warn!("Could not record that mount {id} stopped: {e}");
                }
                self.notify();
            }
        }
        #[cfg(not(unix))]
        let _ = repo;
    }

    /// Every mount: the running ones, then the saved ones that are not running.
    /// Mounted first, then errors, then the rest; by name within each.
    pub async fn list_mounts(
        &self,
        repo: &Repository,
        connection_mgr: &ConnectionManager,
    ) -> Result<Vec<MountInfo>, AppError> {
        self.reconcile(repo).await;
        // Start with in-memory active mounts
        let active: Vec<MountInfo> = {
            let mounts = self.mounts.read().await;
            mounts.values().map(|m| m.to_info()).collect()
        };

        // Also load saved configs from DB that aren't currently active
        let active_ids: Vec<String> = active.iter().map(|m| m.id.to_string()).collect();

        let saved_configs: Vec<(String, String, String, bool, String, String, String, Option<String>)> = repo
            .pool()
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, connection_id, mount_point, auto_mount, status, created_at, options_json, error_message
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
                            row.get::<_, String>(6)?,
                            row.get::<_, Option<String>>(7)?,
                        ))
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(rows)
            })
            .await?;

        let mut result = active;

        // Add saved configs that aren't currently active in memory
        for (id, cid, mp, auto_mount, status, created_at, options_json, error_message) in saved_configs {
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
                // Mounts live in memory: a row still marked mounted belongs to a
                // run of the app that ended (or crashed) with the drive mounted.
                status: match MountStatus::from_str_lossy(&status) {
                    MountStatus::Mounted => MountStatus::Unmounted,
                    other => other,
                },
                auto_mount,
                created_at: ts,
                error_message,
                options: serde_json::from_str(&options_json).unwrap_or_default(),
                policy_notes: Vec::new(),
            });
        }

        let rank = |s: MountStatus| match s {
            MountStatus::Mounted => 0,
            MountStatus::Error => 1,
            MountStatus::Unmounted => 2,
        };
        result.sort_by(|a, b| {
            (rank(a.status), a.connection_name.to_lowercase(), &a.mount_point, a.id).cmp(&(rank(b.status), b.connection_name.to_lowercase(), &b.mount_point, b.id))
        });
        Ok(result)
    }

    /// "Make available offline": read every file under `rel` (inside the
    /// mount) so a full-cache mount keeps it locally. Stops at the cache size
    /// limit. Needs a mounted rclone drive in `full` cache mode.
    pub async fn pin_offline(&self, mount_id: Uuid, rel: &str) -> Result<PinReport, AppError> {
        let m = self.mounts.read().await.get(&mount_id).cloned().ok_or_else(|| AppError::validation("That drive is not mounted"))?;
        if m.status != MountStatus::Mounted || !matches!(m.protocol.as_str(), "rclone" | "s3") {
            return Err(AppError::validation("Only mounted cloud drives (rclone) can be made available offline"));
        }
        if m.options.cache_mode != CacheMode::Full {
            return Err(AppError::validation("Turn on the full cache for this drive to keep files offline"));
        }
        let limit = m.options.max_cache_mb.map(|mb| mb * 1024 * 1024);
        let roots = ufop_core::confine::Roots::new(&[std::path::PathBuf::from(&m.mount_point)]).map_err(AppError::validation)?;
        let start = roots.resolve(&std::path::Path::new(&m.mount_point).join(rel.trim_start_matches(['/', '\\'])).to_string_lossy(), true).map_err(AppError::validation)?;
        tokio::task::spawn_blocking(move || {
            let mut r = PinReport { files: 0, bytes: 0, failed: 0, truncated: false };
            let mut stack = vec![start];
            let mut buf = vec![0u8; 1024 * 1024];
            while let Some(p) = stack.pop() {
                let Ok(md) = std::fs::symlink_metadata(&p) else {
                    r.failed += 1;
                    continue;
                };
                if md.is_dir() {
                    if let Ok(rd) = std::fs::read_dir(&p) {
                        stack.extend(rd.filter_map(Result::ok).map(|e| e.path()));
                    }
                } else if md.is_file() {
                    if limit.is_some_and(|l| r.bytes + md.len() > l) {
                        r.truncated = true;
                        continue;
                    }
                    use std::io::Read;
                    match std::fs::File::open(&p).and_then(|mut f| {
                        let mut n = 0u64;
                        loop {
                            let k = f.read(&mut buf)?;
                            if k == 0 {
                                break Ok(n);
                            }
                            n += k as u64;
                        }
                    }) {
                        Ok(n) => {
                            r.files += 1;
                            r.bytes += n;
                        }
                        Err(_) => r.failed += 1,
                    }
                }
            }
            r
        })
        .await
        .map_err(|e| AppError::internal(format!("Offline pinning failed: {e}")))
    }

    /// Bytes in a mount's local cache (in the app's cache folder).
    pub fn cache_usage(&self, mount_id: Uuid) -> u64 {
        dir_size(&self.cache_dir(mount_id))
    }

    /// Bytes in a mount's local cache, wherever its options keep it.
    pub async fn cache_usage_of(&self, mount_id: Uuid, repo: &Repository) -> u64 {
        let dir = self.resolve_cache_dir(mount_id, repo).await;
        tokio::task::spawn_blocking(move || dir_size(&dir)).await.unwrap_or(0)
    }

    /// Delete a mount's local cache (only while it is not mounted: rclone owns it then).
    pub async fn clear_cache(&self, mount_id: Uuid) -> Result<u64, AppError> {
        self.clear_cache_dir(mount_id, self.cache_dir(mount_id)).await
    }

    /// `clear_cache`, wherever the mount's options keep the cache.
    pub async fn clear_cache_of(&self, mount_id: Uuid, repo: &Repository) -> Result<u64, AppError> {
        let dir = self.resolve_cache_dir(mount_id, repo).await;
        self.clear_cache_dir(mount_id, dir).await
    }

    async fn clear_cache_dir(&self, mount_id: Uuid, dir: std::path::PathBuf) -> Result<u64, AppError> {
        if self.mounts.read().await.get(&mount_id).is_some_and(|m| m.status == MountStatus::Mounted) {
            return Err(AppError::validation("Unmount the drive before clearing its cache (uploads may be pending)"));
        }
        let freed = dir_size(&dir);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(freed)
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
        options: Option<MountOptions>,
        repo: &Repository,
        connection_mgr: &ConnectionManager,
    ) -> Result<MountConfig, AppError> {
        // Validate connection exists
        let profile = connection_mgr
            .get_connection(connection_id)
            .await?
            .ok_or_else(|| AppError::connection(
                format!("Connection {} not found", connection_id),
                "Check the connection ID.",
            ))?;

        let now = Utc::now();
        let cid = connection_id.to_string();
        let os = policy::Os::current();
        let mp = options.as_ref().and_then(|o| policy::drive_mount_point(o, os)).unwrap_or_else(|| expand_home(&mount_point));
        let mp_out = mp.clone();
        let am = auto_mount;
        let ts = now.to_rfc3339();
        // Bad options are refused here, not at the next mount.
        let options = options.map(|o| policy::validate_on(&o, profile.protocol.as_str(), std::path::Path::new(&mp), os)).transpose()?;
        let opts = options.map(|o| serde_json::to_string(&o)).transpose()?;

        // One row per connection and mount point: the mount the dialog just
        // made (or an earlier save) is updated rather than duplicated.
        let config_id = repo
            .pool()
            .execute(move |conn| {
                let existing: Option<String> = rusqlite::OptionalExtension::optional(conn.query_row(
                    "SELECT id FROM mount_configs WHERE connection_id = ?1 AND mount_point = ?2 ORDER BY created_at DESC LIMIT 1",
                    rusqlite::params![cid, mp],
                    |r| r.get(0),
                ))?;
                let id = existing.unwrap_or_else(|| Uuid::new_v4().to_string());
                conn.execute(
                    "INSERT INTO mount_configs (id, connection_id, mount_point, auto_mount, status, created_at, options_json)
                     VALUES (?1, ?2, ?3, ?4, 'unmounted', ?5, COALESCE(?6, '{}'))
                     ON CONFLICT(id) DO UPDATE SET auto_mount = ?4, options_json = COALESCE(?6, options_json)",
                    rusqlite::params![id, cid, mp, am, ts, opts],
                )?;
                // Older duplicates of the same mount (before this was fixed).
                conn.execute("DELETE FROM mount_configs WHERE connection_id = ?1 AND mount_point = ?2 AND id != ?3", rusqlite::params![cid, mp, id])?;
                Ok(id)
            })
            .await?;
        let config_id = Uuid::parse_str(&config_id).unwrap_or_else(|_| Uuid::nil());
        self.notify();

        tracing::info!(
            "Saved mount config: connection={}, mount_point={}, auto_mount={}",
            connection_id,
            mount_point,
            auto_mount
        );

        Ok(MountConfig {
            id: config_id,
            connection_id,
            mount_point: mp_out,
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
        // A mounted drive is unmounted first: dropping it from memory would leave it running.
        if self.mounts.read().await.get(&mount_id).is_some_and(|m| m.status == MountStatus::Mounted) {
            self.unmount_remote(mount_id, repo).await?;
        }
        self.mounts.write().await.remove(&mount_id);

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
        self.notify();

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

    /// Build a repo + connection manager for the auto-mount tests. The
    /// credential store skips the keychain in debug builds, so this stays
    /// hermetic and never prompts.
    async fn auto_mount_fixture() -> (crate::storage::Repository, ConnectionManager) {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let conns = ConnectionManager::new(
            repo.clone(),
            std::sync::Arc::new(crate::security::CredentialStore::new()),
        );
        (repo, conns)
    }

    async fn insert_config(repo: &crate::storage::Repository, mount_point: &str, auto: bool) {
        let mp = mount_point.to_string();
        repo.pool()
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO mount_configs (id, connection_id, mount_point, auto_mount, status) \
                     VALUES (?1, ?2, ?3, ?4, 'unmounted')",
                    rusqlite::params![
                        Uuid::new_v4().to_string(),
                        // A connection that does not exist: mount_remote fails at
                        // its connection lookup and never shells out to
                        // mount_smbfs/sshfs, so the test touches no real mounts.
                        Uuid::new_v4().to_string(),
                        mp,
                        auto as i32,
                    ],
                )?;
                Ok(())
            })
            .await
            .unwrap();
    }

    /// The regression: `auto_mount` was written and read back only to render
    /// "Auto" in the list. No restore path existed, so the checkbox was
    /// decorative — tick it, restart, nothing mounts.
    #[tokio::test]
    async fn auto_mount_attempts_only_flagged_configs() {
        let (repo, conns) = auto_mount_fixture().await;
        insert_config(&repo, "/mnt/auto", true).await;
        insert_config(&repo, "/mnt/manual", false).await;

        let (mounted, failed) = MountManager::new().auto_mount_saved(&repo, &conns).await;

        assert_eq!(mounted, 0, "no real connection exists, so none can mount");
        assert_eq!(
            failed, 1,
            "exactly the auto_mount=1 row may be attempted; the manual one must be left alone"
        );
    }

    /// One unreachable server must not stop the others being tried, and must
    /// never propagate an error into app startup.
    #[tokio::test]
    async fn auto_mount_survives_failures_and_never_errors() {
        let (repo, conns) = auto_mount_fixture().await;
        for i in 0..3 {
            insert_config(&repo, &format!("/mnt/auto{i}"), true).await;
        }

        let (mounted, failed) = MountManager::new().auto_mount_saved(&repo, &conns).await;

        assert_eq!(mounted, 0);
        assert_eq!(failed, 3, "every flagged config is attempted despite failures");
    }

    /// Regression: the engine wrote `error_message`, a column the schema did
    /// not have, so every successful OS mount ended in a database error.
    #[tokio::test]
    async fn mount_writes_match_the_schema() {
        let (repo, _) = auto_mount_fixture().await;
        repo.pool()
            .execute(|c| {
                c.execute(INSERT_MOUNT_SQL, rusqlite::params!["m1", "c1", "/mnt/x", "mounted", Option::<String>::None, "2026-01-01T00:00:00Z", "{}"])?;
                c.execute(UPDATE_MOUNT_STATUS_SQL, rusqlite::params!["m1", "error", "boom"])?;
                let (st, err): (String, String) = c.query_row("SELECT status, error_message FROM mount_configs WHERE id = 'm1'", [], |r| Ok((r.get(0)?, r.get(1)?)))?;
                assert_eq!((st.as_str(), err.as_str()), ("error", "boom"));
                Ok(())
            })
            .await
            .unwrap();
    }

    /// Regression: the dialog's default "~/UFOP-Mounts/x" was rejected as
    /// not absolute.
    #[test]
    fn tilde_mount_points_expand_to_home() {
        let home = directories::UserDirs::new().unwrap().home_dir().to_path_buf();
        assert_eq!(expand_home("~/UFOP-Mounts/a"), format!("{}/UFOP-Mounts/a", home.display()));
        assert!(std::path::Path::new(&expand_home("~/x")).is_absolute());
        assert_eq!(expand_home("/abs/x"), "/abs/x");
        assert_eq!(expand_home("~other/x"), "~other/x");
    }

    #[tokio::test]
    async fn auto_mount_is_a_noop_with_nothing_saved() {
        let (repo, conns) = auto_mount_fixture().await;
        assert_eq!(
            MountManager::new().auto_mount_saved(&repo, &conns).await,
            (0, 0)
        );
    }

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

    fn rclone_profile(remote: &str, path: &str) -> crate::core::types::ConnectionProfile {
        crate::core::types::ConnectionProfile {
            id: Uuid::new_v4(),
            name: "local via rclone".into(),
            protocol: crate::core::types::ConnectionProtocol::Rclone,
            host: remote.into(),
            port: None,
            username: None,
            credential_ref: None,
            remote_path: path.into(),
            created_at: Utc::now(),
            last_used: None,
            group_id: None,
            bandwidth_limit_bps: 0,
            conflict_policy: None,
            retry_policy: None,
            verify_checksums: false,
            checksum_algorithm: None,
            proxy_type: None,
            proxy_host: None,
            proxy_port: None,
            proxy_username: None,
            proxy_password: None,
            default_local_dir: None,
            default_remote_dir: None,
            charset: None,
            default_file_mode: None,
            default_dir_mode: None,
            ftp_use_mlsd: None,
            ftp_force_passive_ip: None,
            ftp_post_login_commands: None,
            symlink_policy: None,
            extras: Default::default(),
        }
    }

    /// Regression: remounting a saved configuration marked it "auto-mount"
    /// whatever the user had chosen (the flag was derived from "has a saved
    /// row"), so the Mounts list claimed manual drives mount on startup.
    #[tokio::test]
    async fn remounting_a_manual_saved_config_keeps_it_manual() {
        if !rclone_mount::rclone_available() || rclone_mount::fuse_available().is_err() {
            return;
        }
        let (repo, conns) = auto_mount_fixture().await;
        // rclone fails at once on an unknown remote: the attempt is recorded as an error.
        let profile = conns.save_connection(rclone_profile("no-such-remote:", "/"), None).await.unwrap();
        let home = tempfile::tempdir().unwrap();
        let mp = home.path().join("Drives/manual").to_string_lossy().to_string();
        let mgr = MountManager::with_cache_root(home.path().join("cache"));
        let cfg = mgr.save_mount_config(profile.id, mp.clone(), false, None, &repo, &conns).await.unwrap();
        assert!(mgr.mount_remote_with(profile.id, mp, MountOptions::default(), Some(cfg.id), &repo, &conns).await.is_err());
        let info = mgr.list_mounts(&repo, &conns).await.unwrap().into_iter().find(|m| m.id == cfg.id).unwrap();
        assert_eq!(info.status, MountStatus::Error);
        assert!(!info.auto_mount, "a manual saved mount must stay manual");
    }

    /// Regression: a row left as 'mounted' by an app that exited (or crashed)
    /// while a drive was mounted was listed as mounted after the restart,
    /// although nothing is mounted until the app mounts it.
    #[tokio::test]
    async fn a_saved_row_marked_mounted_is_not_listed_as_mounted_after_restart() {
        let (repo, conns) = auto_mount_fixture().await;
        let id = Uuid::new_v4().to_string();
        let sid = id.clone();
        repo.pool()
            .execute(move |c| {
                c.execute(
                    "INSERT INTO mount_configs (id, connection_id, mount_point, auto_mount, status) VALUES (?1, ?2, '/mnt/x', 0, 'mounted')",
                    rusqlite::params![sid, Uuid::new_v4().to_string()],
                )?;
                Ok(())
            })
            .await
            .unwrap();
        let list = MountManager::new().list_mounts(&repo, &conns).await.unwrap();
        let info = list.iter().find(|m| m.id.to_string() == id).unwrap();
        assert_eq!(info.status, MountStatus::Unmounted);
    }

    /// A running mount, as the manager records one after a successful mount.
    async fn inject_mounted(mgr: &MountManager, repo: &crate::storage::Repository, name: &str, mount_point: &str) -> Uuid {
        let id = Uuid::new_v4();
        let (sid, mp) = (id.to_string(), mount_point.to_string());
        repo.pool()
            .execute(move |c| {
                c.execute(INSERT_MOUNT_SQL, rusqlite::params![sid, Uuid::new_v4().to_string(), mp, "mounted", Option::<String>::None, "2026-01-01T00:00:00Z", "{}"])?;
                Ok(())
            })
            .await
            .unwrap();
        mgr.mounts.write().await.insert(
            id,
            VirtualMount {
                id,
                connection_id: Uuid::new_v4(),
                connection_name: name.into(),
                protocol: "rclone".into(),
                mount_point: mount_point.into(),
                status: MountStatus::Mounted,
                auto_mount: false,
                created_at: Utc::now(),
                error_message: None,
                options: MountOptions::default(),
                policy_notes: vec![],
            },
        );
        id
    }

    #[tokio::test]
    async fn unmount_all_unmounts_every_mounted_drive_and_notifies() {
        let (repo, conns) = auto_mount_fixture().await;
        let mgr = MountManager::new();
        let a = inject_mounted(&mgr, &repo, "A", "/mnt/a").await;
        let b = inject_mounted(&mgr, &repo, "B", "/mnt/b").await;
        let mut changes = mgr.subscribe();
        let report = mgr.unmount_all(&repo).await;
        assert_eq!(report, UnmountAllReport { unmounted: 2, failed: vec![] });
        assert!(changes.has_changed().unwrap(), "the tray and the UI are told");
        assert_eq!(mgr.get_mount_status(a).await, MountStatus::Unmounted);
        let list = mgr.list_mounts(&repo, &conns).await.unwrap();
        assert!(list.iter().all(|m| m.status == MountStatus::Unmounted), "{list:?}");
        assert!(list.iter().any(|m| m.id == b), "saved rows stay listed");
        assert_eq!(mgr.unmount_all(&repo).await, UnmountAllReport::default(), "nothing left to unmount");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_drive_that_is_gone_is_listed_as_an_error_not_as_mounted() {
        let (repo, conns) = auto_mount_fixture().await;
        let mgr = MountManager::new();
        let dir = tempfile::tempdir().unwrap();
        // Nothing is mounted at this folder (rclone stopped, or it was unmounted outside the app).
        let id = inject_mounted(&mgr, &repo, "Gone", &dir.path().to_string_lossy()).await;
        let info = mgr.list_mounts(&repo, &conns).await.unwrap().into_iter().find(|m| m.id == id).unwrap();
        assert_eq!(info.status, MountStatus::Error);
        assert!(info.error_message.unwrap().contains("no longer mounted"));
        // The record is kept in the database, so it is still an error after a restart.
        let fresh = MountManager::new().list_mounts(&repo, &conns).await.unwrap().into_iter().find(|m| m.id == id).unwrap();
        assert_eq!(fresh.status, MountStatus::Error);
    }

    #[tokio::test]
    async fn list_puts_mounted_first_then_errors_then_unmounted() {
        let (repo, conns) = auto_mount_fixture().await;
        let mgr = MountManager::new();
        for (name, status) in [("zed", "unmounted"), ("beta", "error"), ("alpha", "unmounted")] {
            let (id, st) = (Uuid::new_v4().to_string(), status.to_string());
            let n = name.to_string();
            repo.pool()
                .execute(move |c| {
                    c.execute(INSERT_MOUNT_SQL, rusqlite::params![id, Uuid::new_v4().to_string(), format!("/mnt/{n}"), st, Option::<String>::None, "2026-01-01T00:00:00Z", "{}"])?;
                    Ok(())
                })
                .await
                .unwrap();
        }
        let list = mgr.list_mounts(&repo, &conns).await.unwrap();
        let order: Vec<&str> = list.iter().map(|m| m.mount_point.as_str()).collect();
        assert_eq!(order, ["/mnt/beta", "/mnt/alpha", "/mnt/zed"].map(|x| x), "{order:?}");
    }

    #[tokio::test]
    async fn editing_a_saved_mount_updates_its_row_and_validates() {
        let (repo, conns) = auto_mount_fixture().await;
        let profile = conns.save_connection(rclone_profile("gcs:", "/bucket"), None).await.unwrap();
        let other = conns.save_connection(rclone_profile("box:", "/"), None).await.unwrap();
        let mgr = MountManager::new();
        let cfg = mgr.save_mount_config(profile.id, "/mnt/one".into(), false, Some(MountOptions::default()), &repo, &conns).await.unwrap();
        let second = mgr.save_mount_config(profile.id, "/mnt/two".into(), false, None, &repo, &conns).await.unwrap();
        let mut rx = mgr.subscribe();

        let opts = MountOptions { cache_mode: CacheMode::Full, subfolder: Some("/photos/2024/".into()), extra_flags: vec!["--no-modtime".into()], ..Default::default() };
        let out = mgr.update_mount_config(cfg.id, Some(other.id), "/mnt/renamed".into(), true, opts, &repo, &conns).await.unwrap();
        assert_eq!((out.id, out.connection_id, out.mount_point.as_str(), out.auto_mount), (cfg.id, other.id, "/mnt/renamed", true));
        assert!(rx.has_changed().unwrap());
        let saved = load_saved(&repo, cfg.id).await.unwrap();
        assert_eq!((saved.connection_id, saved.mount_point.as_str(), saved.auto_mount), (other.id, "/mnt/renamed", true));
        assert_eq!(saved.options.subfolder.as_deref(), Some("photos/2024"), "stored normalized");
        assert_eq!(saved.options.extra_flags, ["--no-modtime"]);
        let rows = repo.pool().execute(|c| Ok(c.query_row("SELECT COUNT(*) FROM mount_configs", [], |r| r.get::<_, i64>(0))?)).await.unwrap();
        assert_eq!(rows, 2, "edited in place, not duplicated");

        // Bad input is refused and changes nothing.
        let bad = MountOptions { extra_flags: vec!["--read-only=false".into()], ..Default::default() };
        assert!(mgr.update_mount_config(cfg.id, None, "/mnt/renamed".into(), true, bad, &repo, &conns).await.is_err());
        let dup = mgr.update_mount_config(cfg.id, Some(profile.id), "/mnt/two".into(), true, MountOptions::default(), &repo, &conns).await;
        assert!(dup.unwrap_err().to_string().contains("already uses"), "same connection and mount point as another saved mount");
        assert!(mgr.update_mount_config(cfg.id, None, "  ".into(), true, MountOptions::default(), &repo, &conns).await.is_err());
        assert!(mgr.update_mount_config(Uuid::new_v4(), None, "/mnt/x".into(), true, MountOptions::default(), &repo, &conns).await.is_err());
        assert_eq!(load_saved(&repo, cfg.id).await.unwrap().mount_point, "/mnt/renamed");
        assert_eq!(load_saved(&repo, second.id).await.unwrap().mount_point, "/mnt/two");

        // Not while it is mounted.
        mgr.mounts.write().await.insert(
            cfg.id,
            VirtualMount {
                id: cfg.id,
                connection_id: other.id,
                connection_name: "x".into(),
                protocol: "rclone".into(),
                mount_point: "/mnt/renamed".into(),
                status: MountStatus::Mounted,
                auto_mount: true,
                created_at: Utc::now(),
                error_message: None,
                options: MountOptions::default(),
                policy_notes: vec![],
            },
        );
        let e = mgr.update_mount_config(cfg.id, None, "/mnt/renamed".into(), true, MountOptions::default(), &repo, &conns).await.unwrap_err();
        assert!(e.to_string().contains("Unmount the drive before editing"), "{e}");
    }

    #[tokio::test]
    async fn saving_a_mount_refuses_options_its_protocol_cannot_use() {
        let (repo, conns) = auto_mount_fixture().await;
        let profile = conns.save_connection(rclone_profile("gcs:", "/"), None).await.unwrap();
        let mgr = MountManager::new();
        let bad = MountOptions { extra_flags: vec!["--vfs-cache-max-size=999G".into()], ..Default::default() };
        assert!(mgr.save_mount_config(profile.id, "/mnt/x".into(), false, Some(bad), &repo, &conns).await.is_err());
        let rows = repo.pool().execute(|c| Ok(c.query_row("SELECT COUNT(*) FROM mount_configs", [], |r| r.get::<_, i64>(0))?)).await.unwrap();
        assert_eq!(rows, 0, "nothing saved");
    }

    #[tokio::test]
    async fn mount_saved_needs_a_saved_mount_that_is_not_mounted() {
        let (repo, conns) = auto_mount_fixture().await;
        let mgr = MountManager::new();
        assert!(mgr.mount_saved(Uuid::new_v4(), &repo, &conns).await.unwrap_err().to_string().contains("no longer exists"));
        let id = inject_mounted(&mgr, &repo, "A", "/mnt/a").await;
        assert!(mgr.mount_saved(id, &repo, &conns).await.unwrap_err().to_string().contains("already mounted"));
    }

    #[tokio::test]
    async fn deleting_a_mounted_drive_unmounts_it_first() {
        let (repo, conns) = auto_mount_fixture().await;
        let mgr = MountManager::new();
        let id = inject_mounted(&mgr, &repo, "A", "/mnt/a").await;
        let mut rx = mgr.subscribe();
        mgr.delete_mount_config(id, &repo).await.unwrap();
        assert!(rx.has_changed().unwrap());
        assert!(mgr.list_mounts(&repo, &conns).await.unwrap().is_empty());
        assert_eq!(mgr.get_mount_status(id).await, MountStatus::Unmounted);
    }

    #[tokio::test]
    async fn the_cache_lives_where_the_options_say_and_clearing_stays_in_that_folder() {
        let (repo, conns) = auto_mount_fixture().await;
        let profile = conns.save_connection(rclone_profile("gcs:", "/"), None).await.unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let mgr = MountManager::with_cache_root(tmp.path().join("app-cache"));
        let chosen = tmp.path().join("big-disk");
        let opts = MountOptions { cache_dir: Some(chosen.to_string_lossy().into()), ..Default::default() };
        let cfg = mgr.save_mount_config(profile.id, "/mnt/x".into(), false, Some(opts.clone()), &repo, &conns).await.unwrap();
        let own = chosen.join(cfg.id.to_string());
        assert_eq!(mgr.cache_dir_for(cfg.id, &opts), own, "a folder named after the drive, never the chosen folder itself");
        std::fs::create_dir_all(own.join("vfs")).unwrap();
        std::fs::write(own.join("vfs/f"), vec![1u8; 4096]).unwrap();
        std::fs::write(chosen.join("precious.txt"), "keep").unwrap();
        assert_eq!(mgr.cache_usage_of(cfg.id, &repo).await, 4096);
        assert_eq!(mgr.clear_cache_of(cfg.id, &repo).await.unwrap(), 4096);
        assert!(!own.exists());
        assert!(chosen.join("precious.txt").exists(), "files beside the cache are untouched");
        // Without a chosen folder it is the app's cache folder.
        assert_eq!(mgr.cache_dir_for(cfg.id, &MountOptions::default()), tmp.path().join("app-cache").join(cfg.id.to_string()));
    }

    /// Real rclone + FUSE: cache options reach rclone, files are pinned for
    /// offline use, the mount profile forces read-only and is enforced, and
    /// saving/auto-mounting keeps one row per mount.
    #[tokio::test]
    async fn cache_offline_pinning_and_mount_profile_with_real_rclone() {
        if !rclone_mount::rclone_available() || rclone_mount::fuse_available().is_err() {
            return;
        }
        // An rclone remote of type local, defined through the environment.
        std::env::set_var("RCLONE_CONFIG_UFOPMNTTEST_TYPE", "local");
        let src = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(src.path().join("projects/q3")).unwrap();
        std::fs::write(src.path().join("projects/q3/a.bin"), vec![7u8; 300_000]).unwrap();
        std::fs::write(src.path().join("projects/q3/b.txt"), "plan").unwrap();
        std::fs::write(src.path().join("top.txt"), "top").unwrap();
        let home = tempfile::tempdir().unwrap();
        let mp = home.path().join("Drives/team");
        let (repo, conns) = auto_mount_fixture().await;
        let profile = conns.save_connection(rclone_profile("ufopmnttest:/", &src.path().to_string_lossy()), None).await.unwrap();
        let mgr = MountManager::with_cache_root(home.path().join("cache"));

        let opts = MountOptions { cache_mode: CacheMode::Full, max_cache_mb: Some(64), max_cache_age_hours: Some(12), read_only: false, ..Default::default() };
        let r = match mgr.mount_remote_with(profile.id, mp.to_string_lossy().into(), opts.clone(), None, &repo, &conns).await {
            Ok(r) => r,
            // FUSE present but not permitted (unprivileged container): nothing to verify.
            Err(e) if ["fusermount", "/dev/fuse", "Operation not permitted", "permission denied"].iter().any(|k| format!("{e:?}").contains(k)) => {
                eprintln!("skipping: FUSE not permitted here: {e}");
                return;
            }
            Err(e) => panic!("mount failed: {e:?}"),
        };
        assert_eq!(std::fs::read_to_string(mp.join("top.txt")).unwrap(), "top");
        let info = mgr.list_mounts(&repo, &conns).await.unwrap().into_iter().find(|m| m.id == r.mount_id).unwrap();
        assert_eq!(info.options, opts);
        // Pin a folder: its files land in this mount's own cache.
        let pinned = mgr.pin_offline(r.mount_id, "projects").await.unwrap();
        assert_eq!((pinned.files, pinned.bytes, pinned.failed, pinned.truncated), (2, 300_004, 0, false));
        assert!(mgr.cache_usage(r.mount_id) >= 300_000, "cache holds the pinned files: {}", mgr.cache_usage(r.mount_id));
        assert!(mgr.pin_offline(r.mount_id, "../../etc").await.is_err(), "pinning is confined to the mount");
        assert!(mgr.clear_cache(r.mount_id).await.is_err(), "not while mounted");
        // Writable without a profile.
        std::fs::write(mp.join("new.txt"), "w").unwrap();

        // Saving the mount for auto-mount updates its row instead of adding one.
        mgr.save_mount_config(profile.id, mp.to_string_lossy().into(), true, Some(opts.clone()), &repo, &conns).await.unwrap();
        let rows = || async {
            repo.pool().execute(|c| Ok(c.query_row("SELECT COUNT(*) FROM mount_configs", [], |r| r.get::<_, i64>(0))?)).await.unwrap()
        };
        assert_eq!(rows().await, 1);
        mgr.unmount_remote(r.mount_id, &repo).await.unwrap();
        assert!(mgr.cache_usage(r.mount_id) > 0, "the cache outlives the mount (offline copies)");

        // The organization's profile: read-only and a capped cache; a remount
        // (auto-mount after restart) reuses the saved row.
        mgr.set_managed(Some(ufop_core::agent::ManagedMounts { read_only: Some(true), max_cache_mb: Some(32), ..Default::default() })).await;
        let (mounted, failed) = mgr.auto_mount_saved(&repo, &conns).await;
        assert_eq!((mounted, failed), (1, 0));
        assert_eq!(rows().await, 1, "auto-mount does not duplicate the saved mount");
        let info = mgr.list_mounts(&repo, &conns).await.unwrap().into_iter().find(|m| m.status == MountStatus::Mounted).unwrap();
        assert!(info.options.read_only && info.options.max_cache_mb == Some(32), "{:?}", info.options);
        assert_eq!(info.policy_notes.len(), 2, "{:?}", info.policy_notes);
        assert!(std::fs::write(mp.join("blocked.txt"), "x").is_err(), "read-only mount refuses writes");
        mgr.unmount_remote(info.id, &repo).await.unwrap();

        // Profiles refuse protocols and places outside their rules.
        mgr.set_managed(Some(ufop_core::agent::ManagedMounts { allowed_protocols: Some(vec!["smb".into()]), ..Default::default() })).await;
        let e = mgr.mount_remote(profile.id, mp.to_string_lossy().into(), &repo, &conns).await.unwrap_err();
        assert!(e.to_string().contains("does not allow mounting rclone"), "{e}");
        mgr.set_managed(Some(ufop_core::agent::ManagedMounts { mount_root: Some(home.path().join("Elsewhere").to_string_lossy().into()), ..Default::default() })).await;
        assert!(mgr.mount_remote(profile.id, mp.to_string_lossy().into(), &repo, &conns).await.unwrap_err().to_string().contains("must be inside"));

        // Unmounted: the cache can be cleared.
        assert!(mgr.clear_cache(info.id).await.unwrap() > 0);
        assert_eq!(mgr.cache_usage(info.id), 0);
    }
}
