//! Crash-safe drafts for the in-app text editor.
//!
//! While a file has unsaved edits the editor writes a draft here every few
//! seconds; a draft left behind (crash, force-quit, closed with "keep
//! draft") is offered back the next time the file is opened. One JSON file
//! per edited path, named by the path's SHA-256, owner-only.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::PathBuf;

/// Editor content is capped at 1 MB; leave room for JSON escaping.
const MAX_DRAFT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct EditorDraft {
    pub path: String,
    pub content: String,
    /// Modification time of the file when editing started (RFC 3339), so
    /// recovery can say whether the file changed underneath the draft.
    pub base_modified: Option<String>,
    pub saved_at: String,
}

pub struct DraftStore {
    dir: PathBuf,
}

impl DraftStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn default_store() -> Result<Self, AppError> {
        Ok(Self::new(crate::paths::data_dir().join("editor-drafts")))
    }

    fn file_for(&self, path: &str) -> PathBuf {
        self.dir.join(format!("{}.json", hex::encode(Sha256::digest(path.as_bytes()))))
    }

    pub fn save(&self, path: &str, content: String, base_modified: Option<String>) -> Result<EditorDraft, AppError> {
        let draft = EditorDraft {
            path: path.to_string(),
            content,
            base_modified,
            saved_at: chrono::Utc::now().to_rfc3339(),
        };
        let bytes = serde_json::to_vec(&draft).map_err(|e| AppError::internal(e.to_string()))?;
        if bytes.len() > MAX_DRAFT_BYTES {
            return Err(AppError::validation("Draft too large to keep"));
        }
        ufop_core::fsutil::write_private_atomic(&self.file_for(path), &bytes)
            .map_err(|e| AppError::file_op(format!("Cannot save draft: {e}"), "Check free disk space"))?;
        Ok(draft)
    }

    /// The draft for `path`; an unreadable or mismatched file counts as none.
    pub fn load(&self, path: &str) -> Option<EditorDraft> {
        let bytes = std::fs::read(self.file_for(path)).ok()?;
        serde_json::from_slice::<EditorDraft>(&bytes).ok().filter(|d| d.path == path)
    }

    pub fn discard(&self, path: &str) -> Result<(), AppError> {
        match std::fs::remove_file(self.file_for(path)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                Err(AppError::file_op(format!("Cannot remove draft: {e}"), "Check permissions"))
            }
            _ => Ok(()),
        }
    }

    /// Every stored draft, newest first.
    pub fn list(&self) -> Vec<EditorDraft> {
        let mut out: Vec<EditorDraft> = std::fs::read_dir(&self.dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
            .filter_map(|e| std::fs::read(e.path()).ok())
            .filter_map(|b| serde_json::from_slice(&b).ok())
            .collect();
        out.sort_by(|a, b| b.saved_at.cmp(&a.saved_at));
        out
    }
}

async fn with_store<T: Send + 'static>(f: impl FnOnce(&DraftStore) -> Result<T, AppError> + Send + 'static) -> Result<T, AppError> {
    tokio::task::spawn_blocking(move || f(&DraftStore::default_store()?))
        .await
        .map_err(|e| AppError::internal(format!("Draft task failed: {e}")))?
}

#[tauri::command]
pub async fn editor_draft_save(path: String, content: String, base_modified: Option<String>) -> Result<EditorDraft, AppError> {
    with_store(move |s| s.save(&path, content, base_modified)).await
}

#[tauri::command]
pub async fn editor_draft_load(path: String) -> Result<Option<EditorDraft>, AppError> {
    with_store(move |s| Ok(s.load(&path))).await
}

#[tauri::command]
pub async fn editor_draft_discard(path: String) -> Result<(), AppError> {
    with_store(move |s| s.discard(&path)).await
}

#[tauri::command]
pub async fn editor_draft_list() -> Result<Vec<EditorDraft>, AppError> {
    with_store(|s| Ok(s.list())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_load_discard_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let store = DraftStore::new(dir.path());
        assert!(store.load("/a.txt").is_none());
        store.save("/a.txt", "draft one".into(), Some("2026-01-01T00:00:00Z".into())).unwrap();
        store.save("/b.txt", "other".into(), None).unwrap();
        let d = store.load("/a.txt").unwrap();
        assert_eq!((d.content.as_str(), d.base_modified.as_deref()), ("draft one", Some("2026-01-01T00:00:00Z")));
        store.save("/a.txt", "draft two".into(), None).unwrap();
        assert_eq!(store.load("/a.txt").unwrap().content, "draft two");
        assert_eq!(store.list().len(), 2);
        store.discard("/a.txt").unwrap();
        store.discard("/a.txt").unwrap();
        assert!(store.load("/a.txt").is_none());
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn corrupt_or_foreign_files_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let store = DraftStore::new(dir.path());
        std::fs::write(store.file_for("/x"), b"{not json").unwrap();
        assert!(store.load("/x").is_none());
        assert!(store.list().is_empty());
        assert!(store.save("/big", "x".repeat(MAX_DRAFT_BYTES), None).is_err());
    }
}
