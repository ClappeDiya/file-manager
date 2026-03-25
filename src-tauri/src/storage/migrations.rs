//! Schema migration framework for SQLite.
//!
//! Each migration is a versioned SQL script. The framework tracks
//! which migrations have been applied and runs pending ones in order.
//! All migrations run within a transaction for atomicity.

use crate::core::error::AppError;
use rusqlite::Connection;

/// A single schema migration.
struct Migration {
    version: u32,
    description: &'static str,
    sql: &'static str,
}

/// All migrations in order. Add new migrations to the end.
fn all_migrations() -> Vec<Migration> {
    vec![
        Migration {
            version: 1,
            description: "Initial schema - config, connections, bookmarks, transfer_history, sync_state, compat_mappings, audit_events, workspace_state",
            sql: V1_SCHEMA,
        },
        Migration {
            version: 2,
            description: "Enhanced transfer engine - history log, connection groups, enhanced transfer_history with priority/retry/checksum fields",
            sql: V2_SCHEMA,
        },
        Migration {
            version: 3,
            description: "Three-layer transfer integrity - transfer_jobs journal and transfer_chunks bitmap tables",
            sql: V3_SCHEMA,
        },
        Migration {
            version: 4,
            description: "Enhanced sync engine - triggers, filters, conflict policies, dry-run, rollback, reports",
            sql: V4_SCHEMA,
        },
        Migration {
            version: 5,
            description: "Enhanced compatibility engine - mapping records for reversible filename translations",
            sql: V5_SCHEMA,
        },
        Migration {
            version: 6,
            description: "Connection enhancements - proxy, default paths, charset fields",
            sql: V6_SCHEMA,
        },
        Migration {
            version: 7,
            description: "Mount engine - mount_configs table for Finder-mounted remote storage",
            sql: V7_SCHEMA,
        },
        Migration {
            version: 8,
            description: "Sync time offset compensation and FTP data type",
            sql: V8_SCHEMA,
        },
        Migration {
            version: 9,
            description: "Automation engine - quickflow rules and execution logs",
            sql: V9_SCHEMA,
        },
    ]
}

/// Schema v9: Automation engine - quickflow rules and execution logs.
const V9_SCHEMA: &str = r#"
-- Automation rules (trigger -> conditions -> action)
CREATE TABLE IF NOT EXISTS automation_rules (
    id              TEXT PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 0,
    trigger_json    TEXT NOT NULL,
    conditions_json TEXT NOT NULL DEFAULT '[]',
    action_json     TEXT NOT NULL,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
    last_triggered  TEXT,
    trigger_count   INTEGER NOT NULL DEFAULT 0,
    error_count     INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT
);
CREATE INDEX IF NOT EXISTS idx_automation_rules_enabled ON automation_rules(enabled);

-- Automation execution log
CREATE TABLE IF NOT EXISTS automation_logs (
    id              TEXT PRIMARY KEY NOT NULL,
    rule_id         TEXT NOT NULL REFERENCES automation_rules(id) ON DELETE CASCADE,
    rule_name       TEXT NOT NULL,
    triggered_at    TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at    TEXT,
    trigger_event   TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'running',
    files_affected  TEXT NOT NULL DEFAULT '[]',
    error_message   TEXT
);
CREATE INDEX IF NOT EXISTS idx_automation_logs_rule ON automation_logs(rule_id);
CREATE INDEX IF NOT EXISTS idx_automation_logs_triggered ON automation_logs(triggered_at DESC);
CREATE INDEX IF NOT EXISTS idx_automation_logs_status ON automation_logs(status);
"#;

/// Schema v8: Sync time offset and FTP data type fields.
const V8_SCHEMA: &str = r#"
-- Add time offset compensation to sync pairs (clock skew between local and remote)
ALTER TABLE sync_pairs ADD COLUMN time_offset_secs INTEGER;
"#;

/// Schema v1: All initial tables.
const V1_SCHEMA: &str = r#"
-- Application configuration key-value store
CREATE TABLE IF NOT EXISTS config (
    key         TEXT PRIMARY KEY NOT NULL,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

-- Remote connection profiles
CREATE TABLE IF NOT EXISTS connections (
    id              TEXT PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL,
    protocol        TEXT NOT NULL,
    host            TEXT NOT NULL,
    port            INTEGER,
    username        TEXT,
    credential_ref  TEXT,
    remote_path     TEXT NOT NULL DEFAULT '/',
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    last_used       TEXT
);
CREATE INDEX IF NOT EXISTS idx_connections_protocol ON connections(protocol);

-- Filesystem bookmarks
CREATE TABLE IF NOT EXISTS bookmarks (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    path        TEXT NOT NULL,
    icon        TEXT,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_bookmarks_sort ON bookmarks(sort_order);

-- Transfer history and crash-safe queue
CREATE TABLE IF NOT EXISTS transfer_history (
    id                  TEXT PRIMARY KEY NOT NULL,
    source_path         TEXT NOT NULL,
    dest_path           TEXT NOT NULL,
    total_bytes         INTEGER NOT NULL DEFAULT 0,
    transferred_bytes   INTEGER NOT NULL DEFAULT 0,
    status              TEXT NOT NULL DEFAULT 'queued',
    created_at          TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at          TEXT NOT NULL DEFAULT (datetime('now')),
    error_message       TEXT
);
CREATE INDEX IF NOT EXISTS idx_transfer_status ON transfer_history(status);
CREATE INDEX IF NOT EXISTS idx_transfer_created ON transfer_history(created_at DESC);

-- Sync pair configuration
CREATE TABLE IF NOT EXISTS sync_pairs (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    source_path TEXT NOT NULL,
    dest_path   TEXT NOT NULL,
    mode        TEXT NOT NULL DEFAULT 'one_way',
    enabled     INTEGER NOT NULL DEFAULT 1,
    last_run    TEXT
);

-- Sync state / run history
CREATE TABLE IF NOT EXISTS sync_state (
    id              INTEGER PRIMARY KEY AUTOINCREMENT,
    pair_id         TEXT NOT NULL REFERENCES sync_pairs(id) ON DELETE CASCADE,
    last_sync_at    TEXT NOT NULL DEFAULT (datetime('now')),
    files_synced    INTEGER NOT NULL DEFAULT 0,
    bytes_synced    INTEGER NOT NULL DEFAULT 0,
    errors          INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'success'
);
CREATE INDEX IF NOT EXISTS idx_sync_state_pair ON sync_state(pair_id);

-- Compatibility name mappings
CREATE TABLE IF NOT EXISTS compat_mappings (
    id                  TEXT PRIMARY KEY NOT NULL,
    source_pattern      TEXT NOT NULL,
    target_replacement  TEXT NOT NULL,
    platform            TEXT NOT NULL,
    enabled             INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_compat_platform ON compat_mappings(platform);

-- Audit events
CREATE TABLE IF NOT EXISTS audit_events (
    id          TEXT PRIMARY KEY NOT NULL,
    timestamp   TEXT NOT NULL DEFAULT (datetime('now')),
    event_type  TEXT NOT NULL,
    actor       TEXT NOT NULL,
    resource    TEXT NOT NULL,
    details     TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_audit_timestamp ON audit_events(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_audit_type ON audit_events(event_type);

-- Workspace state (single row, versioned JSON blob)
CREATE TABLE IF NOT EXISTS workspace_state (
    id      INTEGER PRIMARY KEY CHECK (id = 1),
    version INTEGER NOT NULL DEFAULT 1,
    state   TEXT NOT NULL DEFAULT '{}'
);
-- Insert default row
INSERT OR IGNORE INTO workspace_state (id, version, state) VALUES (1, 1, '{}');
"#;

/// Schema v2: Enhanced transfer engine tables.
const V2_SCHEMA: &str = r#"
-- Enhanced transfer history log (separate from active queue)
CREATE TABLE IF NOT EXISTS transfer_history_log (
    id                  TEXT PRIMARY KEY NOT NULL,
    source_path         TEXT NOT NULL,
    dest_path           TEXT NOT NULL,
    total_bytes         INTEGER NOT NULL DEFAULT 0,
    transferred_bytes   INTEGER NOT NULL DEFAULT 0,
    status              TEXT NOT NULL DEFAULT 'completed',
    started_at          TEXT NOT NULL DEFAULT (datetime('now')),
    completed_at        TEXT,
    duration_ms         INTEGER,
    speed_bps           INTEGER NOT NULL DEFAULT 0,
    error_message       TEXT,
    connection_id       TEXT,
    checksum_verified   INTEGER NOT NULL DEFAULT 0,
    checksum_match      INTEGER,
    conflict_policy     TEXT,
    conflict_resolution TEXT,
    retry_count         INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_history_log_status ON transfer_history_log(status);
CREATE INDEX IF NOT EXISTS idx_history_log_started ON transfer_history_log(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_history_log_source ON transfer_history_log(source_path);
CREATE INDEX IF NOT EXISTS idx_history_log_dest ON transfer_history_log(dest_path);
CREATE INDEX IF NOT EXISTS idx_history_log_connection ON transfer_history_log(connection_id);

-- Add enhanced columns to transfer_history (active queue)
-- SQLite ALTER TABLE only supports adding columns, so we add new ones
ALTER TABLE transfer_history ADD COLUMN priority TEXT NOT NULL DEFAULT 'normal';
ALTER TABLE transfer_history ADD COLUMN sort_order INTEGER NOT NULL DEFAULT 0;
ALTER TABLE transfer_history ADD COLUMN retry_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE transfer_history ADD COLUMN partial_checksum TEXT;
ALTER TABLE transfer_history ADD COLUMN conflict_policy TEXT;
ALTER TABLE transfer_history ADD COLUMN connection_id TEXT;
ALTER TABLE transfer_history ADD COLUMN verify_checksum INTEGER NOT NULL DEFAULT 0;
ALTER TABLE transfer_history ADD COLUMN checksum_algorithm TEXT;
ALTER TABLE transfer_history ADD COLUMN source_checksum TEXT;
ALTER TABLE transfer_history ADD COLUMN dest_checksum TEXT;
ALTER TABLE transfer_history ADD COLUMN speed_bps INTEGER NOT NULL DEFAULT 0;
ALTER TABLE transfer_history ADD COLUMN eta_seconds INTEGER;

-- Connection groups (folders for organizing connections)
CREATE TABLE IF NOT EXISTS connection_groups (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    parent_id   TEXT,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    FOREIGN KEY (parent_id) REFERENCES connection_groups(id) ON DELETE SET NULL
);

-- Add enhanced columns to connections
ALTER TABLE connections ADD COLUMN group_id TEXT REFERENCES connection_groups(id) ON DELETE SET NULL;
ALTER TABLE connections ADD COLUMN bandwidth_limit_bps INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN conflict_policy TEXT;
ALTER TABLE connections ADD COLUMN retry_policy TEXT;
ALTER TABLE connections ADD COLUMN verify_checksums INTEGER NOT NULL DEFAULT 0;
ALTER TABLE connections ADD COLUMN checksum_algorithm TEXT;

-- Conflict resolution log
CREATE TABLE IF NOT EXISTS conflict_resolutions (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    job_id              TEXT NOT NULL,
    source_path         TEXT NOT NULL,
    dest_path           TEXT NOT NULL,
    policy_applied      TEXT NOT NULL,
    resolved_dest_path  TEXT NOT NULL,
    source_size         INTEGER NOT NULL DEFAULT 0,
    dest_size           INTEGER NOT NULL DEFAULT 0,
    source_modified     TEXT,
    dest_modified       TEXT,
    timestamp           TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_conflict_job ON conflict_resolutions(job_id);
"#;

/// Schema v3: Three-layer transfer integrity tables.
const V3_SCHEMA: &str = r#"
-- Transfer journal: active transfer jobs with crash recovery metadata
CREATE TABLE IF NOT EXISTS transfer_jobs (
    job_id          TEXT PRIMARY KEY,
    source_path     TEXT NOT NULL,
    dest_path       TEXT NOT NULL,
    temp_path       TEXT NOT NULL,
    file_size       INTEGER NOT NULL,
    chunk_size      INTEGER NOT NULL,
    chunk_count     INTEGER NOT NULL,
    verify_tier     INTEGER NOT NULL DEFAULT 1,
    status          TEXT NOT NULL DEFAULT 'active',
    source_hash     TEXT,
    dest_hash       TEXT,
    started_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    completed_at    TEXT,
    error           TEXT
);
CREATE INDEX IF NOT EXISTS idx_transfer_jobs_status ON transfer_jobs(status);

-- Per-chunk status for bitmap-based resume
CREATE TABLE IF NOT EXISTS transfer_chunks (
    job_id          TEXT NOT NULL REFERENCES transfer_jobs(job_id) ON DELETE CASCADE,
    chunk_index     INTEGER NOT NULL,
    offset          INTEGER NOT NULL,
    length          INTEGER NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending',
    source_hash     TEXT,
    dest_hash       TEXT,
    attempts        INTEGER DEFAULT 0,
    verified_at     TEXT,
    PRIMARY KEY (job_id, chunk_index)
);
CREATE INDEX IF NOT EXISTS idx_transfer_chunks_status ON transfer_chunks(status);
"#;

/// Schema v4: Enhanced sync engine tables for T-039..T-042.
const V4_SCHEMA: &str = r#"
-- Enhance sync_pairs with trigger, filter, conflict policy, verification
ALTER TABLE sync_pairs ADD COLUMN trigger_mode TEXT NOT NULL DEFAULT 'manual';
ALTER TABLE sync_pairs ADD COLUMN cron_expr TEXT;
ALTER TABLE sync_pairs ADD COLUMN filter_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE sync_pairs ADD COLUMN conflict_policy TEXT NOT NULL DEFAULT 'create_conflict_copy';
ALTER TABLE sync_pairs ADD COLUMN verify_mode TEXT NOT NULL DEFAULT 'fast';
ALTER TABLE sync_pairs ADD COLUMN checksum_enabled INTEGER NOT NULL DEFAULT 0;
ALTER TABLE sync_pairs ADD COLUMN created_at TEXT NOT NULL DEFAULT (datetime('now'));

-- Sync run reports
CREATE TABLE IF NOT EXISTS sync_reports (
    id                  TEXT PRIMARY KEY NOT NULL,
    pair_id             TEXT NOT NULL REFERENCES sync_pairs(id) ON DELETE CASCADE,
    pair_name           TEXT NOT NULL,
    started_at          TEXT NOT NULL,
    ended_at            TEXT NOT NULL,
    duration_ms         INTEGER NOT NULL DEFAULT 0,
    files_added         INTEGER NOT NULL DEFAULT 0,
    files_modified      INTEGER NOT NULL DEFAULT 0,
    files_deleted       INTEGER NOT NULL DEFAULT 0,
    files_skipped       INTEGER NOT NULL DEFAULT 0,
    conflicts_resolved  INTEGER NOT NULL DEFAULT 0,
    errors              INTEGER NOT NULL DEFAULT 0,
    bytes_transferred   INTEGER NOT NULL DEFAULT 0,
    status              TEXT NOT NULL DEFAULT 'success',
    health              TEXT NOT NULL DEFAULT 'gray',
    error_messages      TEXT NOT NULL DEFAULT '[]',
    resumed             INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_sync_reports_pair ON sync_reports(pair_id);
CREATE INDEX IF NOT EXISTS idx_sync_reports_started ON sync_reports(started_at DESC);

-- Pre-destructive snapshots for rollback
CREATE TABLE IF NOT EXISTS sync_snapshots (
    id                  TEXT PRIMARY KEY NOT NULL,
    run_id              TEXT NOT NULL,
    pair_id             TEXT NOT NULL REFERENCES sync_pairs(id) ON DELETE CASCADE,
    relative_path       TEXT NOT NULL,
    action              TEXT NOT NULL,
    backup_path         TEXT NOT NULL,
    original_size       INTEGER NOT NULL DEFAULT 0,
    original_modified   TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_sync_snapshots_run ON sync_snapshots(run_id);
CREATE INDEX IF NOT EXISTS idx_sync_snapshots_pair ON sync_snapshots(pair_id);

-- Sync conflict queue (ask mode + quarantine)
CREATE TABLE IF NOT EXISTS sync_conflicts (
    id                  TEXT PRIMARY KEY NOT NULL,
    pair_id             TEXT NOT NULL REFERENCES sync_pairs(id) ON DELETE CASCADE,
    relative_path       TEXT NOT NULL,
    source_size         INTEGER NOT NULL DEFAULT 0,
    dest_size           INTEGER NOT NULL DEFAULT 0,
    source_modified     TEXT,
    dest_modified       TEXT,
    resolution          TEXT,
    created_at          TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_sync_conflicts_pair ON sync_conflicts(pair_id);

-- Quarantine entries
CREATE TABLE IF NOT EXISTS sync_quarantine (
    id                  TEXT PRIMARY KEY NOT NULL,
    pair_id             TEXT NOT NULL REFERENCES sync_pairs(id) ON DELETE CASCADE,
    original_path       TEXT NOT NULL,
    quarantine_path     TEXT NOT NULL,
    source_size         INTEGER NOT NULL DEFAULT 0,
    dest_size           INTEGER NOT NULL DEFAULT 0,
    source_modified     TEXT,
    dest_modified       TEXT,
    quarantined_at      TEXT NOT NULL DEFAULT (datetime('now')),
    resolved            INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_sync_quarantine_pair ON sync_quarantine(pair_id);

-- Resumable sync state for crash recovery
CREATE TABLE IF NOT EXISTS sync_resume_state (
    run_id              TEXT PRIMARY KEY NOT NULL,
    pair_id             TEXT NOT NULL REFERENCES sync_pairs(id) ON DELETE CASCADE,
    last_completed_index INTEGER NOT NULL DEFAULT 0,
    total_files         INTEGER NOT NULL DEFAULT 0,
    plan_json           TEXT NOT NULL DEFAULT '[]',
    started_at          TEXT NOT NULL,
    interrupted_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_sync_resume_pair ON sync_resume_state(pair_id);

-- Sync conflict resolution log
CREATE TABLE IF NOT EXISTS sync_conflict_log (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    pair_id             TEXT NOT NULL,
    run_id              TEXT NOT NULL,
    relative_path       TEXT NOT NULL,
    policy_applied      TEXT NOT NULL,
    source_size         INTEGER,
    dest_size           INTEGER,
    source_modified     TEXT,
    dest_modified       TEXT,
    result_path         TEXT,
    timestamp           TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_sync_conflict_log_run ON sync_conflict_log(run_id);
"#;

/// Schema v5: Enhanced compatibility engine mapping records (T-034..T-038).
const V5_SCHEMA: &str = r#"
-- Compatibility mapping records for reversible filename translations
CREATE TABLE IF NOT EXISTS compat_mapping_records (
    id              TEXT PRIMARY KEY NOT NULL,
    original_name   TEXT NOT NULL,
    translated_name TEXT NOT NULL,
    source_path     TEXT NOT NULL,
    dest_path       TEXT NOT NULL,
    profile_id      TEXT NOT NULL,
    rules_applied   TEXT NOT NULL DEFAULT '[]',
    timestamp       TEXT NOT NULL,
    reversible      INTEGER NOT NULL DEFAULT 1,
    tier            INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_compat_map_translated
    ON compat_mapping_records(translated_name);
CREATE INDEX IF NOT EXISTS idx_compat_map_original
    ON compat_mapping_records(original_name);
CREATE INDEX IF NOT EXISTS idx_compat_map_dest
    ON compat_mapping_records(dest_path);
CREATE INDEX IF NOT EXISTS idx_compat_map_timestamp
    ON compat_mapping_records(timestamp DESC);
CREATE INDEX IF NOT EXISTS idx_compat_map_tier
    ON compat_mapping_records(tier);
"#;

/// Schema v6: Connection enhancements - proxy config, default paths, charset.
const V6_SCHEMA: &str = r#"
-- Per-connection proxy configuration
ALTER TABLE connections ADD COLUMN proxy_type TEXT;
ALTER TABLE connections ADD COLUMN proxy_host TEXT;
ALTER TABLE connections ADD COLUMN proxy_port INTEGER;
ALTER TABLE connections ADD COLUMN proxy_username TEXT;
ALTER TABLE connections ADD COLUMN proxy_password TEXT;

-- Per-connection default paths
ALTER TABLE connections ADD COLUMN default_local_dir TEXT;
ALTER TABLE connections ADD COLUMN default_remote_dir TEXT;

-- Per-connection charset (for FTP/FTPS with legacy encodings)
ALTER TABLE connections ADD COLUMN charset TEXT;
"#;

/// Schema v7: Mount engine - mount_configs table for Finder-mounted remote storage.
const V7_SCHEMA: &str = r#"
-- Mount configurations for virtual/FUSE-mounted remote drives
CREATE TABLE IF NOT EXISTS mount_configs (
    id              TEXT PRIMARY KEY NOT NULL,
    connection_id   TEXT NOT NULL,
    mount_point     TEXT NOT NULL,
    auto_mount      INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'unmounted',
    created_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_mount_configs_connection ON mount_configs(connection_id);
CREATE INDEX IF NOT EXISTS idx_mount_configs_status ON mount_configs(status);
"#;

/// Ensure the migrations tracking table exists.
fn ensure_migrations_table(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS _migrations (
            version     INTEGER PRIMARY KEY NOT NULL,
            description TEXT NOT NULL,
            applied_at  TEXT NOT NULL DEFAULT (datetime('now'))
        );",
    )?;
    Ok(())
}

/// Get the current schema version (highest applied migration).
fn current_version(conn: &Connection) -> Result<u32, AppError> {
    let version: u32 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM _migrations",
            [],
            |row| row.get(0),
        )?;
    Ok(version)
}

/// Run all pending migrations. Returns the number of migrations applied.
pub fn run_migrations(conn: &Connection) -> Result<u32, AppError> {
    ensure_migrations_table(conn)?;
    let current = current_version(conn)?;
    let migrations = all_migrations();
    let mut applied = 0u32;

    for migration in &migrations {
        if migration.version <= current {
            continue;
        }

        tracing::info!(
            "Applying migration v{}: {}",
            migration.version,
            migration.description
        );

        // Run migration in a transaction
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(migration.sql)?;
        tx.execute(
            "INSERT INTO _migrations (version, description) VALUES (?1, ?2)",
            rusqlite::params![migration.version, migration.description],
        )?;
        tx.commit()?;

        applied += 1;
        tracing::info!("Migration v{} applied successfully", migration.version);
    }

    if applied == 0 {
        tracing::info!("Database schema is up to date (v{current})");
    }

    Ok(applied)
}

/// Reset the database by dropping all tables. Used for testing and recovery.
pub fn reset_database(conn: &Connection) -> Result<(), AppError> {
    tracing::warn!("Resetting database - all data will be lost");

    // Get all table names
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    // Drop all tables
    for table in &tables {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{table}\""))?;
    }

    // Re-run migrations
    run_migrations(conn)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_conn() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON;").unwrap();
        conn
    }

    #[test]
    fn test_run_migrations_fresh_db() {
        let conn = test_conn();
        let applied = run_migrations(&conn).unwrap();
        assert_eq!(applied, 9);

        // Verify all tables exist
        let tables: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .filter_map(|r| r.ok())
                .collect()
        };

        assert!(tables.contains(&"config".to_string()));
        assert!(tables.contains(&"connections".to_string()));
        assert!(tables.contains(&"bookmarks".to_string()));
        assert!(tables.contains(&"transfer_history".to_string()));
        assert!(tables.contains(&"sync_pairs".to_string()));
        assert!(tables.contains(&"sync_state".to_string()));
        assert!(tables.contains(&"compat_mappings".to_string()));
        assert!(tables.contains(&"audit_events".to_string()));
        assert!(tables.contains(&"workspace_state".to_string()));
        assert!(tables.contains(&"mount_configs".to_string()));
        assert!(tables.contains(&"_migrations".to_string()));
    }

    #[test]
    fn test_migrations_idempotent() {
        let conn = test_conn();
        let first = run_migrations(&conn).unwrap();
        assert_eq!(first, 9);

        let second = run_migrations(&conn).unwrap();
        assert_eq!(second, 0);
    }

    #[test]
    fn test_current_version() {
        let conn = test_conn();
        ensure_migrations_table(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);

        run_migrations(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 9);
    }

    #[test]
    fn test_reset_database() {
        let conn = test_conn();
        run_migrations(&conn).unwrap();

        // Insert some data
        conn.execute(
            "INSERT INTO config (key, value) VALUES ('test_key', 'test_value')",
            [],
        )
        .unwrap();

        // Reset
        reset_database(&conn).unwrap();

        // Verify tables exist but data is gone
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM config", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn test_workspace_state_default_row() {
        let conn = test_conn();
        run_migrations(&conn).unwrap();

        let (version, state): (u32, String) = conn
            .query_row(
                "SELECT version, state FROM workspace_state WHERE id = 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(version, 1);
        assert_eq!(state, "{}");
    }

    #[test]
    fn test_foreign_key_constraint() {
        let conn = test_conn();
        run_migrations(&conn).unwrap();

        // Inserting sync_state with non-existent pair_id should fail
        let result = conn.execute(
            "INSERT INTO sync_state (pair_id, status) VALUES ('nonexistent', 'success')",
            [],
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_transfer_history_insert() {
        let conn = test_conn();
        run_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO transfer_history (id, source_path, dest_path, total_bytes, status) VALUES (?1, ?2, ?3, ?4, ?5)",
            rusqlite::params!["test-id", "/src/file.txt", "/dst/file.txt", 1024, "queued"],
        )
        .unwrap();

        let status: String = conn
            .query_row(
                "SELECT status FROM transfer_history WHERE id = 'test-id'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(status, "queued");
    }
}
