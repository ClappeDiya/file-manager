//! Mount options (cache, read-only) and the organization's mount profile
//! (`ufop_core::agent::ManagedMounts`) that constrains them.
//!
//! The profile wins: a forced cache mode or read-only replaces what the user
//! chose, size and age limits cap it, protocols outside the allowed list and
//! mount points outside the mount root are refused. `effective` returns the
//! options to use and notes on what the profile changed.
//!
//! Advanced options (drive letter, volume label, permissions, cache folder,
//! directory-cache time, extra rclone flags) are validated here too, per
//! operating system, so a mount never starts with flags rclone would reject
//! or that would undo what the profile forces.

use super::expand_home;
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use ufop_core::agent::ManagedMounts;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum CacheMode {
    /// Read and write straight through; nothing is kept.
    Off,
    /// Only files open for read and write are cached.
    Minimal,
    /// Writes are cached and uploaded in the background (the default).
    #[default]
    Writes,
    /// Whole files are cached on read and write: they stay readable offline.
    Full,
}

impl CacheMode {
    pub fn as_rclone(self) -> &'static str {
        match self {
            CacheMode::Off => "off",
            CacheMode::Minimal => "minimal",
            CacheMode::Writes => "writes",
            CacheMode::Full => "full",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "off" => CacheMode::Off,
            "minimal" => CacheMode::Minimal,
            "writes" => CacheMode::Writes,
            "full" => CacheMode::Full,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct MountOptions {
    pub cache_mode: CacheMode,
    /// Largest cache, in MB (`None` = rclone's default: unlimited).
    pub max_cache_mb: Option<u64>,
    /// Cached files older than this are dropped (`None` = rclone's default, 1 hour).
    pub max_cache_age_hours: Option<u32>,
    pub read_only: bool,
    /// Mount this folder of the connection instead of its whole remote path.
    pub subfolder: Option<String>,
    /// Open the mount point in the file manager once it is mounted (the app does this).
    pub open_after_mount: bool,
    /// Windows: mount as this drive letter (`"X"`) instead of a folder.
    pub drive_letter: Option<String>,
    /// Name the drive shows in Explorer / Finder (`--volname`; Windows and macOS).
    pub volume_label: Option<String>,
    /// Windows: show the drive as a network drive (`--network-mode`).
    pub network_mode: bool,
    /// Let other users of this computer see the mount (`--allow-other`; not Windows).
    pub allow_other: bool,
    /// Permissions files show as, octal (`--file-perms`, e.g. `0644`).
    pub file_perms: Option<String>,
    /// Permissions folders show as, octal (`--dir-perms`, e.g. `0755`).
    pub dir_perms: Option<String>,
    /// Owner files show as (`--uid`; not Windows).
    pub uid: Option<u32>,
    /// Group files show as (`--gid`; not Windows).
    pub gid: Option<u32>,
    /// Keep this drive's cache under this folder (in a folder named after the drive).
    /// `None` = the app's cache folder.
    pub cache_dir: Option<String>,
    /// How long rclone trusts a folder listing (`--dir-cache-time`, e.g. `30s`; `None` = 30s).
    pub dir_cache_time: Option<String>,
    /// Further rclone mount flags from the allow-list (`--name` or `--name=value`).
    pub extra_flags: Vec<String>,
}

/// Default `--dir-cache-time`.
pub const DEFAULT_DIR_CACHE_TIME: &str = "30s";

/// The operating systems whose mount flags differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else {
            Os::Linux
        }
    }
}

/// rclone mount flags the "Extra flags" field accepts. Everything here tunes
/// reading, timeouts or naming; none can change what the profile controls.
const ALLOWED_EXTRA_FLAGS: &[&str] = &[
    "async-read",
    "attr-timeout",
    "buffer-size",
    "contimeout",
    "daemon-timeout",
    "default-permissions",
    "fast-list",
    "ignore-case",
    "links",
    "low-level-retries",
    "max-read-ahead",
    "no-checksum",
    "no-modtime",
    "no-seek",
    "no-unicode-normalization",
    "noappledouble",
    "noapplexattr",
    "poll-interval",
    "retries",
    "timeout",
    "tpslimit",
    "tpslimit-burst",
    "umask",
    "use-server-modtime",
    "user-agent",
    "vfs-cache-poll-interval",
    "vfs-case-insensitive",
    "vfs-fast-fingerprint",
    "vfs-read-ahead",
    "vfs-read-chunk-size",
    "vfs-read-chunk-size-limit",
    "vfs-read-wait",
    "vfs-refresh",
    "vfs-used-is-size",
    "vfs-write-back",
    "vfs-write-wait",
];

/// Flags with their own field (or owned by the profile or the app): refused in
/// "Extra flags" so they can't be used to override a forced setting.
const CONTROLLED_FLAGS: &[(&str, &str)] = &[
    ("read-only", "the Read-only option"),
    ("vfs-cache-mode", "the Cache mode option"),
    ("vfs-cache-max-size", "the Largest cache option"),
    ("vfs-cache-max-age", "the Keep cached files option"),
    ("cache-dir", "the Cache folder option"),
    ("dir-cache-time", "the Directory cache option"),
    ("volname", "the Volume label option"),
    ("network-mode", "the Network drive option"),
    ("allow-other", "the Allow other users option"),
    ("file-perms", "the File permissions option"),
    ("dir-perms", "the Folder permissions option"),
    ("uid", "the Owner option"),
    ("gid", "the Group option"),
];

/// Options that only rclone (cloud) mounts take.
fn rclone_only_set(o: &MountOptions) -> Vec<&'static str> {
    let d = MountOptions::default();
    let mut v = Vec::new();
    let mut check = |set: bool, name: &'static str| {
        if set {
            v.push(name)
        }
    };
    check(o.cache_mode != d.cache_mode, "cache mode");
    check(o.max_cache_mb.is_some(), "largest cache");
    check(o.max_cache_age_hours.is_some(), "keep cached files");
    check(o.read_only, "read-only");
    check(o.drive_letter.is_some(), "drive letter");
    check(o.volume_label.is_some(), "volume label");
    check(o.network_mode, "network drive");
    check(o.allow_other, "allow other users");
    check(o.file_perms.is_some(), "file permissions");
    check(o.dir_perms.is_some(), "folder permissions");
    check(o.uid.is_some(), "owner");
    check(o.gid.is_some(), "group");
    check(o.cache_dir.is_some(), "cache folder");
    check(o.dir_cache_time.is_some(), "directory cache");
    check(!o.extra_flags.is_empty(), "extra flags");
    v
}

/// `"x"`, `"X:"` → `'X'`; `None` unless it is one letter from D to Z.
pub fn parse_drive_letter(s: &str) -> Option<char> {
    let mut cs = s.trim().trim_end_matches(':').chars();
    let c = cs.next()?.to_ascii_uppercase();
    (cs.next().is_none() && c.is_ascii_alphabetic() && !matches!(c, 'A' | 'B' | 'C')).then_some(c)
}

/// The mount point a chosen drive letter stands for (`"X:"`), on Windows.
pub fn drive_mount_point(o: &MountOptions, os: Os) -> Option<String> {
    let l = o.drive_letter.as_deref().and_then(parse_drive_letter)?;
    (os == Os::Windows).then(|| format!("{l}:"))
}

/// Drive letters from D to Z for which `in_use` is false.
pub fn free_drive_letters_with(in_use: impl Fn(char) -> bool) -> Vec<String> {
    ('D'..='Z').filter(|c| !in_use(*c)).map(String::from).collect()
}

/// Drive letters nothing uses now (Windows only; empty elsewhere).
pub fn free_drive_letters() -> Vec<String> {
    if Os::current() != Os::Windows {
        return Vec::new();
    }
    free_drive_letters_with(|c| Path::new(&format!("{c}:\\")).exists())
}

fn bad(msg: impl Into<String>) -> AppError {
    AppError::validation(msg.into())
}

fn blank_to_none(s: &Option<String>) -> Option<String> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string)
}

/// A relative folder inside the connection: forward slashes, no `.`/`..`, no control characters.
pub fn normalize_subfolder(s: &str) -> Result<Option<String>, AppError> {
    if s.chars().any(char::is_control) {
        return Err(bad("The subfolder contains characters that aren't allowed"));
    }
    let mut parts = Vec::new();
    for seg in s.replace('\\', "/").split('/') {
        match seg.trim() {
            "" | "." => {}
            ".." => return Err(bad("The subfolder can't go up a level (..); enter a folder inside the connection")),
            seg => parts.push(seg.to_string()),
        }
    }
    Ok(if parts.is_empty() { None } else { Some(parts.join("/")) })
}

/// `remote_path` + `subfolder`, with one slash between them.
pub fn join_subfolder(remote_path: &str, subfolder: Option<&str>) -> String {
    match subfolder.map(|s| s.trim_matches('/')).filter(|s| !s.is_empty()) {
        None => remote_path.to_string(),
        Some(sub) => format!("{}/{sub}", remote_path.trim_end_matches('/')),
    }
}

fn normalize_perms(s: &str, what: &str) -> Result<String, AppError> {
    let t = s.trim();
    let digits = t.strip_prefix("0o").unwrap_or(t);
    if !(3..=4).contains(&digits.len()) || !digits.chars().all(|c| ('0'..='7').contains(&c)) {
        return Err(bad(format!("{what} must be an octal number such as 0644")));
    }
    Ok(format!("{:0>4}", digits))
}

/// `30s`, `5m`, `1h30m`, `500ms`: what rclone parses as a duration.
fn valid_duration(s: &str) -> bool {
    let mut rest = s;
    let mut any = false;
    while !rest.is_empty() {
        let digits = rest.chars().take_while(char::is_ascii_digit).count();
        if digits == 0 || digits > 6 {
            return false;
        }
        rest = &rest[digits..];
        let unit = ["ms", "s", "m", "h"].iter().find(|u| rest.starts_with(**u));
        let Some(unit) = unit else { return false };
        rest = &rest[unit.len()..];
        any = true;
    }
    any
}

/// Check the extra flags against the allow-list and return them as `--name[=value]`.
/// `--name value` (two words) is joined into `--name=value`.
pub fn normalize_extra_flags(flags: &[String]) -> Result<Vec<String>, AppError> {
    let mut out: Vec<String> = Vec::new();
    let mut open: Option<usize> = None; // a flag that has no value yet and may take the next word
    for raw in flags.iter().flat_map(|f| f.split_whitespace()) {
        if let Some(name) = raw.strip_prefix("--") {
            let (name, value) = match name.split_once('=') {
                Some((n, v)) => (n, Some(v)),
                None => (name, None),
            };
            if name.is_empty() || !name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') {
                return Err(bad(format!("'{raw}' isn't a valid flag")));
            }
            if let Some((_, owner)) = CONTROLLED_FLAGS.iter().find(|(n, _)| *n == name) {
                return Err(bad(format!("--{name} can't be set here: use {owner} (your organization's mount profile may fix it)")));
            }
            if !ALLOWED_EXTRA_FLAGS.contains(&name) {
                return Err(bad(format!("--{name} isn't an allowed mount flag")));
            }
            if value.is_some_and(|v| v.len() > 200) {
                return Err(bad(format!("The value of --{name} is too long")));
            }
            out.push(raw.to_string());
            open = value.is_none().then_some(out.len() - 1);
        } else if raw.starts_with('-') {
            return Err(bad(format!("'{raw}' isn't allowed; write flags as --name or --name=value")));
        } else if let Some(i) = open.take() {
            if raw.len() > 200 {
                return Err(bad("A flag value is too long"));
            }
            out[i] = format!("{}={raw}", out[i]);
        } else {
            return Err(bad(format!("'{raw}' isn't a flag; write flags as --name or --name=value")));
        }
    }
    Ok(out)
}

/// Check `requested` for `protocol` on `os` and return it normalized (letters
/// upper-cased, permissions padded, blank fields cleared). Options a protocol
/// or OS doesn't support are refused rather than silently dropped.
pub fn validate_on(requested: &MountOptions, protocol: &str, mount_point: &Path, os: Os) -> Result<MountOptions, AppError> {
    let mut o = requested.clone();
    o.subfolder = match o.subfolder.as_deref() {
        Some(s) => normalize_subfolder(s)?,
        None => None,
    };
    o.drive_letter = blank_to_none(&o.drive_letter);
    o.volume_label = blank_to_none(&o.volume_label);
    o.file_perms = blank_to_none(&o.file_perms);
    o.dir_perms = blank_to_none(&o.dir_perms);
    o.cache_dir = blank_to_none(&o.cache_dir);
    o.dir_cache_time = blank_to_none(&o.dir_cache_time);

    if !matches!(protocol.to_ascii_lowercase().as_str(), "rclone" | "s3") {
        let set = rclone_only_set(&o);
        if !set.is_empty() {
            return Err(bad(format!("{} applies to cloud (rclone) drives only", set.join(", "))));
        }
        return Ok(o);
    }

    if let Some(l) = &o.drive_letter {
        if os != Os::Windows {
            return Err(bad("Drive letters are only available on Windows"));
        }
        match parse_drive_letter(l) {
            Some(c) => o.drive_letter = Some(c.to_string()),
            None => return Err(bad("Choose a drive letter from D to Z")),
        }
    }
    if let Some(label) = &o.volume_label {
        if os == Os::Linux {
            return Err(bad("A volume label is only available on Windows and macOS"));
        }
        if label.chars().count() > 32 || label.chars().any(char::is_control) {
            return Err(bad("The volume label can be up to 32 characters, without control characters"));
        }
    }
    if o.network_mode && os != Os::Windows {
        return Err(bad("Network drive mode is only available on Windows"));
    }
    if o.allow_other && os == Os::Windows {
        return Err(bad("Allowing other users isn't available on Windows"));
    }
    if (o.uid.is_some() || o.gid.is_some()) && os == Os::Windows {
        return Err(bad("Owner and group can't be set on Windows"));
    }
    if let Some(p) = &o.file_perms {
        o.file_perms = Some(normalize_perms(p, "File permissions")?);
    }
    if let Some(p) = &o.dir_perms {
        o.dir_perms = Some(normalize_perms(p, "Folder permissions")?);
    }
    if let Some(d) = &o.dir_cache_time {
        if !valid_duration(d) {
            return Err(bad("The directory cache time must look like 30s, 5m or 1h"));
        }
    }
    if let Some(dir) = &o.cache_dir {
        let dir = PathBuf::from(expand_home(dir));
        if !dir.is_absolute() || dir.components().any(|c| matches!(c, std::path::Component::ParentDir)) {
            return Err(bad("The cache folder must be a full path without .."));
        }
        if dir.starts_with(mount_point) {
            return Err(bad("The cache folder can't be inside the mount point"));
        }
        o.cache_dir = Some(dir.to_string_lossy().into_owned());
    }
    o.extra_flags = normalize_extra_flags(&o.extra_flags)?;
    Ok(o)
}

/// The options to mount with under `policy`, and what the policy changed.
pub fn effective(requested: &MountOptions, protocol: &str, mount_point: &Path, policy: Option<&ManagedMounts>) -> Result<(MountOptions, Vec<String>), AppError> {
    effective_on(requested, protocol, mount_point, policy, Os::current())
}

/// `effective` for a given operating system.
pub fn effective_on(requested: &MountOptions, protocol: &str, mount_point: &Path, policy: Option<&ManagedMounts>, os: Os) -> Result<(MountOptions, Vec<String>), AppError> {
    // The profile's protocol list comes first: a refused protocol is refused whatever the options.
    if let Some(allowed) = policy.and_then(|p| p.allowed_protocols.as_ref()) {
        if !allowed.iter().any(|a| a.eq_ignore_ascii_case(protocol)) {
            return Err(AppError::PermissionDenied {
                message: format!("Your organization does not allow mounting {protocol} connections"),
                advice: format!("Allowed: {}. Use sync or transfer instead.", if allowed.is_empty() { "none".to_string() } else { allowed.join(", ") }),
            });
        }
    }
    let mut o = validate_on(requested, protocol, mount_point, os)?;
    let mut notes = Vec::new();
    let Some(p) = policy else { return Ok((o, notes)) };
    if let Some(root) = p.mount_root.as_deref() {
        if o.drive_letter.is_some() {
            return Err(AppError::PermissionDenied {
                message: "Your organization requires mount points inside a folder, so a drive letter can't be used".into(),
                advice: "Clear the drive letter and choose a folder inside the mount root.".into(),
            });
        }
        let root = PathBuf::from(expand_home(root));
        let root = root.canonicalize().unwrap_or(root);
        if !mount_point.starts_with(&root) {
            return Err(AppError::PermissionDenied {
                message: format!("Mount points must be inside {}", root.display()),
                advice: "Choose a folder there (your organization's mount profile).".into(),
            });
        }
    }
    if p.read_only == Some(true) && !o.read_only {
        o.read_only = true;
        notes.push("read-only (organization policy)".into());
    }
    if let Some(mode) = p.cache_mode.as_deref().and_then(CacheMode::parse) {
        if mode != o.cache_mode {
            o.cache_mode = mode;
            notes.push(format!("cache mode {} (organization policy)", mode.as_rclone()));
        }
    }
    if let Some(max) = p.max_cache_mb {
        if o.max_cache_mb.map_or(true, |m| m > max) {
            o.max_cache_mb = Some(max);
            notes.push(format!("cache up to {max} MB (organization policy)"));
        }
    }
    if let Some(max) = p.max_cache_age_hours {
        if o.max_cache_age_hours.map_or(true, |h| h > max) {
            o.max_cache_age_hours = Some(max);
            notes.push(format!("cached files kept up to {max} h (organization policy)"));
        }
    }
    Ok((o, notes))
}

/// The Windows mount target for `o`: the drive letter (`X:`) when one is chosen, else `mount_point`.
pub fn mount_target(mount_point: &str, o: &MountOptions, os: Os) -> String {
    match (&o.drive_letter, os) {
        (Some(l), Os::Windows) => format!("{l}:"),
        _ => mount_point.to_string(),
    }
}

/// `rclone mount` flags for `o`, with the cache in `cache_dir`.
pub fn rclone_flags(o: &MountOptions, cache_dir: &Path) -> Vec<String> {
    rclone_flags_on(o, cache_dir, Os::current())
}

/// `rclone_flags` for a given operating system: flags the OS doesn't support
/// are left out (`validate_on` refuses them before this).
pub fn rclone_flags_on(o: &MountOptions, cache_dir: &Path, os: Os) -> Vec<String> {
    let mut v = vec!["--vfs-cache-mode".into(), o.cache_mode.as_rclone().into(), "--cache-dir".into(), cache_dir.to_string_lossy().into_owned()];
    if let Some(mb) = o.max_cache_mb {
        v.extend(["--vfs-cache-max-size".into(), format!("{mb}M")]);
    }
    if let Some(h) = o.max_cache_age_hours {
        v.extend(["--vfs-cache-max-age".into(), format!("{h}h")]);
    }
    if o.read_only {
        v.push("--read-only".into());
    }
    if let Some(d) = &o.dir_cache_time {
        v.extend(["--dir-cache-time".into(), d.clone()]);
    }
    if let (Some(label), true) = (&o.volume_label, os != Os::Linux) {
        v.extend(["--volname".into(), label.clone()]);
    }
    if o.network_mode && os == Os::Windows {
        v.push("--network-mode".into());
    }
    if o.allow_other && os != Os::Windows {
        v.push("--allow-other".into());
    }
    if let Some(p) = &o.file_perms {
        v.extend(["--file-perms".into(), p.clone()]);
    }
    if let Some(p) = &o.dir_perms {
        v.extend(["--dir-perms".into(), p.clone()]);
    }
    if os != Os::Windows {
        if let Some(u) = o.uid {
            v.extend(["--uid".into(), u.to_string()]);
        }
        if let Some(g) = o.gid {
            v.extend(["--gid".into(), g.to_string()]);
        }
    }
    v.extend(o.extra_flags.iter().cloned());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_policy_keeps_the_choice() {
        let o = MountOptions { cache_mode: CacheMode::Full, max_cache_mb: Some(10_000), ..Default::default() };
        assert_eq!(effective(&o, "rclone", Path::new("/home/u/x"), None).unwrap(), (o, vec![]));
    }

    #[test]
    fn policy_forces_caps_and_refuses() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("Drives");
        std::fs::create_dir_all(&root).unwrap();
        let p = ManagedMounts {
            allowed_protocols: Some(vec!["rclone".into(), "s3".into()]),
            read_only: Some(true),
            cache_mode: Some("full".into()),
            max_cache_mb: Some(2048),
            max_cache_age_hours: Some(24),
            mount_root: Some(root.to_string_lossy().into_owned()),
        };
        let asked = MountOptions { cache_mode: CacheMode::Off, max_cache_mb: Some(50_000), max_cache_age_hours: Some(2), read_only: false, ..Default::default() };
        let (o, notes) = effective(&asked, "S3", &root.canonicalize().unwrap().join("bucket"), Some(&p)).unwrap();
        assert_eq!(o, MountOptions { cache_mode: CacheMode::Full, max_cache_mb: Some(2048), max_cache_age_hours: Some(2), read_only: true, ..Default::default() });
        assert_eq!(notes.len(), 3, "{notes:?}");
        assert!(effective(&asked, "smb", &root.join("x"), Some(&p)).unwrap_err().to_string().contains("does not allow mounting smb"));
        assert!(effective(&asked, "rclone", Path::new("/tmp/elsewhere"), Some(&p)).unwrap_err().to_string().contains("must be inside"));
    }

    #[test]
    fn flags() {
        let o = MountOptions { cache_mode: CacheMode::Full, max_cache_mb: Some(512), max_cache_age_hours: Some(48), read_only: true, ..Default::default() };
        assert_eq!(
            rclone_flags(&o, Path::new("/c/m1")).join(" "),
            "--vfs-cache-mode full --cache-dir /c/m1 --vfs-cache-max-size 512M --vfs-cache-max-age 48h --read-only"
        );
        assert_eq!(rclone_flags(&MountOptions::default(), Path::new("/c")).join(" "), "--vfs-cache-mode writes --cache-dir /c");
    }

    fn adv() -> MountOptions {
        MountOptions {
            volume_label: Some("Team drive".into()),
            file_perms: Some("644".into()),
            dir_perms: Some("0755".into()),
            dir_cache_time: Some("5m".into()),
            extra_flags: vec!["--buffer-size".into(), "32M".into(), "--no-modtime".into()],
            ..Default::default()
        }
    }

    fn joined(o: &MountOptions, os: Os) -> String {
        rclone_flags_on(o, Path::new("/c"), os).join(" ")
    }

    #[test]
    fn advanced_flags_per_platform() {
        let mut o = validate_on(&adv(), "rclone", Path::new("/mnt/x"), Os::MacOs).unwrap();
        o.allow_other = true;
        o.uid = Some(501);
        o.gid = Some(20);
        let common = "--vfs-cache-mode writes --cache-dir /c --dir-cache-time 5m";
        let perms = "--file-perms 0644 --dir-perms 0755";
        let extra = "--buffer-size=32M --no-modtime";
        assert_eq!(joined(&o, Os::MacOs), format!("{common} --volname Team drive --allow-other {perms} --uid 501 --gid 20 {extra}"));
        // Linux has no --volname; Windows has no --allow-other/--uid/--gid but has --network-mode.
        assert_eq!(joined(&o, Os::Linux), format!("{common} --allow-other {perms} --uid 501 --gid 20 {extra}"));
        o.network_mode = true;
        assert_eq!(joined(&o, Os::Windows), format!("{common} --volname Team drive --network-mode {perms} {extra}"));
        assert!(!joined(&o, Os::MacOs).contains("--network-mode"));
    }

    #[test]
    fn defaults_add_no_advanced_flags() {
        for os in [Os::Windows, Os::MacOs, Os::Linux] {
            assert_eq!(joined(&MountOptions::default(), os), "--vfs-cache-mode writes --cache-dir /c");
        }
    }

    #[test]
    fn options_are_refused_where_the_platform_lacks_them() {
        let mp = Path::new("/mnt/x");
        let check = |o: MountOptions, os: Os| validate_on(&o, "rclone", mp, os).unwrap_err().to_string();
        assert!(check(MountOptions { drive_letter: Some("X".into()), ..Default::default() }, Os::Linux).contains("only available on Windows"));
        assert!(check(MountOptions { network_mode: true, ..Default::default() }, Os::MacOs).contains("only available on Windows"));
        assert!(check(MountOptions { allow_other: true, ..Default::default() }, Os::Windows).contains("isn't available on Windows"));
        assert!(check(MountOptions { uid: Some(1000), ..Default::default() }, Os::Windows).contains("Owner and group"));
        assert!(check(MountOptions { volume_label: Some("x".into()), ..Default::default() }, Os::Linux).contains("Windows and macOS"));
        // ...and accepted where it has them.
        assert!(validate_on(&MountOptions { volume_label: Some("x".into()), ..Default::default() }, "rclone", mp, Os::Windows).is_ok());
    }

    #[test]
    fn drive_letters() {
        let w = |l: &str| validate_on(&MountOptions { drive_letter: Some(l.into()), ..Default::default() }, "rclone", Path::new("X:"), Os::Windows);
        assert_eq!(w("x:").unwrap().drive_letter.as_deref(), Some("X"));
        assert_eq!(w(" z ").unwrap().drive_letter.as_deref(), Some("Z"));
        for bad in ["C", "A:", "1", "XY", "::"] {
            assert!(w(bad).is_err(), "{bad}");
        }
        let o = MountOptions { drive_letter: Some("e:".into()), ..Default::default() };
        assert_eq!(drive_mount_point(&o, Os::Windows).as_deref(), Some("E:"));
        assert_eq!(drive_mount_point(&o, Os::Linux), None);
        assert_eq!(mount_target("/mnt/x", &MountOptions { drive_letter: Some("E".into()), ..Default::default() }, Os::Windows), "E:");
        assert_eq!(mount_target("/mnt/x", &MountOptions { drive_letter: Some("E".into()), ..Default::default() }, Os::Linux), "/mnt/x");
        assert_eq!(mount_target("C:\\Mounts\\x", &MountOptions::default(), Os::Windows), "C:\\Mounts\\x");
        let free = free_drive_letters_with(|c| matches!(c, 'D' | 'F'));
        assert_eq!((free.first().map(String::as_str), free.get(1).map(String::as_str), free.len()), (Some("E"), Some("G"), 21));
    }

    #[test]
    fn permissions_and_durations_are_checked() {
        let v = |f: MountOptions| validate_on(&f, "rclone", Path::new("/m"), Os::Linux);
        assert_eq!(v(MountOptions { file_perms: Some("600".into()), ..Default::default() }).unwrap().file_perms.as_deref(), Some("0600"));
        for bad in ["888", "rwx", "12", "07777", "0o9"] {
            assert!(v(MountOptions { dir_perms: Some(bad.into()), ..Default::default() }).is_err(), "{bad}");
        }
        for ok in ["30s", "5m", "1h30m", "500ms", "0s"] {
            assert!(v(MountOptions { dir_cache_time: Some(ok.into()), ..Default::default() }).is_ok(), "{ok}");
        }
        for bad in ["", "30", "s", "5 minutes", "-5s", "1.5h"] {
            let r = v(MountOptions { dir_cache_time: Some(bad.into()), ..Default::default() });
            // A blank value means "default" and is cleared, not an error.
            assert_eq!(r.is_err(), !bad.is_empty(), "{bad:?}");
        }
        let long = MountOptions { volume_label: Some("x".repeat(40)), ..Default::default() };
        assert!(validate_on(&long, "rclone", Path::new("/m"), Os::MacOs).is_err());
    }

    #[test]
    fn cache_folder_must_be_absolute_and_outside_the_mount() {
        let v = |dir: &str| validate_on(&MountOptions { cache_dir: Some(dir.into()), ..Default::default() }, "rclone", Path::new("/home/u/Drives/a"), Os::Linux);
        assert_eq!(v("/data/cache").unwrap().cache_dir.as_deref(), Some("/data/cache"));
        assert!(v("relative/cache").is_err());
        assert!(v("/data/../etc").is_err());
        assert!(v("/home/u/Drives/a/cache").unwrap_err().to_string().contains("inside the mount point"));
    }

    #[test]
    fn subfolders_stay_inside_the_connection() {
        assert_eq!(normalize_subfolder("/photos//2024/").unwrap().as_deref(), Some("photos/2024"));
        assert_eq!(normalize_subfolder("a\\b/./c").unwrap().as_deref(), Some("a/b/c"));
        assert_eq!(normalize_subfolder("  / ").unwrap(), None);
        assert!(normalize_subfolder("a/../../etc").is_err());
        assert!(normalize_subfolder("a\u{0}b").is_err());
        assert_eq!(join_subfolder("/bucket", Some("photos")), "/bucket/photos");
        assert_eq!(join_subfolder("/bucket/", Some("photos/2024")), "/bucket/photos/2024");
        assert_eq!(join_subfolder("/", Some("x")), "/x");
        assert_eq!(join_subfolder("/bucket", None), "/bucket");
        // Allowed for every protocol (native mounts take it too).
        let o = validate_on(&MountOptions { subfolder: Some("docs".into()), ..Default::default() }, "sftp", Path::new("/m"), Os::Linux).unwrap();
        assert_eq!(o.subfolder.as_deref(), Some("docs"));
    }

    #[test]
    fn rclone_only_options_are_refused_for_native_mounts() {
        let e = validate_on(&MountOptions { read_only: true, dir_perms: Some("755".into()), ..Default::default() }, "smb", Path::new("/m"), Os::Linux).unwrap_err();
        assert!(e.to_string().contains("read-only, folder permissions applies to cloud (rclone) drives only"), "{e}");
        assert!(validate_on(&MountOptions { open_after_mount: true, subfolder: Some("x".into()), ..Default::default() }, "smb", Path::new("/m"), Os::Linux).is_ok());
    }

    #[test]
    fn extra_flags_use_an_allow_list_and_cannot_override_controlled_ones() {
        let n = |f: &[&str]| normalize_extra_flags(&f.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(n(&["--buffer-size", "64M", "--vfs-read-ahead=128M --no-seek"]).unwrap(), ["--buffer-size=64M", "--vfs-read-ahead=128M", "--no-seek"]);
        assert_eq!(n(&[]).unwrap(), Vec::<String>::new());
        // Controlled by a field (and so by the profile): never accepted here.
        for f in ["--read-only", "--read-only=false", "--vfs-cache-max-size=999G", "--vfs-cache-mode=full", "--vfs-cache-max-age=9999h", "--cache-dir=/x", "--allow-other", "--uid=0"] {
            let e = n(&[f]).unwrap_err().to_string();
            assert!(e.contains("can't be set here"), "{f}: {e}");
        }
        // Not on the list, or not a flag.
        for f in ["--config=/tmp/evil", "--rc", "--password-command=sh", "-v", "bare", "--Bad", "--=x"] {
            assert!(n(&[f]).is_err(), "{f}");
        }
        assert!(n(&["--buffer-size=32M", "extra", "word"]).is_err(), "a value belongs to the flag before it");
    }

    #[test]
    fn profile_wins_over_advanced_options() {
        let home = tempfile::tempdir().unwrap();
        let root = home.path().join("Drives");
        std::fs::create_dir_all(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let p = ManagedMounts { read_only: Some(true), max_cache_mb: Some(100), mount_root: Some(root.to_string_lossy().into_owned()), ..Default::default() };
        // Extra flags can't lift the forced read-only or the cache cap.
        for f in ["--read-only=false", "--vfs-cache-max-size=50G"] {
            let o = MountOptions { extra_flags: vec![f.into()], ..Default::default() };
            assert!(effective_on(&o, "rclone", &root.join("x"), Some(&p), Os::Linux).is_err(), "{f}");
        }
        // Allowed options pass through and the forced ones are still applied.
        let o = MountOptions { extra_flags: vec!["--no-modtime".into()], dir_cache_time: Some("1m".into()), ..Default::default() };
        let (e, notes) = effective_on(&o, "rclone", &root.join("x"), Some(&p), Os::Linux).unwrap();
        assert!(e.read_only && e.max_cache_mb == Some(100) && notes.len() == 2, "{notes:?}");
        assert!(joined(&e, Os::Linux).contains("--read-only") && joined(&e, Os::Linux).contains("--vfs-cache-max-size 100M"));
        // A drive letter is outside any mount root.
        let w = MountOptions { drive_letter: Some("X".into()), ..Default::default() };
        let err = effective_on(&w, "rclone", Path::new("X:"), Some(&p), Os::Windows).unwrap_err();
        assert!(err.to_string().contains("drive letter can't be used"), "{err}");
        assert!(effective_on(&w, "rclone", Path::new("X:"), None, Os::Windows).is_ok());
    }

    #[test]
    fn old_saved_options_still_load() {
        let o: MountOptions = serde_json::from_str(r#"{"cache_mode":"full","max_cache_mb":512,"max_cache_age_hours":null,"read_only":true}"#).unwrap();
        assert_eq!((o.cache_mode, o.max_cache_mb, o.read_only), (CacheMode::Full, Some(512), true));
        assert!(o.subfolder.is_none() && o.extra_flags.is_empty() && !o.open_after_mount);
        assert_eq!(serde_json::from_str::<MountOptions>("{}").unwrap(), MountOptions::default());
    }
}
