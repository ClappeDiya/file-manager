//! Pane locations and the transfers between them.
//!
//! A pane shows either a folder on this computer (a plain path, as always)
//! or a folder of a saved connection, written `conn://<connection id>/<path>`
//! where `<path>` is the connector's own path (what `list_remote` returns).
//! Copies and moves between any two locations run as transfer-queue jobs:
//!
//! - this computer → connection: the connector uploads the file;
//! - connection → this computer: it downloads to a hidden sibling, renamed
//!   into place when complete;
//! - connection → connection: a server-side copy when both ends are the same
//!   connection and the connector has one, else download + upload through a
//!   temporary file on this computer.
//!
//! Folders are expanded here into one job per file (folders are created
//! first). A move is copy jobs plus a [`MoveGroup`]: each source file is
//! deleted once its copy completed, and the emptied source folders once every
//! job of the group did. Existing destinations are never overwritten: the
//! top-level names get the keep-both `(copy)` naming, and a job fails rather
//! than replace a file that appeared meanwhile.

use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::traits::BoxFuture;
use crate::core::types::FileEntry;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use uuid::Uuid;

/// Prefix of a connection location.
pub const CONN_SCHEME: &str = "conn://";

/// Where a pane path points.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// A path on this computer.
    Local(String),
    /// `path` (the connector's path, starting with `/`) of a saved connection.
    Remote { connection_id: Uuid, path: String },
}

impl Location {
    /// `conn://<uuid>/<path>` is a connection location; anything else is a
    /// local path, unchanged.
    pub fn parse(s: &str) -> Result<Self, AppError> {
        let Some(rest) = s.strip_prefix(CONN_SCHEME) else {
            return Ok(Self::Local(s.to_string()));
        };
        let (id, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let connection_id = Uuid::parse_str(id).map_err(|_| AppError::validation(format!("'{s}' does not name a saved connection")))?;
        Ok(Self::Remote { connection_id, path: path.to_string() })
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote { .. })
    }

    /// The pane path form (inverse of [`Location::parse`]).
    pub fn to_location_string(&self) -> String {
        match self {
            Self::Local(p) => p.clone(),
            Self::Remote { connection_id, path } => format!("{CONN_SCHEME}{connection_id}{path}"),
        }
    }

    /// `name` inside this folder.
    pub fn join(&self, name: &str) -> Self {
        match self {
            Self::Local(p) => Self::Local(std::path::Path::new(p).join(name).to_string_lossy().to_string()),
            Self::Remote { connection_id, path } => Self::Remote { connection_id: *connection_id, path: join_remote(path, name) },
        }
    }

    /// Both on this computer, or both on the same connection.
    pub fn same_root(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Local(_), Self::Local(_)) => true,
            (Self::Remote { connection_id: a, .. }, Self::Remote { connection_id: b, .. }) => a == b,
            _ => false,
        }
    }

    pub fn connection_id(&self) -> Option<Uuid> {
        match self {
            Self::Remote { connection_id, .. } => Some(*connection_id),
            Self::Local(_) => None,
        }
    }
}

/// `dir/name` for a connector path.
pub fn join_remote(dir: &str, name: &str) -> String {
    if dir.ends_with('/') {
        format!("{dir}{name}")
    } else {
        format!("{dir}/{name}")
    }
}

/// The parent folder of a connector path (`/` for top-level items).
pub fn remote_parent(path: &str) -> String {
    match path.trim_end_matches('/').rsplit_once('/') {
        Some(("", _)) | None => "/".to_string(),
        Some((dir, _)) => dir.to_string(),
    }
}

/// Signs in to saved connections for the transfer queue.
pub trait RemoteResolver: Send + Sync {
    /// The connector serving saved connection `connection_id`, connected.
    fn connector(&self, connection_id: Uuid) -> BoxFuture<'_, Result<Arc<dyn Connector>, AppError>>;
}

/// One item the user picked (as the pane listed it).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationItem {
    /// Pane path of the item (local path or `conn://…`).
    pub path: String,
    pub name: String,
    pub is_dir: bool,
    #[serde(default)]
    pub size: u64,
    /// Listing extension (Google Drive marks native documents `gdoc`…).
    #[serde(default)]
    pub extension: Option<String>,
}

/// Copy or move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferMode {
    Copy,
    Move,
}

/// A file the plan copies, as pane paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedFile {
    pub source: String,
    pub dest: String,
    pub size: u64,
}

/// Everything a pane transfer does.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransferPlan {
    /// Destination folders to create, parents first.
    pub dirs: Vec<String>,
    pub files: Vec<PlannedFile>,
    /// Source folders a move empties, deepest first.
    pub source_dirs: Vec<String>,
}

/// Most files one pane transfer may expand to.
pub const MAX_PLANNED_FILES: usize = 100_000;

/// Listing of a folder at either kind of location.
async fn list(loc: &Location, resolver: &dyn RemoteResolver) -> Result<Vec<FileEntry>, AppError> {
    match loc {
        Location::Local(p) => {
            let dir = p.clone();
            tokio::task::spawn_blocking(move || -> Result<Vec<FileEntry>, AppError> {
                let mut out = Vec::new();
                let rd = std::fs::read_dir(&dir).map_err(|e| AppError::file_op(format!("Cannot read {dir}: {e}"), "Check the folder exists and you may open it."))?;
                for entry in rd.flatten() {
                    let meta = match entry.metadata() {
                        Ok(m) => m,
                        Err(_) => continue,
                    };
                    let name = entry.file_name().to_string_lossy().to_string();
                    out.push(FileEntry {
                        path: entry.path().to_string_lossy().to_string(),
                        extension: std::path::Path::new(&name).extension().map(|e| e.to_string_lossy().to_string()),
                        name,
                        is_dir: meta.is_dir(),
                        is_symlink: meta.file_type().is_symlink(),
                        size: meta.len(),
                        modified: None,
                        created: None,
                        is_hidden: false,
                        permissions: None,
                    });
                }
                Ok(out)
            })
            .await
            .map_err(|e| AppError::Internal { message: format!("Listing failed: {e}") })?
        }
        Location::Remote { connection_id, path } => resolver.connector(*connection_id).await?.list_remote(path).await,
    }
}

/// The name an item gets at the destination: downloads from a connection
/// use the connector's download name (Google Drive exports get the format's
/// extension).
async fn dest_name(entry: &FileEntry, source: &Location, dest: &Location, resolver: &dyn RemoteResolver) -> Result<String, AppError> {
    match (source, dest) {
        (Location::Remote { connection_id, .. }, Location::Local(_)) if !entry.is_dir => Ok(resolver.connector(*connection_id).await?.download_name(entry)),
        _ => Ok(entry.name.clone()),
    }
}

/// Expand `items` (copied or moved into `dest_dir`) into folders and files.
/// Top-level names already present in `dest_dir` get keep-both `(copy)`
/// names, so nothing at the destination is ever replaced.
pub async fn plan_transfer(items: &[LocationItem], dest_dir: &Location, resolver: &dyn RemoteResolver) -> Result<TransferPlan, AppError> {
    let mut taken: HashSet<String> = list(dest_dir, resolver).await?.into_iter().map(|e| e.name).collect();
    let mut plan = TransferPlan::default();
    for item in items {
        let source = Location::parse(&item.path)?;
        let entry = FileEntry {
            name: item.name.clone(),
            path: item.path.clone(),
            is_dir: item.is_dir,
            is_symlink: false,
            size: item.size,
            modified: None,
            created: None,
            is_hidden: false,
            extension: item.extension.clone(),
            permissions: None,
        };
        let wanted = dest_name(&entry, &source, dest_dir, resolver).await?;
        let name = if taken.contains(&wanted) { crate::commands::file_ops_commands::next_copy_name(&wanted, |n| taken.contains(n))? } else { wanted };
        taken.insert(name.clone());
        let dest = dest_dir.join(&name);
        if item.is_dir {
            if dest_dir.same_root(&source) && is_within(&source, dest_dir) {
                return Err(AppError::validation(format!("Cannot put {} inside itself", item.name)));
            }
            expand_dir(&source, &dest, resolver, &mut plan).await?;
        } else {
            plan.files.push(PlannedFile { source: item.path.clone(), dest: dest.to_location_string(), size: item.size });
        }
        if plan.files.len() > MAX_PLANNED_FILES {
            return Err(AppError::validation(format!("More than {MAX_PLANNED_FILES} files: transfer fewer folders at a time")));
        }
    }
    plan.source_dirs.reverse();
    Ok(plan)
}

/// Whether `inner` is `outer` or below it.
fn is_within(outer: &Location, inner: &Location) -> bool {
    match (outer, inner) {
        (Location::Local(o), Location::Local(i)) => std::path::Path::new(i).starts_with(o),
        (Location::Remote { path: o, .. }, Location::Remote { path: i, .. }) => {
            let o = o.trim_end_matches('/');
            i == o || i.starts_with(&format!("{o}/"))
        }
        _ => false,
    }
}

/// Folders first (parents before children), then every file below, in an
/// iterative walk so deep trees do not recurse.
async fn expand_dir(source: &Location, dest: &Location, resolver: &dyn RemoteResolver, plan: &mut TransferPlan) -> Result<(), AppError> {
    let mut stack = vec![(source.clone(), dest.clone())];
    while let Some((src, dst)) = stack.pop() {
        plan.dirs.push(dst.to_location_string());
        plan.source_dirs.push(src.to_location_string());
        for entry in list(&src, resolver).await? {
            let child_src = match &src {
                Location::Local(_) => Location::Local(entry.path.clone()),
                Location::Remote { connection_id, .. } => Location::Remote { connection_id: *connection_id, path: entry.path.clone() },
            };
            let name = dest_name(&entry, &src, &dst, resolver).await?;
            let child_dst = dst.join(&name);
            if entry.is_dir {
                stack.push((child_src, child_dst));
            } else {
                plan.files.push(PlannedFile { source: child_src.to_location_string(), dest: child_dst.to_location_string(), size: entry.size });
                if plan.files.len() > MAX_PLANNED_FILES {
                    return Err(AppError::validation(format!("More than {MAX_PLANNED_FILES} files: transfer fewer folders at a time")));
                }
            }
        }
    }
    Ok(())
}

/// Create a planned destination folder.
pub async fn make_dir(loc: &Location, resolver: &dyn RemoteResolver) -> Result<(), AppError> {
    match loc {
        Location::Local(p) => tokio::fs::create_dir_all(p).await.map_err(|e| AppError::file_op(format!("Cannot create {p}: {e}"), "Check you may write to the destination.")),
        Location::Remote { connection_id, path } => resolver.connector(*connection_id).await?.make_directory(path).await,
    }
}

/// Whether a remote `path` exists: its parent lists an entry with that path
/// or (for id-path providers such as Google Drive) that name.
pub async fn remote_exists(conn: &dyn Connector, path: &str) -> Result<bool, AppError> {
    let name = path.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    let listing = conn.list_remote(&remote_parent(path)).await?;
    Ok(listing.iter().any(|e| e.path == path || e.name == name))
}

/// The hidden sibling a download is written to before it is renamed into
/// place (so a half-written file never has the final name).
pub fn local_staging_path(dest: &str) -> String {
    let p = std::path::Path::new(dest);
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    p.with_file_name(format!(".{name}{}", crate::connectors::STAGING_SUFFIX)).to_string_lossy().to_string()
}

/// Run one file job between two locations (at least one remote); returns the
/// bytes written. Never replaces an existing destination.
pub async fn transfer_file(job_id: Uuid, source: &Location, dest: &Location, resolver: &dyn RemoteResolver) -> Result<u64, AppError> {
    match (source, dest) {
        (Location::Local(l), Location::Remote { connection_id, path }) => {
            let conn = resolver.connector(*connection_id).await?;
            refuse_existing_remote(conn.as_ref(), path).await?;
            conn.upload(l, path).await
        }
        (Location::Remote { connection_id, path }, Location::Local(l)) => {
            let conn = resolver.connector(*connection_id).await?;
            download_into_place(conn.as_ref(), path, l).await
        }
        (Location::Remote { connection_id: a, path: from }, Location::Remote { connection_id: b, path: to }) => {
            let (src, dst) = (resolver.connector(*a).await?, resolver.connector(*b).await?);
            refuse_existing_remote(dst.as_ref(), to).await?;
            if a == b && src.server_copy(from, to).await? {
                return Ok(0);
            }
            relay(job_id, src.as_ref(), from, dst.as_ref(), to).await
        }
        (Location::Local(_), Location::Local(_)) => Err(AppError::validation("Both ends are on this computer: the local queue runs this job")),
    }
}

async fn refuse_existing_remote(conn: &dyn Connector, path: &str) -> Result<(), AppError> {
    if remote_exists(conn, path).await? {
        return Err(AppError::file_op(format!("{path} already exists; it was not overwritten."), "Rename or remove it, then retry the transfer."));
    }
    Ok(())
}

async fn download_into_place(conn: &dyn Connector, remote: &str, local: &str) -> Result<u64, AppError> {
    if tokio::fs::metadata(local).await.is_ok() {
        return Err(AppError::file_op(format!("{local} already exists; it was not overwritten."), "Rename or remove it, then retry the transfer."));
    }
    if let Some(parent) = std::path::Path::new(local).parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| AppError::file_op(format!("Cannot create the destination folder: {e}"), "Check you may write there."))?;
    }
    let staging = local_staging_path(local);
    let result = conn.download(remote, &staging).await;
    let bytes = match result {
        Ok(b) => b,
        Err(e) => {
            let _ = tokio::fs::remove_file(&staging).await;
            return Err(e);
        }
    };
    if let Err(e) = tokio::fs::rename(&staging, local).await {
        let _ = tokio::fs::remove_file(&staging).await;
        return Err(AppError::file_op(format!("Downloaded, but could not move it into place as {local}: {e}"), "Check you may write to the destination folder."));
    }
    Ok(bytes)
}

/// Download to a temporary file on this computer, upload it, delete it.
async fn relay(job_id: Uuid, src: &dyn Connector, from: &str, dst: &dyn Connector, to: &str) -> Result<u64, AppError> {
    let dir = std::env::temp_dir().join("ufop-relay");
    tokio::fs::create_dir_all(&dir).await.map_err(|e| AppError::file_op(format!("Cannot create a temporary folder: {e}"), "Free some disk space and retry."))?;
    let temp = dir.join(job_id.to_string()).to_string_lossy().to_string();
    let result = async {
        src.download(from, &temp).await?;
        dst.upload(&temp, to).await
    }
    .await;
    let _ = tokio::fs::remove_file(&temp).await;
    result
}

/// Delete a moved file's source (the job's copy is complete).
pub async fn remove_source(source: &Location, resolver: &dyn RemoteResolver) -> Result<(), AppError> {
    match source {
        Location::Local(p) => tokio::fs::remove_file(p).await.map_err(|e| AppError::file_op(format!("Copied, but could not remove {p}: {e}"), "Delete the original yourself.")),
        Location::Remote { connection_id, path } => resolver.connector(*connection_id).await?.remove(path, false).await,
    }
}

/// Remove an emptied source folder of a move (never a non-empty one).
pub async fn remove_empty_dir(dir: &Location, resolver: &dyn RemoteResolver) -> Result<(), AppError> {
    match dir {
        Location::Local(p) => tokio::fs::remove_dir(p).await.map_err(|e| AppError::file_op(format!("Could not remove the folder {p}: {e}"), "It may not be empty.")),
        Location::Remote { connection_id, path } => {
            let conn = resolver.connector(*connection_id).await?;
            if !conn.list_remote(path).await?.is_empty() {
                return Err(AppError::file_op(format!("{path} is not empty; it was kept."), "Check what is still in it."));
            }
            conn.remove(path, true).await
        }
    }
}

/// The jobs of one move: sources are deleted as their copies complete; the
/// source folders once every job completed.
#[derive(Debug, Clone, Default)]
pub struct MoveGroup {
    /// Jobs not finished yet, with the source each one moves.
    pub pending: HashMap<Uuid, String>,
    /// Source folders, deepest first.
    pub source_dirs: Vec<String>,
    /// A job failed or was cancelled: keep the source folders.
    pub incomplete: bool,
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_locations() {
        let id = Uuid::new_v4();
        assert_eq!(Location::parse("/home/a").unwrap(), Location::Local("/home/a".into()));
        assert_eq!(Location::parse(r"C:\Users\a").unwrap(), Location::Local(r"C:\Users\a".into()));
        let r = Location::parse(&format!("conn://{id}/drive/root:/Docs")).unwrap();
        assert_eq!(r, Location::Remote { connection_id: id, path: "/drive/root:/Docs".into() });
        assert_eq!(r.to_location_string(), format!("conn://{id}/drive/root:/Docs"));
        assert_eq!(Location::parse(&format!("conn://{id}")).unwrap(), Location::Remote { connection_id: id, path: "/".into() });
        assert!(Location::parse("conn://not-a-uuid/x").is_err());
        assert_eq!(r.join("a.txt").to_location_string(), format!("conn://{id}/drive/root:/Docs/a.txt"));
        assert_eq!(Location::parse(&format!("conn://{id}/")).unwrap().join("a").to_location_string(), format!("conn://{id}/a"));
        assert!(r.same_root(&Location::parse(&format!("conn://{id}/x")).unwrap()));
        assert!(!r.same_root(&Location::Local("/x".into())));
        assert_eq!((remote_parent("/a/b.txt"), remote_parent("/b.txt"), remote_parent("/a/b/")), ("/a".into(), "/".into(), "/a".into()));
        assert_eq!(local_staging_path("/d/a.txt"), "/d/.a.txt.ufop-part");
    }

    /// An in-memory connection: a map of path → contents (folders are paths
    /// ending in `/` in `dirs`). Records server-side copies.
    #[derive(Default)]
    pub(crate) struct MemConnector {
        pub files: std::sync::Mutex<HashMap<String, Vec<u8>>>,
        pub dirs: std::sync::Mutex<HashSet<String>>,
        pub server_copies: std::sync::Mutex<Vec<(String, String)>>,
        pub can_server_copy: bool,
        pub fail_upload: bool,
    }

    impl MemConnector {
        pub fn with(files: &[(&str, &str)], dirs: &[&str]) -> Self {
            let c = Self::default();
            for (p, d) in files {
                c.files.lock().unwrap().insert(p.to_string(), d.as_bytes().to_vec());
            }
            for d in dirs {
                c.dirs.lock().unwrap().insert(d.to_string());
            }
            c
        }
        pub fn file(&self, p: &str) -> Option<String> {
            self.files.lock().unwrap().get(p).map(|d| String::from_utf8_lossy(d).to_string())
        }
    }

    type Fut<'a, T> = std::pin::Pin<Box<dyn std::future::Future<Output = Result<T, AppError>> + Send + 'a>>;

    impl Connector for MemConnector {
        fn connect(&self, _: &crate::core::types::ConnectionProfile) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn disconnect(&self) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn list_remote(&self, path: &str) -> Fut<'_, Vec<FileEntry>> {
            let dir = path.trim_end_matches('/').to_string();
            Box::pin(async move {
                let entry = |p: &str, is_dir: bool, size: u64| FileEntry {
                    name: p.rsplit('/').next().unwrap_or_default().to_string(),
                    path: p.to_string(),
                    is_dir,
                    is_symlink: false,
                    size,
                    modified: None,
                    created: None,
                    is_hidden: false,
                    extension: None,
                    permissions: None,
                };
                let child = |p: &str| remote_parent(p).trim_end_matches('/') == dir;
                let mut out: Vec<FileEntry> = self.files.lock().unwrap().iter().filter(|(p, _)| child(p)).map(|(p, d)| entry(p, false, d.len() as u64)).collect();
                out.extend(self.dirs.lock().unwrap().iter().filter(|p| child(p)).map(|p| entry(p, true, 0)));
                out.sort_by(|a, b| a.path.cmp(&b.path));
                Ok(out)
            })
        }
        fn upload(&self, local: &str, remote: &str) -> Fut<'_, u64> {
            let (l, r) = (local.to_string(), remote.to_string());
            Box::pin(async move {
                if self.fail_upload {
                    return Err(AppError::connection("upload refused", "test"));
                }
                let data = std::fs::read(&l).map_err(|e| AppError::file_op(e.to_string(), ""))?;
                let n = data.len() as u64;
                self.files.lock().unwrap().insert(r, data);
                Ok(n)
            })
        }
        fn download(&self, remote: &str, local: &str) -> Fut<'_, u64> {
            let (r, l) = (remote.to_string(), local.to_string());
            Box::pin(async move {
                let data = self.files.lock().unwrap().get(&r).cloned().ok_or_else(|| AppError::not_found(format!("{r} not found")))?;
                std::fs::write(&l, &data).map_err(|e| AppError::file_op(e.to_string(), ""))?;
                Ok(data.len() as u64)
            })
        }
        fn make_directory(&self, remote: &str) -> Fut<'_, ()> {
            let r = remote.to_string();
            Box::pin(async move {
                self.dirs.lock().unwrap().insert(r);
                Ok(())
            })
        }
        fn remove(&self, remote: &str, is_dir: bool) -> Fut<'_, ()> {
            let r = remote.to_string();
            Box::pin(async move {
                let gone = if is_dir { self.dirs.lock().unwrap().remove(&r) } else { self.files.lock().unwrap().remove(&r).is_some() };
                if gone { Ok(()) } else { Err(AppError::not_found(format!("{r} not found"))) }
            })
        }
        fn rename_entry(&self, from: &str, to: &str) -> Fut<'_, ()> {
            let (f, t) = (from.to_string(), to.to_string());
            Box::pin(async move {
                let data = self.files.lock().unwrap().remove(&f).ok_or_else(|| AppError::not_found(format!("{f} not found")))?;
                self.files.lock().unwrap().insert(t, data);
                Ok(())
            })
        }
        fn server_copy(&self, from: &str, to: &str) -> Fut<'_, bool> {
            let (f, t) = (from.to_string(), to.to_string());
            Box::pin(async move {
                if !self.can_server_copy {
                    return Ok(false);
                }
                let data = self.files.lock().unwrap().get(&f).cloned().ok_or_else(|| AppError::not_found(format!("{f} not found")))?;
                self.files.lock().unwrap().insert(t.clone(), data);
                self.server_copies.lock().unwrap().push((f, t));
                Ok(true)
            })
        }
    }

    /// Resolves connection ids to in-memory connectors.
    #[derive(Default)]
    pub(crate) struct MemResolver(pub HashMap<Uuid, Arc<MemConnector>>);

    impl RemoteResolver for MemResolver {
        fn connector(&self, id: Uuid) -> BoxFuture<'_, Result<Arc<dyn Connector>, AppError>> {
            let c = self.0.get(&id).cloned();
            Box::pin(async move { c.map(|c| c as Arc<dyn Connector>).ok_or_else(|| AppError::not_found("no such connection")) })
        }
    }

    fn item(path: String, name: &str, is_dir: bool, size: u64) -> LocationItem {
        LocationItem { path, name: name.into(), is_dir, size, extension: None }
    }

    #[tokio::test]
    async fn plans_folders_files_and_keep_both_names() {
        let id = Uuid::new_v4();
        let conn = Arc::new(MemConnector::with(&[("/dst/a.txt", "old"), ("/src/sub/deep.txt", "d"), ("/src/top.txt", "t")], &["/dst", "/src", "/src/sub"]));
        let resolver = MemResolver(HashMap::from([(id, conn)]));
        let local = tempfile::tempdir().unwrap();
        let a = local.path().join("a.txt");
        std::fs::write(&a, "new").unwrap();
        let dest = Location::Remote { connection_id: id, path: "/dst".into() };
        let items = [item(a.to_string_lossy().into(), "a.txt", false, 3), item(format!("conn://{id}/src"), "src", true, 0)];
        let plan = plan_transfer(&items, &dest, &resolver).await.unwrap();
        assert_eq!(plan.files[0], PlannedFile { source: a.to_string_lossy().into(), dest: format!("conn://{id}/dst/a (copy).txt"), size: 3 });
        assert_eq!(plan.dirs, vec![format!("conn://{id}/dst/src"), format!("conn://{id}/dst/src/sub")]);
        let dests: HashSet<_> = plan.files.iter().map(|f| f.dest.clone()).collect();
        assert!(dests.contains(&format!("conn://{id}/dst/src/top.txt")) && dests.contains(&format!("conn://{id}/dst/src/sub/deep.txt")));
        assert_eq!(plan.source_dirs, vec![format!("conn://{id}/src/sub"), format!("conn://{id}/src")], "deepest first");
        // A folder cannot go inside itself.
        let into_self = Location::Remote { connection_id: id, path: "/src/sub".into() };
        assert!(plan_transfer(&items[1..], &into_self, &resolver).await.is_err());
    }

    #[tokio::test]
    async fn every_direction_moves_the_bytes_and_never_overwrites() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let ca = Arc::new(MemConnector { can_server_copy: true, ..MemConnector::with(&[("/r/one.txt", "one")], &["/r"]) });
        let cb = Arc::new(MemConnector::with(&[], &["/in"]));
        let resolver = MemResolver(HashMap::from([(a, ca.clone()), (b, cb.clone())]));
        let local = tempfile::tempdir().unwrap();
        let l = |n: &str| local.path().join(n).to_string_lossy().to_string();
        let ra = |p: &str| Location::Remote { connection_id: a, path: p.into() };
        let rb = |p: &str| Location::Remote { connection_id: b, path: p.into() };
        let job = Uuid::new_v4();

        // Download: lands under the final name only when complete.
        assert_eq!(transfer_file(job, &ra("/r/one.txt"), &Location::Local(l("one.txt")), &resolver).await.unwrap(), 3);
        assert_eq!(std::fs::read_to_string(l("one.txt")).unwrap(), "one");
        assert!(!std::path::Path::new(&local_staging_path(&l("one.txt"))).exists());
        // ...and never replaces an existing file.
        assert!(transfer_file(job, &ra("/r/one.txt"), &Location::Local(l("one.txt")), &resolver).await.is_err());
        // Upload.
        transfer_file(job, &Location::Local(l("one.txt")), &rb("/in/up.txt"), &resolver).await.unwrap();
        assert_eq!(cb.file("/in/up.txt").as_deref(), Some("one"));
        assert!(transfer_file(job, &Location::Local(l("one.txt")), &rb("/in/up.txt"), &resolver).await.is_err(), "no overwrite");
        // Same connection with a server-side copy: no bytes through here.
        transfer_file(job, &ra("/r/one.txt"), &ra("/r/two.txt"), &resolver).await.unwrap();
        assert_eq!(ca.server_copies.lock().unwrap().len(), 1);
        assert_eq!(ca.file("/r/two.txt").as_deref(), Some("one"));
        // Across connections: relayed through a temporary file, removed after.
        transfer_file(job, &ra("/r/one.txt"), &rb("/in/relayed.txt"), &resolver).await.unwrap();
        assert_eq!(cb.file("/in/relayed.txt").as_deref(), Some("one"));
        assert!(!std::env::temp_dir().join("ufop-relay").join(job.to_string()).exists());
        // A failed download leaves no staging file behind.
        assert!(transfer_file(job, &ra("/r/missing.txt"), &Location::Local(l("m.txt")), &resolver).await.is_err());
        assert!(!std::path::Path::new(&local_staging_path(&l("m.txt"))).exists());
    }

    #[tokio::test]
    async fn move_cleanup_helpers() {
        let id = Uuid::new_v4();
        let conn = Arc::new(MemConnector::with(&[("/s/f.txt", "x")], &["/s", "/e"]));
        let resolver = MemResolver(HashMap::from([(id, conn.clone())]));
        let r = |p: &str| Location::Remote { connection_id: id, path: p.into() };
        assert!(remove_empty_dir(&r("/s"), &resolver).await.is_err(), "a non-empty folder is kept");
        remove_source(&r("/s/f.txt"), &resolver).await.unwrap();
        remove_empty_dir(&r("/s"), &resolver).await.unwrap();
        assert!(!conn.dirs.lock().unwrap().contains("/s"));
        assert!(remote_exists(conn.as_ref(), "/e").await.unwrap());
        assert!(!remote_exists(conn.as_ref(), "/s").await.unwrap());
    }
}
