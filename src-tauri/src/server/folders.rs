//! Folder policies for folders ufop-server hosts (under `--share-root`):
//!
//! - **lock**: the folder takes no uploads and no upload links (its
//!   structure and contents stay as they are);
//! - allowed / blocked file extensions and an upload size cap;
//! - link rules: a password is required, links expire within N days;
//! - **intake order**: how uploads are named — `keep` (their name,
//!   `name (2).ext` on a clash), `timestamp` (an arrival-time prefix, so name
//!   order is arrival order) or `sequence` (a per-folder counter prefix).
//!
//! Subfolders inherit: for each setting the nearest folder that sets it wins.
//! A policy can be copied from one folder to another.

use crate::core::error::AppError;
use crate::storage::pool::DbPool;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum IntakeNaming {
    #[default]
    Keep,
    Timestamp,
    Sequence,
}

/// One folder's own settings; `None` = inherit.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct FolderPolicy {
    pub locked: Option<bool>,
    /// Lower-case extensions without the dot; when set, only these are accepted.
    pub allowed_extensions: Option<Vec<String>>,
    pub blocked_extensions: Option<Vec<String>>,
    pub max_upload_mb: Option<i64>,
    pub require_link_password: Option<bool>,
    pub max_link_days: Option<i64>,
    pub intake_naming: Option<IntakeNaming>,
}

/// The settings in force for a folder, and where each came from.
#[derive(Debug, Clone, Serialize, PartialEq, Eq, Default)]
pub struct EffectivePolicy {
    pub locked: bool,
    pub allowed_extensions: Option<Vec<String>>,
    pub blocked_extensions: Vec<String>,
    pub max_upload_mb: Option<i64>,
    pub require_link_password: bool,
    pub max_link_days: Option<i64>,
    pub intake_naming: IntakeNaming,
    /// Folders whose policies contributed, nearest first.
    pub sources: Vec<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct StoredPolicy {
    pub path: String,
    pub policy: FolderPolicy,
    pub updated_at: String,
}

fn bad(m: impl Into<String>) -> AppError {
    AppError::validation(m.into())
}

fn ext_list(v: &Option<Vec<String>>, what: &str) -> Result<Option<Vec<String>>, AppError> {
    let Some(list) = v else { return Ok(None) };
    let mut out = Vec::new();
    for e in list {
        let e = e.trim().trim_start_matches('.').to_ascii_lowercase();
        if e.is_empty() {
            continue;
        }
        if e.len() > 16 || !e.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Err(bad(format!("{what}: '{e}' is not a file extension")));
        }
        if !out.contains(&e) {
            out.push(e);
        }
    }
    Ok(Some(out))
}

impl FolderPolicy {
    /// Normalize and validate.
    pub fn checked(&self) -> Result<Self, AppError> {
        if self.max_upload_mb.is_some_and(|m| !(1..=1024 * 1024).contains(&m)) {
            return Err(bad("The upload limit must be 1 MB–1 TB"));
        }
        if self.max_link_days.is_some_and(|d| !(1..=3650).contains(&d)) {
            return Err(bad("Link expiry must be 1–3650 days"));
        }
        Ok(Self {
            allowed_extensions: ext_list(&self.allowed_extensions, "Allowed extensions")?,
            blocked_extensions: ext_list(&self.blocked_extensions, "Blocked extensions")?,
            ..self.clone()
        })
    }

    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }
}

impl EffectivePolicy {
    /// Whether a file named `name` may be uploaded here.
    pub fn accepts(&self, name: &str) -> Result<(), String> {
        let ext = Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if self.blocked_extensions.iter().any(|b| *b == ext) {
            return Err(format!("{name}: .{ext} files are not accepted here"));
        }
        if let Some(allowed) = &self.allowed_extensions {
            if !allowed.iter().any(|a| *a == ext) {
                return Err(format!("{name}: only {} files are accepted here", allowed.iter().map(|a| format!(".{a}")).collect::<Vec<_>>().join(", ")));
            }
        }
        Ok(())
    }
}

/// The name an upload gets under `naming` (`seq` = the folder's next number).
pub fn intake_name(naming: IntakeNaming, name: &str, now: chrono::DateTime<Utc>, seq: u64) -> String {
    match naming {
        IntakeNaming::Keep => name.to_string(),
        IntakeNaming::Timestamp => format!("{} {name}", now.format("%Y-%m-%dT%H-%M-%S%.3fZ")),
        IntakeNaming::Sequence => format!("{seq:06}-{name}"),
    }
}

/// The next intake number in `dir`: one more than the highest `NNNNNN-` prefix there.
pub fn next_sequence(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    let (num, rest) = n.split_once('-')?;
                    (num.len() == 6 && !rest.is_empty()).then(|| num.parse::<u64>().ok()).flatten()
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
        + 1
}

#[derive(Clone)]
pub struct FolderPolicyStore {
    pool: DbPool,
}

impl FolderPolicyStore {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn list(&self) -> Result<Vec<StoredPolicy>, AppError> {
        self.pool
            .execute(|c| {
                let mut st = c.prepare("SELECT path, settings, updated_at FROM server_folder_policies ORDER BY path")?;
                let v = st
                    .query_map([], |r| {
                        let s: String = r.get(1)?;
                        Ok(StoredPolicy { path: r.get(0)?, policy: serde_json::from_str(&s).unwrap_or_default(), updated_at: r.get(2)? })
                    })?
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(v)
            })
            .await
    }

    pub async fn get(&self, path: &Path) -> Result<Option<FolderPolicy>, AppError> {
        let p = path.to_string_lossy().into_owned();
        self.pool
            .execute(move |c| {
                let s: Option<String> = c.query_row("SELECT settings FROM server_folder_policies WHERE path = ?1", params![p], |r| r.get(0)).optional()?;
                Ok(s.map(|s| serde_json::from_str(&s).unwrap_or_default()))
            })
            .await
    }

    /// Set a folder's own policy (an empty policy removes it).
    pub async fn set(&self, path: &Path, policy: &FolderPolicy) -> Result<FolderPolicy, AppError> {
        let policy = policy.checked()?;
        let (p, json) = (path.to_string_lossy().into_owned(), serde_json::to_string(&policy)?);
        let empty = policy.is_empty();
        self.pool
            .execute(move |c| {
                if empty {
                    c.execute("DELETE FROM server_folder_policies WHERE path = ?1", params![p])?;
                } else {
                    c.execute(
                        "INSERT INTO server_folder_policies (path, settings, updated_at) VALUES (?1, ?2, ?3) ON CONFLICT(path) DO UPDATE SET settings = excluded.settings, updated_at = excluded.updated_at",
                        params![p, json, Utc::now().to_rfc3339()],
                    )?;
                }
                Ok(())
            })
            .await?;
        Ok(policy)
    }

    /// The policy in force for `dir` (canonical), merging the folder's own
    /// settings with those of its ancestors up to `root`.
    pub async fn effective(&self, dir: &Path, root: &Path) -> Result<EffectivePolicy, AppError> {
        let mut chain: Vec<PathBuf> = dir.ancestors().take_while(|a| a.starts_with(root)).map(Path::to_path_buf).collect();
        if chain.is_empty() {
            chain.push(dir.to_path_buf());
        }
        let mut out = EffectivePolicy::default();
        let (mut locked, mut allowed, mut blocked, mut max_up, mut pw, mut days, mut naming) = (None, None, None, None, None, None, None);
        for p in chain {
            let Some(pol) = self.get(&p).await? else { continue };
            out.sources.push(p.to_string_lossy().into_owned());
            locked = locked.or(pol.locked);
            allowed = allowed.or(pol.allowed_extensions);
            blocked = blocked.or(pol.blocked_extensions);
            max_up = max_up.or(pol.max_upload_mb);
            pw = pw.or(pol.require_link_password);
            days = days.or(pol.max_link_days);
            naming = naming.or(pol.intake_naming);
        }
        out.locked = locked.unwrap_or(false);
        out.allowed_extensions = allowed;
        out.blocked_extensions = blocked.unwrap_or_default();
        out.max_upload_mb = max_up;
        out.require_link_password = pw.unwrap_or(false);
        out.max_link_days = days;
        out.intake_naming = naming.unwrap_or_default();
        Ok(out)
    }
}

// ── Admin API ──

use super::{ApiError, ServerState};
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::{Json, Router};
use ufop_core::confine::Roots;

/// The share root containing `path` and the canonical folder (a file's parent).
pub fn locate(st: &ServerState, path: &str) -> Result<(PathBuf, PathBuf), AppError> {
    if st.cfg.share_roots.is_empty() {
        return Err(AppError::state("Sharing is off on this server", "Start ufop-server with --share-root <folder>."));
    }
    let roots = Roots::new(&st.cfg.share_roots).map_err(bad)?;
    let p = roots.resolve(path, true).map_err(bad)?;
    let dir = if p.is_dir() { p } else { p.parent().map(Path::to_path_buf).ok_or_else(|| bad("No folder"))? };
    let root = roots.list().iter().find(|r| dir.starts_with(&r.path)).map(|r| r.path.clone()).ok_or_else(|| bad("Outside the share roots"))?;
    Ok((dir, root))
}

/// The policy in force for a folder (or a file's folder) under the share roots.
pub async fn effective_for(st: &ServerState, path: &Path) -> Result<EffectivePolicy, AppError> {
    let (dir, root) = locate(st, &path.to_string_lossy())?;
    st.auth.folder_policies().effective(&dir, &root).await
}

async fn list(State(st): State<ServerState>) -> Result<Json<serde_json::Value>, ApiError> {
    Ok(Json(serde_json::json!({ "policies": st.auth.folder_policies().list().await? })))
}

#[derive(Deserialize)]
struct PathQuery {
    path: String,
}

async fn effective(State(st): State<ServerState>, Query(q): Query<PathQuery>) -> Result<Json<serde_json::Value>, ApiError> {
    let (dir, root) = locate(&st, &q.path)?;
    let store = st.auth.folder_policies();
    Ok(Json(serde_json::json!({ "path": dir, "own": store.get(&dir).await?, "effective": store.effective(&dir, &root).await? })))
}

#[derive(Deserialize)]
struct SetBody {
    path: String,
    #[serde(default)]
    policy: FolderPolicy,
}

async fn set(State(st): State<ServerState>, Json(b): Json<SetBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let (dir, root) = locate(&st, &b.path)?;
    let store = st.auth.folder_policies();
    let policy = store.set(&dir, &b.policy).await?;
    tracing::info!("Folder policy of {} {}", dir.display(), if policy.is_empty() { "removed" } else { "set" });
    Ok(Json(serde_json::json!({ "path": dir, "own": (!policy.is_empty()).then_some(policy), "effective": store.effective(&dir, &root).await? })))
}

#[derive(Deserialize)]
struct CopyBody {
    from: String,
    to: String,
}

/// Copy a folder's own policy to another folder (replacing that folder's own).
async fn copy(State(st): State<ServerState>, Json(b): Json<CopyBody>) -> Result<Json<serde_json::Value>, ApiError> {
    let (from, _) = locate(&st, &b.from)?;
    let (to, root) = locate(&st, &b.to)?;
    let store = st.auth.folder_policies();
    let policy = store.get(&from).await?.ok_or_else(|| bad(format!("{} has no policy of its own to copy", from.display())))?;
    store.set(&to, &policy).await?;
    tracing::info!("Copied the folder policy of {} to {}", from.display(), to.display());
    Ok(Json(serde_json::json!({ "path": to, "own": policy, "effective": store.effective(&to, &root).await? })))
}

pub fn protected_routes() -> Router<ServerState> {
    Router::new()
        .route("/api/v1/folder-policies", get(list).put(set))
        .route("/api/v1/folder-policies/effective", get(effective))
        .route("/api/v1/folder-policies/copy", post(copy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extensions_and_names() {
        let p = FolderPolicy { allowed_extensions: Some(vec![".PDF".into(), "docx".into(), "pdf".into()]), blocked_extensions: Some(vec!["exe".into()]), ..Default::default() }.checked().unwrap();
        assert_eq!(p.allowed_extensions.as_deref(), Some(&["pdf".to_string(), "docx".to_string()][..]));
        assert!(FolderPolicy { allowed_extensions: Some(vec!["p/df".into()]), ..Default::default() }.checked().is_err());
        assert!(FolderPolicy { max_upload_mb: Some(0), ..Default::default() }.checked().is_err());
        let e = EffectivePolicy { allowed_extensions: p.allowed_extensions.clone(), blocked_extensions: vec!["exe".into()], ..Default::default() };
        assert!(e.accepts("Report.PDF").is_ok());
        assert!(e.accepts("x.exe").unwrap_err().contains(".exe files are not accepted"));
        assert!(e.accepts("notes.txt").unwrap_err().contains("only .pdf, .docx"));
        let t = chrono::DateTime::parse_from_rfc3339("2026-03-04T05:06:07.089Z").unwrap().with_timezone(&Utc);
        assert_eq!(intake_name(IntakeNaming::Timestamp, "a.pdf", t, 0), "2026-03-04T05-06-07.089Z a.pdf");
        assert_eq!(intake_name(IntakeNaming::Sequence, "a.pdf", t, 42), "000042-a.pdf");
        assert_eq!(intake_name(IntakeNaming::Keep, "a.pdf", t, 42), "a.pdf");
        let d = tempfile::tempdir().unwrap();
        assert_eq!(next_sequence(d.path()), 1);
        for n in ["000007-a.pdf", "000012-b.pdf", "12345-x", "abcdef-y", "000099-"] {
            std::fs::write(d.path().join(n), "").unwrap();
        }
        assert_eq!(next_sequence(d.path()), 13);
    }

    #[tokio::test]
    async fn nearest_setting_wins_and_empty_removes() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let s = FolderPolicyStore::new(repo.pool().clone());
        let root = Path::new("/srv/share");
        s.set(root, &FolderPolicy { locked: Some(false), blocked_extensions: Some(vec!["exe".into()]), max_link_days: Some(30), ..Default::default() }).await.unwrap();
        s.set(&root.join("hr"), &FolderPolicy { require_link_password: Some(true), intake_naming: Some(IntakeNaming::Sequence), max_link_days: Some(7), ..Default::default() }).await.unwrap();
        s.set(&root.join("hr/archive"), &FolderPolicy { locked: Some(true), ..Default::default() }).await.unwrap();
        let e = s.effective(&root.join("hr/archive/2025"), root).await.unwrap();
        assert!(e.locked && e.require_link_password);
        assert_eq!((e.max_link_days, e.intake_naming, e.blocked_extensions.clone()), (Some(7), IntakeNaming::Sequence, vec!["exe".to_string()]));
        assert_eq!(e.sources, vec!["/srv/share/hr/archive", "/srv/share/hr", "/srv/share"]);
        let top = s.effective(root, root).await.unwrap();
        assert!(!top.locked && !top.require_link_password && top.max_link_days == Some(30));
        // Nothing above the root is consulted.
        s.set(Path::new("/srv"), &FolderPolicy { locked: Some(true), ..Default::default() }).await.unwrap();
        assert!(!s.effective(root, root).await.unwrap().locked);
        s.set(&root.join("hr"), &FolderPolicy::default()).await.unwrap();
        assert!(s.get(&root.join("hr")).await.unwrap().is_none());
        assert_eq!(s.list().await.unwrap().len(), 3);
    }
}
