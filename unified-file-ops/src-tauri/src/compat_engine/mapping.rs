//! Mapping database and name restoration (T-037).
//!
//! Stores reversible filename translations in SQLite so files can be
//! restored to their original names when returned to a compatible destination.
//!
//! Features:
//! - SQLite mapping table with full audit trail
//! - Restore operation for returning files to original names
//! - Optional sidecar manifest (.ufop-manifest.json)
//! - Optional xattr storage on macOS/Linux
//! - Configurable retention (default 1 year)
//! - Search on both logical (original) and physical (translated) names

use chrono::{DateTime, Utc};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::core::error::AppError;
use crate::storage::DbPool;

/// A single mapping record in the database.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MappingRecord {
    pub id: String,
    pub original_name: String,
    pub translated_name: String,
    pub source_path: String,
    pub dest_path: String,
    pub profile_id: String,
    pub rules_applied: Vec<String>,
    pub timestamp: DateTime<Utc>,
    pub reversible: bool,
    /// Intervention tier: 1=auto-quiet, 2=visible-auto, 3=decision-required
    pub tier: u8,
}

/// Sidecar manifest for a directory of translated files.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarManifest {
    pub version: u32,
    pub created_at: String,
    pub updated_at: String,
    pub profile_id: String,
    pub mappings: Vec<SidecarEntry>,
}

/// A single entry in the sidecar manifest.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SidecarEntry {
    pub original_name: String,
    pub translated_name: String,
    pub rules: Vec<String>,
    pub reversible: bool,
    pub tier: u8,
}

/// Mapping retention configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetentionConfig {
    /// Days to keep mapping records (0 = forever).
    pub retention_days: u32,
    /// Maximum number of records to keep (0 = unlimited).
    pub max_records: u32,
}

impl Default for RetentionConfig {
    fn default() -> Self {
        Self {
            retention_days: 365, // 1 year
            max_records: 0,      // unlimited
        }
    }
}

/// The mapping database manager.
pub struct MappingDb {
    pool: DbPool,
}

impl MappingDb {
    /// Create a new MappingDb with the given connection pool.
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    /// Initialize the mapping table (called from migration v4).
    pub async fn ensure_table(&self) -> Result<(), AppError> {
        self.pool
            .execute(|conn| {
                create_mapping_table(conn)?;
                Ok(())
            })
            .await
    }

    /// Store a mapping record.
    pub async fn store(&self, record: &MappingRecord) -> Result<(), AppError> {
        let record = record.clone();
        self.pool
            .execute(move |conn| {
                let rules_json = serde_json::to_string(&record.rules_applied)
                    .unwrap_or_else(|_| "[]".to_string());
                conn.execute(
                    "INSERT OR REPLACE INTO compat_mapping_records
                     (id, original_name, translated_name, source_path, dest_path,
                      profile_id, rules_applied, timestamp, reversible, tier)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        record.id,
                        record.original_name,
                        record.translated_name,
                        record.source_path,
                        record.dest_path,
                        record.profile_id,
                        rules_json,
                        record.timestamp.to_rfc3339(),
                        record.reversible,
                        record.tier,
                    ],
                )?;
                Ok(())
            })
            .await
    }

    /// Store multiple mapping records in a single transaction.
    pub async fn store_batch(&self, records: &[MappingRecord]) -> Result<(), AppError> {
        let records = records.to_vec();
        self.pool
            .execute(move |conn| {
                let tx = conn.unchecked_transaction()?;
                for record in &records {
                    let rules_json = serde_json::to_string(&record.rules_applied)
                        .unwrap_or_else(|_| "[]".to_string());
                    tx.execute(
                        "INSERT OR REPLACE INTO compat_mapping_records
                         (id, original_name, translated_name, source_path, dest_path,
                          profile_id, rules_applied, timestamp, reversible, tier)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        rusqlite::params![
                            record.id,
                            record.original_name,
                            record.translated_name,
                            record.source_path,
                            record.dest_path,
                            record.profile_id,
                            rules_json,
                            record.timestamp.to_rfc3339(),
                            record.reversible,
                            record.tier,
                        ],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await
    }

    /// Look up the original name for a translated file.
    pub async fn find_original(&self, translated_name: &str, dest_path: &str) -> Result<Option<MappingRecord>, AppError> {
        let translated_name = translated_name.to_string();
        let dest_path = dest_path.to_string();
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, original_name, translated_name, source_path, dest_path,
                            profile_id, rules_applied, timestamp, reversible, tier
                     FROM compat_mapping_records
                     WHERE translated_name = ?1 AND dest_path = ?2
                     ORDER BY timestamp DESC LIMIT 1",
                )?;
                let result = stmt.query_row(
                    rusqlite::params![translated_name, dest_path],
                    row_to_record,
                );
                match result {
                    Ok(record) => Ok(Some(record)),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(AppError::from(e)),
                }
            })
            .await
    }

    /// Search mappings by original or translated name (substring match).
    pub async fn search(&self, query: &str, limit: u32) -> Result<Vec<MappingRecord>, AppError> {
        let query = format!("%{query}%");
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, original_name, translated_name, source_path, dest_path,
                            profile_id, rules_applied, timestamp, reversible, tier
                     FROM compat_mapping_records
                     WHERE original_name LIKE ?1 OR translated_name LIKE ?1
                     ORDER BY timestamp DESC LIMIT ?2",
                )?;
                let rows = stmt.query_map(rusqlite::params![query, limit], |row| {
                    row_to_record(row)
                })?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(row?);
                }
                Ok(records)
            })
            .await
    }

    /// List all mappings for a given destination path.
    pub async fn list_for_dest(&self, dest_path: &str, limit: u32) -> Result<Vec<MappingRecord>, AppError> {
        let dest_path = dest_path.to_string();
        self.pool
            .execute(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, original_name, translated_name, source_path, dest_path,
                            profile_id, rules_applied, timestamp, reversible, tier
                     FROM compat_mapping_records
                     WHERE dest_path LIKE ?1
                     ORDER BY timestamp DESC LIMIT ?2",
                )?;
                let pattern = format!("{dest_path}%");
                let rows = stmt.query_map(rusqlite::params![pattern, limit], |row| {
                    row_to_record(row)
                })?;
                let mut records = Vec::new();
                for row in rows {
                    records.push(row?);
                }
                Ok(records)
            })
            .await
    }

    /// Delete mappings older than the specified retention period.
    pub async fn cleanup(&self, config: &RetentionConfig) -> Result<u64, AppError> {
        let retention_days = config.retention_days;
        let max_records = config.max_records;
        self.pool
            .execute(move |conn| {
                let mut total_deleted = 0u64;

                // Delete by age
                if retention_days > 0 {
                    let cutoff = Utc::now() - chrono::Duration::days(retention_days as i64);
                    let deleted = conn.execute(
                        "DELETE FROM compat_mapping_records WHERE timestamp < ?1",
                        rusqlite::params![cutoff.to_rfc3339()],
                    )?;
                    total_deleted += deleted as u64;
                }

                // Delete excess records (keep newest)
                if max_records > 0 {
                    let deleted = conn.execute(
                        "DELETE FROM compat_mapping_records WHERE id NOT IN
                         (SELECT id FROM compat_mapping_records ORDER BY timestamp DESC LIMIT ?1)",
                        rusqlite::params![max_records],
                    )?;
                    total_deleted += deleted as u64;
                }

                Ok(total_deleted)
            })
            .await
    }

    /// Count total mapping records.
    pub async fn count(&self) -> Result<u64, AppError> {
        self.pool
            .execute(|conn| {
                let count: u64 = conn.query_row(
                    "SELECT COUNT(*) FROM compat_mapping_records",
                    [],
                    |row| row.get(0),
                )?;
                Ok(count)
            })
            .await
    }
}

/// Create the mapping records table.
pub fn create_mapping_table(conn: &Connection) -> Result<(), AppError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS compat_mapping_records (
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
            ON compat_mapping_records(tier);",
    )?;
    Ok(())
}

/// Convert a rusqlite Row to a MappingRecord.
fn row_to_record(row: &rusqlite::Row) -> rusqlite::Result<MappingRecord> {
    let rules_json: String = row.get(6)?;
    let rules: Vec<String> =
        serde_json::from_str(&rules_json).unwrap_or_default();
    let timestamp_str: String = row.get(7)?;
    let timestamp = DateTime::parse_from_rfc3339(&timestamp_str)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    Ok(MappingRecord {
        id: row.get(0)?,
        original_name: row.get(1)?,
        translated_name: row.get(2)?,
        source_path: row.get(3)?,
        dest_path: row.get(4)?,
        profile_id: row.get(5)?,
        rules_applied: rules,
        timestamp,
        reversible: row.get::<_, bool>(8)?,
        tier: row.get::<_, u8>(9)?,
    })
}

// ── Sidecar manifest ──

/// Generate a sidecar manifest from a list of mapping records.
pub fn generate_sidecar(records: &[MappingRecord], profile_id: &str) -> SidecarManifest {
    let now = Utc::now().to_rfc3339();
    SidecarManifest {
        version: 1,
        created_at: now.clone(),
        updated_at: now,
        profile_id: profile_id.to_string(),
        mappings: records
            .iter()
            .map(|r| SidecarEntry {
                original_name: r.original_name.clone(),
                translated_name: r.translated_name.clone(),
                rules: r.rules_applied.clone(),
                reversible: r.reversible,
                tier: r.tier,
            })
            .collect(),
    }
}

/// Serialize a sidecar manifest to JSON.
pub fn sidecar_to_json(manifest: &SidecarManifest) -> Result<String, AppError> {
    serde_json::to_string_pretty(manifest).map_err(|e| {
        AppError::internal(format!("Failed to serialize sidecar manifest: {e}"))
    })
}

/// Parse a sidecar manifest from JSON.
pub fn sidecar_from_json(json: &str) -> Result<SidecarManifest, AppError> {
    serde_json::from_str(json).map_err(|e| {
        AppError::file_op(
            format!("Failed to parse sidecar manifest: {e}"),
            "The manifest file may be corrupted. Check the .ufop-manifest.json file.",
        )
    })
}

// ── xattr storage (macOS/Linux) ──

/// Extended attribute name for storing mapping data.
pub const XATTR_NAME: &str = "user.ufop.original_name";

/// Store the original filename as an xattr on the translated file.
/// Returns Ok(true) if stored, Ok(false) if xattrs not supported.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn store_xattr(file_path: &str, original_name: &str) -> Result<bool, AppError> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    {
        let result = Command::new("xattr")
            .args(["-w", XATTR_NAME, original_name, file_path])
            .output();
        match result {
            Ok(output) if output.status.success() => Ok(true),
            _ => Ok(false),
        }
    }
    #[cfg(target_os = "linux")]
    {
        let result = Command::new("setfattr")
            .args(["-n", XATTR_NAME, "-v", original_name, file_path])
            .output();
        match result {
            Ok(output) if output.status.success() => Ok(true),
            _ => Ok(false),
        }
    }
}

/// Read the original filename from an xattr.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub fn read_xattr(file_path: &str) -> Result<Option<String>, AppError> {
    use std::process::Command;
    #[cfg(target_os = "macos")]
    {
        let result = Command::new("xattr")
            .args(["-p", XATTR_NAME, file_path])
            .output();
        match result {
            Ok(output) if output.status.success() => {
                let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if value.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(value))
                }
            }
            _ => Ok(None),
        }
    }
    #[cfg(target_os = "linux")]
    {
        let result = Command::new("getfattr")
            .args(["--only-values", "-n", XATTR_NAME, file_path])
            .output();
        match result {
            Ok(output) if output.status.success() => {
                let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if value.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(value))
                }
            }
            _ => Ok(None),
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn store_xattr(_file_path: &str, _original_name: &str) -> Result<bool, AppError> {
    Ok(false)
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub fn read_xattr(_file_path: &str) -> Result<Option<String>, AppError> {
    Ok(None)
}

/// Create a MappingRecord from a compatibility check result.
pub fn create_record(
    original_name: &str,
    translated_name: &str,
    source_path: &str,
    dest_path: &str,
    profile_id: &str,
    rules: &[String],
    tier: u8,
) -> MappingRecord {
    MappingRecord {
        id: Uuid::new_v4().to_string(),
        original_name: original_name.to_string(),
        translated_name: translated_name.to_string(),
        source_path: source_path.to_string(),
        dest_path: dest_path.to_string(),
        profile_id: profile_id.to_string(),
        rules_applied: rules.to_vec(),
        timestamp: Utc::now(),
        reversible: true,
        tier,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_test_record(original: &str, translated: &str, tier: u8) -> MappingRecord {
        create_record(
            original,
            translated,
            "/source",
            "/dest",
            "windows-ntfs",
            &["test rule".to_string()],
            tier,
        )
    }

    // ── Database operations ──

    #[tokio::test]
    async fn test_store_and_find() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.execute(|conn| { create_mapping_table(conn)?; Ok(()) }).await.unwrap();

        let db = MappingDb::new(pool);
        let record = make_test_record("file<name>.txt", "file\u{FF1C}name\u{FF1E}.txt", 1);
        db.store(&record).await.unwrap();

        let found = db.find_original("file\u{FF1C}name\u{FF1E}.txt", "/dest").await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.original_name, "file<name>.txt");
    }

    #[tokio::test]
    async fn test_store_batch() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.execute(|conn| { create_mapping_table(conn)?; Ok(()) }).await.unwrap();

        let db = MappingDb::new(pool);
        let records: Vec<MappingRecord> = (0..100)
            .map(|i| make_test_record(&format!("file{i}.txt"), &format!("file{i}_safe.txt"), 1))
            .collect();
        db.store_batch(&records).await.unwrap();

        let count = db.count().await.unwrap();
        assert_eq!(count, 100);
    }

    #[tokio::test]
    async fn test_search_by_original() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.execute(|conn| { create_mapping_table(conn)?; Ok(()) }).await.unwrap();

        let db = MappingDb::new(pool);
        db.store(&make_test_record("Report.txt", "_Report.txt", 1)).await.unwrap();
        db.store(&make_test_record("notes.txt", "notes.txt", 1)).await.unwrap();

        let results = db.search("Report", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_name, "Report.txt");
    }

    #[tokio::test]
    async fn test_search_by_translated() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.execute(|conn| { create_mapping_table(conn)?; Ok(()) }).await.unwrap();

        let db = MappingDb::new(pool);
        db.store(&make_test_record("CON.txt", "_CON.txt", 2)).await.unwrap();

        let results = db.search("_CON", 10).await.unwrap();
        assert_eq!(results.len(), 1);
    }

    #[tokio::test]
    async fn test_list_for_dest() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.execute(|conn| { create_mapping_table(conn)?; Ok(()) }).await.unwrap();

        let db = MappingDb::new(pool);
        let mut r1 = make_test_record("a.txt", "a_safe.txt", 1);
        r1.dest_path = "/dest/dir1".to_string();
        let mut r2 = make_test_record("b.txt", "b_safe.txt", 1);
        r2.dest_path = "/dest/dir2".to_string();
        db.store(&r1).await.unwrap();
        db.store(&r2).await.unwrap();

        let results = db.list_for_dest("/dest/dir1", 10).await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].original_name, "a.txt");
    }

    #[tokio::test]
    async fn test_cleanup_by_retention() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.execute(|conn| { create_mapping_table(conn)?; Ok(()) }).await.unwrap();

        let db = MappingDb::new(pool);
        // Store an old record
        let mut old_record = make_test_record("old.txt", "old_safe.txt", 1);
        old_record.timestamp = Utc::now() - chrono::Duration::days(400);
        db.store(&old_record).await.unwrap();
        // Store a new record
        db.store(&make_test_record("new.txt", "new_safe.txt", 1)).await.unwrap();

        let config = RetentionConfig { retention_days: 365, max_records: 0 };
        let deleted = db.cleanup(&config).await.unwrap();
        assert_eq!(deleted, 1);

        let count = db.count().await.unwrap();
        assert_eq!(count, 1);
    }

    // ── Sidecar manifest ──

    #[test]
    fn test_generate_sidecar() {
        let records = vec![
            make_test_record("file<name>.txt", "file\u{FF1C}name\u{FF1E}.txt", 1),
            make_test_record("CON.txt", "_CON.txt", 2),
        ];
        let manifest = generate_sidecar(&records, "windows-ntfs");
        assert_eq!(manifest.version, 1);
        assert_eq!(manifest.mappings.len(), 2);
        assert_eq!(manifest.profile_id, "windows-ntfs");
    }

    #[test]
    fn test_sidecar_roundtrip() {
        let records = vec![
            make_test_record("test.txt", "test_safe.txt", 1),
        ];
        let manifest = generate_sidecar(&records, "ext4");
        let json = sidecar_to_json(&manifest).unwrap();
        let parsed = sidecar_from_json(&json).unwrap();
        assert_eq!(parsed.version, manifest.version);
        assert_eq!(parsed.mappings.len(), manifest.mappings.len());
        assert_eq!(parsed.mappings[0].original_name, "test.txt");
    }

    // ── Restore workflow ──

    #[tokio::test]
    async fn test_restore_end_to_end() {
        let pool = DbPool::open_in_memory().unwrap();
        pool.execute(|conn| { create_mapping_table(conn)?; Ok(()) }).await.unwrap();

        let db = MappingDb::new(pool);

        // Simulate a transfer that translated names
        let original = "Report<Final>:v2.doc";
        let translated = "Report\u{FF1C}Final\u{FF1E}\u{FF1A}v2.doc";
        let record = create_record(original, translated, "/mac/docs", "/win/share", "windows-ntfs",
            &["colon to fullwidth".to_string()], 1);
        db.store(&record).await.unwrap();

        // Later, find the original name to restore
        let found = db.find_original(translated, "/win/share").await.unwrap();
        assert!(found.is_some());
        let found = found.unwrap();
        assert_eq!(found.original_name, original);
        assert!(found.reversible);
    }
}
