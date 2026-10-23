//! Destination layout of a repository's volumes.
//!
//! - [`RepoLayout::Flat`] (v1 and v2 default): every file in the destination
//!   folder itself.
//! - [`RepoLayout::Sharded`] (v2 only): block (`dblock`) and index (`dindex`)
//!   volumes go into `{shard}/` subfolders, where `shard` is the first
//!   `prefix_len` hex digits of the SHA-256 of the volume name. The header,
//!   file lists, PAR2 and verification files stay at the top. This keeps
//!   folders small on destinations that slow down with many files.
//!
//! The engine always works with *logical* (flat) volume names;
//! [`LayoutBackend`] maps them to physical paths. Sharded layouts need a
//! backend whose [`Backend::supports_subfolders`] is true.
//! [`reshard`] moves an existing repository between layouts.

use super::backend::{Backend, LockMode, Quota, RemoteFile};
use super::volume::{self, RepoHeader, VolumeKind};
use super::{hash_bytes, HashAlgorithm, OpContext, Phase, Progress};
use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RepoLayout {
    #[default]
    Flat,
    /// `prefix_len` (1–4) hex digits per shard folder name.
    Sharded { prefix_len: u8 },
}

impl RepoLayout {
    pub fn validate(self) -> Result<(), AppError> {
        match self {
            Self::Sharded { prefix_len } if !(1..=4).contains(&prefix_len) => Err(AppError::validation("Shard prefix length must be between 1 and 4")),
            _ => Ok(()),
        }
    }

    pub fn is_sharded(self) -> bool {
        matches!(self, Self::Sharded { .. })
    }

    /// Physical path of the logical volume `name`.
    pub fn physical(self, prefix: &str, name: &str) -> String {
        match self {
            Self::Sharded { prefix_len } if matches!(volume::classify(prefix, name), Some(VolumeKind::Block | VolumeKind::Index)) => {
                let h = hash_bytes(HashAlgorithm::Sha256, name.as_bytes());
                format!("{}/{name}", &h[..(prefix_len as usize).clamp(1, 4)])
            }
            _ => name.to_string(),
        }
    }
}

/// The logical name of a physical path (`ab/name` → `name`).
pub fn logical(physical: &str) -> &str {
    physical.rsplit_once('/').map(|(_, n)| n).unwrap_or(physical)
}

/// A backend seen through a repository layout: logical names in, physical
/// paths out. With the flat layout it is a transparent pass-through.
pub struct LayoutBackend<'a> {
    inner: &'a dyn Backend,
    layout: RepoLayout,
    prefix: String,
}

impl<'a> LayoutBackend<'a> {
    pub fn new(inner: &'a dyn Backend, layout: RepoLayout, prefix: &str) -> Self {
        Self { inner, layout, prefix: prefix.to_string() }
    }

    pub fn layout(&self) -> RepoLayout {
        self.layout
    }

    pub fn inner(&self) -> &'a dyn Backend {
        self.inner
    }

    fn physical(&self, name: &str) -> String {
        self.layout.physical(&self.prefix, name)
    }
}

impl Backend for LayoutBackend<'_> {
    fn describe(&self) -> String {
        self.inner.describe()
    }
    fn list(&self) -> Result<Vec<RemoteFile>, AppError> {
        if !self.layout.is_sharded() {
            return self.inner.list();
        }
        // Volumes in any shard folder (and at the top, for repositories
        // that are being resharded) under their logical names.
        let mut by_name: BTreeMap<String, u64> = BTreeMap::new();
        for f in self.inner.list_recursive()? {
            let name = logical(&f.name);
            if f.name.contains('/') && !matches!(volume::classify(&self.prefix, name), Some(VolumeKind::Block | VolumeKind::Index)) {
                continue;
            }
            by_name.entry(name.to_string()).or_insert(f.size);
        }
        Ok(by_name.into_iter().map(|(name, size)| RemoteFile { name, size }).collect())
    }
    fn put(&self, name: &str, data: &[u8]) -> Result<(), AppError> {
        self.inner.put(&self.physical(name), data)
    }
    fn get(&self, name: &str) -> Result<Vec<u8>, AppError> {
        let p = self.physical(name);
        match self.inner.get(&p) {
            // Interrupted reshard: the volume may still be at the top.
            Err(e) if p != name => self.inner.get(name).map_err(|_| e),
            r => r,
        }
    }
    fn delete(&self, name: &str) -> Result<(), AppError> {
        self.inner.delete(&self.physical(name))
    }
    fn create_folder(&self) -> Result<(), AppError> {
        self.inner.create_folder()
    }
    fn rename(&self, from: &str, to: &str) -> Result<(), AppError> {
        self.inner.rename(&self.physical(from), &self.physical(to))
    }
    fn quota(&self) -> Option<Quota> {
        self.inner.quota()
    }
    fn supports_lock(&self) -> bool {
        self.inner.supports_lock()
    }
    fn lock(&self, name: &str, until: chrono::DateTime<chrono::Utc>, mode: LockMode) -> Result<(), AppError> {
        self.inner.lock(&self.physical(name), until, mode)
    }
    fn supports_subfolders(&self) -> bool {
        self.inner.supports_subfolders()
    }
    fn list_recursive(&self) -> Result<Vec<RemoteFile>, AppError> {
        self.inner.list_recursive()
    }
}

/// Result of [`reshard`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ReshardReport {
    pub from: RepoLayout,
    pub to: RepoLayout,
    /// Volumes moved (or that would move, in a dry run).
    pub moved: usize,
    pub bytes_moved: u64,
    /// Volumes already where the new layout wants them.
    pub already_in_place: usize,
    pub dry_run: bool,
    pub errors: Vec<String>,
}

/// Move a v2 repository's volumes into `to` and record it in the header.
/// Each move is copy → read back and compare → delete, so an interruption
/// never loses a volume; running it again finishes the job. The header is
/// rewritten only after every volume moved.
pub fn reshard(backend: &dyn Backend, prefix: &str, to: RepoLayout, dry_run: bool, ctx: &OpContext) -> Result<ReshardReport, AppError> {
    to.validate()?;
    let hname = volume::repo_header_name(prefix);
    let mut header: RepoHeader = serde_json::from_slice(&backend.get(&hname)?)?;
    super::format::check_readable(&header)?;
    if header.format_version < 2 && to.is_sharded() {
        return Err(AppError::state("The sharded layout needs repository format 2", "Upgrade the repository first (Maintenance → Upgrade format, or `ufop backup upgrade`)."));
    }
    if to.is_sharded() && !backend.supports_subfolders() {
        return Err(AppError::state(
            format!("{} does not support subfolders", backend.describe()),
            "Use the flat layout on this destination.",
        ));
    }
    let mut r = ReshardReport { from: header.layout, to, dry_run, ..Default::default() };
    let listing = if backend.supports_subfolders() { backend.list_recursive()? } else { backend.list()? };
    let vols: Vec<&RemoteFile> =
        listing.iter().filter(|f| matches!(volume::classify(prefix, logical(&f.name)), Some(VolumeKind::Block | VolumeKind::Index))).collect();
    let total = vols.len() as u64;
    for (i, f) in vols.iter().enumerate() {
        ctx.check()?;
        let name = logical(&f.name);
        let dest = to.physical(prefix, name);
        if dest == f.name {
            r.already_in_place += 1;
            continue;
        }
        ctx.report(&Progress { phase: Phase::Uploading, files_processed: i as u64, files_total: total, bytes_processed: r.bytes_moved, current_path: Some(name.to_string()) });
        if dry_run {
            r.moved += 1;
            r.bytes_moved += f.size;
            continue;
        }
        let res = (|| -> Result<u64, AppError> {
            let data = backend.get(&f.name)?;
            backend.put(&dest, &data)?;
            let back = backend.get(&dest)?;
            if hash_bytes(HashAlgorithm::Sha256, &back) != hash_bytes(HashAlgorithm::Sha256, &data) {
                let _ = backend.delete(&dest);
                return Err(AppError::transfer(format!("{name} differs after copying"), "The destination corrupted the copy; the original was kept."));
            }
            backend.delete(&f.name)?;
            Ok(data.len() as u64)
        })();
        match res {
            Ok(n) => {
                r.moved += 1;
                r.bytes_moved += n;
            }
            Err(e) => r.errors.push(format!("{name}: {}", e.user_message())),
        }
    }
    if !dry_run && r.errors.is_empty() && header.layout != to {
        header.layout = to;
        super::format::sync_features(&mut header);
        super::format::write_header(backend, prefix, &header)?;
    }
    Ok(r)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup::backend::LocalBackend;

    #[test]
    fn physical_names_and_listing() {
        let d = tempfile::tempdir().unwrap();
        let lb = LocalBackend::new(d.path());
        let l = RepoLayout::Sharded { prefix_len: 2 };
        let b = LayoutBackend::new(&lb, l, "ufop");
        let name = "ufop-b0123.dblock";
        let p = l.physical("ufop", name);
        assert_eq!(p.len(), name.len() + 3);
        assert_eq!(logical(&p), name);
        assert_eq!(l.physical("ufop", "ufop-repo.json"), "ufop-repo.json");
        assert_eq!(l.physical("ufop", "ufop-20260101T000000Z.dlist"), "ufop-20260101T000000Z.dlist");
        b.put(name, b"data").unwrap();
        b.put("ufop-repo.json", b"{}").unwrap();
        assert!(d.path().join(&p).is_file());
        let names: Vec<String> = b.list().unwrap().into_iter().map(|f| f.name).collect();
        assert_eq!(names, vec![name.to_string(), "ufop-repo.json".into()]);
        assert_eq!(b.get(name).unwrap(), b"data");
        // Still at the top (interrupted reshard): found too.
        lb.put("ufop-b9999.dblock", b"x").unwrap();
        assert_eq!(b.get("ufop-b9999.dblock").unwrap(), b"x");
        b.delete(name).unwrap();
        assert!(!d.path().join(&p).exists());
        assert!(RepoLayout::Sharded { prefix_len: 9 }.validate().is_err());
    }
}
