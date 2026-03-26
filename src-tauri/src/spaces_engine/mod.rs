//! Smart Spaces engine — named workspaces bundling local folder + connection + sync pair + automations.
//!
//! A `SmartSpace` is the user-facing concept that ties together:
//! - A local folder path (the "home" directory)
//! - An optional remote connection (by `ConnectionProfile.id`)
//! - An optional sync pair (by `SyncPair.id`)
//! - Optional automation rules (via junction table)
//!
//! `SpacesManager` provides in-memory caching backed by SQLite persistence.

use crate::core::error::AppError;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

// ── Data Structures ──

/// A named, persistent workspace bundling local + remote + sync + automations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartSpace {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub color: String,
    pub local_path: String,
    pub connection_id: Option<String>,
    pub remote_path: Option<String>,
    pub sync_pair_id: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl SmartSpace {
    pub fn new(name: String, local_path: String) -> Self {
        let now = Utc::now().to_rfc3339();
        Self {
            id: Uuid::new_v4().to_string(),
            name,
            icon: "folder-heart".to_string(),
            color: "#6366f1".to_string(),
            local_path,
            connection_id: None,
            remote_path: None,
            sync_pair_id: None,
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

/// Aggregated status of a Smart Space, derived from connection + sync state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SpaceStatus {
    /// No remote connection configured.
    LocalOnly,
    /// Remote connection configured but not reachable.
    Offline,
    /// Connected to remote.
    Connected,
    /// Sync is currently running.
    Syncing,
    /// Sync completed successfully.
    Synced { last_sync_at: String },
    /// Sync has unresolved conflicts.
    Conflicts { count: u64 },
    /// An error occurred.
    Error { message: String },
}

/// Full status result returned to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpaceStatusResult {
    pub space_id: String,
    pub status: SpaceStatus,
    pub connection_alive: bool,
    pub last_sync_at: Option<String>,
    pub conflict_count: u64,
}

// ── Manager ──

/// Manages Smart Spaces: in-memory cache + SQLite persistence.
pub struct SpacesManager {
    spaces: Arc<RwLock<Vec<SmartSpace>>>,
}

impl SpacesManager {
    pub fn new() -> Self {
        Self {
            spaces: Arc::new(RwLock::new(Vec::new())),
        }
    }

    /// Load all spaces from SQLite on startup.
    pub async fn load_from_db(&self, repo: &crate::storage::Repository) -> Result<(), AppError> {
        let spaces = repo
            .pool()
            .execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, icon, color, local_path, connection_id, remote_path, \
                     sync_pair_id, enabled, created_at, updated_at \
                     FROM smart_spaces ORDER BY created_at DESC",
                )?;

                let rows = stmt
                    .query_map([], |row| {
                        Ok(SmartSpace {
                            id: row.get(0)?,
                            name: row.get(1)?,
                            icon: row.get(2)?,
                            color: row.get(3)?,
                            local_path: row.get(4)?,
                            connection_id: row.get(5)?,
                            remote_path: row.get(6)?,
                            sync_pair_id: row.get(7)?,
                            enabled: row.get::<_, i32>(8)? != 0,
                            created_at: row.get(9)?,
                            updated_at: row.get(10)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect::<Vec<_>>();

                Ok(rows)
            })
            .await?;

        let count = spaces.len();
        *self.spaces.write().await = spaces;
        tracing::info!("Loaded {} smart space(s) from database", count);
        Ok(())
    }

    /// Save or update a space in the database (upsert).
    pub async fn save_space(
        &self,
        repo: &crate::storage::Repository,
        space: &SmartSpace,
    ) -> Result<(), AppError> {
        let s = space.clone();
        repo.pool()
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO smart_spaces (id, name, icon, color, local_path, connection_id, \
                     remote_path, sync_pair_id, enabled, created_at, updated_at) \
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                     ON CONFLICT(id) DO UPDATE SET \
                     name = excluded.name, icon = excluded.icon, color = excluded.color, \
                     local_path = excluded.local_path, connection_id = excluded.connection_id, \
                     remote_path = excluded.remote_path, sync_pair_id = excluded.sync_pair_id, \
                     enabled = excluded.enabled, updated_at = excluded.updated_at",
                    rusqlite::params![
                        s.id,
                        s.name,
                        s.icon,
                        s.color,
                        s.local_path,
                        s.connection_id,
                        s.remote_path,
                        s.sync_pair_id,
                        s.enabled as i32,
                        s.created_at,
                        s.updated_at,
                    ],
                )?;
                Ok(())
            })
            .await?;

        // Update in-memory list
        let mut spaces = self.spaces.write().await;
        if let Some(existing) = spaces.iter_mut().find(|sp| sp.id == space.id) {
            *existing = space.clone();
        } else {
            spaces.push(space.clone());
        }

        Ok(())
    }

    /// Delete a space from the database. Junction table rows cascade.
    pub async fn delete_space(
        &self,
        repo: &crate::storage::Repository,
        space_id: &str,
    ) -> Result<(), AppError> {
        let sid = space_id.to_string();
        repo.pool()
            .execute(move |conn| {
                conn.execute(
                    "DELETE FROM smart_spaces WHERE id = ?1",
                    rusqlite::params![sid],
                )?;
                Ok(())
            })
            .await?;

        // Remove from in-memory list
        self.spaces.write().await.retain(|s| s.id != space_id);
        Ok(())
    }

    /// List all spaces.
    pub async fn list_spaces(&self) -> Vec<SmartSpace> {
        self.spaces.read().await.clone()
    }

    /// Get a single space by ID.
    pub async fn get_space(&self, space_id: &str) -> Option<SmartSpace> {
        self.spaces
            .read()
            .await
            .iter()
            .find(|s| s.id == space_id)
            .cloned()
    }

    /// Attach an automation rule to a space.
    pub async fn attach_automation(
        &self,
        repo: &crate::storage::Repository,
        space_id: &str,
        rule_id: &str,
    ) -> Result<(), AppError> {
        let sid = space_id.to_string();
        let rid = rule_id.to_string();
        repo.pool()
            .execute(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO space_automations (space_id, rule_id) VALUES (?1, ?2)",
                    rusqlite::params![sid, rid],
                )?;
                Ok(())
            })
            .await
    }

    /// Detach an automation rule from a space.
    pub async fn detach_automation(
        &self,
        repo: &crate::storage::Repository,
        space_id: &str,
        rule_id: &str,
    ) -> Result<(), AppError> {
        let sid = space_id.to_string();
        let rid = rule_id.to_string();
        repo.pool()
            .execute(move |conn| {
                conn.execute(
                    "DELETE FROM space_automations WHERE space_id = ?1 AND rule_id = ?2",
                    rusqlite::params![sid, rid],
                )?;
                Ok(())
            })
            .await
    }

    /// List automation rule IDs attached to a space.
    pub async fn list_automations(
        &self,
        repo: &crate::storage::Repository,
        space_id: &str,
    ) -> Result<Vec<String>, AppError> {
        let sid = space_id.to_string();
        repo.pool()
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT rule_id FROM space_automations WHERE space_id = ?1 ORDER BY rowid",
                )?;
                let ids = stmt
                    .query_map(rusqlite::params![sid], |row| row.get(0))?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(ids)
            })
            .await
    }
}

impl Default for SpacesManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Clone for SpacesManager {
    fn clone(&self) -> Self {
        Self {
            spaces: Arc::clone(&self.spaces),
        }
    }
}
