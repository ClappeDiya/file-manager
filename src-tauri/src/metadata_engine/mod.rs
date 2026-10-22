//! Persistent file metadata: color tags, free-text labels, key-value
//! metadata and saved smart-folder queries.
//!
//! Replaces the process-local store the integrity commands used, which lost
//! every tag and label on quit. Rows are keyed by absolute path; moving or
//! renaming through the app carries metadata along (`rename_path`).

use crate::core::error::AppError;
use crate::storage::Repository;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// A named color tag.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileTag {
    pub id: String,
    pub name: String,
    /// Hex color, e.g. `#ef4444`.
    pub color: String,
    pub file_count: usize,
}

/// Everything attached to one path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct FileLabel {
    pub path: String,
    /// Tag ids.
    pub tags: Vec<String>,
    pub labels: Vec<String>,
    /// Key-value metadata (business identifiers, classifications, notes).
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
}

/// A saved smart-folder query.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmartFolder {
    pub id: String,
    pub name: String,
    pub query: SmartFolderQuery,
    pub icon: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SmartFolderQuery {
    pub base_paths: Vec<String>,
    pub name_pattern: Option<String>,
    pub extension_filter: Option<Vec<String>>,
    pub min_size: Option<u64>,
    pub max_size: Option<u64>,
    pub modified_after: Option<String>,
    pub modified_before: Option<String>,
    /// Tag ids; a file must carry all of them.
    pub tags: Option<Vec<String>>,
    /// Labels; a file must carry all of them.
    pub labels: Option<Vec<String>>,
    /// Metadata `key = value` pairs a file must carry.
    #[serde(default)]
    pub metadata: Option<BTreeMap<String, String>>,
    pub include_subdirs: bool,
}

/// A hit from a metadata search.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetadataMatch {
    pub path: String,
    pub key: String,
    pub value: String,
}

/// Tags, labels and metadata of many paths, loaded once for a scan.
#[derive(Debug, Default)]
pub struct MetadataSnapshot {
    pub tags: HashMap<String, Vec<String>>,
    pub labels: HashMap<String, Vec<String>>,
    pub metadata: HashMap<String, BTreeMap<String, String>>,
}

impl MetadataSnapshot {
    /// Whether `path` satisfies the tag, label and metadata parts of `query`.
    pub fn matches(&self, path: &str, query: &SmartFolderQuery) -> bool {
        let has_all = |have: Option<&Vec<String>>, want: &Option<Vec<String>>| match want {
            Some(want) if !want.is_empty() => have.is_some_and(|h| want.iter().all(|w| h.contains(w))),
            _ => true,
        };
        if !has_all(self.tags.get(path), &query.tags) || !has_all(self.labels.get(path), &query.labels) {
            return false;
        }
        match &query.metadata {
            Some(want) if !want.is_empty() => self
                .metadata
                .get(path)
                .is_some_and(|have| want.iter().all(|(k, v)| have.get(k) == Some(v))),
            _ => true,
        }
    }
}

/// SQLite-backed metadata store (schema v18).
#[derive(Clone)]
pub struct MetadataStore {
    repo: Repository,
}

const MAX_KEY_LEN: usize = 128;
const MAX_VALUE_LEN: usize = 4096;

fn now() -> String {
    Utc::now().to_rfc3339()
}

fn validate_color(color: &str) -> Result<(), AppError> {
    let hex = color.strip_prefix('#').unwrap_or("");
    if (hex.len() == 6 || hex.len() == 3) && hex.chars().all(|c| c.is_ascii_hexdigit()) {
        Ok(())
    } else {
        Err(AppError::validation(format!("'{color}' is not a hex color like #22c55e")))
    }
}

fn validate_key(key: &str) -> Result<String, AppError> {
    let key = key.trim();
    if key.is_empty() || key.len() > MAX_KEY_LEN {
        return Err(AppError::validation(format!("Metadata keys must be 1–{MAX_KEY_LEN} characters")));
    }
    Ok(key.to_string())
}

impl MetadataStore {
    pub fn new(repo: Repository) -> Self {
        Self { repo }
    }

    // ── Tags ──

    pub async fn create_tag(&self, name: &str, color: &str) -> Result<FileTag, AppError> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::validation("Tag names cannot be empty"));
        }
        validate_color(color)?;
        let tag = FileTag { id: uuid::Uuid::new_v4().to_string(), name, color: color.to_string(), file_count: 0 };
        let t = tag.clone();
        self.repo
            .pool()
            .execute(move |conn| {
                let exists: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM file_tag_defs WHERE name = ?1 COLLATE NOCASE)",
                    params![t.name],
                    |r| r.get(0),
                )?;
                if exists {
                    return Err(AppError::validation(format!("Tag '{}' already exists", t.name)));
                }
                conn.execute(
                    "INSERT INTO file_tag_defs (id, name, color, created_at) VALUES (?1, ?2, ?3, ?4)",
                    params![t.id, t.name, t.color, now()],
                )?;
                Ok(())
            })
            .await?;
        Ok(tag)
    }

    pub async fn list_tags(&self) -> Result<Vec<FileTag>, AppError> {
        self.repo
            .pool()
            .execute(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT d.id, d.name, d.color, COUNT(l.path) FROM file_tag_defs d \
                     LEFT JOIN file_tag_links l ON l.tag_id = d.id GROUP BY d.id ORDER BY d.name COLLATE NOCASE",
                )?;
                let rows = stmt
                    .query_map([], |r| {
                        Ok(FileTag { id: r.get(0)?, name: r.get(1)?, color: r.get(2)?, file_count: r.get::<_, i64>(3)? as usize })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn delete_tag(&self, tag_id: &str) -> Result<(), AppError> {
        let id = tag_id.to_string();
        self.repo
            .pool()
            .execute(move |conn| {
                // Links go with the tag (ON DELETE CASCADE).
                conn.execute("DELETE FROM file_tag_defs WHERE id = ?1", params![id])?;
                Ok(())
            })
            .await
    }

    pub async fn tag_file(&self, path: &str, tag_ids: &[String]) -> Result<FileLabel, AppError> {
        let (p, ids) = (path.to_string(), tag_ids.to_vec());
        self.repo
            .pool()
            .execute(move |conn| {
                let tx = conn.unchecked_transaction()?;
                for id in &ids {
                    let known: bool =
                        tx.query_row("SELECT EXISTS(SELECT 1 FROM file_tag_defs WHERE id = ?1)", params![id], |r| r.get(0))?;
                    if !known {
                        return Err(AppError::not_found(format!("Tag {id} does not exist")));
                    }
                    tx.execute(
                        "INSERT OR IGNORE INTO file_tag_links (path, tag_id, created_at) VALUES (?1, ?2, ?3)",
                        params![p, id, now()],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .await?;
        self.file_info(path).await
    }

    pub async fn untag_file(&self, path: &str, tag_ids: &[String]) -> Result<FileLabel, AppError> {
        let (p, ids) = (path.to_string(), tag_ids.to_vec());
        self.repo
            .pool()
            .execute(move |conn| {
                for id in &ids {
                    conn.execute("DELETE FROM file_tag_links WHERE path = ?1 AND tag_id = ?2", params![p, id])?;
                }
                Ok(())
            })
            .await?;
        self.file_info(path).await
    }

    // ── Labels ──

    pub async fn set_label(&self, path: &str, label: &str) -> Result<FileLabel, AppError> {
        let label = label.trim().to_string();
        if label.is_empty() {
            return Err(AppError::validation("Labels cannot be empty"));
        }
        let p = path.to_string();
        self.repo
            .pool()
            .execute(move |conn| {
                conn.execute(
                    "INSERT OR IGNORE INTO file_labels (path, label, created_at) VALUES (?1, ?2, ?3)",
                    params![p, label, now()],
                )?;
                Ok(())
            })
            .await?;
        self.file_info(path).await
    }

    pub async fn remove_label(&self, path: &str, label: &str) -> Result<FileLabel, AppError> {
        let (p, l) = (path.to_string(), label.to_string());
        self.repo
            .pool()
            .execute(move |conn| {
                conn.execute("DELETE FROM file_labels WHERE path = ?1 AND label = ?2", params![p, l])?;
                Ok(())
            })
            .await?;
        self.file_info(path).await
    }

    // ── Key-value metadata ──

    pub async fn set_metadata(&self, path: &str, key: &str, value: &str) -> Result<FileLabel, AppError> {
        let key = validate_key(key)?;
        if value.len() > MAX_VALUE_LEN {
            return Err(AppError::validation(format!("Metadata values are limited to {MAX_VALUE_LEN} bytes")));
        }
        let (p, v) = (path.to_string(), value.to_string());
        self.repo
            .pool()
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO file_metadata (path, key, value, updated_at) VALUES (?1, ?2, ?3, ?4) \
                     ON CONFLICT(path, key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
                    params![p, key, v, now()],
                )?;
                Ok(())
            })
            .await?;
        self.file_info(path).await
    }

    pub async fn remove_metadata(&self, path: &str, key: &str) -> Result<FileLabel, AppError> {
        let (p, k) = (path.to_string(), key.to_string());
        self.repo
            .pool()
            .execute(move |conn| {
                conn.execute("DELETE FROM file_metadata WHERE path = ?1 AND key = ?2", params![p, k])?;
                Ok(())
            })
            .await?;
        self.file_info(path).await
    }

    /// Paths whose metadata `key` equals `value` (or contains `value` when
    /// `contains`), optionally under `under`. `key = None` searches all keys.
    pub async fn search_metadata(
        &self,
        key: Option<&str>,
        value: &str,
        contains: bool,
        under: Option<&str>,
        limit: usize,
    ) -> Result<Vec<MetadataMatch>, AppError> {
        let key = key.map(str::to_string);
        let pattern = if contains { format!("%{}%", escape_like(value)) } else { value.to_string() };
        let prefix = under.map(|u| format!("{}%", escape_like(&dir_prefix(u))));
        let limit = limit.clamp(1, 10_000) as i64;
        self.repo
            .pool()
            .execute(move |conn| {
                let op = if contains { "LIKE ?2 ESCAPE '\\'" } else { "= ?2" };
                let sql = format!(
                    "SELECT path, key, value FROM file_metadata WHERE (?1 IS NULL OR key = ?1) AND value {op} \
                     AND (?3 IS NULL OR path LIKE ?3 ESCAPE '\\') ORDER BY path, key LIMIT ?4"
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt
                    .query_map(params![key, pattern, prefix, limit], |r| {
                        Ok(MetadataMatch { path: r.get(0)?, key: r.get(1)?, value: r.get(2)? })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    /// Every metadata key in use (for filter pickers), most used first.
    pub async fn metadata_keys(&self) -> Result<Vec<String>, AppError> {
        self.repo
            .pool()
            .execute(|conn| {
                let mut stmt = conn.prepare("SELECT key FROM file_metadata GROUP BY key ORDER BY COUNT(*) DESC, key")?;
                let rows = stmt.query_map([], |r| r.get(0))?.collect::<Result<Vec<String>, _>>()?;
                Ok(rows)
            })
            .await
    }

    // ── Per-path view ──

    pub async fn file_info(&self, path: &str) -> Result<FileLabel, AppError> {
        let p = path.to_string();
        self.repo
            .pool()
            .execute(move |conn| {
                let tags = conn
                    .prepare("SELECT tag_id FROM file_tag_links WHERE path = ?1 ORDER BY created_at")?
                    .query_map(params![p], |r| r.get(0))?
                    .collect::<Result<Vec<String>, _>>()?;
                let labels = conn
                    .prepare("SELECT label FROM file_labels WHERE path = ?1 ORDER BY created_at")?
                    .query_map(params![p], |r| r.get(0))?
                    .collect::<Result<Vec<String>, _>>()?;
                let metadata = conn
                    .prepare("SELECT key, value FROM file_metadata WHERE path = ?1")?
                    .query_map(params![p], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<BTreeMap<String, String>, _>>()?;
                Ok(FileLabel { path: p, tags, labels, metadata })
            })
            .await
    }

    /// Tags, labels and metadata of every path under `roots` (or all paths
    /// when empty), for evaluating smart folders without a query per file.
    pub async fn snapshot(&self, roots: &[String]) -> Result<MetadataSnapshot, AppError> {
        let prefixes: Vec<String> = roots.iter().map(|r| dir_prefix(r)).collect();
        self.repo
            .pool()
            .execute(move |conn| {
                let keep = |p: &str| prefixes.is_empty() || prefixes.iter().any(|pre| p.starts_with(pre.as_str()));
                let mut snap = MetadataSnapshot::default();
                let mut stmt = conn.prepare("SELECT path, tag_id FROM file_tag_links")?;
                for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                    let (p, t) = row?;
                    if keep(&p) {
                        snap.tags.entry(p).or_default().push(t);
                    }
                }
                let mut stmt = conn.prepare("SELECT path, label FROM file_labels")?;
                for row in stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
                    let (p, l) = row?;
                    if keep(&p) {
                        snap.labels.entry(p).or_default().push(l);
                    }
                }
                let mut stmt = conn.prepare("SELECT path, key, value FROM file_metadata")?;
                for row in stmt.query_map([], |r| {
                    Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?))
                })? {
                    let (p, k, v) = row?;
                    if keep(&p) {
                        snap.metadata.entry(p).or_default().insert(k, v);
                    }
                }
                Ok(snap)
            })
            .await
    }

    /// Carry tags, labels and metadata along when `old` is moved or renamed
    /// to `new` (descendants too, for folders). Existing rows at the target
    /// are kept on conflict.
    pub async fn rename_path(&self, old: &str, new: &str) -> Result<usize, AppError> {
        if old == new {
            return Ok(0);
        }
        let (old, new) = (old.to_string(), new.to_string());
        self.repo
            .pool()
            .execute(move |conn| {
                let tx = conn.unchecked_transaction()?;
                let old_prefix = format!("{}%", escape_like(&dir_prefix(&old)));
                let mut moved = 0usize;
                for table in ["file_tag_links", "file_labels", "file_metadata"] {
                    // `?2 || substr(path, len(old)+1)` maps old/x → new/x.
                    moved += tx.execute(
                        &format!(
                            "UPDATE OR IGNORE {table} SET path = ?2 || substr(path, length(?1) + 1) \
                             WHERE path = ?1 OR path LIKE ?3 ESCAPE '\\'"
                        ),
                        params![old, new, old_prefix],
                    )?;
                    // Rows that conflicted with existing target rows stay at
                    // the old path; drop them (the target already has metadata).
                    tx.execute(
                        &format!("DELETE FROM {table} WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'"),
                        params![old, old_prefix],
                    )?;
                }
                tx.commit()?;
                Ok(moved)
            })
            .await
    }

    /// Drop everything attached to `path` (and descendants) after a delete.
    pub async fn forget_path(&self, path: &str) -> Result<(), AppError> {
        let p = path.to_string();
        self.repo
            .pool()
            .execute(move |conn| {
                let prefix = format!("{}%", escape_like(&dir_prefix(&p)));
                for table in ["file_tag_links", "file_labels", "file_metadata"] {
                    conn.execute(
                        &format!("DELETE FROM {table} WHERE path = ?1 OR path LIKE ?2 ESCAPE '\\'"),
                        params![p, prefix],
                    )?;
                }
                Ok(())
            })
            .await
    }

    /// Keep metadata attached to files after a filesystem operation the app
    /// performed. `kind` is the file-op kind ("move", "rename",
    /// "delete_permanent"); `reversed` is true when undoing it. Failures are
    /// logged, never surfaced: the file operation itself already succeeded.
    pub async fn follow_fs_op(&self, kind: &str, sources: &[String], dests: &[String], reversed: bool) {
        let result = match kind {
            "move" | "rename" => {
                let mut r = Ok(0);
                for (src, dst) in sources.iter().zip(dests) {
                    let (from, to) = if reversed { (dst, src) } else { (src, dst) };
                    if let Err(e) = self.rename_path(from, to).await {
                        r = Err(e);
                    }
                }
                r.map(|_| ())
            }
            "delete_permanent" if !reversed => {
                let mut r = Ok(());
                for src in sources {
                    if let Err(e) = self.forget_path(src).await {
                        r = Err(e);
                    }
                }
                r
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            tracing::warn!("File metadata not updated after {kind}: {e}");
        }
    }

    // ── Smart folders ──

    pub async fn create_smart_folder(&self, name: &str, query: SmartFolderQuery, icon: Option<String>) -> Result<SmartFolder, AppError> {
        let name = name.trim().to_string();
        if name.is_empty() {
            return Err(AppError::validation("Smart folder names cannot be empty"));
        }
        if let Some(p) = &query.name_pattern {
            regex::Regex::new(p).map_err(|e| AppError::validation(format!("Invalid name pattern: {e}")))?;
        }
        let ts = now();
        let folder = SmartFolder { id: uuid::Uuid::new_v4().to_string(), name, query, icon, created_at: ts.clone(), updated_at: ts };
        let f = folder.clone();
        let json = serde_json::to_string(&f.query).map_err(|e| AppError::internal(e.to_string()))?;
        self.repo
            .pool()
            .execute(move |conn| {
                conn.execute(
                    "INSERT INTO smart_folders (id, name, query_json, icon, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![f.id, f.name, json, f.icon, f.created_at, f.updated_at],
                )?;
                Ok(())
            })
            .await?;
        Ok(folder)
    }

    pub async fn list_smart_folders(&self) -> Result<Vec<SmartFolder>, AppError> {
        self.repo
            .pool()
            .execute(|conn| {
                let mut stmt =
                    conn.prepare("SELECT id, name, query_json, icon, created_at, updated_at FROM smart_folders ORDER BY name COLLATE NOCASE")?;
                let rows = stmt
                    .query_map([], |r| {
                        let json: String = r.get(2)?;
                        Ok(SmartFolder {
                            id: r.get(0)?,
                            name: r.get(1)?,
                            query: serde_json::from_str(&json).unwrap_or_default(),
                            icon: r.get(3)?,
                            created_at: r.get(4)?,
                            updated_at: r.get(5)?,
                        })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(rows)
            })
            .await
    }

    pub async fn get_smart_folder(&self, id: &str) -> Result<Option<SmartFolder>, AppError> {
        Ok(self.list_smart_folders().await?.into_iter().find(|f| f.id == id))
    }

    pub async fn delete_smart_folder(&self, id: &str) -> Result<(), AppError> {
        let id = id.to_string();
        self.repo
            .pool()
            .execute(move |conn| {
                conn.execute("DELETE FROM smart_folders WHERE id = ?1", params![id])?;
                Ok(())
            })
            .await
    }

    /// Whether a tag id exists.
    pub async fn tag_exists(&self, tag_id: &str) -> Result<bool, AppError> {
        let id = tag_id.to_string();
        self.repo
            .pool()
            .execute(move |conn| {
                Ok(conn
                    .query_row("SELECT 1 FROM file_tag_defs WHERE id = ?1", params![id], |_| Ok(()))
                    .optional()?
                    .is_some())
            })
            .await
    }
}

/// `dir` with exactly one trailing separator, for descendant prefix matches
/// (so `/a/b` never matches `/a/bc`).
fn dir_prefix(dir: &str) -> String {
    let sep = if dir.contains('\\') && !dir.contains('/') { '\\' } else { '/' };
    format!("{}{sep}", dir.trim_end_matches(['/', '\\']))
}

fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn store() -> (MetadataStore, Repository) {
        let repo = Repository::open_in_memory().await.unwrap();
        (MetadataStore::new(repo.clone()), repo)
    }

    /// Regression: tags lived in a process-local map and vanished on quit.
    #[tokio::test]
    async fn tags_labels_and_metadata_persist_in_sqlite() {
        let (s, repo) = store().await;
        let tag = s.create_tag("Urgent", "#ef4444").await.unwrap();
        s.tag_file("/docs/a.pdf", std::slice::from_ref(&tag.id)).await.unwrap();
        s.set_label("/docs/a.pdf", "reviewed").await.unwrap();
        s.set_metadata("/docs/a.pdf", "invoice", "INV-42").await.unwrap();

        // A second store over the same database sees everything.
        let again = MetadataStore::new(repo);
        let info = again.file_info("/docs/a.pdf").await.unwrap();
        assert_eq!(info.tags, vec![tag.id.clone()]);
        assert_eq!(info.labels, vec!["reviewed".to_string()]);
        assert_eq!(info.metadata.get("invoice").map(String::as_str), Some("INV-42"));
        assert_eq!(again.list_tags().await.unwrap()[0].file_count, 1);
    }

    #[tokio::test]
    async fn tag_rules() {
        let (s, _) = store().await;
        s.create_tag("Blue", "#00f").await.unwrap();
        assert!(s.create_tag("blue", "#0000ff").await.is_err(), "names are unique, case-insensitively");
        assert!(s.create_tag("Bad", "red").await.is_err(), "colors must be hex");
        assert!(s.tag_file("/x", &["nope".into()]).await.is_err(), "unknown tag ids are rejected");
        let t = s.create_tag("Green", "#22c55e").await.unwrap();
        s.tag_file("/x", &[t.id.clone(), t.id.clone()]).await.unwrap();
        assert_eq!(s.file_info("/x").await.unwrap().tags.len(), 1, "tagging is idempotent");
        s.delete_tag(&t.id).await.unwrap();
        assert!(s.file_info("/x").await.unwrap().tags.is_empty(), "deleting a tag unlinks it");
    }

    #[tokio::test]
    async fn metadata_search_and_update() {
        let (s, _) = store().await;
        s.set_metadata("/p/a.txt", "client", "Acme Corp").await.unwrap();
        s.set_metadata("/p/b.txt", "client", "Acme Ltd").await.unwrap();
        s.set_metadata("/q/c.txt", "client", "Globex").await.unwrap();
        s.set_metadata("/p/a.txt", "client", "Acme Inc").await.unwrap(); // update in place

        let exact = s.search_metadata(Some("client"), "Globex", false, None, 50).await.unwrap();
        assert_eq!(exact.iter().map(|m| m.path.as_str()).collect::<Vec<_>>(), vec!["/q/c.txt"]);
        let fuzzy = s.search_metadata(Some("client"), "acme", true, Some("/p"), 50).await.unwrap();
        assert_eq!(fuzzy.len(), 2);
        assert_eq!(fuzzy[0].value, "Acme Inc");
        // LIKE wildcards in user input are literal.
        assert!(s.search_metadata(None, "%", true, None, 50).await.unwrap().is_empty());
        assert_eq!(s.metadata_keys().await.unwrap(), vec!["client".to_string()]);
        assert!(s.set_metadata("/p/a.txt", "  ", "v").await.is_err());
        s.remove_metadata("/p/a.txt", "client").await.unwrap();
        assert!(s.file_info("/p/a.txt").await.unwrap().metadata.is_empty());
    }

    #[tokio::test]
    async fn rename_carries_metadata_for_files_and_folders() {
        let (s, _) = store().await;
        let t = s.create_tag("T", "#123456").await.unwrap();
        s.tag_file("/a/one.txt", std::slice::from_ref(&t.id)).await.unwrap();
        s.set_metadata("/a/sub/two.txt", "k", "v").await.unwrap();
        s.set_label("/ab/keep.txt", "untouched").await.unwrap();

        s.rename_path("/a", "/b").await.unwrap();
        assert_eq!(s.file_info("/b/one.txt").await.unwrap().tags, vec![t.id.clone()]);
        assert_eq!(s.file_info("/b/sub/two.txt").await.unwrap().metadata["k"], "v");
        assert!(s.file_info("/a/one.txt").await.unwrap().tags.is_empty());
        assert_eq!(s.file_info("/ab/keep.txt").await.unwrap().labels, vec!["untouched".to_string()], "sibling prefix untouched");

        s.forget_path("/b").await.unwrap();
        assert!(s.file_info("/b/sub/two.txt").await.unwrap().metadata.is_empty());
    }

    #[tokio::test]
    async fn smart_folders_persist_and_snapshot_matches() {
        let (s, repo) = store().await;
        let t = s.create_tag("Q4", "#abcdef").await.unwrap();
        s.tag_file("/r/a.csv", std::slice::from_ref(&t.id)).await.unwrap();
        s.set_label("/r/a.csv", "final").await.unwrap();
        s.set_metadata("/r/a.csv", "dept", "finance").await.unwrap();
        s.tag_file("/r/b.csv", std::slice::from_ref(&t.id)).await.unwrap();

        let query = SmartFolderQuery {
            base_paths: vec!["/r".into()],
            tags: Some(vec![t.id.clone()]),
            labels: Some(vec!["final".into()]),
            metadata: Some(BTreeMap::from([("dept".into(), "finance".into())])),
            ..Default::default()
        };
        let f = s.create_smart_folder("Final Q4", query.clone(), None).await.unwrap();
        assert!(s.create_smart_folder("bad", SmartFolderQuery { name_pattern: Some("(".into()), ..Default::default() }, None).await.is_err());

        let again = MetadataStore::new(repo);
        assert_eq!(again.list_smart_folders().await.unwrap()[0].id, f.id);
        let snap = again.snapshot(&["/r".into()]).await.unwrap();
        assert!(snap.matches("/r/a.csv", &query));
        assert!(!snap.matches("/r/b.csv", &query), "label and metadata are required too");
        again.delete_smart_folder(&f.id).await.unwrap();
        assert!(again.list_smart_folders().await.unwrap().is_empty());
    }

    #[test]
    fn dir_prefix_never_matches_siblings() {
        assert_eq!(dir_prefix("/a/b/"), "/a/b/");
        assert_eq!(dir_prefix("/a/b"), "/a/b/");
        assert_eq!(dir_prefix(r"C:\data"), r"C:\data\");
    }
}
