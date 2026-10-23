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
        Migration {
            version: 10,
            description: "Smart Spaces - named workspaces bundling local folder + connection + sync pair + automations",
            sql: V10_SCHEMA,
        },
        Migration {
            version: 11,
            description: "Operation Ledger - unified append-only event store across all engines",
            sql: V11_SCHEMA,
        },
        Migration {
            version: 12,
            description: "Sync pairs - Robocopy-equivalent copy options",
            sql: V12_SCHEMA,
        },
        Migration {
            version: 13,
            description: "Backup engine - versioned backup jobs and run history",
            sql: V13_SCHEMA,
        },
        Migration {
            version: 14,
            description: "Server auth tokens - refresh tokens and used sign-in tokens",
            sql: V14_SCHEMA,
        },
        Migration {
            version: 15,
            description: "Sync pairs - destination-state mode and checksum algorithm",
            sql: V15_SCHEMA,
        },
        Migration {
            version: 16,
            description: "Sync run history",
            sql: V16_SCHEMA,
        },
        Migration {
            version: 17,
            description: "Security audit log - hash chain, signed anchors, append-only triggers",
            // The work needs SHA-256, so it runs in Rust: see `rust_step`.
            sql: "",
        },
        Migration {
            version: 18,
            description: "File metadata - color tags, labels, key-value metadata, smart folders",
            sql: V18_SCHEMA,
        },
        Migration {
            version: 19,
            description: "Automation v1.5 - condition trees, steps, options; per-step run results",
            sql: V19_SCHEMA,
        },
        Migration {
            version: 20,
            description: "Sync run options - settle window, completion marker, version naming",
            sql: V20_SCHEMA,
        },
        Migration {
            version: 21,
            description: "Connection settings_json - persists advanced options and extras",
            sql: V21_SCHEMA,
        },
        Migration {
            version: 22,
            description: "ufop-server scoped, expiring API tokens",
            sql: V22_SCHEMA,
        },
        Migration {
            version: 23,
            description: "Metadata schemas - typed, required fields per folder",
            sql: V23_SCHEMA,
        },
        Migration {
            version: 24,
            description: "ufop-server share portal - links and their access log",
            sql: V24_SCHEMA,
        },
        Migration {
            version: 25,
            description: "Mount options - cache mode, cache limits, read-only; mount error column",
            sql: V25_SCHEMA,
        },
        Migration {
            version: 26,
            description: "ufop-server folder policies - lock, extensions, limits, link rules, intake naming",
            sql: V26_SCHEMA,
        },
        Migration {
            version: 27,
            description: "Named schedules and holiday calendars shared by sync, automation and backup",
            sql: V27_SCHEMA,
        },
        Migration {
            version: 28,
            description: "Automation rule revisions (workflow history)",
            sql: V28_SCHEMA,
        },
        Migration {
            version: 29,
            description: "OAuth clients and refresh tokens for ufop-server's MCP endpoint",
            sql: V29_SCHEMA,
        },
    ]
}

/// Migration steps that cannot be expressed as static SQL. They run in the
/// same transaction as the migration's `sql`, after it.
fn rust_step(version: u32) -> Option<fn(&Connection) -> Result<(), AppError>> {
    match version {
        // Hash existing audit rows into a chain, then lock the table down.
        17 => Some(crate::security::audit_chain::migrate_v17),
        _ => None,
    }
}

/// Schema v21: profile fields with no column (upload modes, FTP tuning,
/// symlink policy, connector extras) as JSON; previously never saved.
/// Schema v22: named `ufop-server` API tokens (SHA-256 hashes) with scopes
/// (JSON list) and a mandatory expiry.
/// Schema v26: folder policies of folders ufop-server shares (`server::folders`).
/// Schema v29: OAuth 2.1 for `ufop-server`'s `/mcp` (`server::mcp_oauth`):
/// dynamically registered public clients and rotating refresh tokens
/// (SHA-256 hashes), each tied to the API token it last issued.
const V29_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS server_oauth_clients (
    client_id     TEXT PRIMARY KEY NOT NULL,
    client_name   TEXT NOT NULL,
    redirect_uris TEXT NOT NULL,
    created_at    TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS server_oauth_refresh (
    token_hash      TEXT PRIMARY KEY NOT NULL,
    client_id       TEXT NOT NULL REFERENCES server_oauth_clients(client_id) ON DELETE CASCADE,
    scope           TEXT NOT NULL,
    access_token_id TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    revoked         INTEGER NOT NULL DEFAULT 0
);
"#;

/// Schema v28: earlier versions of automation rules, kept when a rule is edited.
const V28_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS automation_rule_revisions (
    rule_id TEXT NOT NULL,
    revision INTEGER NOT NULL,
    data TEXT NOT NULL,
    saved_at TEXT NOT NULL,
    PRIMARY KEY (rule_id, revision)
);
"#;

/// Schema v27: the schedule library (`schedule_library`): named cron
/// schedules and holiday calendars (each as JSON).
const V27_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS named_schedules (
    id TEXT PRIMARY KEY NOT NULL,
    data TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS holiday_calendars (
    id TEXT PRIMARY KEY NOT NULL,
    data TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

const V26_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS server_folder_policies (
    path TEXT PRIMARY KEY NOT NULL,
    settings TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

/// Schema v25: per-mount options (`mount_engine::MountOptions` as JSON), and
/// the `error_message` column the mount engine always wrote but v7 never
/// created (every mount ended in a database error).
const V25_SCHEMA: &str = r#"
ALTER TABLE mount_configs ADD COLUMN options_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE mount_configs ADD COLUMN error_message TEXT;
"#;

/// Schema v24: ufop-server share links (`server::shares`). Only a hash of
/// each link's token is stored.
const V24_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS server_shares (
    id TEXT PRIMARY KEY NOT NULL,
    token_hash TEXT NOT NULL UNIQUE,
    token_prefix TEXT NOT NULL,
    name TEXT NOT NULL,
    path TEXT NOT NULL,
    kind TEXT NOT NULL,
    password_hash TEXT,
    expires_at TEXT,
    max_downloads INTEGER,
    downloads INTEGER NOT NULL DEFAULT 0,
    max_upload_bytes INTEGER,
    created_at TEXT NOT NULL,
    revoked_at TEXT,
    last_access_at TEXT
);
CREATE TABLE IF NOT EXISTS server_share_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    share_id TEXT NOT NULL REFERENCES server_shares(id) ON DELETE CASCADE,
    at TEXT NOT NULL,
    ip TEXT NOT NULL,
    action TEXT NOT NULL,
    detail TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_server_share_events_share ON server_share_events(share_id, id);
"#;

/// Schema v23: metadata schemas (`metadata_engine::schema`). One per folder;
/// `recursive` schemas also govern subfolders (the nearest one wins).
const V23_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS metadata_schemas (
    id TEXT PRIMARY KEY NOT NULL,
    name TEXT NOT NULL,
    folder TEXT NOT NULL UNIQUE,
    fields_json TEXT NOT NULL DEFAULT '[]',
    strict INTEGER NOT NULL DEFAULT 0,
    recursive INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

const V22_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS server_api_tokens (
    id           TEXT PRIMARY KEY NOT NULL,
    name         TEXT NOT NULL,
    token_hash   TEXT NOT NULL UNIQUE,
    scopes       TEXT NOT NULL DEFAULT '[]',
    created_at   TEXT NOT NULL,
    expires_at   TEXT NOT NULL,
    last_used_at TEXT,
    revoked      INTEGER NOT NULL DEFAULT 0
);
"#;

const V21_SCHEMA: &str = r#"
ALTER TABLE connections ADD COLUMN settings_json TEXT NOT NULL DEFAULT '{}';
"#;

/// Schema v20: per-pair run options (`SyncRunOptions`); `{}` = defaults.
const V20_SCHEMA: &str = r#"
ALTER TABLE sync_pairs ADD COLUMN run_options_json TEXT NOT NULL DEFAULT '{}';
"#;

/// Schema v19: automation rule extras (condition tree, steps, options) and
/// per-step results/attempt counts on runs. Old rows read as `{}`/`[]`,
/// which deserialize to the previous behaviour.
const V19_SCHEMA: &str = r#"
ALTER TABLE automation_rules ADD COLUMN extra_json TEXT NOT NULL DEFAULT '{}';
ALTER TABLE automation_logs ADD COLUMN attempts INTEGER NOT NULL DEFAULT 1;
ALTER TABLE automation_logs ADD COLUMN steps_json TEXT NOT NULL DEFAULT '[]';
"#;

/// Schema v18: file metadata (previously held in memory and lost on quit).
/// Rows are keyed by absolute path; see `metadata_engine`.
const V18_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS file_tag_defs (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE COLLATE NOCASE,
    color TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS file_tag_links (
    path TEXT NOT NULL,
    tag_id TEXT NOT NULL REFERENCES file_tag_defs(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (path, tag_id)
);
CREATE INDEX IF NOT EXISTS idx_file_tag_links_tag ON file_tag_links(tag_id);
CREATE TABLE IF NOT EXISTS file_labels (
    path TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (path, label)
);
CREATE TABLE IF NOT EXISTS file_metadata (
    path TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (path, key)
);
CREATE INDEX IF NOT EXISTS idx_file_metadata_key_value ON file_metadata(key, value);
CREATE TABLE IF NOT EXISTS smart_folders (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    query_json TEXT NOT NULL,
    icon TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
"#;

/// Schema v16: every finished sync run, kept across restarts.
const V16_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS sync_runs (
    id TEXT PRIMARY KEY,
    pair_id TEXT NOT NULL,
    pair_name TEXT NOT NULL,
    status TEXT NOT NULL,
    started_at TEXT NOT NULL,
    ended_at TEXT NOT NULL,
    report_json TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sync_runs_pair ON sync_runs(pair_id, started_at DESC);
"#;

/// Schema v15: how a sync pair learns the destination state (scan, trusted
/// inventory, blind upload) and which hash full verification uses.
const V15_SCHEMA: &str = r#"
ALTER TABLE sync_pairs ADD COLUMN dest_state TEXT NOT NULL DEFAULT 'scan';
ALTER TABLE sync_pairs ADD COLUMN checksum_algorithm TEXT NOT NULL DEFAULT 'xxh3';
"#;

/// Schema v14: `ufop-server` refresh tokens (stored as SHA-256 hashes) and
/// redeemed one-time sign-in token ids.
const V14_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS server_tokens (
    id          TEXT PRIMARY KEY NOT NULL,
    kind        TEXT NOT NULL,
    token_hash  TEXT NOT NULL,
    expires_at  TEXT NOT NULL,
    revoked     INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_server_tokens_hash ON server_tokens(token_hash);
"#;

/// Schema v13: versioned (deduplicated, encrypted) backup jobs. The job
/// definition is JSON so options can grow without migrations; each job's
/// block/version index lives in its own SQLite file next to the app DB.
const V13_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS backup_jobs (
    id          TEXT PRIMARY KEY NOT NULL,
    name        TEXT NOT NULL,
    config_json TEXT NOT NULL,
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE TABLE IF NOT EXISTS backup_runs (
    id          TEXT PRIMARY KEY NOT NULL,
    job_id      TEXT NOT NULL,
    operation   TEXT NOT NULL,
    status      TEXT NOT NULL,
    started_at  TEXT NOT NULL,
    finished_at TEXT,
    summary     TEXT NOT NULL DEFAULT '',
    report_json TEXT NOT NULL DEFAULT '{}',
    log_json    TEXT NOT NULL DEFAULT '[]'
);
CREATE INDEX IF NOT EXISTS idx_backup_runs_job ON backup_runs(job_id, started_at DESC);
"#;

/// Schema v12: per-pair Robocopy-equivalent copy options, stored as JSON so
/// new options never need another column. `{}` deserializes to the defaults.
const V12_SCHEMA: &str = r#"
ALTER TABLE sync_pairs ADD COLUMN options_json TEXT NOT NULL DEFAULT '{}';
"#;

/// Schema v11: Operation Ledger - unified append-only event store across all engines.
///
/// One row per operation event from any engine (transfer, sync, automation, fs, mount,
/// spaces, ai, vault). Powers cross-engine timeline, "what happened to file X" lookup,
/// undo of last operation, and forensic queries — without each engine inventing its
/// own log table. Writes are fail-open from engine call sites: a ledger error never
/// fails the underlying operation.
const V11_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS operation_ledger (
    id              TEXT PRIMARY KEY NOT NULL,
    occurred_at     TEXT NOT NULL DEFAULT (datetime('now')),
    engine          TEXT NOT NULL,
    kind            TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'ok',
    subject_path    TEXT,
    target_path     TEXT,
    bytes           INTEGER,
    correlation_id  TEXT,
    summary         TEXT NOT NULL DEFAULT '',
    details_json    TEXT NOT NULL DEFAULT '{}',
    undo_token      TEXT
);
CREATE INDEX IF NOT EXISTS idx_oplog_occurred  ON operation_ledger(occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_oplog_engine    ON operation_ledger(engine, occurred_at DESC);
CREATE INDEX IF NOT EXISTS idx_oplog_subject   ON operation_ledger(subject_path);
CREATE INDEX IF NOT EXISTS idx_oplog_correlation ON operation_ledger(correlation_id);
"#;

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

/// Schema v10: Smart Spaces - named workspaces bundling local folder + connection + sync pair + automations.
const V10_SCHEMA: &str = r#"
-- Smart Spaces: named persistent workspaces
CREATE TABLE IF NOT EXISTS smart_spaces (
    id              TEXT PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL,
    icon            TEXT NOT NULL DEFAULT 'folder-heart',
    color           TEXT NOT NULL DEFAULT '#6366f1',
    local_path      TEXT NOT NULL,
    connection_id   TEXT,
    remote_path     TEXT,
    sync_pair_id    TEXT,
    enabled         INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now'))
);
CREATE INDEX IF NOT EXISTS idx_smart_spaces_enabled ON smart_spaces(enabled);

-- Junction table: space <-> automation rules
CREATE TABLE IF NOT EXISTS space_automations (
    space_id    TEXT NOT NULL REFERENCES smart_spaces(id) ON DELETE CASCADE,
    rule_id     TEXT NOT NULL,
    PRIMARY KEY (space_id, rule_id)
);
CREATE INDEX IF NOT EXISTS idx_space_automations_space ON space_automations(space_id);
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
    run_migrations_up_to(conn, u32::MAX)
}

/// Run pending migrations with `version <= target` (tests use this to
/// build a database at an older schema version).
pub(crate) fn run_migrations_up_to(conn: &Connection, target: u32) -> Result<u32, AppError> {
    ensure_migrations_table(conn)?;
    let current = current_version(conn)?;
    let migrations = all_migrations();
    let mut applied = 0u32;

    for migration in &migrations {
        if migration.version <= current || migration.version > target {
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
        if let Some(step) = rust_step(migration.version) {
            step(&tx)?;
        }
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

/// Tables a reset never drops: the security audit log is append-only and
/// must outlive "reset all data" (#357). The reset itself is audited.
const RESET_PRESERVED_TABLES: [&str; 2] = ["audit_events", "audit_anchors"];

/// Reset the database by dropping all tables. Used for testing and recovery.
///
/// The security audit log (and its fallback signing key in `config`) is
/// kept, so a reset cannot be used to erase audit history.
pub fn reset_database(conn: &Connection) -> Result<(), AppError> {
    tracing::warn!("Resetting database - all data except the security audit log will be lost");

    // Get all table names
    let mut stmt = conn.prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",
    )?;
    let tables: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .filter_map(|r| r.ok())
        .collect();

    let audit_key: Option<String> = if tables.iter().any(|t| t == "config") {
        conn.query_row(
            "SELECT value FROM config WHERE key = ?1",
            [crate::security::audit::SETTINGS_KEY],
            |r| r.get(0),
        )
        .ok()
    } else {
        None
    };

    // Drop all tables
    for table in tables.iter().filter(|t| !RESET_PRESERVED_TABLES.contains(&t.as_str())) {
        conn.execute_batch(&format!("DROP TABLE IF EXISTS \"{}\"", table.replace('"', "\"\"")))?;
    }

    // Re-run migrations (v1 and v17 are no-ops on the preserved audit tables)
    run_migrations(conn)?;

    if let Some(k) = audit_key {
        conn.execute(
            "INSERT OR REPLACE INTO config (key, value) VALUES (?1, ?2)",
            rusqlite::params![crate::security::audit::SETTINGS_KEY, k],
        )?;
    }

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
        assert_eq!(applied, 29);

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
        assert!(tables.contains(&"backup_jobs".to_string()));
        assert!(tables.contains(&"backup_runs".to_string()));
        for t in ["file_tag_defs", "file_tag_links", "file_labels", "file_metadata", "smart_folders"] {
            assert!(tables.contains(&t.to_string()), "missing {t}");
        }
        assert!(tables.contains(&"_migrations".to_string()));
    }

    #[test]
    fn test_migrations_idempotent() {
        let conn = test_conn();
        let first = run_migrations(&conn).unwrap();
        assert_eq!(first, 29);

        let second = run_migrations(&conn).unwrap();
        assert_eq!(second, 0);
    }

    #[test]
    fn test_current_version() {
        let conn = test_conn();
        ensure_migrations_table(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 0);

        run_migrations(&conn).unwrap();
        assert_eq!(current_version(&conn).unwrap(), 29);
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
