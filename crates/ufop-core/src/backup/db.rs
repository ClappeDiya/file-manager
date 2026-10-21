//! Per-job local database: a SQLite cache of the remote repository state
//! (volumes, blocks, versions and their file lists). It can always be rebuilt
//! from the remote files (`maintenance::recreate_database`).

use super::volume::VolumeKind;
use super::{EntryKind, FileEntry, Fileset};
use crate::error::AppError;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS remote_volumes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    size INTEGER NOT NULL,
    hash TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at TEXT NOT NULL,
    lock_until TEXT
);
CREATE TABLE IF NOT EXISTS blocks (
    hash TEXT PRIMARY KEY,
    size INTEGER NOT NULL,
    volume_id INTEGER NOT NULL REFERENCES remote_volumes(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_blocks_volume ON blocks(volume_id);
CREATE TABLE IF NOT EXISTS filesets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    timestamp TEXT NOT NULL UNIQUE,
    volume_name TEXT NOT NULL,
    partial INTEGER NOT NULL DEFAULT 0,
    file_count INTEGER NOT NULL DEFAULT 0,
    total_size INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    fileset_id INTEGER NOT NULL REFERENCES filesets(id) ON DELETE CASCADE,
    path TEXT NOT NULL,
    kind TEXT NOT NULL,
    size INTEGER NOT NULL,
    mtime INTEGER,
    mode INTEGER,
    hash TEXT,
    blocks TEXT NOT NULL,
    symlink_target TEXT
);
CREATE INDEX IF NOT EXISTS idx_files_fileset ON files(fileset_id);
CREATE INDEX IF NOT EXISTS idx_files_path ON files(path);
CREATE TABLE IF NOT EXISTS kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
"#;

/// Current job-database schema version (`PRAGMA user_version`).
pub const SCHEMA_VERSION: i32 = 2;

/// Summary of a job database file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DbInfo {
    pub path: String,
    pub schema_version: i32,
    pub latest_schema: i32,
    pub versions: u64,
    pub volumes: u64,
    pub blocks: u64,
    pub files: u64,
    pub size_bytes: u64,
    pub integrity_ok: bool,
    pub integrity: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub from: i32,
    pub to: i32,
    pub backup_path: Option<String>,
}

/// A remote volume row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VolumeRow {
    pub id: i64,
    pub name: String,
    pub kind: VolumeKind,
    pub size: u64,
    pub hash: String,
    pub state: String,
    pub lock_until: Option<String>,
}

pub struct BackupDb {
    conn: Connection,
}

fn kind_str(k: EntryKind) -> &'static str {
    match k {
        EntryKind::File => "file",
        EntryKind::Dir => "dir",
        EntryKind::Symlink => "symlink",
    }
}

fn meta_json(f: &FileEntry) -> Result<Option<String>, AppError> {
    Ok(match &f.meta {
        Some(m) if !m.is_empty() => Some(serde_json::to_string(m)?),
        _ => None,
    })
}

fn kind_parse(s: &str) -> EntryKind {
    match s {
        "dir" => EntryKind::Dir,
        "symlink" => EntryKind::Symlink,
        _ => EntryKind::File,
    }
}

impl BackupDb {
    pub fn open(path: &Path) -> Result<Self, AppError> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        Self::init(Connection::open(path)?)
    }

    pub fn open_in_memory() -> Result<Self, AppError> {
        Self::init(Connection::open_in_memory()?)
    }

    fn init(conn: Connection) -> Result<Self, AppError> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Self::upgrade(&conn)?;
        Ok(Self { conn })
    }

    /// Schema version stored in the file (0 = empty file). Databases from
    /// before versioning have tables but `user_version` 0: they count as 1.
    fn stored_version(conn: &Connection) -> Result<i32, AppError> {
        let v: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if v > 0 {
            return Ok(v);
        }
        let has_tables: bool = conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'filesets'")?.exists([])?;
        if !has_tables {
            return Ok(0);
        }
        let has_meta: bool = conn.prepare("SELECT 1 FROM pragma_table_info('files') WHERE name = 'meta'")?.exists([])?;
        Ok(if has_meta { 2 } else { 1 })
    }

    /// Apply every migration after the stored version, in one transaction.
    fn upgrade(conn: &Connection) -> Result<(i32, i32), AppError> {
        let from = Self::stored_version(conn)?;
        if from > SCHEMA_VERSION {
            return Err(AppError::validation(format!(
                "This backup database uses schema {from}, newer than this version supports ({SCHEMA_VERSION}); update the app"
            )));
        }
        if from == SCHEMA_VERSION {
            return Ok((from, from));
        }
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let r = (|| -> Result<(), AppError> {
            if from < 1 {
                conn.execute_batch(SCHEMA)?;
            }
            if from < 2 {
                // v2: extended metadata (xattrs, ACLs, Windows attributes).
                let has_meta: bool = conn.prepare("SELECT 1 FROM pragma_table_info('files') WHERE name = 'meta'")?.exists([])?;
                if !has_meta {
                    conn.execute_batch("ALTER TABLE files ADD COLUMN meta TEXT")?;
                }
            }
            conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
            Ok(())
        })();
        match r {
            Ok(()) => conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        Ok((from, SCHEMA_VERSION))
    }

    /// Inspect a database file without changing it.
    pub fn inspect(path: &Path) -> Result<DbInfo, AppError> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let schema = Self::stored_version(&conn)?;
        let count = |sql: &str| -> u64 { conn.query_row(sql, [], |r| r.get::<_, i64>(0)).map(|n| n as u64).unwrap_or(0) };
        let size = ["", "-wal", "-shm"].iter().filter_map(|x| std::fs::metadata(format!("{}{x}", path.display())).ok()).map(|m| m.len()).sum();
        let integrity: String = conn.query_row("PRAGMA quick_check", [], |r| r.get(0)).unwrap_or_else(|e| e.to_string());
        Ok(DbInfo {
            path: path.to_string_lossy().into_owned(),
            schema_version: schema,
            latest_schema: SCHEMA_VERSION,
            versions: count("SELECT COUNT(*) FROM filesets"),
            volumes: count("SELECT COUNT(*) FROM remote_volumes"),
            blocks: count("SELECT COUNT(*) FROM blocks"),
            files: count("SELECT COUNT(*) FROM files"),
            size_bytes: size,
            integrity_ok: integrity == "ok",
            integrity,
        })
    }

    /// Standalone migration: upgrade a database file in place, copying it
    /// to `<file>.v<from>.bak` first when `backup` is set.
    pub fn migrate(path: &Path, backup: bool) -> Result<MigrationReport, AppError> {
        if !path.is_file() {
            return Err(AppError::file_op(format!("{} does not exist", path.display()), "Check the database path."));
        }
        let before = Self::inspect(path)?;
        let mut backup_path = None;
        if backup && before.schema_version < SCHEMA_VERSION {
            let conn = Connection::open(path)?;
            let b = PathBuf::from(format!("{}.v{}.bak", path.display(), before.schema_version));
            let _ = std::fs::remove_file(&b);
            // VACUUM INTO writes a consistent copy even with a WAL present.
            conn.execute("VACUUM INTO ?1", [b.to_string_lossy()])?;
            backup_path = Some(b.to_string_lossy().into_owned());
        }
        let conn = Connection::open(path)?;
        let (from, to) = Self::upgrade(&conn)?;
        Ok(MigrationReport { from, to, backup_path })
    }

    pub fn begin(&self) -> Result<(), AppError> {
        Ok(self.conn.execute_batch("BEGIN")?)
    }
    pub fn commit(&self) -> Result<(), AppError> {
        Ok(self.conn.execute_batch("COMMIT")?)
    }

    pub fn get_kv(&self, key: &str) -> Result<Option<String>, AppError> {
        Ok(self.conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()?)
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<(), AppError> {
        self.conn.execute("INSERT OR REPLACE INTO kv(key, value) VALUES (?1, ?2)", params![key, value])?;
        Ok(())
    }

    pub fn add_volume(&self, name: &str, kind: VolumeKind, size: u64, hash: &str, state: &str) -> Result<i64, AppError> {
        self.conn.execute(
            "INSERT INTO remote_volumes(name, kind, size, hash, state, created_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(name) DO UPDATE SET size = excluded.size, hash = excluded.hash, state = excluded.state",
            params![name, kind.as_str(), size as i64, hash, state, chrono::Utc::now().to_rfc3339()],
        )?;
        Ok(self.conn.query_row("SELECT id FROM remote_volumes WHERE name = ?1", [name], |r| r.get(0))?)
    }

    pub fn set_volume_state(&self, name: &str, state: &str) -> Result<(), AppError> {
        self.conn.execute("UPDATE remote_volumes SET state = ?2 WHERE name = ?1", params![name, state])?;
        Ok(())
    }

    pub fn set_volume_lock(&self, name: &str, until: Option<&str>) -> Result<(), AppError> {
        self.conn.execute("UPDATE remote_volumes SET lock_until = ?2 WHERE name = ?1", params![name, until])?;
        Ok(())
    }

    pub fn delete_volume(&self, name: &str) -> Result<(), AppError> {
        self.conn.execute("DELETE FROM remote_volumes WHERE name = ?1", [name])?;
        Ok(())
    }

    pub fn volumes(&self, kind: Option<VolumeKind>) -> Result<Vec<VolumeRow>, AppError> {
        let mut st = self.conn.prepare(
            "SELECT id, name, kind, size, hash, state, lock_until FROM remote_volumes
             WHERE (?1 IS NULL OR kind = ?1) ORDER BY id",
        )?;
        let rows = st
            .query_map([kind.map(|k| k.as_str())], |r| {
                let k: String = r.get(2)?;
                Ok(VolumeRow {
                    id: r.get(0)?,
                    name: r.get(1)?,
                    kind: VolumeKind::parse(&k).unwrap_or(VolumeKind::Block),
                    size: r.get::<_, i64>(3)? as u64,
                    hash: r.get(4)?,
                    state: r.get(5)?,
                    lock_until: r.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    pub fn volume_by_name(&self, name: &str) -> Result<Option<VolumeRow>, AppError> {
        Ok(self.volumes(None)?.into_iter().find(|v| v.name == name))
    }

    pub fn has_block(&self, hash: &str) -> Result<bool, AppError> {
        Ok(self.conn.query_row("SELECT 1 FROM blocks WHERE hash = ?1", [hash], |_| Ok(())).optional()?.is_some())
    }

    pub fn add_block(&self, hash: &str, size: u64, volume_id: i64) -> Result<(), AppError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO blocks(hash, size, volume_id) VALUES (?1, ?2, ?3)",
            params![hash, size as i64, volume_id],
        )?;
        Ok(())
    }

    /// Map block hash → dblock volume name.
    pub fn block_locations(&self) -> Result<HashMap<String, String>, AppError> {
        let mut st = self.conn.prepare("SELECT b.hash, v.name FROM blocks b JOIN remote_volumes v ON v.id = b.volume_id")?;
        let m = st.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<HashMap<_, _>, _>>()?;
        Ok(m)
    }

    /// Blocks per volume: volume name → [(hash, size)].
    pub fn blocks_by_volume(&self) -> Result<HashMap<String, Vec<(String, u64)>>, AppError> {
        let mut st = self.conn.prepare("SELECT v.name, b.hash, b.size FROM blocks b JOIN remote_volumes v ON v.id = b.volume_id")?;
        let mut m: HashMap<String, Vec<(String, u64)>> = HashMap::new();
        for row in st.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))? {
            let (v, h, s) = row?;
            m.entry(v).or_default().push((h, s as u64));
        }
        Ok(m)
    }

    pub fn add_fileset(&self, timestamp: &str, volume_name: &str, partial: bool, files: &[FileEntry]) -> Result<i64, AppError> {
        let total: u64 = files.iter().map(|f| f.size).sum();
        self.conn.execute(
            "INSERT INTO filesets(timestamp, volume_name, partial, file_count, total_size) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![timestamp, volume_name, partial as i64, files.len() as i64, total as i64],
        )?;
        let id = self.conn.last_insert_rowid();
        let mut st = self.conn.prepare(
            "INSERT INTO files(fileset_id, path, kind, size, mtime, mode, hash, blocks, symlink_target, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for f in files {
            st.execute(params![
                id,
                f.path,
                kind_str(f.kind),
                f.size as i64,
                f.mtime,
                f.mode.map(|m| m as i64),
                f.hash,
                serde_json::to_string(&f.blocks)?,
                f.symlink_target,
                meta_json(f)?
            ])?;
        }
        Ok(id)
    }

    pub fn filesets(&self) -> Result<Vec<Fileset>, AppError> {
        let mut st = self.conn.prepare(
            "SELECT id, timestamp, volume_name, partial, file_count, total_size FROM filesets ORDER BY timestamp DESC",
        )?;
        let v = st
            .query_map([], |r| {
                Ok(Fileset {
                    id: r.get(0)?,
                    timestamp: r.get(1)?,
                    volume_name: r.get(2)?,
                    partial: r.get::<_, i64>(3)? != 0,
                    file_count: r.get::<_, i64>(4)? as u64,
                    total_size: r.get::<_, i64>(5)? as u64,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(v)
    }

    /// Resolve a version: index 0 = newest.
    pub fn fileset_by_version(&self, version: usize) -> Result<Fileset, AppError> {
        self.filesets()?.into_iter().nth(version).ok_or_else(|| {
            AppError::validation(format!("Backup version {version} does not exist"))
        })
    }

    pub fn delete_fileset(&self, id: i64) -> Result<(), AppError> {
        self.conn.execute("DELETE FROM filesets WHERE id = ?1", [id])?;
        Ok(())
    }

    /// Replace a fileset's file list (used by purge).
    pub fn replace_fileset_files(&self, id: i64, volume_name: &str, files: &[FileEntry]) -> Result<(), AppError> {
        self.conn.execute("DELETE FROM files WHERE fileset_id = ?1", [id])?;
        let total: u64 = files.iter().map(|f| f.size).sum();
        self.conn.execute(
            "UPDATE filesets SET volume_name = ?2, file_count = ?3, total_size = ?4 WHERE id = ?1",
            params![id, volume_name, files.len() as i64, total as i64],
        )?;
        let mut st = self.conn.prepare(
            "INSERT INTO files(fileset_id, path, kind, size, mtime, mode, hash, blocks, symlink_target, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for f in files {
            st.execute(params![
                id, f.path, kind_str(f.kind), f.size as i64, f.mtime, f.mode.map(|m| m as i64), f.hash,
                serde_json::to_string(&f.blocks)?, f.symlink_target, meta_json(f)?
            ])?;
        }
        Ok(())
    }

    pub fn files(&self, fileset_id: i64) -> Result<Vec<FileEntry>, AppError> {
        let mut st = self.conn.prepare(
            "SELECT path, kind, size, mtime, mode, hash, blocks, symlink_target, meta FROM files WHERE fileset_id = ?1 ORDER BY path",
        )?;
        let rows = st
            .query_map([fileset_id], |r| {
                let blocks: String = r.get(6)?;
                Ok(FileEntry {
                    path: r.get(0)?,
                    kind: kind_parse(&r.get::<_, String>(1)?),
                    size: r.get::<_, i64>(2)? as u64,
                    mtime: r.get(3)?,
                    mode: r.get::<_, Option<i64>>(4)?.map(|m| m as u32),
                    hash: r.get(5)?,
                    blocks: serde_json::from_str(&blocks).unwrap_or_default(),
                    symlink_target: r.get(7)?,
                    meta: r.get::<_, Option<String>>(8)?.and_then(|m| serde_json::from_str(&m).ok()),
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Every block hash referenced by any version.
    pub fn referenced_blocks(&self) -> Result<HashSet<String>, AppError> {
        let mut st = self.conn.prepare("SELECT blocks FROM files WHERE kind = 'file'")?;
        let mut set = HashSet::new();
        for b in st.query_map([], |r| r.get::<_, String>(0))? {
            let v: Vec<String> = serde_json::from_str(&b?).unwrap_or_default();
            set.extend(v);
        }
        Ok(set)
    }

    pub fn remove_blocks(&self, hashes: &[String]) -> Result<(), AppError> {
        let mut st = self.conn.prepare("DELETE FROM blocks WHERE hash = ?1")?;
        for h in hashes {
            st.execute([h])?;
        }
        Ok(())
    }

    /// Wipe everything (before a recreate).
    pub fn clear(&self) -> Result<(), AppError> {
        self.conn.execute_batch("DELETE FROM files; DELETE FROM filesets; DELETE FROM blocks; DELETE FROM remote_volumes;")?;
        Ok(())
    }

    /// SQLite VACUUM.
    pub fn vacuum(&self) -> Result<(), AppError> {
        Ok(self.conn.execute_batch("VACUUM")?)
    }

    /// Foreign-key and orphan checks; returns human-readable problems.
    pub fn integrity_problems(&self) -> Result<Vec<String>, AppError> {
        let mut out = Vec::new();
        let ok: String = self.conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if ok != "ok" {
            out.push(format!("SQLite integrity check: {ok}"));
        }
        let loc = self.block_locations()?;
        let missing = self.referenced_blocks()?.into_iter().filter(|h| !loc.contains_key(h)).count();
        if missing > 0 {
            out.push(format!("{missing} referenced block(s) have no known volume"));
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrates_legacy_database_and_refuses_newer() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("job.sqlite");
        {
            // A database from before schema versioning: v1 tables, no meta column.
            let c = Connection::open(&p).unwrap();
            c.execute_batch(SCHEMA).unwrap();
            c.execute_batch("ALTER TABLE files DROP COLUMN meta").unwrap_or(());
            c.execute("INSERT INTO kv(key, value) VALUES ('x', 'y')", []).unwrap();
        }
        let info = BackupDb::inspect(&p).unwrap();
        assert_eq!((info.schema_version, info.latest_schema), (1, SCHEMA_VERSION));
        assert!(info.integrity_ok);
        let r = BackupDb::migrate(&p, true).unwrap();
        assert_eq!((r.from, r.to), (1, SCHEMA_VERSION));
        let bak = r.backup_path.unwrap();
        assert_eq!(BackupDb::inspect(Path::new(&bak)).unwrap().schema_version, 1, "backup keeps the old schema");
        let db = BackupDb::open(&p).unwrap();
        assert_eq!(db.get_kv("x").unwrap().as_deref(), Some("y"), "data survives");
        drop(db);
        assert_eq!(BackupDb::inspect(&p).unwrap().schema_version, SCHEMA_VERSION);
        assert_eq!(BackupDb::migrate(&p, true).unwrap().backup_path, None, "already current");

        Connection::open(&p).unwrap().execute_batch("PRAGMA user_version = 99").unwrap();
        assert!(BackupDb::open(&p).is_err());
        assert!(BackupDb::migrate(&d.path().join("missing.sqlite"), false).is_err());
    }
}
