//! Pane locations and the transfers between them.
//!
//! A pane shows either a folder on this computer (a plain path, as always)
//! or a folder of a saved connection, written `conn://<connection id>/<path>`
//! where `<path>` is the connector's own path (what `list_remote` returns).
//! Copies and moves between any two locations run as transfer-queue jobs:
//!
//! - this computer → connection: the connector uploads the file;
//! - connection → this computer: it downloads to a hidden sibling, given
//!   the final name when complete (never over a file saved meanwhile);
//! - connection → connection: a server-side copy when both ends are the same
//!   connection and the connector has one, else (or when that copy fails,
//!   e.g. an S3 object over CopyObject's 5 GB) download + upload through a
//!   temporary file on this computer.
//!
//! A write that fails, is cancelled or paused removes what it left at the
//! destination (which the job checked was free before it started), so a
//! retry is not refused by a half-written file. Google documents are
//! exported when they leave their Drive (the name gets the format's
//! extension); a move then keeps the original document.
//!
//! Folders are expanded here into one job per file (folders are created
//! first). A move is copy jobs plus a [`MoveGroup`]: each source file is
//! deleted once its copy completed, and the emptied source folders once every
//! job of the group did. Existing destinations are never overwritten: the
//! top-level names get the keep-both `(copy)` naming (names compared
//! ignoring case, as many providers match them), and a job fails rather
//! than replace a file that appeared meanwhile.

use super::JobControl;
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

    /// `name` inside this folder, refused when `name` is not one plain
    /// name (see [`checked_name`]): names come from remote listings and
    /// must never climb out of the folder.
    pub fn join_checked(&self, name: &str) -> Result<Self, AppError> {
        Ok(self.join(checked_name(name)?))
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

/// A connector folder path in one spelling for comparisons: OneDrive's
/// Graph prefixes (`/drive/root:`, `/drives/<id>/root:`) name the drive
/// root, which panes opened at `/` call `/`; trailing slashes are dropped.
/// Mirrored by `normalizeRemoteFolder` in `src/lib/location.ts`.
pub fn normalize_remote_folder(path: &str) -> String {
    let p = path.trim();
    let graph_rest = |rest: &str| rest.is_empty() || rest.starts_with('/');
    let rest = if let Some(r) = p.strip_prefix("/drive/root:").filter(|r| graph_rest(r)) {
        r
    } else if let Some(r) = p.strip_prefix("/drives/").and_then(|r| r.split_once('/')).and_then(|(_, r)| r.strip_prefix("root:")).filter(|r| graph_rest(r)) {
        r
    } else {
        p
    };
    let rest = rest.trim_end_matches('/');
    if rest.is_empty() {
        "/".to_string()
    } else if rest.starts_with('/') {
        rest.to_string()
    } else {
        format!("/{rest}")
    }
}

/// Whether two connector paths name the same folder.
pub fn same_remote_folder(a: &str, b: &str) -> bool {
    normalize_remote_folder(a) == normalize_remote_folder(b)
}

/// Whether `c` may not appear in a file or folder name written here: path
/// separators (`\` too: one on Windows, and kept out everywhere so a
/// name means the same on every computer), NUL, and on Windows `:` (a
/// drive prefix or an alternate data stream).
fn forbidden_in_name(c: char) -> bool {
    matches!(c, '/' | '\\' | '\0') || (cfg!(windows) && c == ':')
}

/// Names that are not a name of their own: they mean this folder or its parent.
fn reserved_name(name: &str) -> bool {
    matches!(name, "" | "." | "..")
}

/// `name` when it is one plain name, safe to join onto a destination
/// folder; names supplied by a server or a shared folder (Google Drive
/// allows `/` and `..`, a WebDAV href can decode to `..`) are refused when
/// empty, `.`, `..`, or holding a separator or NUL (see [`forbidden_in_name`]).
pub fn checked_name(name: &str) -> Result<&str, AppError> {
    if reserved_name(name) || name.chars().any(forbidden_in_name) {
        return Err(AppError::file_op(
            format!("The name {name:?} cannot be used as a file or folder name here: it is not a plain name and could place the item outside the destination folder."),
            "Rename the item at its source (for example in the provider's web app), then retry.",
        ));
    }
    Ok(name)
}

/// `name` made into one plain name (for caches, where an item must be
/// stored whatever it is called): characters [`checked_name`] refuses
/// become `_`, and `.`, `..` or an empty name become `file`.
pub fn sanitized_name(name: &str) -> String {
    let safe: String = name.chars().map(|c| if forbidden_in_name(c) { '_' } else { c }).collect();
    if reserved_name(&safe) { "file".into() } else { safe }
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
    /// The copy is a conversion (a Google document exported to a file):
    /// a move keeps the original.
    pub converted: bool,
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
pub async fn list(loc: &Location, resolver: &dyn RemoteResolver) -> Result<Vec<FileEntry>, AppError> {
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

/// The name a file gets at the destination, and whether copying it there
/// converts it. A file leaving its connection (to this computer or another
/// connection) is what the connector downloads: Google documents are
/// exported and get the format's extension. Within one connection it is
/// copied as it is (Google Drive copies a document as a document).
async fn dest_name(entry: &FileEntry, source: &Location, dest: &Location, resolver: &dyn RemoteResolver) -> Result<(String, bool), AppError> {
    match source {
        Location::Remote { connection_id, .. } if !entry.is_dir && dest.connection_id() != Some(*connection_id) => {
            let conn = resolver.connector(*connection_id).await?;
            Ok((conn.download_name(entry), conn.converts_on_download(entry)))
        }
        _ => Ok((entry.name.clone(), false)),
    }
}

/// The names `listing` takes, lowercased: many providers match names
/// ignoring case (Dropbox, SFTP/FTP servers on Windows, this computer on
/// Windows and macOS) and write or rename over a name that differs only by
/// case, so a name is free only when no listed name matches it ignoring case.
pub fn taken_names(listing: impl IntoIterator<Item = FileEntry>) -> HashSet<String> {
    listing.into_iter().map(|e| e.name.to_lowercase()).collect()
}

/// `wanted`, or its first keep-both `(copy)` name free in `taken` (see
/// [`taken_names`]); the name chosen is added to `taken`.
pub fn free_name(wanted: &str, taken: &mut HashSet<String>) -> Result<String, AppError> {
    let name = if taken.contains(&wanted.to_lowercase()) { crate::commands::file_ops_commands::next_copy_name(wanted, |n| taken.contains(&n.to_lowercase()))? } else { wanted.to_string() };
    taken.insert(name.to_lowercase());
    Ok(name)
}

/// Expand `items` (copied or moved into `dest_dir`) into folders and files.
/// Top-level names already present in `dest_dir` (ignoring case) get
/// keep-both `(copy)` names, so nothing at the destination is ever replaced.
pub async fn plan_transfer(items: &[LocationItem], dest_dir: &Location, resolver: &dyn RemoteResolver) -> Result<TransferPlan, AppError> {
    let mut taken = taken_names(list(dest_dir, resolver).await?);
    plan_transfer_taking(items, dest_dir, resolver, &mut taken).await
}

/// [`plan_transfer`] with the destination's names already `taken` (see
/// [`taken_names`]); the top-level names chosen are added to it. Every
/// name joined onto a destination is checked ([`checked_name`]) before it
/// is planned.
pub async fn plan_transfer_taking(items: &[LocationItem], dest_dir: &Location, resolver: &dyn RemoteResolver, taken: &mut HashSet<String>) -> Result<TransferPlan, AppError> {
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
        let (wanted, converted) = dest_name(&entry, &source, dest_dir, resolver).await?;
        let name = free_name(checked_name(&wanted)?, taken)?;
        let dest = dest_dir.join_checked(&name)?;
        if item.is_dir {
            if dest_dir.same_root(&source) && is_within(&source, dest_dir) {
                return Err(AppError::validation(format!("Cannot put {} inside itself", item.name)));
            }
            expand_dir(&source, &dest, resolver, &mut plan).await?;
        } else {
            plan.files.push(PlannedFile { source: item.path.clone(), dest: dest.to_location_string(), size: item.size, converted });
        }
        if plan.files.len() > MAX_PLANNED_FILES {
            return Err(AppError::validation(format!("More than {MAX_PLANNED_FILES} files: transfer fewer folders at a time")));
        }
    }
    plan.source_dirs.reverse();
    Ok(plan)
}

/// Whether `inner` is `outer` or below it.
pub fn is_within(outer: &Location, inner: &Location) -> bool {
    match (outer, inner) {
        (Location::Local(o), Location::Local(i)) => std::path::Path::new(i).starts_with(o),
        (Location::Remote { path: o, .. }, Location::Remote { path: i, .. }) => {
            let (o, i) = (normalize_remote_folder(o), normalize_remote_folder(i));
            o == "/" || i == o || i.starts_with(&format!("{o}/"))
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
            let (name, converted) = dest_name(&entry, &src, &dst, resolver).await?;
            let child_dst = dst.join_checked(&name)?;
            if entry.is_dir {
                stack.push((child_src, child_dst));
            } else {
                plan.files.push(PlannedFile { source: child_src.to_location_string(), dest: child_dst.to_location_string(), size: entry.size, converted });
                if plan.files.len() > MAX_PLANNED_FILES {
                    return Err(AppError::validation(format!("More than {MAX_PLANNED_FILES} files: transfer fewer folders at a time")));
                }
            }
        }
    }
    Ok(())
}

/// Create a planned destination folder; returns where it is (a connector
/// may address it differently: Google Drive by the new folder's id).
pub async fn make_dir(loc: &Location, resolver: &dyn RemoteResolver) -> Result<Location, AppError> {
    match loc {
        Location::Local(p) => {
            tokio::fs::create_dir_all(p).await.map_err(|e| AppError::file_op(format!("Cannot create {p}: {e}"), "Check you may write to the destination."))?;
            Ok(loc.clone())
        }
        Location::Remote { connection_id, path } => {
            let made = resolver.connector(*connection_id).await?.make_directory_path(path).await?;
            Ok(Location::Remote { connection_id: *connection_id, path: made })
        }
    }
}

/// Create the plan's destination folders, parents first, and move what is
/// planned below each one under the path its connector gave it. The plan
/// names folders by name; Google Drive addresses them by id, so a folder's
/// children (folders and files) must go under its new id, not its name.
pub async fn create_dirs(plan: &mut TransferPlan, resolver: &dyn RemoteResolver) -> Result<(), AppError> {
    // Planned folder (pane path) → created folder, where they differ.
    let mut created: HashMap<String, Location> = HashMap::new();
    for i in 0..plan.dirs.len() {
        let planned = plan.dirs[i].clone();
        let made = make_dir(&rebase(&planned, &created)?, resolver).await?;
        let made_str = made.to_location_string();
        if made_str != planned {
            created.insert(planned, made);
        }
        plan.dirs[i] = made_str;
    }
    if !created.is_empty() {
        for f in &mut plan.files {
            f.dest = rebase(&f.dest, &created)?.to_location_string();
        }
    }
    Ok(())
}

/// Planned pane path `path`, under its parent's created folder when that
/// was created somewhere other than planned.
fn rebase(path: &str, created: &HashMap<String, Location>) -> Result<Location, AppError> {
    let loc = Location::parse(path)?;
    let Location::Remote { connection_id, path: p } = &loc else {
        return Ok(loc);
    };
    let parent = Location::Remote { connection_id: *connection_id, path: remote_parent(p) }.to_location_string();
    let name = p.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
    Ok(match created.get(&parent) {
        Some(made) => made.join(name),
        None => loc,
    })
}

/// Whether a remote `path` exists: its parent lists an entry with that path
/// or (for id-path providers such as Google Drive) that name.
pub async fn remote_exists(conn: &dyn Connector, path: &str) -> Result<bool, AppError> {
    let listing = conn.list_remote(&remote_parent(path)).await?;
    Ok(listing.iter().any(|e| names(e, path)))
}

/// Whether something other than the item at `from` is at `to` (by path or
/// name, as [`remote_exists`], names ignoring case: case-insensitive servers
/// would replace `b.txt` with a rename to `B.txt`): a rename to `to` would
/// replace it. The item itself does not count, so a case-only rename stays
/// possible.
pub async fn rename_target_taken(conn: &dyn Connector, from: &str, to: &str) -> Result<bool, AppError> {
    let listing = conn.list_remote(&remote_parent(to)).await?;
    let name = last_name(to).to_lowercase();
    Ok(listing.iter().any(|e| (at_path(e, to) || e.name.to_lowercase() == name) && !at_path(e, from)))
}

/// Whether listed `entry` is the item at `path`, or carries its name
/// (ignoring case, as [`taken_names`]).
fn names(entry: &FileEntry, path: &str) -> bool {
    at_path(entry, path) || entry.name.to_lowercase() == last_name(path).to_lowercase()
}

/// The last segment of a connector path.
fn last_name(path: &str) -> &str {
    path.trim_end_matches('/').rsplit('/').next().unwrap_or_default()
}

/// Whether listed `entry` is at connector path `path` (OneDrive lists
/// `/drive/root:/a` for `/a`).
pub fn at_path(entry: &FileEntry, path: &str) -> bool {
    entry.path == path || same_remote_folder(&entry.path, path)
}

/// The hidden sibling a download is written to before it is placed (so a
/// half-written file never has the final name). `tag` (the job's id) makes
/// it the writer's own: two jobs downloading to one name never write one
/// staging file together.
pub fn local_staging_path(dest: &str, tag: impl std::fmt::Display) -> String {
    let p = std::path::Path::new(dest);
    let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    p.with_file_name(format!(".{name}.{tag}{}", crate::connectors::STAGING_SUFFIX)).to_string_lossy().to_string()
}

/// Run one file job between two locations (at least one remote); returns the
/// bytes written. Never replaces an existing destination.
pub async fn transfer_file(job_id: Uuid, source: &Location, dest: &Location, resolver: &dyn RemoteResolver) -> Result<u64, AppError> {
    transfer_file_until(job_id, source, dest, resolver, &JobControl::default())
        .await?
        .ok_or_else(|| AppError::internal("The transfer stopped"))
}

/// [`transfer_file`], stopped as soon as `control` is cancelled or paused:
/// `Ok(None)`, with what it had written removed. A server-side copy is
/// waited for first (the server completes it anyway), then removed.
pub async fn transfer_file_until(job_id: Uuid, source: &Location, dest: &Location, resolver: &dyn RemoteResolver, control: &JobControl) -> Result<Option<u64>, AppError> {
    match (source, dest) {
        (Location::Local(l), Location::Remote { connection_id, path }) => {
            let conn = resolver.connector(*connection_id).await?;
            let before = refuse_existing_remote(conn.as_ref(), path).await?;
            write_remote(control, conn.as_ref(), path, &before, conn.upload(l, path)).await
        }
        (Location::Remote { connection_id, path }, Location::Local(l)) => {
            let conn = resolver.connector(*connection_id).await?;
            download_into_place(job_id, control, conn.as_ref(), path, l).await
        }
        (Location::Remote { connection_id: a, path: from }, Location::Remote { connection_id: b, path: to }) => {
            let (src, dst) = (resolver.connector(*a).await?, resolver.connector(*b).await?);
            let before = refuse_existing_remote(dst.as_ref(), to).await?;
            if a == b {
                // Not raced against a pause or cancel: the server finishes a
                // copy it accepted even when nobody waits for the answer, so
                // it is waited for and then its result removed, and a
                // resumed job starts over instead of finding it there.
                let copied = src.server_copy(from, to).await;
                let stopped = control.is_cancelled() || control.is_paused();
                if stopped || copied.is_err() {
                    remove_partial(dst.as_ref(), to, &before).await;
                }
                match copied {
                    _ if stopped => return Ok(None),
                    Ok(true) => return Ok(Some(0)),
                    Ok(false) => {}
                    // Not possible for this file (S3 CopyObject stops at
                    // 5 GB; some WebDAV servers refuse COPY): relay it.
                    Err(e) => {
                        tracing::info!("Server-side copy of {from} failed ({e}); copying it through this computer");
                        refuse_conversion(src.as_ref(), from).await?;
                    }
                }
            }
            relay(job_id, control, src.as_ref(), from, dst.as_ref(), to, &before).await
        }
        (Location::Local(_), Location::Local(_)) => Err(AppError::validation("Both ends are on this computer: the local queue runs this job")),
    }
}

/// `op`, unless `control` is cancelled or paused first (`None`: `op` was
/// dropped, mid-way or before it started).
async fn unless_stopped<T>(control: &JobControl, op: impl std::future::Future<Output = Result<T, AppError>>) -> Result<Option<T>, AppError> {
    tokio::select! {
        biased;
        _ = control.stopped() => Ok(None),
        r = op => r.map(Some),
    }
}

/// Run `op`, which writes remote `path`: if it fails or is stopped, remove
/// what it left there. `before` is what [`refuse_existing_remote`] saw in
/// the folder (`path` was free then).
async fn write_remote<T>(control: &JobControl, conn: &dyn Connector, path: &str, before: &HashSet<String>, op: impl std::future::Future<Output = Result<T, AppError>>) -> Result<Option<T>, AppError> {
    let result = unless_stopped(control, op).await;
    if !matches!(result, Ok(Some(_))) {
        remove_partial(conn, path, before).await;
    }
    result
}

/// Best-effort removal of a file a failed or stopped write left at `path`.
/// The file at `path` itself is removed. Where items are listed by id
/// (Google Drive) the write's own item can only be found by name: a file of
/// that name is removed only when it was not in the folder before the write
/// (`before`) and it is the only one, so nothing anyone else put there is.
async fn remove_partial(conn: &dyn Connector, path: &str, before: &HashSet<String>) {
    let listing = match conn.list_remote(&remote_parent(path)).await {
        Ok(listing) => listing,
        Err(e) => return tracing::warn!("Could not check for an incomplete {path}: {e}"),
    };
    let files: Vec<FileEntry> = listing.into_iter().filter(|e| !e.is_dir && !before.contains(&e.path)).collect();
    let target = match files.iter().find(|e| at_path(e, path)) {
        Some(exact) => exact,
        None => {
            let named: Vec<&FileEntry> = files.iter().filter(|e| e.name == last_name(path)).collect();
            match named[..] {
                [only] => only,
                [] => return,
                _ => return tracing::warn!("Several new files are named like the incomplete {path}; none was removed"),
            }
        }
    };
    if let Err(e) = conn.remove(&target.path, false).await {
        tracing::warn!("Could not remove the incomplete {path}: {e}");
    }
}

/// Relaying `from` within its connection would convert it (a Google
/// document whose server-side copy failed would come back as an exported
/// file): refuse instead.
async fn refuse_conversion(conn: &dyn Connector, from: &str) -> Result<(), AppError> {
    let listing = conn.list_remote(&remote_parent(from)).await?;
    match listing.iter().find(|e| e.path == from) {
        Some(entry) if conn.converts_on_download(entry) => Err(AppError::file_op(
            format!("{} could not be copied on the server, and copying it through this computer would convert it.", entry.name),
            "Retry later, or copy it in the provider's web app.",
        )),
        _ => Ok(()),
    }
}

/// Refuse a write to `path` when something is there already; otherwise the
/// paths its folder lists now (for [`remove_partial`]).
async fn refuse_existing_remote(conn: &dyn Connector, path: &str) -> Result<HashSet<String>, AppError> {
    let listing = conn.list_remote(&remote_parent(path)).await?;
    if listing.iter().any(|e| names(e, path)) {
        return Err(AppError::file_op(format!("{path} already exists; it was not overwritten."), "Rename or remove it, then retry the transfer."));
    }
    Ok(listing.into_iter().map(|e| e.path).collect())
}

/// Download `remote` to `local` through the job's own staging file, placed
/// without replacing anything (see [`place_without_replacing`]).
async fn download_into_place(job_id: Uuid, control: &JobControl, conn: &dyn Connector, remote: &str, local: &str) -> Result<Option<u64>, AppError> {
    // Planned names are plain names (checked_name); a path that still
    // climbs (`..`) or has no name of its own is refused, never written.
    let path = std::path::Path::new(local);
    if path.file_name().is_none() || path.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::CurDir)) {
        return Err(AppError::file_op(format!("{local} is not a plain destination path; nothing was written."), "Choose the destination folder again and retry."));
    }
    if tokio::fs::metadata(local).await.is_ok() {
        return Err(AppError::file_op(format!("{local} already exists; it was not overwritten."), "Rename or remove it, then retry the transfer."));
    }
    if let Some(parent) = std::path::Path::new(local).parent() {
        tokio::fs::create_dir_all(parent).await.map_err(|e| AppError::file_op(format!("Cannot create the destination folder: {e}"), "Check you may write there."))?;
    }
    let staging = local_staging_path(local, job_id);
    // Never write through something already there (a leftover or a link).
    let _ = tokio::fs::remove_file(&staging).await;
    let bytes = match unless_stopped(control, conn.download(remote, &staging)).await {
        Ok(Some(b)) => b,
        other => {
            let _ = tokio::fs::remove_file(&staging).await;
            return other;
        }
    };
    let placed = place_without_replacing(&staging, local).await;
    let _ = tokio::fs::remove_file(&staging).await;
    placed.map(|()| Some(bytes))
}

/// Give the finished `staging` file its final name `local`, failing if a
/// file of that name appeared since the download started (a rename would
/// replace it): a hard link, which never replaces, then the staging name
/// is dropped by the caller. Where links are not supported (FAT, some
/// network shares) `local` is re-checked right before a rename.
async fn place_without_replacing(staging: &str, local: &str) -> Result<(), AppError> {
    let appeared = || AppError::file_op(format!("{local} appeared during the download; it was not overwritten."), "Rename or remove it, then retry the transfer.");
    match tokio::fs::hard_link(staging, local).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Err(appeared()),
        Err(_) if tokio::fs::symlink_metadata(local).await.is_ok() => Err(appeared()),
        Err(_) => tokio::fs::rename(staging, local).await.map_err(|e| AppError::file_op(format!("Downloaded, but could not move it into place as {local}: {e}"), "Check you may write to the destination folder.")),
    }
}

/// Where relayed files are held: this user's private temporary folder
/// (mode 0700 on Unix, never one another user planted).
pub fn relay_dir() -> std::path::PathBuf {
    ufop_core::session::user_temp_subdir("relay")
}

/// Download to a temporary file on this computer, upload it, delete it.
async fn relay(job_id: Uuid, control: &JobControl, src: &dyn Connector, from: &str, dst: &dyn Connector, to: &str, before: &HashSet<String>) -> Result<Option<u64>, AppError> {
    let dir = relay_dir();
    tokio::fs::create_dir_all(&dir).await.map_err(|e| AppError::file_op(format!("Cannot create a temporary folder: {e}"), "Free some disk space and retry."))?;
    let temp = dir.join(job_id.to_string()).to_string_lossy().to_string();
    // Never write through something already there (a planted link).
    let _ = tokio::fs::remove_file(&temp).await;
    let result = async {
        if unless_stopped(control, src.download(from, &temp)).await?.is_none() {
            return Ok(None);
        }
        write_remote(control, dst, to, before, dst.upload(&temp, to)).await
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
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
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
        // OneDrive's Graph prefixes name the drive root.
        for (a, b) in [("/drive/root:", "/"), ("/drive/root:/", ""), ("/drives/LIB1/root:/Q3", "/Q3/"), ("/drive/root:/Docs", "/Docs")] {
            assert!(same_remote_folder(a, b), "{a} = {b}");
        }
        for (a, b) in [("/drive/rootx:/Docs", "/Docs"), ("/drive/root:/Docs", "/"), ("/a", "/b")] {
            assert!(!same_remote_folder(a, b), "{a} != {b}");
        }
        let (id, r) = (Uuid::new_v4(), |id: Uuid, p: &str| Location::Remote { connection_id: id, path: p.into() });
        assert!(is_within(&r(id, "/drive/root:/Docs"), &r(id, "/Docs/sub")));
        assert!(is_within(&r(id, "/"), &r(id, "/drive/root:/Docs")));
        assert!(!is_within(&r(id, "/drive/root:/Docs"), &r(id, "/")));
        assert_eq!(local_staging_path("/d/a.txt", "j1"), "/d/.a.txt.j1.ufop-part");
    }

    #[test]
    fn only_plain_names_are_joined() {
        for ok in ["a.txt", "..a", "a..", "...", ".hidden", "a b (copy).txt", "名前.pdf"] {
            assert_eq!(checked_name(ok).unwrap(), ok);
        }
        for bad in ["", ".", "..", "../x", "a/b", "/abs", "a\\b", "..\\x", "a\0b"] {
            let err = checked_name(bad).unwrap_err();
            assert!(err.to_string().contains("cannot be used"), "{bad:?}: {err}");
        }
        assert_eq!(checked_name("C:x").is_err(), cfg!(windows), "':' only matters on Windows");
        assert_eq!(sanitized_name("../../etc"), ".._.._etc");
        for reserved in ["", ".", ".."] {
            assert_eq!(sanitized_name(reserved), "file");
        }
        let dir = Location::Local("/d".into());
        assert!(dir.join_checked("..").is_err());
        assert_eq!(dir.join_checked("a").unwrap(), Location::Local("/d/a".into()));
    }

    /// An in-memory connection: a map of path → contents (folders are paths
    /// ending in `/` in `dirs`). Records server-side copies.
    #[derive(Default)]
    pub(crate) struct MemConnector {
        pub files: std::sync::Mutex<HashMap<String, Vec<u8>>>,
        pub dirs: std::sync::Mutex<HashSet<String>>,
        pub server_copies: std::sync::Mutex<Vec<(String, String)>>,
        /// Server-side renames done (folders move with what is below them).
        pub renames: std::sync::Mutex<Vec<(String, String)>>,
        pub can_server_copy: bool,
        pub fail_upload: bool,
        /// Server-side copies write a partial destination, then fail.
        pub fail_server_copy: bool,
        /// Uploads still to fail (each writes a partial file first).
        pub fail_next_uploads: std::sync::atomic::AtomicUsize,
        /// Uploads write a partial file, wait this long, then complete.
        pub upload_delay_ms: std::sync::atomic::AtomicU64,
        /// Uploads running now, and the most ever at once.
        pub active_uploads: std::sync::atomic::AtomicUsize,
        pub peak_uploads: std::sync::atomic::AtomicUsize,
        /// Paths listed as Google-style native documents (`gdoc`): they
        /// download as `<name>.docx` exports.
        pub native: std::sync::Mutex<HashSet<String>>,
    }

    /// Counts an upload as running until dropped (also when cancelled).
    struct Running<'a>(&'a std::sync::atomic::AtomicUsize);

    impl Drop for Running<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        }
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
                let native = self.native.lock().unwrap().clone();
                let mut out: Vec<FileEntry> = self
                    .files
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|(p, _)| child(p))
                    .map(|(p, d)| FileEntry { extension: native.contains(p.as_str()).then(|| "gdoc".to_string()), ..entry(p, false, d.len() as u64) })
                    .collect();
                out.extend(self.dirs.lock().unwrap().iter().filter(|p| child(p)).map(|p| entry(p, true, 0)));
                out.sort_by(|a, b| a.path.cmp(&b.path));
                Ok(out)
            })
        }
        fn upload(&self, local: &str, remote: &str) -> Fut<'_, u64> {
            let (l, r) = (local.to_string(), remote.to_string());
            Box::pin(async move {
                use std::sync::atomic::Ordering::SeqCst;
                if self.fail_upload {
                    return Err(AppError::connection("upload refused", "test"));
                }
                let now = self.active_uploads.fetch_add(1, SeqCst) + 1;
                let _running = Running(&self.active_uploads);
                self.peak_uploads.fetch_max(now, SeqCst);
                let delay = self.upload_delay_ms.load(SeqCst);
                let failing = self.fail_next_uploads.fetch_update(SeqCst, SeqCst, |n| n.checked_sub(1)).is_ok();
                if delay > 0 || failing {
                    self.files.lock().unwrap().insert(r.clone(), b"part".to_vec());
                }
                if delay > 0 {
                    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
                }
                if failing {
                    return Err(AppError::connection("connection lost mid-upload", "test"));
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
                if self.dirs.lock().unwrap().remove(&f) {
                    // A folder: everything below it moves along.
                    let under = |p: &str| p.strip_prefix(&format!("{f}/")).map(|rest| format!("{t}/{rest}"));
                    let mut files = self.files.lock().unwrap();
                    let moved: Vec<_> = files.keys().filter_map(|p| under(p).map(|n| (p.clone(), n))).collect();
                    for (old, new) in moved {
                        let data = files.remove(&old).unwrap_or_default();
                        files.insert(new, data);
                    }
                    let mut dirs = self.dirs.lock().unwrap();
                    let moved: Vec<_> = dirs.iter().filter_map(|p| under(p).map(|n| (p.clone(), n))).collect();
                    for (old, new) in moved {
                        dirs.remove(&old);
                        dirs.insert(new);
                    }
                    dirs.insert(t.clone());
                } else {
                    let data = self.files.lock().unwrap().remove(&f).ok_or_else(|| AppError::not_found(format!("{f} not found")))?;
                    self.files.lock().unwrap().insert(t.clone(), data);
                }
                self.renames.lock().unwrap().push((f, t));
                Ok(())
            })
        }
        fn server_copy(&self, from: &str, to: &str) -> Fut<'_, bool> {
            let (f, t) = (from.to_string(), to.to_string());
            Box::pin(async move {
                if !self.can_server_copy {
                    return Ok(false);
                }
                if self.fail_server_copy {
                    self.files.lock().unwrap().insert(t.clone(), b"part".to_vec());
                    return Err(AppError::transfer("copy too large for the server", "test"));
                }
                let data = self.files.lock().unwrap().get(&f).cloned().ok_or_else(|| AppError::not_found(format!("{f} not found")))?;
                self.files.lock().unwrap().insert(t.clone(), data);
                self.server_copies.lock().unwrap().push((f, t));
                Ok(true)
            })
        }
        fn download_name(&self, entry: &FileEntry) -> String {
            if self.converts_on_download(entry) { format!("{}.docx", entry.name) } else { entry.name.clone() }
        }
        fn converts_on_download(&self, entry: &FileEntry) -> bool {
            entry.extension.as_deref() == Some("gdoc")
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
        assert_eq!(plan.files[0], PlannedFile { source: a.to_string_lossy().into(), dest: format!("conn://{id}/dst/a (copy).txt"), size: 3, converted: false });
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
        assert!(!std::path::Path::new(&local_staging_path(&l("one.txt"), job)).exists());
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
        assert!(!relay_dir().join(job.to_string()).exists());
        assert!(relay_dir().starts_with(ufop_core::session::user_temp_dir().unwrap()), "a private folder of this user");
        // A failed download leaves no staging file behind.
        assert!(transfer_file(job, &ra("/r/missing.txt"), &Location::Local(l("m.txt")), &resolver).await.is_err());
        assert!(!std::path::Path::new(&local_staging_path(&l("m.txt"), job)).exists());
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

    /// Regression: a server-side copy that failed (an S3 object over
    /// CopyObject's 5 GB, a WebDAV server refusing COPY) failed the job
    /// instead of copying through this computer.
    #[tokio::test]
    async fn a_failed_server_copy_is_relayed_and_leaves_nothing_behind() {
        let id = Uuid::new_v4();
        let conn = Arc::new(MemConnector { can_server_copy: true, fail_server_copy: true, ..MemConnector::with(&[("/r/big.bin", "big"), ("/r/Plan", "exported")], &["/r", "/o"]) });
        let resolver = MemResolver(HashMap::from([(id, conn.clone())]));
        let r = |p: &str| Location::Remote { connection_id: id, path: p.into() };
        assert_eq!(transfer_file(Uuid::new_v4(), &r("/r/big.bin"), &r("/o/big.bin"), &resolver).await.unwrap(), 3);
        assert_eq!(conn.file("/o/big.bin").as_deref(), Some("big"), "relayed, over the failed copy's partial file");
        // A Google-style document would come back converted: refused, and
        // the failed copy's partial file is removed.
        conn.native.lock().unwrap().insert("/r/Plan".into());
        let err = transfer_file(Uuid::new_v4(), &r("/r/Plan"), &r("/o/Plan"), &resolver).await.unwrap_err();
        assert!(err.to_string().contains("convert"), "{err}");
        assert!(conn.file("/o/Plan").is_none());
    }

    /// A folder listing as given (paths need not end in the name, as on
    /// Google Drive), recording what is removed.
    struct Listed {
        entries: Vec<(&'static str, &'static str)>,
        removed: std::sync::Mutex<Vec<String>>,
    }

    impl Connector for Listed {
        fn connect(&self, _: &crate::core::types::ConnectionProfile) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn disconnect(&self) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn list_remote(&self, _: &str) -> Fut<'_, Vec<FileEntry>> {
            let out = self
                .entries
                .iter()
                .map(|(path, name)| FileEntry { name: name.to_string(), path: path.to_string(), is_dir: false, is_symlink: false, size: 1, modified: None, created: None, is_hidden: false, extension: None, permissions: None })
                .collect();
            Box::pin(async move { Ok(out) })
        }
        fn remove(&self, remote: &str, _: bool) -> Fut<'_, ()> {
            self.removed.lock().unwrap().push(remote.to_string());
            Box::pin(async { Ok(()) })
        }
    }

    /// Regression: cleanup after a failed or stopped write removed the first
    /// file in the folder with the written name, so on Google Drive (items
    /// listed by id, duplicate names allowed) it could delete a file of that
    /// name that was there before. Only the write's own file goes now.
    #[tokio::test]
    async fn partial_cleanup_removes_only_the_writes_own_file() {
        /// What `remove_partial` removes for a write to `path`, with the
        /// folder listing `entries` now and `before` at the pre-check.
        async fn removed(entries: &[(&'static str, &'static str)], path: &str, before: &[&str]) -> Vec<String> {
            let conn = Listed { entries: entries.to_vec(), removed: Default::default() };
            remove_partial(&conn, path, &before.iter().map(|p| p.to_string()).collect()).await;
            conn.removed.into_inner().unwrap()
        }
        // Path-addressed: the file at the written path.
        assert_eq!(removed(&[("/d/a.txt", "a.txt"), ("/d/b.txt", "b.txt")], "/d/a.txt", &["/d/b.txt"]).await, ["/d/a.txt"]);
        // OneDrive lists `/d/a.txt` in Graph's form.
        assert_eq!(removed(&[("/drive/root:/d/a.txt", "a.txt")], "/d/a.txt", &[]).await, ["/drive/root:/d/a.txt"]);
        // Drive: a same-named file that was there before is kept.
        assert!(removed(&[("/f1/o1", "a.txt")], "/f1/a.txt", &["/f1/o1"]).await.is_empty());
        // Drive: the one new file of that name is the write's.
        assert_eq!(removed(&[("/f1/o1", "a.txt"), ("/f1/n1", "a.txt")], "/f1/a.txt", &["/f1/o1"]).await, ["/f1/n1"]);
        // Drive: two new files of that name: which is ours is unknown, keep both.
        assert!(removed(&[("/f1/n1", "a.txt"), ("/f1/n2", "a.txt")], "/f1/a.txt", &[]).await.is_empty());
        // Nothing of that name: nothing removed.
        assert!(removed(&[("/f1/o1", "b.txt")], "/f1/a.txt", &[]).await.is_empty());
    }

    /// Regression: an upload that failed half-way left a partial file under
    /// the final name; the retry then refused it as "already exists".
    #[tokio::test]
    async fn a_failed_upload_leaves_no_partial_file_and_a_retry_works() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let ca = Arc::new(MemConnector::with(&[("/r/one.txt", "one")], &["/r"]));
        let cb = Arc::new(MemConnector::with(&[], &["/in"]));
        cb.fail_next_uploads.store(2, std::sync::atomic::Ordering::SeqCst);
        let resolver = MemResolver(HashMap::from([(a, ca), (b, cb.clone())]));
        let local = tempfile::tempdir().unwrap();
        let f = local.path().join("f.txt");
        std::fs::write(&f, "full").unwrap();
        let dest = Location::Remote { connection_id: b, path: "/in/f.txt".into() };
        let up = Location::Local(f.to_string_lossy().into());
        assert!(transfer_file(Uuid::new_v4(), &up, &dest, &resolver).await.is_err());
        assert!(cb.file("/in/f.txt").is_none(), "no partial upload under the real name");
        // Relayed between connections: the same.
        let relayed = Location::Remote { connection_id: b, path: "/in/one.txt".into() };
        let from = Location::Remote { connection_id: a, path: "/r/one.txt".into() };
        assert!(transfer_file(Uuid::new_v4(), &from, &relayed, &resolver).await.is_err());
        assert!(cb.file("/in/one.txt").is_none());
        // The retries go through.
        transfer_file(Uuid::new_v4(), &up, &dest, &resolver).await.unwrap();
        transfer_file(Uuid::new_v4(), &from, &relayed, &resolver).await.unwrap();
        assert_eq!((cb.file("/in/f.txt").as_deref(), cb.file("/in/one.txt").as_deref()), (Some("full"), Some("one")));
    }

    /// Regression: a Google document relayed to another connection was
    /// uploaded under its bare name ("Plan", no extension) although its
    /// bytes are an export. A file leaving its connection gets the download
    /// name and is marked converted; within the connection it keeps its own.
    #[tokio::test]
    async fn documents_leaving_their_connection_get_the_export_name() {
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let ca = Arc::new(MemConnector::with(&[("/r/Plan", "doc"), ("/r/n.txt", "n")], &["/r"]));
        ca.native.lock().unwrap().insert("/r/Plan".into());
        let resolver = MemResolver(HashMap::from([(a, ca), (b, Arc::new(MemConnector::with(&[], &["/in"])))]));
        let mut plan_item = item(format!("conn://{a}/r/Plan"), "Plan", false, 3);
        plan_item.extension = Some("gdoc".into());
        let to_b = plan_transfer(&[plan_item.clone(), item(format!("conn://{a}/r/n.txt"), "n.txt", false, 1)], &Location::Remote { connection_id: b, path: "/in".into() }, &resolver).await.unwrap();
        assert_eq!((to_b.files[0].dest.as_str(), to_b.files[0].converted), (format!("conn://{b}/in/Plan.docx").as_str(), true));
        assert!(!to_b.files[1].converted);
        let folder = plan_transfer(&[item(format!("conn://{a}/r"), "r", true, 0)], &Location::Remote { connection_id: b, path: "/in".into() }, &resolver).await.unwrap();
        assert!(folder.files.iter().any(|f| f.dest == format!("conn://{b}/in/r/Plan.docx") && f.converted), "{folder:?}");
        let within = plan_transfer(&[plan_item], &Location::Remote { connection_id: a, path: "/".into() }, &resolver).await.unwrap();
        assert_eq!((within.files[0].dest.as_str(), within.files[0].converted), (format!("conn://{a}/Plan").as_str(), false));
    }

    /// Resolves connection ids to any connector.
    pub(crate) struct DynResolver(pub HashMap<Uuid, Arc<dyn Connector>>);

    impl RemoteResolver for DynResolver {
        fn connector(&self, id: Uuid) -> BoxFuture<'_, Result<Arc<dyn Connector>, AppError>> {
            let c = self.0.get(&id).cloned();
            Box::pin(async move { c.ok_or_else(|| AppError::not_found("no such connection")) })
        }
    }

    /// Regression: "never overwrite" compared names exactly, but Dropbox
    /// (uploads with mode=overwrite) and case-insensitive SFTP/FTP servers
    /// treat `A.txt` and `a.txt` as one file, so copying `A.txt` into a
    /// folder holding `a.txt` replaced it. Names are taken ignoring case.
    #[tokio::test]
    async fn destination_names_are_taken_ignoring_case() {
        let id = Uuid::new_v4();
        let conn = Arc::new(MemConnector::with(&[("/d/a.txt", "old")], &["/d"]));
        let resolver = MemResolver(HashMap::from([(id, conn.clone())]));
        let local = tempfile::tempdir().unwrap();
        let upper = local.path().join("A.txt");
        std::fs::write(&upper, "new").unwrap();
        let plan = plan_transfer(&[item(upper.to_string_lossy().into(), "A.txt", false, 3)], &Location::Remote { connection_id: id, path: "/d".into() }, &resolver).await.unwrap();
        assert_eq!(plan.files[0].dest, format!("conn://{id}/d/A (copy).txt"), "a keep-both name");
        // A direct write onto the other-case name is refused.
        let err = transfer_file(Uuid::new_v4(), &Location::Local(upper.to_string_lossy().into()), &Location::Remote { connection_id: id, path: "/d/A.txt".into() }, &resolver).await.unwrap_err();
        assert!(err.to_string().contains("already exists"), "{err}");
        assert!(remote_exists(conn.as_ref(), "/d/A.TXT").await.unwrap());
        assert_eq!(conn.file("/d/a.txt").as_deref(), Some("old"));
        assert!(conn.file("/d/A.txt").is_none());
    }

    /// A server-side copy the server completes even when nobody waits for
    /// its answer any more (as S3, Drive, WebDAV and Dropbox do).
    struct LateServerCopy(Arc<MemConnector>);

    impl Connector for LateServerCopy {
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
            self.0.list_remote(path)
        }
        fn remove(&self, remote: &str, is_dir: bool) -> Fut<'_, ()> {
            self.0.remove(remote, is_dir)
        }
        fn server_copy(&self, from: &str, to: &str) -> Fut<'_, bool> {
            let (inner, f, t) = (self.0.clone(), from.to_string(), to.to_string());
            let done = tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(80)).await;
                let data = inner.files.lock().unwrap().get(&f).cloned().unwrap_or_default();
                inner.files.lock().unwrap().insert(t, data);
            });
            Box::pin(async move {
                done.await.map_err(|e| AppError::internal(e.to_string()))?;
                Ok(true)
            })
        }
    }

    /// Regression: pausing a server-side copy dropped the request and
    /// cleaned up at once; the server then finished the copy anyway, so the
    /// resumed job was refused ("already exists"). The copy is waited for
    /// and its result removed, so a resumed job starts over.
    #[tokio::test]
    async fn a_stopped_server_copy_is_waited_for_and_removed() {
        let id = Uuid::new_v4();
        let mem = Arc::new(MemConnector::with(&[("/r/a.txt", "a")], &["/r", "/o"]));
        let resolver = DynResolver(HashMap::from([(id, Arc::new(LateServerCopy(mem.clone())) as Arc<dyn Connector>)]));
        let r = |p: &str| Location::Remote { connection_id: id, path: p.into() };
        let control = Arc::new(JobControl::default());
        let pauser = control.clone();
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            pauser.pause();
        });
        assert_eq!(transfer_file_until(Uuid::new_v4(), &r("/r/a.txt"), &r("/o/a.txt"), &resolver, &control).await.unwrap(), None);
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
        assert!(mem.file("/o/a.txt").is_none(), "nothing of the stopped copy is left");
        control.unpause();
        assert_eq!(transfer_file_until(Uuid::new_v4(), &r("/r/a.txt"), &r("/o/a.txt"), &resolver, &control).await.unwrap(), Some(0), "the resumed copy is not refused");
        assert_eq!(mem.file("/o/a.txt").as_deref(), Some("a"));
    }

    /// A download during which a file of the destination's name appears
    /// (saved by the user or a sync tool).
    struct AppearingDuringDownload(std::path::PathBuf);

    impl Connector for AppearingDuringDownload {
        fn connect(&self, _: &crate::core::types::ConnectionProfile) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn disconnect(&self) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn list_remote(&self, _: &str) -> Fut<'_, Vec<FileEntry>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn download(&self, _: &str, local: &str) -> Fut<'_, u64> {
            let (local, appear) = (local.to_string(), self.0.clone());
            Box::pin(async move {
                std::fs::write(&local, "downloaded").map_err(|e| AppError::file_op(e.to_string(), ""))?;
                std::fs::write(&appear, "saved meanwhile").map_err(|e| AppError::file_op(e.to_string(), ""))?;
                Ok(10)
            })
        }
    }

    /// Regression: the downloaded file was renamed into place, which
    /// replaces a file saved at the destination during the download.
    #[tokio::test]
    async fn a_file_saved_during_a_download_is_not_replaced() {
        let id = Uuid::new_v4();
        let local = tempfile::tempdir().unwrap();
        let dest = local.path().join("a.txt");
        let resolver = DynResolver(HashMap::from([(id, Arc::new(AppearingDuringDownload(dest.clone())) as Arc<dyn Connector>)]));
        let dest_str = dest.to_string_lossy().to_string();
        let job = Uuid::new_v4();
        let err = transfer_file(job, &Location::Remote { connection_id: id, path: "/a.txt".into() }, &Location::Local(dest_str.clone()), &resolver).await.unwrap_err();
        assert!(err.to_string().contains("not overwritten"), "{err}");
        assert_eq!(std::fs::read_to_string(&dest).unwrap(), "saved meanwhile");
        assert!(!std::path::Path::new(&local_staging_path(&dest_str, job)).exists(), "the staging file is removed");
    }

    /// A connection listing whatever names a server or a shared folder
    /// supplies: `folders` maps a listed path to `(path, name, is_dir)`.
    #[derive(Default)]
    pub(crate) struct Hostile {
        pub folders: HashMap<String, Vec<(String, String, bool)>>,
    }

    /// Listed `(path, name, is_dir)` entries.
    pub(crate) type Listing<'a> = &'a [(&'a str, &'a str, bool)];

    impl Hostile {
        pub fn with(folders: &[(&str, Listing<'_>)]) -> Self {
            Self { folders: folders.iter().map(|(p, es)| (p.to_string(), es.iter().map(|(a, b, d)| (a.to_string(), b.to_string(), *d)).collect())).collect() }
        }
    }

    impl Connector for Hostile {
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
            let out = self
                .folders
                .get(path)
                .into_iter()
                .flatten()
                .map(|(path, name, is_dir)| FileEntry { name: name.clone(), path: path.clone(), is_dir: *is_dir, is_symlink: false, size: 1, modified: None, created: None, is_hidden: false, extension: None, permissions: None })
                .collect();
            Box::pin(async move { Ok(out) })
        }
        fn download(&self, _: &str, local: &str) -> Fut<'_, u64> {
            let local = local.to_string();
            Box::pin(async move {
                std::fs::write(&local, "payload").map_err(|e| AppError::file_op(e.to_string(), ""))?;
                Ok(7)
            })
        }
    }

    /// Regression: names a remote listing supplies (a Google Drive name may
    /// hold `/` and `..`, a WebDAV href may decode to `..`; on Windows `\`
    /// separates too) were joined onto local destination paths unchecked, so
    /// a download could write outside the chosen folder (e.g. into
    /// `~/.config/autostart`). Such names are refused before anything is
    /// planned, created or written.
    #[tokio::test]
    async fn remote_names_cannot_leave_the_destination_folder() {
        let id = Uuid::new_v4();
        let hostile = Hostile::with(&[
            ("/shared", &[("/shared/f1", "../../escape.txt", false)]),
            ("/dotdot", &[("/dotdot/d1", "..", true)]),
            ("/d1", &[("/d1/f2", "inside-parent.txt", false)]),
            ("/abs", &[("/abs/f3", "/tmp/abs.txt", false)]),
            ("/back", &[("/back/f4", "..\\..\\win.txt", false)]),
            ("/nul", &[("/nul/f5", "a\0b", false)]),
        ]);
        let resolver = DynResolver(HashMap::from([(id, Arc::new(hostile) as Arc<dyn Connector>)]));
        let root = tempfile::tempdir().unwrap();
        let dst = root.path().join("dst");
        std::fs::create_dir_all(&dst).unwrap();
        let dest = Location::Local(dst.to_string_lossy().into());
        for folder in ["shared", "dotdot", "abs", "back", "nul"] {
            let err = plan_transfer(&[item(format!("conn://{id}/{folder}"), folder, true, 0)], &dest, &resolver).await;
            assert!(err.is_err(), "{folder}: {err:?}");
        }
        // The picked items' own names (sent by the frontend) are checked too.
        for name in ["..", ".", "", "../up.txt", "/etc/x", "a\\b"] {
            assert!(plan_transfer(&[item(format!("conn://{id}/shared/f1"), name, false, 1)], &dest, &resolver).await.is_err(), "{name:?}");
        }
        // A direct job onto a path that climbs out is refused as well.
        let climbing = format!("{}/sub/../../escape.txt", dst.to_string_lossy());
        assert!(transfer_file(Uuid::new_v4(), &Location::Remote { connection_id: id, path: "/shared/f1".into() }, &Location::Local(climbing), &resolver).await.is_err());
        let left: Vec<_> = std::fs::read_dir(root.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("dst")], "nothing was written outside dst");
        assert_eq!(std::fs::read_dir(&dst).unwrap().count(), 0);
    }

    /// A download in a fixed order with its racing twin: the `first`
    /// writes its bytes, then the other writes its own, then the first
    /// finishes (and is placed) while the other is still running.
    struct Racing {
        data: &'static str,
        first: bool,
        barrier: Arc<tokio::sync::Barrier>,
    }

    impl Connector for Racing {
        fn connect(&self, _: &crate::core::types::ConnectionProfile) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn disconnect(&self) -> Fut<'_, ()> {
            Box::pin(async { Ok(()) })
        }
        fn is_connected(&self) -> bool {
            true
        }
        fn list_remote(&self, _: &str) -> Fut<'_, Vec<FileEntry>> {
            Box::pin(async { Ok(Vec::new()) })
        }
        fn download(&self, _: &str, local: &str) -> Fut<'_, u64> {
            let (local, data, first, barrier) = (local.to_string(), self.data, self.first, self.barrier.clone());
            Box::pin(async move {
                let write = || std::fs::write(&local, data).map_err(|e| AppError::file_op(e.to_string(), ""));
                if first {
                    write()?;
                }
                barrier.wait().await;
                if !first {
                    write()?;
                }
                barrier.wait().await;
                if !first {
                    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
                }
                Ok(data.len() as u64)
            })
        }
    }

    /// Regression: every download to one local name was staged in the same
    /// `.<name>.ufop-part`, so two jobs planning the same destination (e.g.
    /// two moves from different connections into one folder) wrote one file
    /// together; one placed the mix and completed (its move then deleted the
    /// source) while the other failed. Each job stages in its own file: one
    /// wins with exactly its bytes, the other is refused.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_downloads_to_one_name_never_mix() {
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let (a, b) = (Uuid::new_v4(), Uuid::new_v4());
        let resolver = Arc::new(DynResolver(HashMap::from([
            (a, Arc::new(Racing { data: "AAAAaaaa", first: true, barrier: barrier.clone() }) as Arc<dyn Connector>),
            (b, Arc::new(Racing { data: "BBBBbbbb", first: false, barrier }) as Arc<dyn Connector>),
        ])));
        let local = tempfile::tempdir().unwrap();
        let dest = Location::Local(local.path().join("report.pdf").to_string_lossy().into());
        let run = |id: Uuid| {
            let (resolver, dest) = (resolver.clone(), dest.clone());
            tokio::spawn(async move { transfer_file_until(Uuid::new_v4(), &Location::Remote { connection_id: id, path: "/report.pdf".into() }, &dest, resolver.as_ref(), &JobControl::default()).await })
        };
        let (ra, rb) = (run(a), run(b));
        let (ra, rb) = (ra.await.unwrap(), rb.await.unwrap());
        let content = std::fs::read_to_string(local.path().join("report.pdf")).unwrap();
        assert!(matches!(ra, Ok(Some(8))), "the first placed wins: {ra:?}");
        assert_eq!(content, "AAAAaaaa", "the winner's bytes, never the other job's");
        let err = rb.unwrap_err();
        assert!(err.to_string().contains("not overwritten"), "{err}");
        let left: Vec<_> = std::fs::read_dir(local.path()).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().to_string()).collect();
        assert_eq!(left, vec!["report.pdf".to_string()], "no staging file is left");
    }
}
