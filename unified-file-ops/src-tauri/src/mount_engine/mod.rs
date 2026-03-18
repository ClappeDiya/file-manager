//! Mount Engine - Virtual mount management for remote/cloud storage.
//!
//! This module provides a stub/simulation of FUSE-based mounting.
//! It tracks mount state (connection_id, mount_point, status) and persists
//! mount configurations in SQLite for auto-mount on startup.
//!
//! Actual FUSE integration (via macFUSE / libfuse) will be added later.
//! For now, the module validates connections and manages mount metadata.

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

/// Manages virtual mounts and their persistence.
///
/// In stub mode, this tracks mount state in memory and SQLite
/// without actually creating FUSE mounts. The connection is validated
/// to exist before a mount entry is created.
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

        let mount_id = Uuid::new_v4();
        let now = Utc::now();

        let virtual_mount = VirtualMount {
            id: mount_id,
            connection_id,
            connection_name: profile.name.clone(),
            protocol: profile.protocol.as_str().to_string(),
            mount_point: mount_point.clone(),
            status: MountStatus::Mounted,
            auto_mount: false,
            created_at: now,
            error_message: None,
        };

        // Persist to SQLite
        let mp = mount_point.clone();
        let cid = connection_id.to_string();
        let mid = mount_id.to_string();
        let ts = now.to_rfc3339();
        repo.pool()
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO mount_configs (id, connection_id, mount_point, auto_mount, status, created_at)
                     VALUES (?1, ?2, ?3, 0, 'mounted', ?4)",
                    rusqlite::params![mid, cid, mp, ts],
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
            "Mounted connection '{}' ({}) at '{}' (mount_id: {})",
            profile.name,
            profile.protocol.as_str(),
            mount_point,
            mount_id
        );

        Ok(MountResult {
            mount_id,
            mount_point,
            status: MountStatus::Mounted,
            message: format!(
                "Successfully mounted '{}' (stub mode - FUSE integration pending)",
                profile.name
            ),
        })
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
