//! Repository - high-level data access layer over SQLite.
//!
//! Implements `StorageOperations` trait and provides typed
//! CRUD operations for all entity types.

use crate::core::error::AppError;
use crate::core::traits::{BoxFuture, StorageOperations};
use crate::core::types::*;
use crate::storage::migrations;
use crate::storage::pool::DbPool;
use chrono::Utc;
use uuid::Uuid;

/// Repository wrapping a database pool.
#[derive(Clone, Debug)]
pub struct Repository {
    pool: DbPool,
}

impl Repository {
    /// Create a new Repository and run migrations.
    pub async fn new(pool: DbPool) -> Result<Self, AppError> {
        // Run migrations on first use
        pool.execute(|conn| {
            migrations::run_migrations(conn)?;
            Ok(())
        })
        .await?;

        Ok(Self { pool })
    }

    /// Open the default database (OS-appropriate location) and run migrations.
    pub async fn open_default() -> Result<Self, AppError> {
        let db_path = DbPool::default_db_path()?;
        tracing::info!("Opening database at: {}", db_path.display());
        let pool = DbPool::open(&db_path)?;
        Self::new(pool).await
    }

    /// Open an in-memory database (for testing).
    pub async fn open_in_memory() -> Result<Self, AppError> {
        let pool = DbPool::open_in_memory()?;
        Self::new(pool).await
    }

    /// Get the underlying pool (for advanced operations).
    pub fn pool(&self) -> &DbPool {
        &self.pool
    }

    // ── Config operations ──

    pub async fn get_config(&self, key: &str) -> Result<Option<String>, AppError> {
        let key = key.to_string();
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare("SELECT value FROM config WHERE key = ?1")?;
                let result = stmt
                    .query_row(rusqlite::params![key], |row| row.get(0))
                    .ok();
                Ok(result)
            })
            .await
    }

    pub async fn set_config(&self, key: &str, value: &str) -> Result<(), AppError> {
        let key = key.to_string();
        let value = value.to_string();
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO config (key, value, updated_at) VALUES (?1, ?2, ?3)
                     ON CONFLICT(key) DO UPDATE SET value = ?2, updated_at = ?3",
                    rusqlite::params![key, value, Utc::now().to_rfc3339()],
                )?;
                Ok(())
            })
            .await
    }

    // ── Bookmark operations ──

    pub async fn list_bookmarks(&self) -> Result<Vec<Bookmark>, AppError> {
        self.pool
            .execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, path, icon, sort_order, created_at FROM bookmarks ORDER BY sort_order",
                )?;
                let bookmarks = stmt
                    .query_map([], |row| {
                        Ok(Bookmark {
                            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                            name: row.get(1)?,
                            path: row.get(2)?,
                            icon: row.get(3)?,
                            sort_order: row.get(4)?,
                            created_at: chrono::DateTime::parse_from_rfc3339(
                                &row.get::<_, String>(5)?,
                            )
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(bookmarks)
            })
            .await
    }

    pub async fn add_bookmark(&self, bookmark: Bookmark) -> Result<(), AppError> {
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO bookmarks (id, name, path, icon, sort_order, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        bookmark.id.to_string(),
                        bookmark.name,
                        bookmark.path,
                        bookmark.icon,
                        bookmark.sort_order,
                        bookmark.created_at.to_rfc3339(),
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn remove_bookmark(&self, id: Uuid) -> Result<(), AppError> {
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "DELETE FROM bookmarks WHERE id = ?1",
                    rusqlite::params![id.to_string()],
                )?;
                Ok(())
            })
            .await
    }

    // ── Transfer history / queue ──

    pub async fn save_transfer_job(&self, job: &TransferJob) -> Result<(), AppError> {
        let job = job.clone();
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO transfer_history (id, source_path, dest_path, total_bytes, transferred_bytes,
                     status, created_at, updated_at, error_message, priority, sort_order, retry_count,
                     partial_checksum, conflict_policy, connection_id, verify_checksum, checksum_algorithm,
                     source_checksum, dest_checksum, speed_bps, eta_seconds)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
                     ON CONFLICT(id) DO UPDATE SET
                        transferred_bytes = ?5,
                        status = ?6,
                        updated_at = ?8,
                        error_message = ?9,
                        priority = ?10,
                        sort_order = ?11,
                        retry_count = ?12,
                        partial_checksum = ?13,
                        conflict_policy = ?14,
                        speed_bps = ?20,
                        eta_seconds = ?21",
                    rusqlite::params![
                        job.id.to_string(),
                        job.source_path,
                        job.dest_path,
                        job.total_bytes as i64,
                        job.transferred_bytes as i64,
                        job.status.as_str(),
                        job.created_at.to_rfc3339(),
                        job.updated_at.to_rfc3339(),
                        job.error_message,
                        job.priority.as_str(),
                        job.sort_order,
                        job.retry_count as i64,
                        job.partial_checksum,
                        job.conflict_policy.map(|p| p.as_str().to_string()),
                        job.connection_id.map(|id| id.to_string()),
                        job.verify_checksum,
                        job.checksum_algorithm.map(|a| a.as_str().to_string()),
                        job.source_checksum,
                        job.dest_checksum,
                        job.speed_bps as i64,
                        job.eta_seconds.map(|e| e as i64),
                    ],
                )?;
                Ok(())
            })
            .await
    }

    fn row_to_transfer_job(
        row: &rusqlite::Row<'_>,
    ) -> Result<TransferJob, rusqlite::Error> {
        Ok(TransferJob {
            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
            source_path: row.get(1)?,
            dest_path: row.get(2)?,
            total_bytes: row.get::<_, i64>(3)? as u64,
            transferred_bytes: row.get::<_, i64>(4)? as u64,
            status: TransferStatus::from_str_lossy(&row.get::<_, String>(5)?),
            created_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(6)?)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            updated_at: chrono::DateTime::parse_from_rfc3339(&row.get::<_, String>(7)?)
                .map(|dt| dt.with_timezone(&Utc))
                .unwrap_or_else(|_| Utc::now()),
            error_message: row.get(8)?,
            priority: row
                .get::<_, Option<String>>(9)?
                .map(|s| TransferPriority::from_str_lossy(&s))
                .unwrap_or_default(),
            sort_order: row.get::<_, Option<i32>>(10)?.unwrap_or(0),
            retry_count: row.get::<_, Option<i64>>(11)?.unwrap_or(0) as u32,
            partial_checksum: row.get(12)?,
            conflict_policy: row
                .get::<_, Option<String>>(13)?
                .map(|s| ConflictPolicy::from_str_lossy(&s)),
            connection_id: row
                .get::<_, Option<String>>(14)?
                .and_then(|s| Uuid::parse_str(&s).ok()),
            verify_checksum: row.get::<_, Option<bool>>(15)?.unwrap_or(false),
            checksum_algorithm: row
                .get::<_, Option<String>>(16)?
                .map(|s| ChecksumAlgorithm::from_str_lossy(&s)),
            source_checksum: row.get(17)?,
            dest_checksum: row.get(18)?,
            speed_bps: row.get::<_, Option<i64>>(19)?.unwrap_or(0) as u64,
            eta_seconds: row.get::<_, Option<i64>>(20)?.map(|e| e as u64),
        })
    }

    // ── Connection operations ──

    pub async fn save_connection(&self, profile: &ConnectionProfile) -> Result<(), AppError> {
        let profile = profile.clone();
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO connections (id, name, protocol, host, port, username,
                     credential_ref, remote_path, created_at, last_used, group_id,
                     bandwidth_limit_bps, conflict_policy, retry_policy, verify_checksums,
                     checksum_algorithm, proxy_type, proxy_host, proxy_port, proxy_username,
                     proxy_password, default_local_dir, default_remote_dir, charset)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16,
                             ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)
                     ON CONFLICT(id) DO UPDATE SET
                        name = ?2, protocol = ?3, host = ?4, port = ?5,
                        username = ?6, credential_ref = ?7, remote_path = ?8,
                        last_used = ?10, group_id = ?11, bandwidth_limit_bps = ?12,
                        conflict_policy = ?13, retry_policy = ?14,
                        verify_checksums = ?15, checksum_algorithm = ?16,
                        proxy_type = ?17, proxy_host = ?18, proxy_port = ?19,
                        proxy_username = ?20, proxy_password = ?21,
                        default_local_dir = ?22, default_remote_dir = ?23,
                        charset = ?24",
                    rusqlite::params![
                        profile.id.to_string(),
                        profile.name,
                        profile.protocol.as_str(),
                        profile.host,
                        profile.port,
                        profile.username,
                        profile.credential_ref,
                        profile.remote_path,
                        profile.created_at.to_rfc3339(),
                        profile.last_used.map(|dt| dt.to_rfc3339()),
                        profile.group_id.map(|id| id.to_string()),
                        profile.bandwidth_limit_bps as i64,
                        profile.conflict_policy.map(|p| p.as_str().to_string()),
                        profile.retry_policy.as_ref().and_then(|p| serde_json::to_string(p).ok()),
                        profile.verify_checksums,
                        profile.checksum_algorithm.map(|a| a.as_str().to_string()),
                        profile.proxy_type,
                        profile.proxy_host,
                        profile.proxy_port,
                        profile.proxy_username,
                        // Security (M-2): proxy_password arrives here already encrypted
                        // by the command layer (connection_commands.rs) via MasterPasswordManager.
                        // Values are stored as "enc:<base64>" when encrypted, or plaintext
                        // as a graceful fallback when the master password is not set/locked.
                        profile.proxy_password,
                        profile.default_local_dir,
                        profile.default_remote_dir,
                        profile.charset,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn list_connections(&self) -> Result<Vec<ConnectionProfile>, AppError> {
        self.pool
            .execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, protocol, host, port, username, credential_ref,
                            remote_path, created_at, last_used, group_id, bandwidth_limit_bps,
                            conflict_policy, retry_policy, verify_checksums, checksum_algorithm,
                            proxy_type, proxy_host, proxy_port, proxy_username, proxy_password,
                            default_local_dir, default_remote_dir, charset
                     FROM connections ORDER BY name",
                )?;
                let connections = stmt
                    .query_map([], |row| {
                        Ok(ConnectionProfile {
                            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                            name: row.get(1)?,
                            protocol: ConnectionProtocol::from_str_lossy(&row.get::<_, String>(2)?),
                            host: row.get(3)?,
                            port: row.get(4)?,
                            username: row.get(5)?,
                            credential_ref: row.get(6)?,
                            remote_path: row.get(7)?,
                            created_at: chrono::DateTime::parse_from_rfc3339(
                                &row.get::<_, String>(8)?,
                            )
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                            last_used: row
                                .get::<_, Option<String>>(9)?
                                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                                .map(|dt| dt.with_timezone(&Utc)),
                            group_id: row
                                .get::<_, Option<String>>(10)?
                                .and_then(|s| Uuid::parse_str(&s).ok()),
                            bandwidth_limit_bps: row.get::<_, Option<i64>>(11)?.unwrap_or(0) as u64,
                            conflict_policy: row
                                .get::<_, Option<String>>(12)?
                                .map(|s| ConflictPolicy::from_str_lossy(&s)),
                            retry_policy: row
                                .get::<_, Option<String>>(13)?
                                .and_then(|s| serde_json::from_str(&s).ok()),
                            verify_checksums: row.get::<_, Option<bool>>(14)?.unwrap_or(false),
                            checksum_algorithm: row
                                .get::<_, Option<String>>(15)?
                                .map(|s| ChecksumAlgorithm::from_str_lossy(&s)),
                            proxy_type: row.get(16)?,
                            proxy_host: row.get(17)?,
                            proxy_port: row.get(18)?,
                            proxy_username: row.get(19)?,
                            // TODO(security/M-2): proxy_password loaded in plaintext — see save_connection comment
                            proxy_password: row.get(20)?,
                            default_local_dir: row.get(21)?,
                            default_remote_dir: row.get(22)?,
                            charset: row.get(23)?,
                            default_file_mode: None,
                            default_dir_mode: None,
                            ftp_use_mlsd: None,
                            ftp_force_passive_ip: None,
                            ftp_post_login_commands: None,
                            symlink_policy: None,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(connections)
            })
            .await
    }

    pub async fn delete_connection(&self, id: Uuid) -> Result<(), AppError> {
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "DELETE FROM connections WHERE id = ?1",
                    rusqlite::params![id.to_string()],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn get_connection(&self, id: Uuid) -> Result<Option<ConnectionProfile>, AppError> {
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, protocol, host, port, username, credential_ref,
                            remote_path, created_at, last_used, group_id, bandwidth_limit_bps,
                            conflict_policy, retry_policy, verify_checksums, checksum_algorithm,
                            proxy_type, proxy_host, proxy_port, proxy_username, proxy_password,
                            default_local_dir, default_remote_dir, charset
                     FROM connections WHERE id = ?1",
                )?;
                let result = stmt
                    .query_row(rusqlite::params![id.to_string()], |row| {
                        Ok(ConnectionProfile {
                            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                            name: row.get(1)?,
                            protocol: ConnectionProtocol::from_str_lossy(&row.get::<_, String>(2)?),
                            host: row.get(3)?,
                            port: row.get(4)?,
                            username: row.get(5)?,
                            credential_ref: row.get(6)?,
                            remote_path: row.get(7)?,
                            created_at: chrono::DateTime::parse_from_rfc3339(
                                &row.get::<_, String>(8)?,
                            )
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                            last_used: row
                                .get::<_, Option<String>>(9)?
                                .and_then(|s| chrono::DateTime::parse_from_rfc3339(&s).ok())
                                .map(|dt| dt.with_timezone(&Utc)),
                            group_id: row
                                .get::<_, Option<String>>(10)?
                                .and_then(|s| Uuid::parse_str(&s).ok()),
                            bandwidth_limit_bps: row.get::<_, Option<i64>>(11)?.unwrap_or(0) as u64,
                            conflict_policy: row
                                .get::<_, Option<String>>(12)?
                                .map(|s| ConflictPolicy::from_str_lossy(&s)),
                            retry_policy: row
                                .get::<_, Option<String>>(13)?
                                .and_then(|s| serde_json::from_str(&s).ok()),
                            verify_checksums: row.get::<_, Option<bool>>(14)?.unwrap_or(false),
                            checksum_algorithm: row
                                .get::<_, Option<String>>(15)?
                                .map(|s| ChecksumAlgorithm::from_str_lossy(&s)),
                            proxy_type: row.get(16)?,
                            proxy_host: row.get(17)?,
                            proxy_port: row.get(18)?,
                            proxy_username: row.get(19)?,
                            // TODO(security/M-2): proxy_password loaded in plaintext — see save_connection comment
                            proxy_password: row.get(20)?,
                            default_local_dir: row.get(21)?,
                            default_remote_dir: row.get(22)?,
                            charset: row.get(23)?,
                            default_file_mode: None,
                            default_dir_mode: None,
                            ftp_use_mlsd: None,
                            ftp_force_passive_ip: None,
                            ftp_post_login_commands: None,
                            symlink_policy: None,
                        })
                    })
                    .ok();
                Ok(result)
            })
            .await
    }

    // ── Connection Group operations ──

    pub async fn save_connection_group(&self, group: &ConnectionGroup) -> Result<(), AppError> {
        let group = group.clone();
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO connection_groups (id, name, parent_id, sort_order, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)
                     ON CONFLICT(id) DO UPDATE SET
                        name = ?2, parent_id = ?3, sort_order = ?4",
                    rusqlite::params![
                        group.id.to_string(),
                        group.name,
                        group.parent_id.map(|id| id.to_string()),
                        group.sort_order,
                        group.created_at.to_rfc3339(),
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn list_connection_groups(&self) -> Result<Vec<ConnectionGroup>, AppError> {
        self.pool
            .execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, name, parent_id, sort_order, created_at
                     FROM connection_groups ORDER BY sort_order, name",
                )?;
                let groups = stmt
                    .query_map([], |row| {
                        Ok(ConnectionGroup {
                            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                            name: row.get(1)?,
                            parent_id: row
                                .get::<_, Option<String>>(2)?
                                .and_then(|s| Uuid::parse_str(&s).ok()),
                            sort_order: row.get(3)?,
                            created_at: chrono::DateTime::parse_from_rfc3339(
                                &row.get::<_, String>(4)?,
                            )
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(groups)
            })
            .await
    }

    pub async fn delete_connection_group(&self, id: Uuid) -> Result<(), AppError> {
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "DELETE FROM connection_groups WHERE id = ?1",
                    rusqlite::params![id.to_string()],
                )?;
                Ok(())
            })
            .await
    }

    // ── Conflict resolution log ──

    pub async fn log_conflict_resolution(
        &self,
        resolution: &ConflictResolution,
    ) -> Result<(), AppError> {
        let r = resolution.clone();
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO conflict_resolutions (job_id, source_path, dest_path,
                     policy_applied, resolved_dest_path, source_size, dest_size,
                     source_modified, dest_modified, timestamp)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        r.job_id.to_string(),
                        r.source_path,
                        r.dest_path,
                        r.policy_applied.as_str(),
                        r.resolved_dest_path,
                        r.source_size as i64,
                        r.dest_size as i64,
                        r.source_modified.map(|dt| dt.to_rfc3339()),
                        r.dest_modified.map(|dt| dt.to_rfc3339()),
                        r.timestamp.to_rfc3339(),
                    ],
                )?;
                Ok(())
            })
            .await
    }

    // ── Audit event operations ──

    pub async fn log_audit_event(&self, event: AuditEvent) -> Result<(), AppError> {
        self.pool
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO audit_events (id, timestamp, event_type, actor, resource, details)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    rusqlite::params![
                        event.id.to_string(),
                        event.timestamp.to_rfc3339(),
                        event.event_type,
                        event.actor,
                        event.resource,
                        event.details,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    pub async fn query_audit_events(
        &self,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<AuditEvent>, AppError> {
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, timestamp, event_type, actor, resource, details
                     FROM audit_events ORDER BY timestamp DESC LIMIT ?1 OFFSET ?2",
                )?;
                let events = stmt
                    .query_map(rusqlite::params![limit, offset], |row| {
                        Ok(AuditEvent {
                            id: Uuid::parse_str(&row.get::<_, String>(0)?).unwrap_or_default(),
                            timestamp: chrono::DateTime::parse_from_rfc3339(
                                &row.get::<_, String>(1)?,
                            )
                            .map(|dt| dt.with_timezone(&Utc))
                            .unwrap_or_else(|_| Utc::now()),
                            event_type: row.get(2)?,
                            actor: row.get(3)?,
                            resource: row.get(4)?,
                            details: row.get(5)?,
                        })
                    })?
                    .filter_map(|r| r.ok())
                    .collect();
                Ok(events)
            })
            .await
    }

    /// Reset the entire database (drops all data and re-runs migrations).
    pub async fn reset(&self) -> Result<(), AppError> {
        self.pool
            .execute(|conn| {
                migrations::reset_database(conn)?;
                Ok(())
            })
            .await
    }
}

// ── StorageOperations trait implementation ──

impl StorageOperations for Repository {
    fn get_config(&self, key: &str) -> BoxFuture<'_, Result<Option<String>, AppError>> {
        let key = key.to_string();
        Box::pin(async move { self.get_config(&key).await })
    }

    fn set_config(&self, key: &str, value: &str) -> BoxFuture<'_, Result<(), AppError>> {
        let key = key.to_string();
        let value = value.to_string();
        Box::pin(async move { self.set_config(&key, &value).await })
    }

    fn save_workspace_state(
        &self,
        state: &WorkspaceState,
    ) -> BoxFuture<'_, Result<(), AppError>> {
        let state_json = match serde_json::to_string(state) {
            Ok(j) => j,
            Err(e) => return Box::pin(async move { Err(AppError::from(e)) }),
        };
        let version = state.version;
        Box::pin(async move {
            self.pool
                .execute(move |conn| {
                    conn.execute(
                        "UPDATE workspace_state SET version = ?1, state = ?2 WHERE id = 1",
                        rusqlite::params![version, state_json],
                    )?;
                    Ok(())
                })
                .await
        })
    }

    fn load_workspace_state(&self) -> BoxFuture<'_, Result<WorkspaceState, AppError>> {
        Box::pin(async move {
            self.pool
                .execute(|conn| {
                    let state_json: String = conn
                        .query_row(
                            "SELECT state FROM workspace_state WHERE id = 1",
                            [],
                            |row| row.get(0),
                        )
                        .unwrap_or_else(|_| "{}".to_string());

                    // Gracefully handle corrupted state
                    let state: WorkspaceState =
                        serde_json::from_str(&state_json).unwrap_or_default();
                    Ok(state)
                })
                .await
        })
    }

    fn save_transfer_queue(
        &self,
        jobs: &[TransferJob],
    ) -> BoxFuture<'_, Result<(), AppError>> {
        let jobs: Vec<TransferJob> = jobs.to_vec();
        Box::pin(async move {
            self.pool
                .execute(move |conn| {
                    let tx = conn.unchecked_transaction()?;

                    // Upsert all active/queued/paused jobs
                    for job in &jobs {
                        tx.execute(
                            "INSERT INTO transfer_history (id, source_path, dest_path, total_bytes,
                             transferred_bytes, status, created_at, updated_at, error_message,
                             priority, sort_order, retry_count, partial_checksum, conflict_policy,
                             connection_id, verify_checksum, checksum_algorithm, source_checksum,
                             dest_checksum, speed_bps, eta_seconds)
                             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21)
                             ON CONFLICT(id) DO UPDATE SET
                                transferred_bytes = ?5,
                                status = ?6,
                                updated_at = ?8,
                                error_message = ?9,
                                priority = ?10,
                                sort_order = ?11,
                                retry_count = ?12,
                                partial_checksum = ?13,
                                conflict_policy = ?14,
                                speed_bps = ?20,
                                eta_seconds = ?21",
                            rusqlite::params![
                                job.id.to_string(),
                                job.source_path,
                                job.dest_path,
                                job.total_bytes as i64,
                                job.transferred_bytes as i64,
                                job.status.as_str(),
                                job.created_at.to_rfc3339(),
                                job.updated_at.to_rfc3339(),
                                job.error_message,
                                job.priority.as_str(),
                                job.sort_order,
                                job.retry_count as i64,
                                job.partial_checksum,
                                job.conflict_policy.map(|p| p.as_str().to_string()),
                                job.connection_id.map(|id| id.to_string()),
                                job.verify_checksum,
                                job.checksum_algorithm.map(|a| a.as_str().to_string()),
                                job.source_checksum,
                                job.dest_checksum,
                                job.speed_bps as i64,
                                job.eta_seconds.map(|e| e as i64),
                            ],
                        )?;
                    }

                    tx.commit()?;
                    Ok(())
                })
                .await
        })
    }

    fn load_transfer_queue(&self) -> BoxFuture<'_, Result<Vec<TransferJob>, AppError>> {
        Box::pin(async move {
            self.pool
                .execute(|conn| {
                    let mut stmt = conn.prepare(
                        "SELECT id, source_path, dest_path, total_bytes, transferred_bytes,
                                status, created_at, updated_at, error_message,
                                priority, sort_order, retry_count, partial_checksum,
                                conflict_policy, connection_id, verify_checksum,
                                checksum_algorithm, source_checksum, dest_checksum,
                                speed_bps, eta_seconds
                         FROM transfer_history
                         WHERE status IN ('queued', 'active', 'paused')
                         ORDER BY
                            CASE priority WHEN 'high' THEN 0 WHEN 'normal' THEN 1 WHEN 'low' THEN 2 ELSE 1 END,
                            sort_order ASC,
                            created_at ASC",
                    )?;
                    let jobs = stmt
                        .query_map([], Self::row_to_transfer_job)?
                        .filter_map(|r| r.ok())
                        .map(|mut job| {
                            // Active transfers that were interrupted should be re-queued
                            if job.status == TransferStatus::Active {
                                job.status = TransferStatus::Queued;
                            }
                            // Reset speed/eta since we're restarting
                            job.speed_bps = 0;
                            job.eta_seconds = None;
                            job
                        })
                        .collect();
                    Ok(jobs)
                })
                .await
        })
    }
}

// Need to reference Self in the closure inside load_transfer_queue
// The row_to_transfer_job is a standalone method to keep closures simple.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::traits::StorageOperations;

    #[tokio::test]
    async fn test_repository_creation() {
        let repo = Repository::open_in_memory().await.unwrap();
        // Verify migrations ran
        let version = repo
            .pool()
            .execute(|conn| {
                let v: u32 = conn
                    .query_row("SELECT MAX(version) FROM _migrations", [], |row| {
                        row.get(0)
                    })?;
                Ok(v)
            })
            .await
            .unwrap();
        assert_eq!(version, 9);
    }

    #[tokio::test]
    async fn test_config_crud() {
        let repo = Repository::open_in_memory().await.unwrap();

        // Initially empty
        assert!(repo.get_config("theme").await.unwrap().is_none());

        // Set
        repo.set_config("theme", "dark").await.unwrap();
        assert_eq!(
            repo.get_config("theme").await.unwrap().unwrap(),
            "dark"
        );

        // Update
        repo.set_config("theme", "light").await.unwrap();
        assert_eq!(
            repo.get_config("theme").await.unwrap().unwrap(),
            "light"
        );
    }

    #[tokio::test]
    async fn test_workspace_state_persistence() {
        let repo = Repository::open_in_memory().await.unwrap();

        let mut state = WorkspaceState {
            theme: "dark".to_string(),
            ..Default::default()
        };
        state.tabs.push(TabState {
            id: Uuid::new_v4(),
            path: "/tmp".to_string(),
            label: "Temp".to_string(),
        });

        // Save
        StorageOperations::save_workspace_state(&repo, &state)
            .await
            .unwrap();

        // Load
        let loaded = StorageOperations::load_workspace_state(&repo)
            .await
            .unwrap();
        assert_eq!(loaded.theme, "dark");
        assert_eq!(loaded.tabs.len(), 2);
    }

    #[tokio::test]
    async fn test_corrupted_workspace_state_fallback() {
        let repo = Repository::open_in_memory().await.unwrap();

        // Write corrupt JSON directly
        repo.pool()
            .execute(|conn| {
                conn.execute(
                    "UPDATE workspace_state SET state = 'not-valid-json{' WHERE id = 1",
                    [],
                )?;
                Ok(())
            })
            .await
            .unwrap();

        // Should fall back to defaults, not crash
        let loaded = StorageOperations::load_workspace_state(&repo)
            .await
            .unwrap();
        assert_eq!(loaded.version, 1); // default version
        assert_eq!(loaded.theme, "system"); // default theme
    }

    #[tokio::test]
    async fn test_transfer_queue_persistence() {
        let repo = Repository::open_in_memory().await.unwrap();
        let now = Utc::now();

        let jobs = vec![
            TransferJob {
                id: Uuid::new_v4(),
                source_path: "/a".to_string(),
                dest_path: "/b".to_string(),
                total_bytes: 1000,
                transferred_bytes: 500,
                status: TransferStatus::Paused,
                priority: TransferPriority::Normal,
                sort_order: 0,
                created_at: now,
                updated_at: now,
                error_message: None,
                retry_count: 0,
                partial_checksum: None,
                conflict_policy: None,
                connection_id: None,
                verify_checksum: false,
                checksum_algorithm: None,
                source_checksum: None,
                dest_checksum: None,
                speed_bps: 0,
                eta_seconds: None,
            },
            TransferJob {
                id: Uuid::new_v4(),
                source_path: "/c".to_string(),
                dest_path: "/d".to_string(),
                total_bytes: 2000,
                transferred_bytes: 0,
                status: TransferStatus::Active,
                priority: TransferPriority::High,
                sort_order: 1,
                created_at: now,
                updated_at: now,
                error_message: None,
                retry_count: 0,
                partial_checksum: None,
                conflict_policy: None,
                connection_id: None,
                verify_checksum: false,
                checksum_algorithm: None,
                source_checksum: None,
                dest_checksum: None,
                speed_bps: 0,
                eta_seconds: None,
            },
        ];

        // Save
        StorageOperations::save_transfer_queue(&repo, &jobs)
            .await
            .unwrap();

        // Load
        let loaded = StorageOperations::load_transfer_queue(&repo)
            .await
            .unwrap();
        assert_eq!(loaded.len(), 2);

        // Active transfers should be re-queued on load
        let active_job = loaded.iter().find(|j| j.id == jobs[1].id).unwrap();
        assert_eq!(active_job.status, TransferStatus::Queued);

        // Paused transfers remain paused
        let paused_job = loaded.iter().find(|j| j.id == jobs[0].id).unwrap();
        assert_eq!(paused_job.status, TransferStatus::Paused);
        assert_eq!(paused_job.transferred_bytes, 500);
    }

    #[tokio::test]
    async fn test_bookmark_crud() {
        let repo = Repository::open_in_memory().await.unwrap();

        let bookmark = Bookmark {
            id: Uuid::new_v4(),
            name: "Home".to_string(),
            path: "/home/user".to_string(),
            icon: Some("home".to_string()),
            sort_order: 0,
            created_at: Utc::now(),
        };

        repo.add_bookmark(bookmark.clone()).await.unwrap();

        let bookmarks = repo.list_bookmarks().await.unwrap();
        assert_eq!(bookmarks.len(), 1);
        assert_eq!(bookmarks[0].name, "Home");

        repo.remove_bookmark(bookmark.id).await.unwrap();
        let bookmarks = repo.list_bookmarks().await.unwrap();
        assert!(bookmarks.is_empty());
    }

    #[tokio::test]
    async fn test_audit_event_crud() {
        let repo = Repository::open_in_memory().await.unwrap();

        for i in 0..5 {
            repo.log_audit_event(AuditEvent {
                id: Uuid::new_v4(),
                timestamp: Utc::now(),
                event_type: format!("test_{i}"),
                actor: "user".to_string(),
                resource: "/tmp".to_string(),
                details: "test".to_string(),
            })
            .await
            .unwrap();
        }

        let events = repo.query_audit_events(3, 0).await.unwrap();
        assert_eq!(events.len(), 3);

        let events = repo.query_audit_events(10, 3).await.unwrap();
        assert_eq!(events.len(), 2);
    }

    #[tokio::test]
    async fn test_reset_database() {
        let repo = Repository::open_in_memory().await.unwrap();
        repo.set_config("key", "value").await.unwrap();

        repo.reset().await.unwrap();

        // Data should be gone but tables should exist
        assert!(repo.get_config("key").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_database_persists_to_file() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("test_persist.db");

        // Write data
        {
            let pool = DbPool::open(&db_path).unwrap();
            let repo = Repository::new(pool).await.unwrap();
            repo.set_config("persist_test", "hello").await.unwrap();
        }

        // Reopen and verify data persists
        {
            let pool = DbPool::open(&db_path).unwrap();
            let repo = Repository::new(pool).await.unwrap();
            let val = repo.get_config("persist_test").await.unwrap();
            assert_eq!(val.unwrap(), "hello");
        }
    }
}
