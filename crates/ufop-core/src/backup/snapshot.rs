//! Point-in-time source snapshots so a backup reads a consistent view of
//! files that change (or are locked) while it runs:
//!
//! - Windows: Volume Shadow Copy (VSS) through the VSS COM API, with a
//!   selectable provider and excluded writer classes.
//! - Linux: ZFS dataset snapshots (read through `.zfs/snapshot`), btrfs
//!   read-only subvolume snapshots, or LVM copy-on-write snapshots mounted
//!   read-only. With the `auto` provider each source's filesystem type
//!   picks the method: `zfs` → ZFS, `btrfs` → btrfs, anything else → LVM.
//! - macOS: APFS local snapshots (`tmutil`), mounted read-only.
//!
//! The scanner keeps the original paths in the backup and reads through
//! [`Snapshot::real`]. Dropping the [`Snapshot`] (as soon as the sources
//! are fully read) unmounts and deletes it.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// When to use snapshots.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotPolicy {
    /// Read the live files.
    #[default]
    Off,
    /// Try; silently read live files when no snapshot can be made.
    Auto,
    /// Try; warn when falling back to live files.
    On,
    /// Fail the backup when no snapshot can be made.
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SnapshotOptions {
    /// LVM snapshot size: `-l` extents (`10%ORIGIN`, `20%FREE`) or `-L` size (`2G`).
    pub lvm_size: String,
    /// VSS provider id (GUID); empty = system default.
    pub vss_provider: Option<String>,
    /// VSS writer class ids (GUIDs) excluded from the snapshot.
    pub vss_exclude_writers: Vec<String>,
    /// Folder for snapshot mount points (default: system temp).
    pub mount_root: Option<String>,
    /// Linux snapshot method (`auto` picks it from each source's filesystem).
    pub provider: SnapshotProvider,
}

impl Default for SnapshotOptions {
    fn default() -> Self {
        Self { lvm_size: "10%ORIGIN".into(), vss_provider: None, vss_exclude_writers: Vec::new(), mount_root: None, provider: SnapshotProvider::Auto }
    }
}

/// Snapshot method on Linux (Windows always uses VSS, macOS APFS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotProvider {
    /// By filesystem type: ZFS, then btrfs, else LVM.
    #[default]
    Auto,
    Lvm,
    Zfs,
    Btrfs,
}

impl SnapshotProvider {
    pub fn name(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Lvm => "lvm",
            Self::Zfs => "zfs",
            Self::Btrfs => "btrfs",
        }
    }
}

/// Runs external commands (replaced in tests).
pub trait Cmd: Send + Sync {
    fn run(&self, prog: &str, args: &[&str]) -> Result<String, AppError>;
}

/// Real process runner.
pub struct SystemCmd;

impl Cmd for SystemCmd {
    fn run(&self, prog: &str, args: &[&str]) -> Result<String, AppError> {
        let out = std::process::Command::new(prog)
            .args(args)
            .output()
            .map_err(|e| AppError::file_op(format!("Could not run {prog}: {e}"), "Install the tool or turn snapshots off."))?;
        if !out.status.success() {
            return Err(AppError::file_op(
                format!("{prog} {} failed: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim()),
                "Snapshots need administrator/root rights and a supported volume.",
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }
}

/// Records every command and its outcome (diagnostics).
struct Recording {
    inner: Arc<dyn Cmd>,
    log: Arc<Mutex<Vec<String>>>,
}

impl Cmd for Recording {
    fn run(&self, prog: &str, args: &[&str]) -> Result<String, AppError> {
        let r = self.inner.run(prog, args);
        let line = match &r {
            Ok(o) => format!("$ {prog} {} → ok {}", args.join(" "), o.trim().lines().next().unwrap_or_default()),
            Err(e) => format!("$ {prog} {} → {}", args.join(" "), e.user_message()),
        };
        if let Ok(mut l) = self.log.lock() {
            l.push(line);
        }
        r
    }
}

/// An active snapshot. Dropping it releases everything it created.
pub struct Snapshot {
    pub method: String,
    /// (original root, snapshot root), longest roots first.
    maps: Vec<(PathBuf, PathBuf)>,
    /// Commands run on release, in order.
    cleanup: Vec<(String, Vec<String>)>,
    /// Folders removed after the cleanup commands.
    dirs: Vec<PathBuf>,
    cmd: Arc<dyn Cmd>,
    /// Platform objects whose drop releases the snapshot (VSS).
    #[allow(dead_code)]
    guards: Vec<Box<dyn std::any::Any>>,
    released: bool,
}

impl Snapshot {
    fn new(method: &str, cmd: Arc<dyn Cmd>) -> Self {
        Self { method: method.into(), maps: Vec::new(), cleanup: Vec::new(), dirs: Vec::new(), cmd, guards: Vec::new(), released: false }
    }

    fn add_map(&mut self, original: PathBuf, snap: PathBuf) {
        self.maps.push((original, snap));
        self.maps.sort_by_key(|(o, _)| std::cmp::Reverse(o.as_os_str().len()));
    }

    /// Where an original path can be read inside the snapshot.
    pub fn real(&self, original: &Path) -> PathBuf {
        for (o, s) in &self.maps {
            if let Ok(rel) = original.strip_prefix(o) {
                return if rel.as_os_str().is_empty() { s.clone() } else { s.join(rel) };
            }
            // macOS firmlinks: /Users/… lives at the root of the Data volume.
            if o == Path::new("/System/Volumes/Data") {
                if let Ok(rel) = original.strip_prefix("/") {
                    return s.join(rel);
                }
            }
        }
        original.to_path_buf()
    }

    /// Release now and report cleanup problems (Drop ignores them).
    pub fn release(mut self) -> Vec<String> {
        self.do_release()
    }

    fn do_release(&mut self) -> Vec<String> {
        if self.released {
            return Vec::new();
        }
        self.released = true;
        self.guards.clear();
        let mut warn = Vec::new();
        for (prog, args) in self.cleanup.iter().rev() {
            let a: Vec<&str> = args.iter().map(String::as_str).collect();
            if let Err(e) = self.cmd.run(prog, &a) {
                warn.push(e.user_message());
            }
        }
        for d in &self.dirs {
            let _ = std::fs::remove_dir(d);
        }
        warn
    }
}

impl Drop for Snapshot {
    fn drop(&mut self) {
        for w in self.do_release() {
            tracing::warn!("Snapshot cleanup: {w}");
        }
    }
}

#[cfg_attr(windows, allow(dead_code))]
fn mount_dir(opts: &SnapshotOptions, name: &str) -> Result<PathBuf, AppError> {
    let base = opts.mount_root.as_ref().map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
    let d = base.join(name);
    std::fs::create_dir_all(&d)?;
    Ok(d)
}

#[cfg_attr(windows, allow(dead_code))]
fn short_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..8].to_string()
}

/// Create a snapshot covering every source with the platform's method.
pub fn create(sources: &[PathBuf], opts: &SnapshotOptions) -> Result<Snapshot, AppError> {
    create_with(sources, opts, Arc::new(SystemCmd))
}

pub fn create_with(sources: &[PathBuf], opts: &SnapshotOptions, cmd: Arc<dyn Cmd>) -> Result<Snapshot, AppError> {
    if sources.is_empty() {
        return Err(AppError::validation("No sources to snapshot"));
    }
    #[cfg(test)]
    if let Some(maps) = test_hook::take_for(sources) {
        let mut s = Snapshot::new("test", cmd);
        for (o, r) in maps {
            s.add_map(o, r);
        }
        return Ok(s);
    }
    #[cfg(target_os = "linux")]
    return linux::create(sources, opts, cmd);
    #[cfg(target_os = "macos")]
    return apfs::create(sources, opts, cmd);
    #[cfg(windows)]
    return vss::create(sources, opts, cmd);
    #[allow(unreachable_code)]
    {
        let _ = (opts, cmd);
        Err(AppError::validation("Snapshots are not supported on this platform"))
    }
}

/// Result of [`diagnose`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SnapshotDiagnostics {
    pub method: String,
    pub ok: bool,
    /// Original root → snapshot root.
    pub mappings: Vec<(String, String)>,
    pub steps: Vec<String>,
    pub error: Option<String>,
    /// Snapshot methods of this platform and whether their tools are installed.
    #[serde(default)]
    pub providers: Vec<ProviderStatus>,
}

/// Whether a snapshot method can be used here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderStatus {
    pub name: String,
    pub available: bool,
    pub detail: String,
}

/// Find an executable on `PATH` (no process is started).
fn find_program(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exts: Vec<String> = if cfg!(windows) { vec![".exe".into(), ".cmd".into(), String::new()] } else { vec![String::new()] };
    std::env::split_paths(&path).flat_map(|d| exts.iter().map(move |e| d.join(format!("{name}{e}")))).find(|p| p.is_file())
}

/// The snapshot methods of this platform, in `auto` order.
pub fn providers() -> Vec<ProviderStatus> {
    let tool = |name: &str, bins: &[&str]| {
        let missing: Vec<&str> = bins.iter().copied().filter(|b| find_program(b).is_none()).collect();
        ProviderStatus {
            name: name.into(),
            available: missing.is_empty(),
            detail: if missing.is_empty() { format!("{} found", bins.join(", ")) } else { format!("{} not installed", missing.join(", ")) },
        }
    };
    if cfg!(windows) {
        vec![ProviderStatus { name: "vss".into(), available: true, detail: "Volume Shadow Copy (needs administrator rights)".into() }]
    } else if cfg!(target_os = "macos") {
        vec![tool("apfs", &["tmutil", "mount_apfs"])]
    } else {
        vec![tool("zfs", &["zfs"]), tool("btrfs", &["btrfs"]), tool("lvm", &["lvcreate", "lvs", "lvremove", "findmnt"])]
    }
}

/// Create and release a test snapshot, recording every step.
pub fn diagnose(sources: &[PathBuf], opts: &SnapshotOptions) -> SnapshotDiagnostics {
    diagnose_with(sources, opts, Arc::new(SystemCmd))
}

pub fn diagnose_with(sources: &[PathBuf], opts: &SnapshotOptions, cmd: Arc<dyn Cmd>) -> SnapshotDiagnostics {
    let log: Arc<Mutex<Vec<String>>> = Arc::default();
    let rec: Arc<dyn Cmd> = Arc::new(Recording { inner: cmd, log: log.clone() });
    let note = |m: String| {
        if let Ok(mut l) = log.lock() {
            l.push(m);
        }
    };
    let mut d = SnapshotDiagnostics { providers: providers(), ..Default::default() };
    match create_with(sources, opts, rec) {
        Ok(s) => {
            d.method = s.method.clone();
            d.mappings = s.maps.iter().map(|(o, r)| (o.to_string_lossy().into_owned(), r.to_string_lossy().into_owned())).collect();
            for src in sources {
                let real = s.real(src);
                let readable = std::fs::symlink_metadata(&real).is_ok();
                note(format!("{} → {} ({})", src.display(), real.display(), if readable { "readable" } else { "NOT readable" }));
            }
            let warns = s.release();
            d.ok = warns.is_empty();
            if !warns.is_empty() {
                d.error = Some(warns.join("; "));
            }
        }
        Err(e) => d.error = Some(e.user_message()),
    }
    d.steps = log.lock().map(|l| l.clone()).unwrap_or_default();
    d
}

/// Tests install the mappings the next snapshot should have.
#[cfg(test)]
pub(crate) mod test_hook {
    use std::path::PathBuf;
    use std::sync::Mutex;
    static NEXT: Mutex<Option<Vec<(PathBuf, PathBuf)>>> = Mutex::new(None);
    pub fn set(maps: Vec<(PathBuf, PathBuf)>) {
        *NEXT.lock().unwrap() = Some(maps);
    }
    /// The installed mappings, when they cover `sources` (tests run in
    /// parallel, so another test's snapshot must not take them).
    pub fn take_for(sources: &[PathBuf]) -> Option<Vec<(PathBuf, PathBuf)>> {
        let mut next = NEXT.lock().unwrap();
        let covers = next.as_ref().is_some_and(|m| sources.iter().all(|s| m.iter().any(|(o, _)| s.starts_with(o))));
        if covers {
            next.take()
        } else {
            None
        }
    }
}

#[cfg(any(target_os = "linux", test))]
mod lvm {
    use super::*;

    /// `KEY="value"` pairs from `findmnt -P`.
    pub(super) fn pairs(line: &str) -> std::collections::HashMap<String, String> {
        let mut out = std::collections::HashMap::new();
        let mut rest = line.trim();
        while let Some(eq) = rest.find("=\"") {
            let key = rest[..eq].trim().to_string();
            let after = &rest[eq + 2..];
            let Some(end) = after.find('"') else { break };
            out.insert(key, after[..end].to_string());
            rest = &after[end + 1..];
        }
        out
    }

    /// The mount of `src`: `TARGET`, `SOURCE` and `FSTYPE` from findmnt.
    pub(super) fn mount_of(src: &Path, cmd: &dyn Cmd) -> Result<std::collections::HashMap<String, String>, AppError> {
        let src_s = src.to_string_lossy();
        let info = pairs(&cmd.run("findmnt", &["-n", "-P", "-o", "TARGET,SOURCE,FSTYPE", "-T", &src_s])?);
        if info.get("TARGET").is_none() || info.get("SOURCE").is_none() {
            return Err(AppError::file_op(format!("Cannot find the mount of {src_s}"), "Check that the source exists."));
        }
        Ok(info)
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(super) fn create(sources: &[PathBuf], opts: &SnapshotOptions, cmd: Arc<dyn Cmd>) -> Result<Snapshot, AppError> {
        let mut snap = Snapshot::new("lvm", cmd.clone());
        let mut done: Vec<String> = Vec::new();
        for src in sources {
            let info = mount_of(src, cmd.as_ref())?;
            if done.contains(&info["TARGET"]) {
                continue;
            }
            add(&mut snap, &src.to_string_lossy(), &info, opts, cmd.as_ref())?;
            done.push(info["TARGET"].clone());
        }
        Ok(snap)
    }

    /// Snapshot the LVM volume mounted at `info["TARGET"]` and mount it.
    pub(super) fn add(snap: &mut Snapshot, src_s: &str, info: &std::collections::HashMap<String, String>, opts: &SnapshotOptions, cmd: &dyn Cmd) -> Result<(), AppError> {
        let (target, device) = (&info["TARGET"], &info["SOURCE"]);
        {
            let lv = cmd.run("lvs", &["--noheadings", "--separator", "|", "-o", "vg_name,lv_name", device]).map_err(|_| {
                AppError::file_op(format!("{src_s} is on {device}, which is not an LVM logical volume"), "LVM snapshots need the source on an LVM volume.")
            })?;
            let mut it = lv.trim().split('|');
            let (Some(vg), Some(lvname)) = (it.next().map(str::trim), it.next().map(str::trim)) else {
                return Err(AppError::file_op(format!("Unexpected lvs output for {device}"), "Check the LVM setup."));
            };
            let name = format!("ufop-snap-{}", short_id());
            let size_flag = if opts.lvm_size.contains('%') { "-l" } else { "-L" };
            cmd.run("lvcreate", &["--snapshot", "--name", &name, size_flag, &opts.lvm_size, &format!("{vg}/{lvname}")])?;
            snap.cleanup.push(("lvremove".into(), vec!["-f".into(), format!("{vg}/{name}")]));
            let dir = mount_dir(opts, &name)?;
            snap.dirs.push(dir.clone());
            let mut mo = "ro".to_string();
            if info.get("FSTYPE").map(String::as_str) == Some("xfs") {
                mo.push_str(",nouuid");
            }
            cmd.run("mount", &["-o", &mo, &format!("/dev/{vg}/{name}"), &dir.to_string_lossy()])?;
            snap.cleanup.push(("umount".into(), vec![dir.to_string_lossy().into_owned()]));
            snap.add_map(PathBuf::from(target), dir);
        }
        Ok(())
    }
}

/// Snapshot name stamp: UTC time plus a short random id.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn stamp() -> String {
    format!("{}-{}", chrono::Utc::now().format("%Y%m%dT%H%M%SZ"), short_id())
}

/// Linux: pick ZFS, btrfs or LVM per source (by filesystem type, or the
/// configured provider) and put every snapshot into one [`Snapshot`].
#[cfg(any(target_os = "linux", test))]
mod linux {
    use super::*;

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub(super) fn create(sources: &[PathBuf], opts: &SnapshotOptions, cmd: Arc<dyn Cmd>) -> Result<Snapshot, AppError> {
        let mut snap = Snapshot::new("lvm", cmd.clone());
        let mut done: Vec<String> = Vec::new();
        let mut methods: Vec<&'static str> = Vec::new();
        for src in sources {
            let info = lvm::mount_of(src, cmd.as_ref())?;
            if done.contains(&info["TARGET"]) {
                continue;
            }
            let provider = match (opts.provider, info.get("FSTYPE").map(String::as_str)) {
                (SnapshotProvider::Auto, Some("zfs")) => SnapshotProvider::Zfs,
                (SnapshotProvider::Auto, Some("btrfs")) => SnapshotProvider::Btrfs,
                (SnapshotProvider::Auto, _) => SnapshotProvider::Lvm,
                (p, _) => p,
            };
            let src_s = src.to_string_lossy();
            match provider {
                SnapshotProvider::Zfs => zfs::add(&mut snap, src, &info, cmd.as_ref())?,
                SnapshotProvider::Btrfs => btrfs::add(&mut snap, src, &info, cmd.as_ref())?,
                _ => lvm::add(&mut snap, &src_s, &info, opts, cmd.as_ref())?,
            }
            if !methods.contains(&provider.name()) {
                methods.push(provider.name());
            }
            done.push(info["TARGET"].clone());
        }
        snap.method = methods.join("+");
        Ok(snap)
    }
}

#[cfg(any(target_os = "linux", test))]
mod zfs {
    use super::*;

    /// `zfs list -H -o name,mountpoint` → (dataset, mount point) of datasets
    /// mounted at a path.
    pub(super) fn datasets(out: &str) -> Vec<(String, PathBuf)> {
        out.lines()
            .filter_map(|l| {
                let (name, mp) = l.split_once('\t')?;
                let mp = mp.trim();
                mp.starts_with('/').then(|| (name.trim().to_string(), PathBuf::from(mp)))
            })
            .collect()
    }

    /// Snapshot the dataset holding `src`; it is read through
    /// `<mount point>/.zfs/snapshot/<name>` and destroyed on release.
    pub(super) fn add(snap: &mut Snapshot, src: &Path, info: &std::collections::HashMap<String, String>, cmd: &dyn Cmd) -> Result<(), AppError> {
        let list = cmd.run("zfs", &["list", "-H", "-o", "name,mountpoint"])?;
        let (dataset, mountpoint) = datasets(&list)
            .into_iter()
            .filter(|(_, mp)| src.starts_with(mp))
            .max_by_key(|(_, mp)| mp.as_os_str().len())
            // Legacy mount points: the mount itself names the dataset.
            .or_else(|| (info.get("FSTYPE").map(String::as_str) == Some("zfs")).then(|| (info["SOURCE"].clone(), PathBuf::from(&info["TARGET"]))))
            .ok_or_else(|| AppError::file_op(format!("{} is not on a ZFS dataset", src.display()), "Choose another snapshot method or turn snapshots off."))?;
        let name = format!("ufop-{}", stamp());
        let full = format!("{dataset}@{name}");
        cmd.run("zfs", &["snapshot", &full])?;
        snap.cleanup.push(("zfs".into(), vec!["destroy".into(), full]));
        snap.add_map(mountpoint.clone(), mountpoint.join(".zfs").join("snapshot").join(&name));
        Ok(())
    }
}

#[cfg(any(target_os = "linux", test))]
mod btrfs {
    use super::*;

    /// Snapshot the subvolume holding `src` read-only next to it
    /// (`<subvolume>/.ufop-snap-<stamp>`); deleted on release.
    pub(super) fn add(snap: &mut Snapshot, src: &Path, info: &std::collections::HashMap<String, String>, cmd: &dyn Cmd) -> Result<(), AppError> {
        let target = PathBuf::from(&info["TARGET"]);
        // The nearest folder at or above the source that is a subvolume.
        let mut cand = src.to_path_buf();
        let subvol = loop {
            if cmd.run("btrfs", &["subvolume", "show", &cand.to_string_lossy()]).is_ok() {
                break cand;
            }
            if cand == target || !cand.pop() || !cand.starts_with(&target) {
                return Err(AppError::file_op(format!("No btrfs subvolume found for {}", src.display()), "btrfs snapshots need the source inside a subvolume."));
            }
        };
        let dir = subvol.join(format!(".ufop-snap-{}", stamp()));
        let dir_s = dir.to_string_lossy().into_owned();
        cmd.run("btrfs", &["subvolume", "snapshot", "-r", &subvol.to_string_lossy(), &dir_s])?;
        snap.cleanup.push(("btrfs".into(), vec!["subvolume".into(), "delete".into(), dir_s]));
        snap.add_map(subvol, dir);
        Ok(())
    }
}

#[cfg(any(target_os = "macos", test))]
mod apfs {
    use super::*;

    /// Date stamp from `tmutil localsnapshot` output.
    pub(super) fn snapshot_date(out: &str) -> Option<String> {
        out.split(|c: char| c.is_whitespace()).find(|w| w.len() >= 17 && w.chars().filter(|c| *c == '-').count() == 3 && w.chars().next().is_some_and(|c| c.is_ascii_digit())).map(str::to_string)
    }

    /// Mount point column of `df -P <path>`.
    pub(super) fn df_mount(out: &str) -> Option<String> {
        let line = out.lines().nth(1)?;
        // Columns: Filesystem 512-blocks Used Available Capacity Mounted-on (may contain spaces).
        let cols: Vec<&str> = line.split_whitespace().collect();
        (cols.len() >= 6).then(|| cols[5..].join(" "))
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    pub(super) fn create(sources: &[PathBuf], opts: &SnapshotOptions, cmd: Arc<dyn Cmd>) -> Result<Snapshot, AppError> {
        let mut snap = Snapshot::new("apfs", cmd.clone());
        let out = cmd.run("tmutil", &["localsnapshot"])?;
        let date = snapshot_date(&out).ok_or_else(|| AppError::file_op(format!("Unexpected tmutil output: {}", out.trim()), "Check Time Machine local snapshots."))?;
        snap.cleanup.push(("tmutil".into(), vec!["deletelocalsnapshots".into(), date.clone()]));
        let mut done: Vec<String> = Vec::new();
        for src in sources {
            let mount = df_mount(&cmd.run("df", &["-P", &src.to_string_lossy()])?).ok_or_else(|| AppError::file_op(format!("Cannot find the volume of {}", src.display()), "Check that the source exists."))?;
            if done.contains(&mount) {
                continue;
            }
            let dir = mount_dir(opts, &format!("ufop-snap-{}", short_id()))?;
            snap.dirs.push(dir.clone());
            cmd.run("mount_apfs", &["-o", "rdonly", "-s", &format!("com.apple.TimeMachine.{date}.local"), &mount, &dir.to_string_lossy()])?;
            snap.cleanup.push(("umount".into(), vec![dir.to_string_lossy().into_owned()]));
            snap.add_map(PathBuf::from(&mount), dir);
            done.push(mount);
        }
        Ok(snap)
    }
}

/// PowerShell / WMI shadow copies (`Win32_ShadowCopy`): the fallback when
/// the VSS COM path is unavailable and no provider/writer control is needed.
#[cfg(any(windows, test))]
mod wmi {
    use super::*;

    pub(super) fn create_script(volume: &str) -> String {
        format!(
            "$r = (Get-WmiObject -List Win32_ShadowCopy).Create('{}', 'ClientAccessible'); if ($r.ReturnValue -ne 0) {{ throw \"Win32_ShadowCopy.Create returned $($r.ReturnValue)\" }}; $s = Get-WmiObject Win32_ShadowCopy | Where-Object {{ $_.ID -eq $r.ShadowID }}; Write-Output \"$($s.ID)|$($s.DeviceObject)\"",
            volume.replace('\'', "''")
        )
    }

    pub(super) fn delete_script(id: &str) -> String {
        format!("Get-WmiObject Win32_ShadowCopy | Where-Object {{ $_.ID -eq '{}' }} | ForEach-Object {{ $_.Delete() }}", id.replace('\'', "''"))
    }

    pub(super) fn create(volumes: &[String], cmd: Arc<dyn Cmd>) -> Result<Snapshot, AppError> {
        let mut snap = Snapshot::new("vss", cmd.clone());
        for v in volumes {
            let out = cmd.run("powershell", &["-NoProfile", "-NonInteractive", "-Command", &create_script(v)])?;
            let line = out.lines().map(str::trim).find(|l| l.contains('|')).unwrap_or_default();
            let (id, dev) = line.split_once('|').ok_or_else(|| AppError::file_op(format!("Unexpected shadow copy output: {}", out.trim()), "Run as administrator."))?;
            snap.cleanup.push(("powershell".into(), vec!["-NoProfile".into(), "-NonInteractive".into(), "-Command".into(), delete_script(id)]));
            snap.add_map(PathBuf::from(v), PathBuf::from(format!("{}\\", dev.trim_end_matches('\\'))));
        }
        Ok(snap)
    }
}

/// VSS through the COM API (`IVssBackupComponents`), which supports a
/// chosen provider and excluded writer classes. The interface is a C++
/// class without type metadata, so its vtable is bound by hand (method
/// order from `vsbackup.h`).
#[cfg(windows)]
mod vss {
    use super::*;
    use std::ffi::c_void;
    use windows::core::{Interface, GUID, HRESULT};
    use windows::Win32::Storage::Vss::{IVssAsync, VSS_BT_COPY, VSS_CTX_BACKUP, VSS_SNAPSHOT_PROP};
    use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

    #[repr(C)]
    struct Vtbl {
        _qi: usize,
        _add_ref: usize,
        release: unsafe extern "system" fn(*mut Bc) -> u32,
        m: [usize; 48],
    }
    #[repr(C)]
    struct Bc {
        vtbl: *const Vtbl,
    }

    // Indices after IUnknown (vsbackup.h).
    const INITIALIZE_FOR_BACKUP: usize = 2;
    const SET_BACKUP_STATE: usize = 3;
    const GATHER_WRITER_METADATA: usize = 6;
    const PREPARE_FOR_BACKUP: usize = 11;
    const BACKUP_COMPLETE: usize = 24;
    const SET_CONTEXT: usize = 32;
    const START_SNAPSHOT_SET: usize = 33;
    const ADD_TO_SNAPSHOT_SET: usize = 34;
    const DO_SNAPSHOT_SET: usize = 35;
    const GET_SNAPSHOT_PROPERTIES: usize = 39;
    const DISABLE_WRITER_CLASSES: usize = 42;

    #[link(name = "vssapi")]
    extern "system" {
        fn CreateVssBackupComponentsInternal(pp: *mut *mut Bc) -> HRESULT;
        fn VssFreeSnapshotPropertiesInternal(p: *mut VSS_SNAPSHOT_PROP);
    }

    struct Components(*mut Bc);

    impl Components {
        unsafe fn f<T: Copy>(&self, i: usize) -> T {
            let addr = (*(*self.0).vtbl).m[i];
            std::mem::transmute_copy::<usize, T>(&addr)
        }
        unsafe fn async_call(&self, i: usize, what: &str) -> Result<(), AppError> {
            let f: unsafe extern "system" fn(*mut Bc, *mut *mut c_void) -> HRESULT = self.f(i);
            let mut a: *mut c_void = std::ptr::null_mut();
            f(self.0, &mut a).ok().map_err(|e| err(what, e))?;
            let a = IVssAsync::from_raw(a);
            a.Wait(u32::MAX).map_err(|e| err(what, e))?;
            let mut hr = HRESULT(0);
            a.QueryStatus(&mut hr, std::ptr::null_mut()).map_err(|e| err(what, e))?;
            hr.ok().map_err(|e| err(what, e))
        }
    }

    impl Drop for Components {
        fn drop(&mut self) {
            // Non-persistent shadow copies are deleted when released.
            unsafe {
                let _ = self.async_call(BACKUP_COMPLETE, "BackupComplete");
                ((*(*self.0).vtbl).release)(self.0);
            }
        }
    }

    fn err(what: &str, e: windows::core::Error) -> AppError {
        AppError::file_op(format!("VSS {what} failed: {e}"), "Run as administrator; check `vssadmin list writers`.")
    }

    fn guid(s: &str) -> Result<GUID, AppError> {
        let t = s.trim().trim_matches(['{', '}']);
        let u = uuid::Uuid::parse_str(t).map_err(|_| AppError::validation(format!("'{s}' is not a GUID")))?;
        Ok(GUID::from_u128(u.as_u128()))
    }

    fn com(volumes: &[String], opts: &SnapshotOptions, cmd: Arc<dyn Cmd>) -> Result<Snapshot, AppError> {
        let provider = match &opts.vss_provider {
            Some(p) if !p.trim().is_empty() => guid(p)?,
            _ => GUID::zeroed(),
        };
        let writers: Vec<GUID> = opts.vss_exclude_writers.iter().map(|w| guid(w)).collect::<Result<_, _>>()?;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let mut raw: *mut Bc = std::ptr::null_mut();
            CreateVssBackupComponentsInternal(&mut raw).ok().map_err(|e| err("initialisation", e))?;
            let bc = Components(raw);
            let init: unsafe extern "system" fn(*mut Bc, *const u16) -> HRESULT = bc.f(INITIALIZE_FOR_BACKUP);
            init(bc.0, std::ptr::null()).ok().map_err(|e| err("InitializeForBackup", e))?;
            let ctx: unsafe extern "system" fn(*mut Bc, i32) -> HRESULT = bc.f(SET_CONTEXT);
            ctx(bc.0, VSS_CTX_BACKUP.0).ok().map_err(|e| err("SetContext", e))?;
            let state: unsafe extern "system" fn(*mut Bc, u8, u8, i32, u8) -> HRESULT = bc.f(SET_BACKUP_STATE);
            state(bc.0, 0, 1, VSS_BT_COPY.0, 0).ok().map_err(|e| err("SetBackupState", e))?;
            if !writers.is_empty() {
                let dis: unsafe extern "system" fn(*mut Bc, *const GUID, u32) -> HRESULT = bc.f(DISABLE_WRITER_CLASSES);
                dis(bc.0, writers.as_ptr(), writers.len() as u32).ok().map_err(|e| err("DisableWriterClasses", e))?;
            }
            bc.async_call(GATHER_WRITER_METADATA, "GatherWriterMetadata")?;
            let start: unsafe extern "system" fn(*mut Bc, *mut GUID) -> HRESULT = bc.f(START_SNAPSHOT_SET);
            let mut set = GUID::zeroed();
            start(bc.0, &mut set).ok().map_err(|e| err("StartSnapshotSet", e))?;
            let add: unsafe extern "system" fn(*mut Bc, *const u16, GUID, *mut GUID) -> HRESULT = bc.f(ADD_TO_SNAPSHOT_SET);
            let mut ids = Vec::new();
            for v in volumes {
                let w: Vec<u16> = v.encode_utf16().chain(Some(0)).collect();
                let mut id = GUID::zeroed();
                add(bc.0, w.as_ptr(), provider, &mut id).ok().map_err(|e| err("AddToSnapshotSet", e))?;
                ids.push((v.clone(), id));
            }
            bc.async_call(PREPARE_FOR_BACKUP, "PrepareForBackup")?;
            bc.async_call(DO_SNAPSHOT_SET, "DoSnapshotSet")?;
            let props: unsafe extern "system" fn(*mut Bc, GUID, *mut VSS_SNAPSHOT_PROP) -> HRESULT = bc.f(GET_SNAPSHOT_PROPERTIES);
            let mut snap = Snapshot::new("vss", cmd);
            for (v, id) in ids {
                let mut p = VSS_SNAPSHOT_PROP::default();
                props(bc.0, id, &mut p).ok().map_err(|e| err("GetSnapshotProperties", e))?;
                let dev = windows::core::PCWSTR(p.m_pwszSnapshotDeviceObject).to_string().unwrap_or_default();
                VssFreeSnapshotPropertiesInternal(&mut p);
                snap.add_map(PathBuf::from(&v), PathBuf::from(format!("{}\\", dev.trim_end_matches('\\'))));
            }
            snap.guards.push(Box::new(bc));
            Ok(snap)
        }
    }

    pub(super) fn create(sources: &[PathBuf], opts: &SnapshotOptions, cmd: Arc<dyn Cmd>) -> Result<Snapshot, AppError> {
        let mut vols: Vec<String> = Vec::new();
        for s in sources {
            let v = crate::backup::usn::volume_root(&s.to_string_lossy())
                .ok_or_else(|| AppError::validation(format!("{} is not on a local drive; VSS needs a drive letter", s.display())))?;
            if !vols.contains(&v) {
                vols.push(v);
            }
        }
        match com(&vols, opts, cmd.clone()) {
            Ok(s) => Ok(s),
            Err(e) if opts.vss_provider.is_none() && opts.vss_exclude_writers.is_empty() => {
                tracing::warn!("VSS COM path failed ({}); trying WMI", e.user_message());
                wmi::create(&vols, cmd)
            }
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// Scripted runner: answers in order (then empty output) and records the calls.
    struct Script {
        steps: Mutex<VecDeque<Result<String, String>>>,
        calls: Mutex<Vec<String>>,
    }
    impl Script {
        fn new(steps: &[Result<&str, &str>]) -> Arc<Self> {
            Arc::new(Self { steps: Mutex::new(steps.iter().map(|r| r.map(String::from).map_err(String::from)).collect()), calls: Mutex::new(Vec::new()) })
        }
        fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }
    impl Cmd for Script {
        fn run(&self, prog: &str, args: &[&str]) -> Result<String, AppError> {
            self.calls.lock().unwrap().push(format!("{prog} {}", args.join(" ")));
            match self.steps.lock().unwrap().pop_front() {
                Some(Ok(s)) => Ok(s),
                Some(Err(e)) => Err(AppError::file_op(e, "x")),
                None => Ok(String::new()),
            }
        }
    }

    #[test]
    fn lvm_flow_maps_paths_and_cleans_up() {
        let root = tempfile::tempdir().unwrap();
        let opts = SnapshotOptions { mount_root: Some(root.path().to_string_lossy().into()), ..Default::default() };
        let s = Script::new(&[
            Ok("TARGET=\"/home\" SOURCE=\"/dev/mapper/vg0-home\" FSTYPE=\"xfs\"\n"),
            Ok("  vg0|home\n"),
            Ok(""), // lvcreate
            Ok(""), // mount
            Ok("TARGET=\"/home\" SOURCE=\"/dev/mapper/vg0-home\" FSTYPE=\"xfs\"\n"),
        ]);
        let snap = lvm::create(&["/home/a/docs".into(), "/home/b".into()], &opts, s.clone()).unwrap();
        let calls = s.calls();
        assert!(calls[2].starts_with("lvcreate --snapshot --name ufop-snap-") && calls[2].ends_with("-l 10%ORIGIN vg0/home"), "{calls:?}");
        assert!(calls[3].starts_with("mount -o ro,nouuid /dev/vg0/ufop-snap-"), "{calls:?}");
        assert_eq!(calls.iter().filter(|c| c.starts_with("lvcreate")).count(), 1, "one snapshot per volume");
        let real = snap.real(Path::new("/home/a/docs/x.txt"));
        assert!(real.starts_with(root.path()) && real.ends_with("a/docs/x.txt"), "{real:?}");
        assert_eq!(snap.real(Path::new("/etc/passwd")), PathBuf::from("/etc/passwd"));
        let warns = snap.release();
        assert!(warns.is_empty());
        let calls = s.calls();
        let n = calls.len();
        assert!(calls[n - 2].starts_with("umount ") && calls[n - 1].starts_with("lvremove -f vg0/ufop-snap-"), "{calls:?}");
    }

    #[test]
    fn lvm_rejects_non_lvm_and_cleans_partial_state() {
        let root = tempfile::tempdir().unwrap();
        let opts = SnapshotOptions { mount_root: Some(root.path().to_string_lossy().into()), lvm_size: "2G".into(), ..Default::default() };
        let s = Script::new(&[Ok("TARGET=\"/\" SOURCE=\"/dev/sda1\" FSTYPE=\"ext4\""), Err("not an LV")]);
        assert!(lvm::create(&["/data".into()], &opts, s.clone()).is_err());
        // Mount fails after lvcreate: the snapshot volume is removed again.
        let s = Script::new(&[Ok("TARGET=\"/\" SOURCE=\"/dev/vg/root\" FSTYPE=\"ext4\""), Ok("vg|root"), Ok(""), Err("mount failed")]);
        assert!(lvm::create(&["/data".into()], &opts, s.clone()).is_err());
        let calls = s.calls();
        assert!(calls.iter().any(|c| c.contains("-L 2G")), "{calls:?}");
        assert!(calls.last().unwrap().starts_with("lvremove -f vg/ufop-snap-"), "{calls:?}");
    }

    #[test]
    fn apfs_flow_and_firmlinks() {
        assert_eq!(apfs::snapshot_date("Created local snapshot with date: 2026-10-21-101530\n").as_deref(), Some("2026-10-21-101530"));
        assert_eq!(
            apfs::df_mount("Filesystem 512-blocks Used Available Capacity Mounted on\n/dev/disk3s5 1 2 3 4% /System/Volumes/Data\n").as_deref(),
            Some("/System/Volumes/Data")
        );
        let root = tempfile::tempdir().unwrap();
        let opts = SnapshotOptions { mount_root: Some(root.path().to_string_lossy().into()), ..Default::default() };
        let s = Script::new(&[
            Ok("Created local snapshot with date: 2026-10-21-101530"),
            Ok("Filesystem 512-blocks Used Available Capacity Mounted on\n/dev/disk3s5 1 2 3 4% /System/Volumes/Data\n"),
            Ok(""),
        ]);
        let snap = apfs::create(&["/Users/me/Documents".into()], &opts, s.clone()).unwrap();
        assert!(s.calls()[2].starts_with("mount_apfs -o rdonly -s com.apple.TimeMachine.2026-10-21-101530.local /System/Volumes/Data "));
        let real = snap.real(Path::new("/Users/me/Documents/a.txt"));
        assert!(real.ends_with("Users/me/Documents/a.txt") && real.starts_with(root.path()), "{real:?}");
        drop(snap);
        let calls = s.calls();
        assert!(calls.last().unwrap().starts_with("tmutil deletelocalsnapshots 2026-10-21-101530"), "{calls:?}");
    }

    #[test]
    fn zfs_flow_reads_through_dot_zfs_and_destroys() {
        let s = Script::new(&[
            Ok("TARGET=\"/tank/home\" SOURCE=\"tank/home\" FSTYPE=\"zfs\"\n"),
            Ok("tank\t/tank\ntank/home\t/tank/home\ntank/swap\tnone\n"),
            Ok(""), // zfs snapshot
        ]);
        let snap = linux::create(&["/tank/home/me/docs".into()], &SnapshotOptions::default(), s.clone()).unwrap();
        assert_eq!(snap.method, "zfs");
        let calls = s.calls();
        assert_eq!(calls[1], "zfs list -H -o name,mountpoint");
        assert!(calls[2].starts_with("zfs snapshot tank/home@ufop-"), "{calls:?}");
        let name = calls[2].rsplit('@').next().unwrap().to_string();
        assert_eq!(snap.real(Path::new("/tank/home/me/docs/a.txt")), PathBuf::from(format!("/tank/home/.zfs/snapshot/{name}/me/docs/a.txt")));
        assert!(snap.release().is_empty());
        assert_eq!(s.calls().last().unwrap(), &format!("zfs destroy tank/home@{name}"));
    }

    #[test]
    fn btrfs_flow_finds_the_subvolume_and_deletes_the_snapshot() {
        let s = Script::new(&[
            Ok("TARGET=\"/data\" SOURCE=\"/dev/sdb1[/@data]\" FSTYPE=\"btrfs\"\n"),
            Err("not a subvolume"), // /data/projects/app
            Err("not a subvolume"), // /data/projects
            Ok("data\n\tName: \t@data\n"), // /data
            Ok(""), // snapshot
        ]);
        let snap = linux::create(&["/data/projects/app".into()], &SnapshotOptions::default(), s.clone()).unwrap();
        assert_eq!(snap.method, "btrfs");
        let calls = s.calls();
        assert_eq!(&calls[1..4], ["btrfs subvolume show /data/projects/app", "btrfs subvolume show /data/projects", "btrfs subvolume show /data"]);
        assert!(calls[4].starts_with("btrfs subvolume snapshot -r /data /data/.ufop-snap-"), "{calls:?}");
        let real = snap.real(Path::new("/data/projects/app/x"));
        assert!(real.starts_with("/data") && real.to_string_lossy().contains(".ufop-snap-") && real.ends_with("projects/app/x"), "{real:?}");
        drop(snap);
        assert!(s.calls().last().unwrap().starts_with("btrfs subvolume delete /data/.ufop-snap-"));
        // No subvolume anywhere up to the mount: an error, nothing created.
        let s = Script::new(&[Ok("TARGET=\"/d\" SOURCE=\"/dev/x\" FSTYPE=\"btrfs\""), Err("no"), Err("no")]);
        assert!(linux::create(&["/d/a".into()], &SnapshotOptions::default(), s.clone()).is_err());
        assert!(!s.calls().iter().any(|c| c.contains("snapshot -r")));
    }

    #[test]
    fn auto_mixes_providers_and_explicit_provider_wins() {
        let root = tempfile::tempdir().unwrap();
        let opts = SnapshotOptions { mount_root: Some(root.path().to_string_lossy().into()), ..Default::default() };
        let s = Script::new(&[
            Ok("TARGET=\"/tank\" SOURCE=\"tank\" FSTYPE=\"zfs\""),
            Ok("tank\t/tank\n"),
            Ok(""),
            Ok("TARGET=\"/home\" SOURCE=\"/dev/mapper/vg-home\" FSTYPE=\"ext4\""),
            Ok("vg|home"),
            Ok(""),
            Ok(""),
        ]);
        let snap = linux::create(&["/tank/a".into(), "/home/b".into()], &opts, s.clone()).unwrap();
        assert_eq!(snap.method, "zfs+lvm");
        assert!(s.calls().iter().any(|c| c.starts_with("lvcreate")));
        drop(snap);
        // Forcing ZFS on a non-ZFS mount fails clearly.
        let s = Script::new(&[Ok("TARGET=\"/\" SOURCE=\"/dev/sda1\" FSTYPE=\"ext4\""), Ok("tank\t/tank\n")]);
        let e = linux::create(&["/srv".into()], &SnapshotOptions { provider: SnapshotProvider::Zfs, ..Default::default() }, s).err().unwrap();
        assert!(e.user_message().contains("not on a ZFS dataset"));
        let p: SnapshotOptions = serde_json::from_str(r#"{"provider":"btrfs"}"#).unwrap();
        assert_eq!(p.provider, SnapshotProvider::Btrfs);
        assert_eq!(serde_json::from_str::<SnapshotOptions>("{}").unwrap().provider, SnapshotProvider::Auto);
    }

    #[test]
    fn diagnostics_list_providers() {
        let names: Vec<String> = providers().into_iter().map(|p| p.name).collect();
        #[cfg(target_os = "linux")]
        assert_eq!(names, ["zfs", "btrfs", "lvm"]);
        assert!(!names.is_empty());
    }

    #[test]
    fn findmnt_pairs() {
        let p = lvm::pairs(r#"TARGET="/mnt/my disk" SOURCE="/dev/mapper/vg-lv" FSTYPE="ext4""#);
        assert_eq!(p["TARGET"], "/mnt/my disk");
        assert_eq!(p["FSTYPE"], "ext4");
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn diagnostics_record_steps_and_errors() {
        let s = Script::new(&[Ok("TARGET=\"/\" SOURCE=\"/dev/sda1\" FSTYPE=\"ext4\""), Err("not an LV")]);
        let d = diagnose_with(&["/data".into()], &SnapshotOptions::default(), s);
        assert!(!d.ok && d.error.is_some());
        assert_eq!(d.steps.len(), 2, "{:?}", d.steps);
        assert!(d.steps[1].contains("lvs"));
    }

    #[test]
    fn wmi_shadow_copy_flow() {
        let s = Script::new(&[Ok("{AB-12}|\\\\?\\GLOBALROOT\\Device\\HarddiskVolumeShadowCopy7\r\n")]);
        let snap = wmi::create(&["C:\\".into()], s.clone()).unwrap();
        assert!(s.calls()[0].contains("Win32_ShadowCopy).Create('C:\\', 'ClientAccessible')"), "{:?}", s.calls());
        assert_eq!(snap.method, "vss");
        assert!(snap.maps[0].1.to_string_lossy().starts_with("\\\\?\\GLOBALROOT\\Device\\HarddiskVolumeShadowCopy7"));
        drop(snap);
        assert!(s.calls()[1].contains("$_.ID -eq '{AB-12}'") && s.calls()[1].contains("Delete()"));
        assert!(wmi::create(&["C:\\".into()], Script::new(&[Ok("garbage")])).is_err());
    }
}
