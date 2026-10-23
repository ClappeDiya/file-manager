//! Per-version metadata: a note, tags, a pin ("Keep forever"), the client
//! identity that wrote the version and what the version added to storage.
//!
//! The metadata lives in each version's file list (`dlist`) so it survives
//! a database rebuild, and is mirrored in the local database (`filesets.meta`,
//! sealed like the other sensitive columns when the database is encrypted).
//! Every field is optional with a serde default, and the file-list reader
//! has never refused unknown fields, so older app versions read these file
//! lists and simply ignore the metadata.

use super::backend::Backend;
use super::engine::Repo;
use super::volume::{self, FileListVolume, VolumeKind};
use super::{hash_bytes, Fileset, HashAlgorithm};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Longest note accepted.
pub const MAX_NOTE_CHARS: usize = 2000;
/// Most tags per version.
pub const MAX_TAGS: usize = 32;
const MAX_TAG_KEY: usize = 64;
const MAX_TAG_VALUE: usize = 256;
const MAX_PIN_REASON: usize = 500;

/// Who wrote a version: the computer, the account and the job. In a shared
/// repository the identity decides which "source" a version belongs to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientIdentity {
    pub hostname: String,
    pub username: String,
    /// The job (or CLI source set) on that computer.
    pub job_id: String,
    /// Human-readable label (the job name).
    pub source_label: String,
}

impl ClientIdentity {
    /// Fill an empty host name / user name with this computer's.
    pub fn resolve(&self) -> Self {
        let mut me = self.clone();
        if me.hostname.trim().is_empty() {
            me.hostname = crate::session::hostname();
        }
        if me.username.trim().is_empty() {
            me.username = crate::session::user_name();
        }
        me
    }

    /// Stable, opaque id of the source this identity writes to
    /// (`user@host:job`, hashed so the local database does not show names).
    pub fn source_id(&self) -> String {
        source_id_of(&self.hostname, &self.username, &self.job_id)
    }

    /// `user@host` plus the label when there is one.
    pub fn display(&self) -> String {
        let base = format!("{}@{}", self.username, self.hostname);
        if self.source_label.is_empty() {
            base
        } else {
            format!("{base} ({})", self.source_label)
        }
    }

    pub fn is_empty(&self) -> bool {
        self.hostname.is_empty() && self.username.is_empty() && self.job_id.is_empty() && self.source_label.is_empty()
    }
}

/// Source id for an identity's parts (case-insensitive host name).
pub fn source_id_of(hostname: &str, username: &str, job_id: &str) -> String {
    let key = format!("{}@{}:{}", username.trim(), hostname.trim().to_lowercase(), job_id.trim());
    hash_bytes(HashAlgorithm::Sha256, key.as_bytes())[..16].to_string()
}

/// What a version added to the repository, recorded at backup time.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VersionStats {
    /// Bytes uploaded for new data: the sealed block volumes and their
    /// indexes, after compression and encryption (not the file list).
    pub bytes_added: u64,
    /// Blocks that were not in the repository before.
    pub new_blocks: u64,
    pub files_added: u64,
    pub files_changed: u64,
    pub files_deleted: u64,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Metadata stored with a version (flattened into the file list).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct VersionMeta {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub tags: BTreeMap<String, String>,
    /// Kept forever: retention never removes it and deleting needs `force`.
    #[serde(skip_serializing_if = "is_false")]
    pub pinned: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pin_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub identity: Option<ClientIdentity>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stats: Option<VersionStats>,
}

impl VersionMeta {
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Source id of the identity ("" for versions written before identities).
    pub fn source_id(&self) -> String {
        self.identity.as_ref().map(ClientIdentity::source_id).unwrap_or_default()
    }

    /// Check note, tags and pin reason against the limits.
    pub fn validate(&self) -> Result<(), AppError> {
        validate_note(self.note.as_deref())?;
        validate_tags(&self.tags)?;
        if self.pin_reason.as_deref().is_some_and(|r| r.chars().count() > MAX_PIN_REASON) {
            return Err(AppError::validation(format!("The pin reason is limited to {MAX_PIN_REASON} characters")));
        }
        Ok(())
    }
}

/// A note is optional and at most [`MAX_NOTE_CHARS`] characters.
pub fn validate_note(note: Option<&str>) -> Result<(), AppError> {
    if note.is_some_and(|n| n.chars().count() > MAX_NOTE_CHARS) {
        return Err(AppError::validation(format!("The note is limited to {MAX_NOTE_CHARS} characters")));
    }
    Ok(())
}

/// Tag keys: 1–64 of `A-Z a-z 0-9 _ . : -`; values up to 256 characters.
pub fn validate_tags(tags: &BTreeMap<String, String>) -> Result<(), AppError> {
    if tags.len() > MAX_TAGS {
        return Err(AppError::validation(format!("A version can have at most {MAX_TAGS} tags")));
    }
    for (k, v) in tags {
        let ok = !k.is_empty() && k.len() <= MAX_TAG_KEY && k.chars().all(|c| c.is_ascii_alphanumeric() || "_.:-".contains(c));
        if !ok {
            return Err(AppError::validation(format!("Invalid tag name '{k}' (1–{MAX_TAG_KEY} letters, digits, _ . : -)")));
        }
        if v.chars().count() > MAX_TAG_VALUE || v.contains('\n') {
            return Err(AppError::validation(format!("The value of tag '{k}' is too long or has a line break")));
        }
    }
    Ok(())
}

/// Parse `key=value` (a bare `key` means an empty value), as given to `--tag`.
pub fn parse_tag(s: &str) -> Result<(String, String), AppError> {
    let (k, v) = s.split_once('=').unwrap_or((s, ""));
    let (k, v) = (k.trim().to_string(), v.trim().to_string());
    validate_tags(&BTreeMap::from([(k.clone(), v.clone())]))?;
    Ok((k, v))
}

/// Parse several `key=value` tags.
pub fn parse_tags(items: &[String]) -> Result<BTreeMap<String, String>, AppError> {
    let mut m = BTreeMap::new();
    for s in items.iter().filter(|s| !s.trim().is_empty()) {
        let (k, v) = parse_tag(s)?;
        m.insert(k, v);
    }
    validate_tags(&m)?;
    Ok(m)
}

/// Does a version carry every tag of `filter`? A filter `key=value` needs
/// that value; a bare `key` (empty value) matches any value of the key.
pub fn matches_tags(tags: &BTreeMap<String, String>, filter: &BTreeMap<String, String>) -> bool {
    filter.iter().all(|(k, v)| tags.get(k).is_some_and(|have| v.is_empty() || have == v))
}

/// Changes to a version's metadata; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MetadataPatch {
    /// New note; an empty string removes it.
    pub note: Option<String>,
    /// Replace all tags (an empty map removes them).
    pub tags: Option<BTreeMap<String, String>>,
    pub pinned: Option<bool>,
    /// Why the version is pinned; an empty string removes it.
    pub pin_reason: Option<String>,
}

impl MetadataPatch {
    pub fn is_empty(&self) -> bool {
        self.note.is_none() && self.tags.is_none() && self.pinned.is_none() && self.pin_reason.is_none()
    }

    /// Apply to `meta` (unpinning also clears the reason).
    pub fn apply(&self, meta: &mut VersionMeta) -> Result<(), AppError> {
        if let Some(n) = &self.note {
            let n = n.trim();
            meta.note = (!n.is_empty()).then(|| n.to_string());
        }
        if let Some(t) = &self.tags {
            meta.tags = t.iter().map(|(k, v)| (k.trim().to_string(), v.trim().to_string())).collect();
        }
        if let Some(p) = self.pinned {
            meta.pinned = p;
        }
        if let Some(r) = &self.pin_reason {
            let r = r.trim();
            meta.pin_reason = (!r.is_empty()).then(|| r.to_string());
        }
        if !meta.pinned {
            meta.pin_reason = None;
        }
        meta.validate()
    }
}

/// Re-seal one version's file list with new metadata: the remote file list
/// is downloaded (hash-checked), changed and uploaded under the same name
/// (a single object write, which replaces the old one atomically), then the
/// local database is updated.
pub fn rewrite_version_meta(repo: &Repo, fs: &Fileset, meta: &VersionMeta) -> Result<Fileset, AppError> {
    let mut fl: FileListVolume = volume::open_json(&repo.cipher, &repo.get_volume(&fs.volume_name)?)?;
    fl.meta = meta.clone();
    let data = volume::seal_json(&repo.cipher, &fl)?;
    repo.note_write()?;
    repo.backend.put(&fs.volume_name, &data)?;
    repo.put_recovery(&fs.volume_name, &data)?;
    repo.db.add_volume(&fs.volume_name, VolumeKind::FileList, data.len() as u64, &hash_bytes(HashAlgorithm::Sha256, &data), "uploaded")?;
    repo.db.set_fileset_meta(fs.id, meta)?;
    repo.db.fileset_by_id(fs.id)
}

/// Change the note, tags or pin of version `version` (0 = newest of the
/// current source). Returns the updated version.
pub fn set_version_metadata(repo: &Repo, version: usize, patch: &MetadataPatch) -> Result<Fileset, AppError> {
    let fs = repo.db.fileset_by_version(version)?;
    if patch.is_empty() {
        return Ok(fs);
    }
    let mut meta = fs.meta();
    patch.apply(&mut meta)?;
    if meta == fs.meta() {
        return Ok(fs);
    }
    rewrite_version_meta(repo, &fs, &meta)
}

/// Versions of the current source carrying every tag of `filter`.
pub fn filter_by_tags(versions: Vec<Fileset>, filter: &BTreeMap<String, String>) -> Vec<Fileset> {
    if filter.is_empty() {
        return versions;
    }
    versions.into_iter().filter(|f| matches_tags(&f.tags, filter)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_parse_validate_and_match() {
        let t = parse_tags(&["env=prod".into(), "release".into(), " team = ops ".into()]).unwrap();
        assert_eq!(t.get("env").map(String::as_str), Some("prod"));
        assert_eq!(t.get("release").map(String::as_str), Some(""));
        assert_eq!(t.get("team").map(String::as_str), Some("ops"));
        assert!(parse_tag("bad key=x").is_err());
        assert!(parse_tag("=x").is_err());
        assert!(parse_tag(&format!("k={}", "v".repeat(300))).is_err());
        let many: Vec<String> = (0..40).map(|i| format!("k{i}=v")).collect();
        assert!(parse_tags(&many).is_err());
        assert!(matches_tags(&t, &BTreeMap::from([("env".into(), "prod".into())])));
        assert!(matches_tags(&t, &BTreeMap::from([("env".into(), String::new())])), "bare key matches any value");
        assert!(!matches_tags(&t, &BTreeMap::from([("env".into(), "dev".into())])));
        assert!(!matches_tags(&t, &BTreeMap::from([("missing".into(), String::new())])));
    }

    #[test]
    fn patch_applies_and_clears() {
        let mut m = VersionMeta::default();
        MetadataPatch { note: Some(" before upgrade ".into()), pinned: Some(true), pin_reason: Some("audit".into()), ..Default::default() }.apply(&mut m).unwrap();
        assert_eq!((m.note.as_deref(), m.pinned, m.pin_reason.as_deref()), (Some("before upgrade"), true, Some("audit")));
        MetadataPatch { note: Some(String::new()), pinned: Some(false), ..Default::default() }.apply(&mut m).unwrap();
        assert_eq!((m.note.as_deref(), m.pinned, m.pin_reason.as_deref()), (None, false, None), "unpinning clears the reason");
        assert!(MetadataPatch { note: Some("x".repeat(MAX_NOTE_CHARS + 1)), ..Default::default() }.apply(&mut m).is_err());
        assert!(MetadataPatch::default().is_empty());
    }

    #[test]
    fn meta_serializes_compactly_and_reads_old_lists() {
        assert_eq!(serde_json::to_string(&VersionMeta::default()).unwrap(), "{}");
        let fl: FileListVolume = serde_json::from_str(r#"{"timestamp":"2026-01-01T00:00:00Z","partial":false,"files":[]}"#).unwrap();
        assert!(fl.meta.is_empty(), "file lists from before metadata read with defaults");
        let m = VersionMeta { note: Some("n".into()), pinned: true, identity: Some(ClientIdentity { hostname: "pc".into(), ..Default::default() }), ..Default::default() };
        let fl = FileListVolume { meta: m.clone(), ..fl };
        let json = serde_json::to_value(&fl).unwrap();
        assert_eq!(json["note"], "n");
        assert_eq!(json["pinned"], true);
        assert!(json.get("tags").is_none() && json.get("stats").is_none());
        // An older reader (no metadata fields) ignores them.
        #[derive(Deserialize)]
        struct OldList {
            timestamp: String,
        }
        let old: OldList = serde_json::from_value(json.clone()).unwrap();
        assert_eq!(old.timestamp, "2026-01-01T00:00:00Z");
        let back: FileListVolume = serde_json::from_value(json).unwrap();
        assert_eq!(back.meta, m);
    }

    #[test]
    fn identity_source_ids() {
        let a = ClientIdentity { hostname: "Laptop".into(), username: "ann".into(), job_id: "j1".into(), source_label: "Docs".into() };
        let b = ClientIdentity { hostname: "laptop".into(), source_label: "Renamed".into(), ..a.clone() };
        assert_eq!(a.source_id(), b.source_id(), "label and host-name case do not change the source");
        assert_ne!(a.source_id(), ClientIdentity { job_id: "j2".into(), ..a.clone() }.source_id());
        assert_eq!(a.source_id().len(), 16);
        assert_eq!(a.display(), "ann@Laptop (Docs)");
        let r = ClientIdentity::default().resolve();
        assert!(!r.hostname.is_empty() || !r.username.is_empty() || cfg!(not(unix)));
    }

    mod repo {
        use super::super::*;
        use crate::backup::backend::LocalBackend;
        use crate::backup::crypto::KdfParams;
        use crate::backup::db::{BackupDb, DbKey};
        use crate::backup::test_util::write;
        use crate::backup::{maintenance, run_backup, BackupOptions, OpContext, RetentionPolicy, SourceFilter};
        use std::path::Path;

        fn o() -> BackupOptions {
            BackupOptions {
                block_size: 1024,
                volume_size: 4096,
                verify_samples: 0,
                default_note: Some("nightly".into()),
                default_tags: BTreeMap::from([("env".into(), "prod".into())]),
                identity: ClientIdentity { hostname: "old-laptop".into(), username: "ann".into(), job_id: "job-1".into(), source_label: "Docs".into() },
                ..Default::default()
            }
        }

        fn run(src: &Path, be: &LocalBackend, db: &BackupDb, ret: &RetentionPolicy) -> crate::backup::BackupReport {
            let r = run_backup(be, db, &[src.into()], &SourceFilter::default(), &o(), ret, Some("pw"), KdfParams::insecure_fast(), &OpContext::new()).unwrap();
            assert!(r.errors.is_empty(), "{:?}", r.errors);
            r
        }

        fn tick() {
            std::thread::sleep(std::time::Duration::from_millis(1100));
        }

        #[test]
        fn backup_records_note_tags_identity_and_stats_that_survive_a_rebuild() {
            let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            write(src.path(), "a.bin", &(0..6000u32).map(|i| (i % 251) as u8).collect::<Vec<_>>());
            let be = LocalBackend::new(dst.path());
            let db = BackupDb::open_in_memory().unwrap();
            let r = run(src.path(), &be, &db, &RetentionPolicy::KeepAll);
            let v = &db.filesets().unwrap()[0];
            assert_eq!(v.note.as_deref(), Some("nightly"));
            assert_eq!(v.tags.get("env").map(String::as_str), Some("prod"));
            let id = v.identity.clone().unwrap();
            assert_eq!((id.hostname.as_str(), id.username.as_str(), id.job_id.as_str()), ("old-laptop", "ann", "job-1"), "overridden identity");
            assert_eq!(v.source, id.source_id());
            let st = v.stats.unwrap();
            assert_eq!(st.bytes_added, r.bytes_uploaded);
            assert!(st.bytes_added > 0 && st.new_blocks == r.new_blocks && st.files_added == 1);
            tick();
            write(src.path(), "b.txt", b"b");
            let r2 = run(src.path(), &be, &db, &RetentionPolicy::KeepAll);
            let st2 = db.filesets().unwrap()[0].stats.unwrap();
            assert_eq!((st2.files_added, st2.new_blocks, st2.bytes_added), (1, 1, r2.bytes_uploaded));

            // Pin and annotate the older version, then rebuild the database.
            let repo = Repo::open(&be, &db, Some("pw"), &o(), false, KdfParams::insecure_fast()).unwrap();
            let patch = MetadataPatch { note: Some("before migration".into()), pinned: Some(true), pin_reason: Some("audit".into()), tags: Some(BTreeMap::from([("keep".into(), String::new())])) };
            let f = set_version_metadata(&repo, 1, &patch).unwrap();
            assert!(f.pinned && f.note.as_deref() == Some("before migration") && f.tags.contains_key("keep"));
            assert_eq!(f.stats, Some(st), "statistics kept");
            assert!(repo.consistency_check().unwrap().is_clean(), "rewritten list recorded");
            let fresh = BackupDb::open_in_memory().unwrap();
            maintenance::recreate_database(&be, &fresh, Some("pw"), "ufop", &OpContext::new()).unwrap();
            let back = &fresh.filesets().unwrap()[1];
            assert!(back.pinned && back.pin_reason.as_deref() == Some("audit") && back.note.as_deref() == Some("before migration"));
            assert_eq!(back.identity.as_ref().map(|i| i.hostname.as_str()), Some("old-laptop"));
            assert_eq!(filter_by_tags(fresh.filesets().unwrap(), &BTreeMap::from([("keep".into(), String::new())])).len(), 1);
            assert!(set_version_metadata(&repo, 9, &patch).is_err());
        }

        #[test]
        fn pinned_versions_survive_retention_purge_and_plain_delete() {
            let (src, dst) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
            let be = LocalBackend::new(dst.path());
            let db = BackupDb::open_in_memory().unwrap();
            for i in 0..3u8 {
                write(src.path(), "f.txt", &[i; 10]);
                write(src.path(), "secret.log", &[i; 5]);
                run(src.path(), &be, &db, &RetentionPolicy::KeepAll);
                tick();
            }
            let repo = Repo::open(&be, &db, Some("pw"), &o(), false, KdfParams::insecure_fast()).unwrap();
            let oldest = db.filesets().unwrap()[2].timestamp.clone();
            set_version_metadata(&repo, 2, &MetadataPatch { pinned: Some(true), ..Default::default() }).unwrap();
            // Retention keeps the newest plus the pinned one.
            let preview = maintenance::preview_retention(&db, &RetentionPolicy::KeepCount { count: 1 }, false).unwrap();
            assert_eq!((preview.keep, preview.delete), (2, 1));
            assert_eq!(preview.versions[2].summary, "kept: pinned");
            assert_eq!(maintenance::apply_retention(&repo, &RetentionPolicy::KeepCount { count: 1 }, false).unwrap(), 1);
            let left: Vec<String> = db.filesets().unwrap().into_iter().map(|f| f.timestamp).collect();
            assert_eq!(left.len(), 2);
            assert!(left.contains(&oldest));
            // Purge leaves the pinned version alone.
            let p = maintenance::purge(&repo, &["*.log".into()], None, false).unwrap();
            assert_eq!((p.versions_rewritten, p.pinned_skipped), (1, 1));
            assert_eq!(db.files(db.filesets().unwrap()[1].id).unwrap().len(), 3, "pinned version still has secret.log (+ folder)");
            // Deleting it needs force.
            let e = maintenance::delete_versions(&repo, &[1], false).unwrap_err();
            assert!(e.user_message().contains("pinned"), "{e}");
            assert_eq!(maintenance::delete_versions_ex(&repo, &[1], false, true).unwrap(), 1);
            assert_eq!(db.filesets().unwrap().len(), 1);
        }

        #[test]
        fn encrypted_database_seals_notes_and_migrates_from_schema_2() {
            let d = tempfile::tempdir().unwrap();
            let p = d.path().join("job.sqlite");
            {
                let c = rusqlite::Connection::open(&p).unwrap();
                c.execute_batch(
                    "CREATE TABLE remote_volumes (id INTEGER PRIMARY KEY AUTOINCREMENT, name TEXT NOT NULL UNIQUE, kind TEXT NOT NULL, size INTEGER NOT NULL, hash TEXT NOT NULL, state TEXT NOT NULL, created_at TEXT NOT NULL, lock_until TEXT);
                     CREATE TABLE blocks (hash TEXT PRIMARY KEY, size INTEGER NOT NULL, volume_id INTEGER NOT NULL);
                     CREATE TABLE filesets (id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp TEXT NOT NULL UNIQUE, volume_name TEXT NOT NULL, partial INTEGER NOT NULL DEFAULT 0, file_count INTEGER NOT NULL DEFAULT 0, total_size INTEGER NOT NULL DEFAULT 0);
                     CREATE TABLE files (id INTEGER PRIMARY KEY AUTOINCREMENT, fileset_id INTEGER NOT NULL, path TEXT NOT NULL, kind TEXT NOT NULL, size INTEGER NOT NULL, mtime INTEGER, mode INTEGER, hash TEXT, blocks TEXT NOT NULL, symlink_target TEXT, meta TEXT);
                     CREATE TABLE kv (key TEXT PRIMARY KEY, value TEXT NOT NULL);
                     INSERT INTO filesets(timestamp, volume_name) VALUES ('2026-01-01T00:00:00+00:00', 'ufop-20260101T000000Z.dlist');
                     PRAGMA user_version = 2;",
                )
                .unwrap();
            }
            let key = DbKey::generate();
            let mut db = BackupDb::open(&p).unwrap();
            let old = &db.filesets().unwrap()[0];
            assert!(old.stats.is_none() && old.source.is_empty() && !old.pinned, "old versions: no data");
            db.rekey(Some(&key)).unwrap();
            let meta = VersionMeta { note: Some("zzSecretNote".into()), pinned: true, ..Default::default() };
            db.set_fileset_meta(old.id, &meta).unwrap();
            drop(db);
            let mut all = Vec::new();
            for ext in ["", "-wal"] {
                all.extend(std::fs::read(format!("{}{ext}", p.display())).unwrap_or_default());
            }
            assert!(!String::from_utf8_lossy(&all).contains("zzSecretNote"));
            let db = BackupDb::open_with_key(&p, Some(&key)).unwrap();
            assert_eq!(db.filesets().unwrap()[0].meta(), meta);
            assert_eq!(BackupDb::inspect(&p).unwrap().schema_version, crate::backup::db::SCHEMA_VERSION);
            // Bug reports keep the pin but not the note.
            let out = d.path().join("report.sqlite");
            db.export_sanitized(&out).unwrap();
            let r = BackupDb::open(&out).unwrap().filesets().unwrap();
            assert!(r[0].pinned && r[0].note.is_none());
        }
    }
}
