//! Per-job local database: a SQLite cache of the remote repository state
//! (volumes, blocks, versions and their file lists). It can always be rebuilt
//! from the remote files (`maintenance::recreate_database`).
//!
//! **Encryption at rest.** With a [`DbKey`] the columns that describe your
//! files are sealed with AES-256-GCM before they reach the file: paths,
//! symbolic-link targets, extended metadata, file and block-list hashes and
//! every settings value (embedded job configuration, change-journal state).
//! Each value has its own random nonce and is bound to its column (and kv
//! key) as associated data, so values cannot be swapped between columns.
//! What stays readable is what the destination shows anyway (volume names,
//! sizes, version times) plus the block-hash index the deduplicator looks
//! up. A key-check value in the `kv` table (`db_encryption`) tells whether
//! the file is encrypted and whether a key is the right one. The file is
//! created with owner-only permissions (Unix) either way.

use super::volume::VolumeKind;
use super::versions::VersionMeta;
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

/// What [`BackupDb::export_sanitized`] changed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SanitizeReport {
    pub paths_obfuscated: u64,
    pub settings_removed: u64,
}

/// Replace each component of a path with a short salted hash, keeping the
/// separators and a short alphanumeric extension.
pub fn obfuscate_path(path: &str, salt: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut out = String::with_capacity(path.len());
    let mut part = String::new();
    let flush = |part: &mut String, out: &mut String| {
        if part.is_empty() {
            return;
        }
        let ext = part.rsplit_once('.').map(|(stem, e)| (stem, e)).filter(|(stem, e)| !stem.is_empty() && e.len() <= 8 && e.chars().all(|c| c.is_ascii_alphanumeric())).map(|(_, e)| e.to_string());
        let mut h = Sha256::new();
        h.update(salt);
        h.update(part.as_bytes());
        out.push_str(&hex::encode(&h.finalize()[..6]));
        if let Some(e) = ext {
            out.push('.');
            out.push_str(&e);
        }
        part.clear();
    };
    for ch in path.chars() {
        if ch == '/' || ch == '\\' {
            flush(&mut part, &mut out);
            out.push(ch);
        } else {
            part.push(ch);
        }
    }
    flush(&mut part, &mut out);
    out
}

/// Current job-database schema version (`PRAGMA user_version`).
pub const SCHEMA_VERSION: i32 = 3;

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
    /// Sensitive columns are encrypted (needs the job's key to open).
    #[serde(default)]
    pub encrypted: bool,
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
    /// Seals sensitive columns; `None` for a plain database.
    cols: Option<ColumnCipher>,
    /// Shared repositories: [`BackupDb::filesets`] lists only this source
    /// (`None` = every version).
    scope: std::cell::RefCell<Option<String>>,
}

/// Restores the previous source scope when dropped (see [`BackupDb::scoped`]).
pub struct ScopeGuard<'a> {
    db: &'a BackupDb,
    previous: Option<String>,
}

impl Drop for ScopeGuard<'_> {
    fn drop(&mut self) {
        *self.db.scope.borrow_mut() = self.previous.take();
    }
}

/// 256-bit key for an encrypted job database (kept by the caller, e.g. in
/// the OS keychain; this crate never stores it).
#[derive(Clone, PartialEq, Eq)]
pub struct DbKey([u8; 32]);

impl DbKey {
    pub fn generate() -> Self {
        Self(rand::random())
    }

    pub fn from_hex(s: &str) -> Result<Self, AppError> {
        let b = hex::decode(s.trim()).map_err(|_| AppError::validation("The database key is not valid hex"))?;
        let arr: [u8; 32] = b.try_into().map_err(|_| AppError::validation("The database key must be 32 bytes (64 hex digits)"))?;
        Ok(Self(arr))
    }

    pub fn to_hex(&self) -> String {
        hex::encode(self.0)
    }
}

impl std::fmt::Debug for DbKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DbKey(…)")
    }
}

/// How a database file relates to a key (see [`BackupDb::key_check`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyCheck {
    /// No database file yet.
    NoFile,
    /// Plain database: opens without a key.
    Plain,
    /// Encrypted and the key matches.
    Encrypted,
    /// Encrypted, but no key was given.
    KeyMissing,
    /// Encrypted with another key.
    WrongKey,
}

/// What [`BackupDb::rekey`] changed.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RekeyReport {
    pub encrypted: bool,
    pub files: u64,
    pub settings: u64,
}

const ENC_PREFIX: &str = "e1:";
const MARKER_KEY: &str = "db_encryption";
const CHECK_PLAIN: &str = "ufop-local-db-key-check";

#[derive(Serialize, Deserialize)]
struct Marker {
    version: u32,
    cipher: String,
    check: String,
}

/// AES-256-GCM for single column values: `e1:` + base64(nonce ‖ ciphertext).
struct ColumnCipher(aes_gcm::Aes256Gcm);

impl ColumnCipher {
    fn new(k: &DbKey) -> Self {
        use aes_gcm::KeyInit;
        Self(aes_gcm::Aes256Gcm::new(&k.0.into()))
    }

    fn seal(&self, aad: &str, plain: &str) -> String {
        use aes_gcm::aead::{Aead, Payload};
        use base64::Engine;
        let nonce: [u8; 12] = rand::random();
        // Encryption with a valid key and a fresh nonce cannot fail.
        let ct = self.0.encrypt(&nonce.into(), Payload { msg: plain.as_bytes(), aad: aad.as_bytes() }).unwrap_or_default();
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(&nonce);
        out.extend_from_slice(&ct);
        format!("{ENC_PREFIX}{}", base64::engine::general_purpose::STANDARD_NO_PAD.encode(out))
    }

    fn open(&self, aad: &str, sealed: &str) -> Result<String, AppError> {
        use aes_gcm::aead::{Aead, Payload};
        use base64::Engine;
        let bad = || AppError::encryption("A value in the local backup database cannot be decrypted", "Recreate the database from the destination.");
        let raw = base64::engine::general_purpose::STANDARD_NO_PAD.decode(sealed.strip_prefix(ENC_PREFIX).ok_or_else(bad)?).map_err(|_| bad())?;
        if raw.len() < 12 {
            return Err(bad());
        }
        let (n, ct) = raw.split_at(12);
        let nonce: [u8; 12] = n.try_into().map_err(|_| bad())?;
        let pt = self.0.decrypt(&nonce.into(), Payload { msg: ct, aad: aad.as_bytes() }).map_err(|_| bad())?;
        String::from_utf8(pt).map_err(|_| bad())
    }
}

fn key_missing() -> AppError {
    AppError::encryption(
        "The local backup database is encrypted and its key is not available",
        "The key is kept in this computer's keychain. Without it the database is rebuilt from the destination (Recreate database).",
    )
}

fn wrong_key() -> AppError {
    AppError::encryption(
        "The local backup database is encrypted with a different key",
        "The keychain entry does not match this database. Rebuild it from the destination (Recreate database).",
    )
}

/// Owner-only permissions for a database file (Unix).
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Err(e) = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)) {
            tracing::debug!("Cannot restrict permissions of {}: {e}", path.display());
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Raw `files` row before decryption.
struct RawFile {
    id: i64,
    path: String,
    kind: String,
    size: i64,
    mtime: Option<i64>,
    mode: Option<i64>,
    hash: Option<String>,
    blocks: String,
    target: Option<String>,
    meta: Option<String>,
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
    /// Open a plain database (an encrypted one needs [`BackupDb::open_with_key`]).
    pub fn open(path: &Path) -> Result<Self, AppError> {
        Self::open_with_key(path, None)
    }

    /// Open a database; `key` is required when the file is encrypted and
    /// ignored otherwise (use [`BackupDb::rekey`] to encrypt a plain one).
    pub fn open_with_key(path: &Path, key: Option<&DbKey>) -> Result<Self, AppError> {
        if let Some(p) = path.parent() {
            std::fs::create_dir_all(p)?;
        }
        let db = Self::init(Connection::open(path)?, key)?;
        restrict_permissions(path);
        Ok(db)
    }

    pub fn open_in_memory() -> Result<Self, AppError> {
        Self::init(Connection::open_in_memory()?, None)
    }

    fn init(conn: Connection, key: Option<&DbKey>) -> Result<Self, AppError> {
        conn.execute_batch("PRAGMA foreign_keys = ON; PRAGMA journal_mode = WAL;")?;
        Self::upgrade(&conn)?;
        let cols = match Self::marker(&conn)? {
            None => None,
            Some(m) => {
                let k = key.ok_or_else(key_missing)?;
                let c = ColumnCipher::new(k);
                if c.open(MARKER_KEY, &m.check).ok().as_deref() != Some(CHECK_PLAIN) {
                    return Err(wrong_key());
                }
                // Deleted rows are overwritten, not left readable in free pages.
                conn.execute_batch("PRAGMA secure_delete = ON;")?;
                Some(c)
            }
        };
        Ok(Self { conn, cols, scope: Default::default() })
    }

    fn marker(conn: &Connection) -> Result<Option<Marker>, AppError> {
        let v: Option<String> = conn.query_row("SELECT value FROM kv WHERE key = ?1", [MARKER_KEY], |r| r.get(0)).optional()?;
        Ok(v.and_then(|v| serde_json::from_str(&v).ok()))
    }

    /// Whether the file at `path` is encrypted and whether `key` opens it,
    /// without changing anything.
    pub fn key_check(path: &Path, key: Option<&DbKey>) -> Result<KeyCheck, AppError> {
        if !path.is_file() {
            return Ok(KeyCheck::NoFile);
        }
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let has_kv: bool = conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'kv'")?.exists([])?;
        let marker = if has_kv { Self::marker(&conn)? } else { None };
        Ok(match (marker, key) {
            (None, _) => KeyCheck::Plain,
            (Some(_), None) => KeyCheck::KeyMissing,
            (Some(m), Some(k)) if ColumnCipher::new(k).open(MARKER_KEY, &m.check).ok().as_deref() == Some(CHECK_PLAIN) => KeyCheck::Encrypted,
            (Some(_), Some(_)) => KeyCheck::WrongKey,
        })
    }

    /// Sensitive columns are sealed with a key.
    pub fn is_encrypted(&self) -> bool {
        self.cols.is_some()
    }

    /// No versions, volumes or settings yet.
    pub fn is_empty(&self) -> Result<bool, AppError> {
        let n: i64 = self.conn.query_row(
            "SELECT (SELECT COUNT(*) FROM filesets) + (SELECT COUNT(*) FROM remote_volumes) + (SELECT COUNT(*) FROM kv WHERE key <> ?1)",
            [MARKER_KEY],
            |r| r.get(0),
        )?;
        Ok(n == 0)
    }

    /// Encrypt (`Some`), re-key or decrypt (`None`) every sensitive value
    /// in one transaction, then rewrite the file so no plaintext stays in
    /// free pages or the write-ahead log.
    pub fn rekey(&mut self, key: Option<&DbKey>) -> Result<RekeyReport, AppError> {
        let new = key.map(ColumnCipher::new);
        let mut rep = RekeyReport { encrypted: new.is_some(), ..Default::default() };
        let seal = |aad: &str, v: String| -> String { match &new { Some(c) => c.seal(aad, &v), None => v } };
        let files = self.raw_files("SELECT id, path, kind, size, mtime, mode, hash, blocks, symlink_target, meta FROM files", [])?;
        let kv: Vec<(String, String)> = self
            .conn
            .prepare("SELECT key, value FROM kv WHERE key <> ?1")?
            .query_map([MARKER_KEY], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        self.conn.execute_batch("BEGIN IMMEDIATE")?;
        let r = (|| -> Result<(), AppError> {
            let mut up = self.conn.prepare("UPDATE files SET path = ?2, hash = ?3, blocks = ?4, symlink_target = ?5, meta = ?6 WHERE id = ?1")?;
            for f in files {
                let id = f.id;
                let e = self.decode(f)?;
                up.execute(params![
                    id,
                    seal("path", e.path),
                    e.hash.map(|h| seal("hash", h)),
                    seal("blocks", serde_json::to_string(&e.blocks)?),
                    e.symlink_target.map(|t| seal("target", t)),
                    e.meta.filter(|m| !m.is_empty()).map(|m| serde_json::to_string(&m)).transpose()?.map(|m| seal("meta", m))
                ])?;
                rep.files += 1;
            }
            let metas: Vec<(i64, String)> = self.conn.prepare("SELECT id, meta FROM filesets WHERE meta IS NOT NULL")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            for (id, m) in metas {
                let plain = self.dec("fileset_meta", m)?;
                self.conn.execute("UPDATE filesets SET meta = ?2 WHERE id = ?1", params![id, seal("fileset_meta", plain)])?;
            }
            let mut kvu = self.conn.prepare("UPDATE kv SET value = ?2 WHERE key = ?1")?;
            for (k, v) in kv {
                let plain = self.dec(&format!("kv:{k}"), v)?;
                kvu.execute(params![k, seal(&format!("kv:{k}"), plain)])?;
                rep.settings += 1;
            }
            match &new {
                Some(c) => {
                    let m = Marker { version: 1, cipher: "aes-256-gcm".into(), check: c.seal(MARKER_KEY, CHECK_PLAIN) };
                    self.conn.execute("INSERT OR REPLACE INTO kv(key, value) VALUES (?1, ?2)", params![MARKER_KEY, serde_json::to_string(&m)?])?;
                }
                None => {
                    self.conn.execute("DELETE FROM kv WHERE key = ?1", [MARKER_KEY])?;
                }
            }
            Ok(())
        })();
        match r {
            Ok(()) => self.conn.execute_batch("COMMIT")?,
            Err(e) => {
                let _ = self.conn.execute_batch("ROLLBACK");
                return Err(e);
            }
        }
        self.cols = new;
        self.conn.execute_batch(if self.cols.is_some() { "PRAGMA secure_delete = ON;" } else { "PRAGMA secure_delete = OFF;" })?;
        // Rewrite the file and empty the WAL: old plaintext pages are gone.
        self.conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE); VACUUM; PRAGMA wal_checkpoint(TRUNCATE);")?;
        Ok(rep)
    }

    fn enc(&self, aad: &str, v: &str) -> String {
        match &self.cols {
            Some(c) => c.seal(aad, v),
            None => v.to_string(),
        }
    }

    fn dec(&self, aad: &str, v: String) -> Result<String, AppError> {
        match &self.cols {
            Some(c) if v.starts_with(ENC_PREFIX) => c.open(aad, &v),
            _ => Ok(v),
        }
    }

    fn raw_files<P: rusqlite::Params>(&self, sql: &str, p: P) -> Result<Vec<RawFile>, AppError> {
        let mut st = self.conn.prepare(sql)?;
        let rows = st
            .query_map(p, |r| {
                Ok(RawFile {
                    id: r.get(0)?,
                    path: r.get(1)?,
                    kind: r.get(2)?,
                    size: r.get(3)?,
                    mtime: r.get(4)?,
                    mode: r.get(5)?,
                    hash: r.get(6)?,
                    blocks: r.get(7)?,
                    target: r.get(8)?,
                    meta: r.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    fn decode(&self, f: RawFile) -> Result<FileEntry, AppError> {
        let blocks = self.dec("blocks", f.blocks)?;
        let meta = f.meta.map(|m| self.dec("meta", m)).transpose()?;
        Ok(FileEntry {
            path: self.dec("path", f.path)?,
            kind: kind_parse(&f.kind),
            size: f.size as u64,
            mtime: f.mtime,
            mode: f.mode.map(|m| m as u32),
            hash: f.hash.map(|h| self.dec("hash", h)).transpose()?,
            blocks: serde_json::from_str(&blocks).unwrap_or_default(),
            symlink_target: f.target.map(|t| self.dec("target", t)).transpose()?,
            meta: meta.and_then(|m| serde_json::from_str(&m).ok()),
        })
    }

    fn insert_files(&self, id: i64, files: &[FileEntry]) -> Result<(), AppError> {
        let mut st = self.conn.prepare(
            "INSERT INTO files(fileset_id, path, kind, size, mtime, mode, hash, blocks, symlink_target, meta)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
        )?;
        for f in files {
            st.execute(params![
                id,
                self.enc("path", &f.path),
                kind_str(f.kind),
                f.size as i64,
                f.mtime,
                f.mode.map(|m| m as i64),
                f.hash.as_deref().map(|h| self.enc("hash", h)),
                self.enc("blocks", &serde_json::to_string(&f.blocks)?),
                f.symlink_target.as_deref().map(|t| self.enc("target", t)),
                meta_json(f)?.map(|m| self.enc("meta", &m))
            ])?;
        }
        Ok(())
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
        let has_version_meta: bool = conn.prepare("SELECT 1 FROM pragma_table_info('filesets') WHERE name = 'meta'")?.exists([])?;
        Ok(if has_version_meta { 3 } else if has_meta { 2 } else { 1 })
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
            if from < 3 {
                // v3: per-version metadata (note, tags, pin, identity, stats)
                // and the source a version belongs to (shared repositories).
                let has: bool = conn.prepare("SELECT 1 FROM pragma_table_info('filesets') WHERE name = 'meta'")?.exists([])?;
                if !has {
                    conn.execute_batch(
                        "ALTER TABLE filesets ADD COLUMN source TEXT NOT NULL DEFAULT '';
                         ALTER TABLE filesets ADD COLUMN meta TEXT;
                         CREATE INDEX IF NOT EXISTS idx_filesets_source ON filesets(source);",
                    )?;
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
            encrypted: conn.prepare("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'kv'")?.exists([])? && Self::marker(&conn)?.is_some(),
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
        let v: Option<String> = self.conn.query_row("SELECT value FROM kv WHERE key = ?1", [key], |r| r.get(0)).optional()?;
        v.map(|v| self.dec(&format!("kv:{key}"), v)).transpose()
    }

    pub fn set_kv(&self, key: &str, value: &str) -> Result<(), AppError> {
        if key == MARKER_KEY {
            return Err(AppError::validation(format!("'{MARKER_KEY}' is reserved")));
        }
        self.conn.execute("INSERT OR REPLACE INTO kv(key, value) VALUES (?1, ?2)", params![key, self.enc(&format!("kv:{key}"), value)])?;
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
        self.add_fileset_with_meta(timestamp, volume_name, partial, files, &VersionMeta::default())
    }

    /// Record a version with its metadata (note, tags, pin, identity, stats).
    pub fn add_fileset_with_meta(&self, timestamp: &str, volume_name: &str, partial: bool, files: &[FileEntry], meta: &VersionMeta) -> Result<i64, AppError> {
        let total: u64 = files.iter().map(|f| f.size).sum();
        self.conn.execute(
            "INSERT INTO filesets(timestamp, volume_name, partial, file_count, total_size, source, meta) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![timestamp, volume_name, partial as i64, files.len() as i64, total as i64, meta.source_id(), self.meta_value(meta)?],
        )?;
        let id = self.conn.last_insert_rowid();
        self.insert_files(id, files)?;
        Ok(id)
    }

    fn meta_value(&self, meta: &VersionMeta) -> Result<Option<String>, AppError> {
        Ok(if meta.is_empty() { None } else { Some(self.enc("fileset_meta", &serde_json::to_string(meta)?)) })
    }

    /// Replace a version's metadata (and the source it belongs to).
    pub fn set_fileset_meta(&self, id: i64, meta: &VersionMeta) -> Result<(), AppError> {
        self.conn.execute("UPDATE filesets SET source = ?2, meta = ?3 WHERE id = ?1", params![id, meta.source_id(), self.meta_value(meta)?])?;
        Ok(())
    }

    fn query_filesets(&self, filter: &str, p: &[&dyn rusqlite::ToSql]) -> Result<Vec<Fileset>, AppError> {
        let mut st = self.conn.prepare(&format!(
            "SELECT id, timestamp, volume_name, partial, file_count, total_size, source, meta FROM filesets {filter} ORDER BY timestamp DESC"
        ))?;
        let rows: Vec<(Fileset, Option<String>)> = st
            .query_map(p, |r| {
                Ok((
                    Fileset {
                        id: r.get(0)?,
                        timestamp: r.get(1)?,
                        volume_name: r.get(2)?,
                        partial: r.get::<_, i64>(3)? != 0,
                        file_count: r.get::<_, i64>(4)? as u64,
                        total_size: r.get::<_, i64>(5)? as u64,
                        source: r.get(6)?,
                        ..Default::default()
                    },
                    r.get(7)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        rows.into_iter()
            .map(|(mut f, m)| {
                if let Some(m) = m {
                    let meta: VersionMeta = serde_json::from_str(&self.dec("fileset_meta", m)?).unwrap_or_default();
                    let source = std::mem::take(&mut f.source);
                    f.set_meta(meta);
                    // The column is authoritative (identities may be absent).
                    f.source = source;
                }
                Ok(f)
            })
            .collect()
    }

    /// Versions of the current source (every version when no scope is
    /// set), newest first.
    pub fn filesets(&self) -> Result<Vec<Fileset>, AppError> {
        match self.scope() {
            Some(s) => self.query_filesets("WHERE source = ?1", &[&s]),
            None => self.all_filesets(),
        }
    }

    /// Every version of every source, newest first.
    pub fn all_filesets(&self) -> Result<Vec<Fileset>, AppError> {
        self.query_filesets("", &[])
    }

    /// Versions of one source, newest first.
    pub fn filesets_of(&self, source: &str) -> Result<Vec<Fileset>, AppError> {
        self.query_filesets("WHERE source = ?1", &[&source])
    }

    pub fn fileset_by_id(&self, id: i64) -> Result<Fileset, AppError> {
        self.query_filesets("WHERE id = ?1", &[&id])?.into_iter().next().ok_or_else(|| AppError::validation(format!("Backup version #{id} does not exist")))
    }

    /// Limit [`filesets`](Self::filesets) (and so version numbers, restore,
    /// retention, pins and comparisons) to one source; `None` = all.
    pub fn set_scope(&self, source: Option<String>) {
        *self.scope.borrow_mut() = source;
    }

    pub fn scope(&self) -> Option<String> {
        self.scope.borrow().clone()
    }

    /// Change the scope until the guard is dropped.
    pub fn scoped(&self, source: Option<String>) -> ScopeGuard<'_> {
        let previous = self.scope.replace(source);
        ScopeGuard { db: self, previous }
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
        self.insert_files(id, files)
    }

    pub fn files(&self, fileset_id: i64) -> Result<Vec<FileEntry>, AppError> {
        let raw = self.raw_files(
            "SELECT id, path, kind, size, mtime, mode, hash, blocks, symlink_target, meta FROM files WHERE fileset_id = ?1 ORDER BY path",
            [fileset_id],
        )?;
        let mut out = raw.into_iter().map(|f| self.decode(f)).collect::<Result<Vec<_>, _>>()?;
        if self.cols.is_some() {
            // Ciphertext order is random: sort the decrypted paths.
            out.sort_by(|a, b| a.path.cmp(&b.path));
        }
        Ok(out)
    }

    /// Every block hash referenced by any version.
    pub fn referenced_blocks(&self) -> Result<HashSet<String>, AppError> {
        let mut st = self.conn.prepare("SELECT blocks FROM files WHERE kind = 'file'")?;
        let mut set = HashSet::new();
        for b in st.query_map([], |r| r.get::<_, String>(0))? {
            let v: Vec<String> = serde_json::from_str(&self.dec("blocks", b?)?).unwrap_or_default();
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

    /// Copy this database to `out` for a bug report: every path component and
    /// symlink target is replaced by a salted hash (extensions kept), extended
    /// metadata (ACLs, xattrs) is cleared, and job-config snapshots and change-journal state are dropped.
    pub fn export_sanitized(&self, out: &Path) -> Result<SanitizeReport, AppError> {
        if out.exists() {
            std::fs::remove_file(out)?;
        }
        self.conn.execute("VACUUM INTO ?1", [out.to_string_lossy()])?;
        let mut c = Connection::open(out)?;
        let salt: [u8; 16] = rand::random();
        let tx = c.transaction()?;
        let mut rep = SanitizeReport::default();
        {
            let rows: Vec<(i64, String, Option<String>)> =
                tx.prepare("SELECT id, path, symlink_target FROM files")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?;
            // An encrypted database becomes a plain (sanitized) report copy.
            let mut up = tx.prepare("UPDATE files SET path = ?2, symlink_target = ?3, meta = NULL, hash = ?4, blocks = ?5 WHERE id = ?1")?;
            let others: HashMap<i64, (Option<String>, String)> =
                tx.prepare("SELECT id, hash, blocks FROM files")?.query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?.collect::<Result<_, _>>()?;
            for (id, path, target) in rows {
                let path = self.dec("path", path)?;
                let target = target.map(|t| self.dec("target", t)).transpose()?;
                let (hash, blocks) = others.get(&id).cloned().unwrap_or_default();
                let hash = hash.map(|h| self.dec("hash", h)).transpose()?;
                let blocks = self.dec("blocks", blocks)?;
                up.execute(params![id, obfuscate_path(&path, &salt), target.map(|t| obfuscate_path(&t, &salt)), hash, blocks])?;
                rep.paths_obfuscated += 1;
            }
            rep.settings_removed = tx.execute("DELETE FROM kv WHERE key LIKE 'dlist_extra:%' OR key = 'usn_state'", [])? as u64;
            // Version metadata: notes, tags and identities can name people
            // or projects; only the pin and the statistics stay.
            let metas: Vec<(i64, String)> = tx.prepare("SELECT id, meta FROM filesets WHERE meta IS NOT NULL")?.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
            for (id, m) in metas {
                let full: VersionMeta = serde_json::from_str(&self.dec("fileset_meta", m)?).unwrap_or_default();
                let kept = VersionMeta { pinned: full.pinned, stats: full.stats, ..Default::default() };
                tx.execute("UPDATE filesets SET meta = ?2 WHERE id = ?1", params![id, (!kept.is_empty()).then(|| serde_json::to_string(&kept)).transpose()?])?;
            }
            if self.cols.is_some() {
                let kv: Vec<(String, String)> = tx.prepare("SELECT key, value FROM kv WHERE key <> ?1")?.query_map([MARKER_KEY], |r| Ok((r.get(0)?, r.get(1)?)))?.collect::<Result<_, _>>()?;
                for (k, v) in kv {
                    tx.execute("UPDATE kv SET value = ?2 WHERE key = ?1", params![k, self.dec(&format!("kv:{k}"), v)?])?;
                }
                tx.execute("DELETE FROM kv WHERE key = ?1", [MARKER_KEY])?;
            }
            tx.execute("INSERT OR REPLACE INTO kv(key, value) VALUES ('bug_report_created', ?1)", [chrono::Utc::now().to_rfc3339()])?;
        }
        tx.commit()?;
        c.execute_batch("VACUUM")?;
        Ok(rep)
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

    #[test]
    fn sanitized_export_hides_paths_and_config() {
        let d = tempfile::tempdir().unwrap();
        let db = BackupDb::open(&d.path().join("job.sqlite")).unwrap();
        let f = |p: &str, t: Option<&str>| FileEntry { path: p.into(), kind: EntryKind::File, size: 3, mtime: None, mode: None, hash: None, blocks: vec![], symlink_target: t.map(String::from), meta: None };
        db.add_fileset("2026-01-01T00:00:00Z", "v.dlist", false, &[f("/home/alice/Secret Plans/budget.xlsx", None), f("C:\\Users\\alice\\link", Some("/home/alice/zzlinkdest"))]).unwrap();
        db.set_kv("dlist_extra:2026", "{\"task_config\":\"password=hunter2\"}").unwrap();
        db.set_kv("index:b1", "i1").unwrap();
        let out = d.path().join("report.sqlite");
        let rep = db.export_sanitized(&out).unwrap();
        assert_eq!((rep.paths_obfuscated, rep.settings_removed), (2, 1));
        let bytes = std::fs::read(&out).unwrap();
        let text = String::from_utf8_lossy(&bytes);
        for leak in ["alice", "Secret", "budget", "hunter2", "zzlinkdest"] {
            assert!(!text.contains(leak), "{leak} leaked into the bug report");
        }
        let r = BackupDb::open(&out).unwrap();
        let files = r.files(r.filesets().unwrap()[0].id).unwrap();
        assert!(files.iter().any(|e| e.path.starts_with('/') && e.path.ends_with(".xlsx") && e.path.matches('/').count() == 4));
        assert!(files.iter().any(|e| e.path.matches('\\').count() == 3));
        assert_eq!(r.get_kv("index:b1").unwrap().as_deref(), Some("i1"), "volume bookkeeping kept");
        // Same component, same hash within one report.
        let salt = [1u8; 16];
        assert_eq!(obfuscate_path("/a/b", &salt).split('/').nth(1), obfuscate_path("/a/c", &salt).split('/').nth(1));
    }

    fn sample(p: &str) -> FileEntry {
        FileEntry {
            path: p.into(),
            kind: EntryKind::File,
            size: 3,
            mtime: Some(1),
            mode: Some(0o644),
            hash: Some("feedface".into()),
            blocks: vec!["b1".into(), "b2".into()],
            symlink_target: Some("/target/zzsecretlink".into()),
            meta: None,
        }
    }

    fn file_text(p: &Path) -> String {
        let mut all = Vec::new();
        for ext in ["", "-wal"] {
            all.extend(std::fs::read(format!("{}{ext}", p.display())).unwrap_or_default());
        }
        String::from_utf8_lossy(&all).into_owned()
    }

    #[test]
    fn encrypted_database_needs_its_key_and_hides_paths() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("job.sqlite");
        let key = DbKey::generate();
        assert_eq!(BackupDb::key_check(&p, Some(&key)).unwrap(), KeyCheck::NoFile);
        {
            let mut db = BackupDb::open(&p).unwrap();
            db.rekey(Some(&key)).unwrap();
            assert!(db.is_encrypted());
            db.add_fileset("2026-01-01T00:00:00Z", "v.dlist", false, &[sample("/home/alice/zzTopSecret/b.txt"), sample("/home/alice/a.txt")]).unwrap();
            db.set_kv("dlist_extra:x", "zzjobconfig").unwrap();
            assert!(db.set_kv("db_encryption", "x").is_err(), "reserved");
            let files = db.files(db.filesets().unwrap()[0].id).unwrap();
            assert_eq!(files[0].path, "/home/alice/a.txt", "sorted after decryption");
            assert_eq!(files[1], sample("/home/alice/zzTopSecret/b.txt"));
            assert_eq!(db.referenced_blocks().unwrap().len(), 2);
            assert_eq!(db.get_kv("dlist_extra:x").unwrap().as_deref(), Some("zzjobconfig"));
        }
        let text = file_text(&p);
        for leak in ["zzTopSecret", "alice", "zzsecretlink", "zzjobconfig", "feedface"] {
            assert!(!text.contains(leak), "{leak} is readable in the file");
        }
        assert!(BackupDb::inspect(&p).unwrap().encrypted);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        // Without the key, or with another one, it does not open.
        assert_eq!(BackupDb::key_check(&p, None).unwrap(), KeyCheck::KeyMissing);
        assert_eq!(BackupDb::key_check(&p, Some(&DbKey::generate())).unwrap(), KeyCheck::WrongKey);
        assert_eq!(BackupDb::key_check(&p, Some(&key)).unwrap(), KeyCheck::Encrypted);
        assert!(matches!(BackupDb::open(&p).err(), Some(AppError::Encryption { .. })));
        assert!(BackupDb::open_with_key(&p, Some(&DbKey::generate())).err().unwrap().user_message().contains("different key"));
        let db = BackupDb::open_with_key(&p, Some(&key)).unwrap();
        assert_eq!(db.files(db.filesets().unwrap()[0].id).unwrap().len(), 2);
        // Keys round-trip through hex (how the keychain stores them).
        assert_eq!(DbKey::from_hex(&key.to_hex()).unwrap(), key);
        assert!(DbKey::from_hex("abcd").is_err());
        assert_eq!(format!("{key:?}"), "DbKey(…)");
    }

    #[test]
    fn migration_between_plain_and_encrypted_keeps_everything() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("job.sqlite");
        let db = BackupDb::open(&p).unwrap();
        let vid = db.add_volume("ufop-b1.dblock", VolumeKind::Block, 10, "h", "uploaded").unwrap();
        db.add_block("b1", 5, vid).unwrap();
        let mut with_meta = sample("/data/zzPlainName.doc");
        with_meta.meta = Some(crate::backup::meta::ExtMeta { flags: Some(2), ..Default::default() });
        db.add_fileset("2026-01-01T00:00:00Z", "v1.dlist", false, &[with_meta.clone()]).unwrap();
        db.add_fileset("2026-01-02T00:00:00Z", "v2.dlist", true, &[sample("/data/other")]).unwrap();
        db.set_kv("usn_state", "{\"zzjournal\":1}").unwrap();
        let before: Vec<Vec<FileEntry>> = db.filesets().unwrap().iter().map(|f| db.files(f.id).unwrap()).collect();
        drop(db);
        assert!(file_text(&p).contains("zzPlainName"), "plain to begin with");
        assert_eq!(BackupDb::key_check(&p, None).unwrap(), KeyCheck::Plain);

        let key = DbKey::generate();
        let mut db = BackupDb::open_with_key(&p, Some(&key)).unwrap();
        assert!(!db.is_encrypted(), "a key alone does not encrypt");
        let r = db.rekey(Some(&key)).unwrap();
        assert_eq!((r.files, r.settings, r.encrypted), (2, 1, true));
        let after: Vec<Vec<FileEntry>> = db.filesets().unwrap().iter().map(|f| db.files(f.id).unwrap()).collect();
        assert_eq!(before, after);
        assert_eq!(db.get_kv("usn_state").unwrap().as_deref(), Some("{\"zzjournal\":1}"));
        assert!(db.has_block("b1").unwrap());
        drop(db);
        let text = file_text(&p);
        assert!(!text.contains("zzPlainName") && !text.contains("zzjournal"), "no plaintext left in free pages or the WAL");

        // Another key, then back to plain.
        let k2 = DbKey::generate();
        let mut db = BackupDb::open_with_key(&p, Some(&key)).unwrap();
        db.rekey(Some(&k2)).unwrap();
        drop(db);
        assert_eq!(BackupDb::key_check(&p, Some(&key)).unwrap(), KeyCheck::WrongKey);
        let mut db = BackupDb::open_with_key(&p, Some(&k2)).unwrap();
        assert!(!db.rekey(None).unwrap().encrypted);
        drop(db);
        let db = BackupDb::open(&p).unwrap();
        assert_eq!(before, db.filesets().unwrap().iter().map(|f| db.files(f.id).unwrap()).collect::<Vec<_>>());
    }

    #[test]
    fn sanitized_export_of_an_encrypted_database_is_plain_and_hashed() {
        let d = tempfile::tempdir().unwrap();
        let key = DbKey::generate();
        let mut db = BackupDb::open(&d.path().join("job.sqlite")).unwrap();
        db.rekey(Some(&key)).unwrap();
        db.add_fileset("2026-01-01T00:00:00Z", "v.dlist", false, &[sample("/home/alice/x.pdf")]).unwrap();
        db.set_kv("index:b1", "i1").unwrap();
        let out = d.path().join("report.sqlite");
        db.export_sanitized(&out).unwrap();
        let r = BackupDb::open(&out).unwrap();
        let files = r.files(r.filesets().unwrap()[0].id).unwrap();
        assert!(files[0].path.ends_with(".pdf") && !files[0].path.contains("alice"));
        assert_eq!(files[0].blocks, ["b1", "b2"]);
        assert_eq!(r.get_kv("index:b1").unwrap().as_deref(), Some("i1"));
    }
}
