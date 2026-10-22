//! Platform metadata and I/O helpers for the backup engine.
//!
//! - Extended attributes (Linux/macOS), which also carry POSIX ACLs on
//!   Linux (`system.posix_acl_access` / `system.posix_acl_default`).
//! - macOS ACLs (`com.apple.acl` is not an xattr; read through `ls -le`'s
//!   API equivalent `acl_get_file`) and BSD file flags (`chflags`).
//! - Windows file attributes and security descriptors (SDDL), and NTFS
//!   alternate data streams.
//! - Backup-exclusion attributes, advisory locks, background I/O priority
//!   and free-space queries.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// Extended metadata captured per entry (all optional, omitted when empty).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtMeta {
    /// Extended attributes, values base64-encoded.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub xattrs: BTreeMap<String, String>,
    /// macOS ACL in text form (`acl_to_text`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acl: Option<String>,
    /// macOS/BSD file flags (`st_flags`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flags: Option<u32>,
    /// Windows file attributes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_attrs: Option<u32>,
    /// Windows security descriptor (owner, group, DACL) as SDDL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub win_sddl: Option<String>,
    /// The link is an NTFS junction (mount point), not a symbolic link.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub junction: bool,
}

impl ExtMeta {
    pub fn is_empty(&self) -> bool {
        self == &Self::default()
    }
}

/// Reparse tag of an NTFS mount point (junction).
pub const IO_REPARSE_TAG_MOUNT_POINT: u32 = 0xA000_0003;

/// Whether `path` is an NTFS junction (always false off Windows).
#[allow(unused_variables)]
pub fn is_junction(path: &Path) -> bool {
    #[cfg(windows)]
    return win_sec::reparse_tag(path) == Some(IO_REPARSE_TAG_MOUNT_POINT);
    #[allow(unreachable_code)]
    false
}

/// Recreate a junction at `link` pointing to the folder `target`.
#[allow(unused_variables)]
pub fn create_junction(link: &Path, target: &str) -> std::io::Result<()> {
    #[cfg(windows)]
    {
        // `\??\C:\x` (as read back from the reparse point) → `C:\x`.
        let t = target.strip_prefix(r"\??\").unwrap_or(target);
        let st = std::process::Command::new("cmd").args(["/C", "mklink", "/J"]).arg(link).arg(t).output()?;
        if !st.status.success() {
            return Err(std::io::Error::other(String::from_utf8_lossy(&st.stderr).trim().to_string()));
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(std::io::Error::new(std::io::ErrorKind::Unsupported, "junctions exist only on Windows"))
}

/// Which metadata to capture.
#[derive(Debug, Clone, Copy)]
pub struct MetaOptions {
    pub xattrs: bool,
    pub acls: bool,
}

#[cfg(unix)]
fn b64() -> base64::engine::GeneralPurpose {
    base64::engine::general_purpose::STANDARD
}

/// Capture extended metadata for `path` (never follows symlinks).
#[allow(unused_variables, unused_mut)]
pub fn capture(path: &Path, opts: MetaOptions) -> ExtMeta {
    #[cfg(unix)]
    use base64::Engine;
    let mut m = ExtMeta::default();
    #[cfg(unix)]
    if opts.xattrs || opts.acls {
        if let Ok(names) = xattr::list_deref(path).or_else(|_| xattr::list(path)) {
            for n in names {
                let name = n.to_string_lossy().into_owned();
                let is_acl = name.starts_with("system.posix_acl");
                if (is_acl && !opts.acls) || (!is_acl && !opts.xattrs) {
                    continue;
                }
                if let Ok(Some(v)) = xattr::get(path, &n) {
                    m.xattrs.insert(name, b64().encode(v));
                }
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::MetadataExt;
        if let Ok(md) = std::fs::symlink_metadata(path) {
            if md.st_flags() != 0 {
                m.flags = Some(md.st_flags());
            }
        }
        if opts.acls {
            m.acl = macos_acl::get(path);
        }
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if let Ok(md) = std::fs::symlink_metadata(path) {
            m.win_attrs = Some(md.file_attributes());
        }
        if opts.acls {
            m.win_sddl = win_sec::get_sddl(path);
        }
    }
    m
}

/// Restore extended metadata; returns warnings for anything that failed
/// (e.g. `trusted.*` or ACL attributes without privileges).
#[allow(unused_variables)]
pub fn apply(path: &Path, m: &ExtMeta, acls: bool) -> Vec<String> {
    #[cfg(unix)]
    use base64::Engine;
    let mut warn = Vec::new();
    #[cfg(unix)]
    for (k, v) in &m.xattrs {
        if k.starts_with("system.posix_acl") && !acls {
            continue;
        }
        let Ok(val) = b64().decode(v) else { continue };
        if let Err(e) = xattr::set(path, k, &val) {
            warn.push(format!("{}: attribute {k}: {e}", path.display()));
        }
    }
    #[cfg(target_os = "macos")]
    {
        if acls {
            if let Some(a) = &m.acl {
                if let Err(e) = macos_acl::set(path, a) {
                    warn.push(format!("{}: ACL: {e}", path.display()));
                }
            }
        }
        if let Some(f) = m.flags {
            let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).unwrap_or_default();
            if unsafe { libc::chflags(c.as_ptr(), f as _) } != 0 {
                warn.push(format!("{}: flags: {}", path.display(), std::io::Error::last_os_error()));
            }
        }
    }
    #[cfg(windows)]
    {
        if acls {
            if let Some(s) = &m.win_sddl {
                if let Err(e) = win_sec::set_sddl(path, s) {
                    warn.push(format!("{}: security descriptor: {e}", path.display()));
                }
            }
        }
        if let Some(a) = m.win_attrs {
            if let Err(e) = win_sec::set_attributes(path, a) {
                warn.push(format!("{}: attributes: {e}", path.display()));
            }
        }
    }
    warn
}

/// macOS Time Machine and freedesktop backup-exclusion attributes.
pub const EXCLUDE_XATTRS: &[&str] = &["com.apple.metadata:com_apple_backup_excludeItem", "user.xdg.robots.backup"];

/// True when the item carries a backup-exclusion attribute: the macOS
/// Time Machine attribute (any value), or `user.xdg.robots.backup` set to
/// `false`/`0`/`no`.
#[allow(unused_variables)]
pub fn has_exclusion_attribute(path: &Path) -> bool {
    #[cfg(unix)]
    {
        if let Ok(Some(_)) = xattr::get(path, EXCLUDE_XATTRS[0]) {
            return true;
        }
        if let Ok(Some(v)) = xattr::get(path, EXCLUDE_XATTRS[1]) {
            let s = String::from_utf8_lossy(&v).trim().to_ascii_lowercase();
            return matches!(s.as_str(), "false" | "0" | "no");
        }
    }
    false
}

/// Whether another process holds an advisory lock on the file (Unix
/// `flock`). Always `false` on Windows, where locks are mandatory.
#[allow(unused_variables)]
pub fn is_advisory_locked(file: &std::fs::File) -> bool {
    #[cfg(unix)]
    {
        use std::os::fd::AsRawFd;
        let fd = file.as_raw_fd();
        if unsafe { libc::flock(fd, libc::LOCK_SH | libc::LOCK_NB) } != 0 {
            return std::io::Error::last_os_error().raw_os_error() == Some(libc::EWOULDBLOCK);
        }
        unsafe { libc::flock(fd, libc::LOCK_UN) };
    }
    false
}

/// Lower the current thread's CPU and I/O priority (background mode).
pub fn set_background_priority() {
    #[cfg(target_os = "linux")]
    unsafe {
        // ioprio_set(IOPRIO_WHO_PROCESS, 0 = this thread, IOPRIO_CLASS_IDLE << 13)
        libc::syscall(libc::SYS_ioprio_set, 1, 0, 3 << 13);
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10);
    }
    #[cfg(target_os = "macos")]
    unsafe {
        // IOPOL_TYPE_DISK = 0, IOPOL_SCOPE_THREAD = 1, IOPOL_THROTTLE = 3
        extern "C" {
            fn setiopolicy_np(iotype: i32, scope: i32, policy: i32) -> i32;
        }
        setiopolicy_np(0, 1, 3);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};
        SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
    }
}

/// Free bytes available to the user on the filesystem holding `path`.
#[allow(unused_variables)]
pub fn free_space(path: &Path) -> Option<u64> {
    let mut p = path.to_path_buf();
    while !p.exists() {
        p = p.parent()?.to_path_buf();
    }
    #[cfg(unix)]
    {
        let c = std::ffi::CString::new(p.as_os_str().as_encoded_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        return Some(st.f_bavail as u64 * st.f_frsize as u64);
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let w: Vec<u16> = p.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut avail = 0u64;
        if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, std::ptr::null_mut(), std::ptr::null_mut()) } == 0 {
            return None;
        }
        return Some(avail);
    }
    #[allow(unreachable_code)]
    None
}

/// Total size of the filesystem holding `path`.
#[allow(unused_variables)]
pub fn total_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
        let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
            return None;
        }
        return Some(st.f_blocks as u64 * st.f_frsize as u64);
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let w: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        let mut total = 0u64;
        if unsafe { GetDiskFreeSpaceExW(w.as_ptr(), std::ptr::null_mut(), &mut total, std::ptr::null_mut()) } == 0 {
            return None;
        }
        return Some(total);
    }
    #[allow(unreachable_code)]
    None
}

/// Set a symlink's own modification time (does not follow it).
#[allow(unused_variables)]
pub fn set_symlink_mtime(path: &Path, mtime: i64) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let c = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())?;
        let t = libc::timespec { tv_sec: mtime as libc::time_t, tv_nsec: 0 };
        let times = [t, t];
        if unsafe { libc::utimensat(libc::AT_FDCWD, c.as_ptr(), times.as_ptr(), libc::AT_SYMLINK_NOFOLLOW) } != 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

/// NTFS alternate data streams of a file: `(stream name, size)`, excluding
/// the unnamed main stream. Empty on other platforms.
#[allow(unused_variables)]
pub fn alternate_streams(path: &Path) -> Vec<(String, u64)> {
    #[cfg(windows)]
    {
        return win_sec::streams(path);
    }
    #[allow(unreachable_code)]
    Vec::new()
}

#[cfg(windows)]
mod win_sec {
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use windows_sys::Win32::Foundation::{LocalFree, ERROR_SUCCESS, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Security::Authorization::{
        ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SE_FILE_OBJECT, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::{
        GetSecurityDescriptorDacl, SetFileSecurityW, DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
    };
    use windows_sys::Win32::Storage::FileSystem::{FindClose, FindFirstStreamW, FindNextStreamW, FindStreamInfoStandard, SetFileAttributesW, WIN32_FIND_STREAM_DATA};

    /// Reparse tag of a file or folder, without following it.
    pub fn reparse_tag(p: &Path) -> Option<u32> {
        use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ};
        use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
        use windows_sys::Win32::System::Ioctl::FSCTL_GET_REPARSE_POINT;
        use windows_sys::Win32::System::IO::DeviceIoControl;
        let w = wide(p);
        unsafe {
            let h = CreateFileW(w.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, std::ptr::null(), OPEN_EXISTING, FILE_FLAG_OPEN_REPARSE_POINT | FILE_FLAG_BACKUP_SEMANTICS, std::ptr::null_mut());
            if h == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut buf = vec![0u8; 16 * 1024];
            let mut ret = 0u32;
            let ok = DeviceIoControl(h, FSCTL_GET_REPARSE_POINT, std::ptr::null(), 0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut ret, std::ptr::null_mut());
            CloseHandle(h);
            (ok != 0 && ret >= 4).then(|| u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]))
        }
    }

    fn wide(p: &Path) -> Vec<u16> {
        p.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    pub fn get_sddl(path: &Path) -> Option<String> {
        let w = wide(path);
        let info = OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        let rc = unsafe { GetNamedSecurityInfoW(w.as_ptr(), SE_FILE_OBJECT, info, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut sd) };
        if rc != ERROR_SUCCESS || sd.is_null() {
            return None;
        }
        let mut out: *mut u16 = std::ptr::null_mut();
        let mut len = 0u32;
        let ok = unsafe { ConvertSecurityDescriptorToStringSecurityDescriptorW(sd, SDDL_REVISION_1, info, &mut out, &mut len) };
        let s = if ok != 0 && !out.is_null() {
            let sl = unsafe { std::slice::from_raw_parts(out, len as usize) };
            Some(String::from_utf16_lossy(sl).trim_end_matches('\0').to_string())
        } else {
            None
        };
        unsafe {
            LocalFree(out as _);
            LocalFree(sd as _);
        }
        s
    }

    pub fn set_sddl(path: &Path, sddl: &str) -> std::io::Result<()> {
        let ws: Vec<u16> = sddl.encode_utf16().chain(Some(0)).collect();
        let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        if unsafe { ConvertStringSecurityDescriptorToSecurityDescriptorW(ws.as_ptr(), SDDL_REVISION_1, &mut sd, std::ptr::null_mut()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut present = 0;
        let mut dacl = std::ptr::null_mut();
        let mut defaulted = 0;
        unsafe { GetSecurityDescriptorDacl(sd, &mut present, &mut dacl, &mut defaulted) };
        let w = wide(path);
        let ok = unsafe { SetFileSecurityW(w.as_ptr(), OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION, sd) };
        unsafe { LocalFree(sd as _) };
        if ok == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn set_attributes(path: &Path, attrs: u32) -> std::io::Result<()> {
        let w = wide(path);
        if unsafe { SetFileAttributesW(w.as_ptr(), attrs) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn streams(path: &Path) -> Vec<(String, u64)> {
        let w = wide(path);
        let mut data: WIN32_FIND_STREAM_DATA = unsafe { std::mem::zeroed() };
        let h = unsafe { FindFirstStreamW(w.as_ptr(), FindStreamInfoStandard, &mut data as *mut _ as *mut _, 0) };
        if h == INVALID_HANDLE_VALUE {
            return Vec::new();
        }
        let mut out = Vec::new();
        loop {
            let len = data.cStreamName.iter().position(|c| *c == 0).unwrap_or(data.cStreamName.len());
            let name = String::from_utf16_lossy(&data.cStreamName[..len]);
            // Names look like ":stream:$DATA"; skip the unnamed "::$DATA".
            if let Some(n) = name.strip_prefix(':').and_then(|n| n.strip_suffix(":$DATA")) {
                if !n.is_empty() {
                    out.push((n.to_string(), data.StreamSize as u64));
                }
            }
            if unsafe { FindNextStreamW(h, &mut data as *mut _ as *mut _) } == 0 {
                break;
            }
        }
        unsafe { FindClose(h) };
        out
    }
}

#[cfg(target_os = "macos")]
mod macos_acl {
    use std::ffi::{CStr, CString};
    use std::os::raw::{c_char, c_int, c_void};
    use std::path::Path;

    type AclT = *mut c_void;
    const ACL_TYPE_EXTENDED: c_int = 0x00000100;
    extern "C" {
        fn acl_get_file(path: *const c_char, ty: c_int) -> AclT;
        fn acl_set_file(path: *const c_char, ty: c_int, acl: AclT) -> c_int;
        fn acl_to_text(acl: AclT, len: *mut isize) -> *mut c_char;
        fn acl_from_text(text: *const c_char) -> AclT;
        fn acl_free(obj: *mut c_void) -> c_int;
    }

    pub fn get(path: &Path) -> Option<String> {
        let c = CString::new(path.as_os_str().as_encoded_bytes()).ok()?;
        let acl = unsafe { acl_get_file(c.as_ptr(), ACL_TYPE_EXTENDED) };
        if acl.is_null() {
            return None;
        }
        let t = unsafe { acl_to_text(acl, std::ptr::null_mut()) };
        let s = (!t.is_null()).then(|| unsafe { CStr::from_ptr(t) }.to_string_lossy().into_owned());
        unsafe {
            if !t.is_null() {
                acl_free(t as *mut c_void);
            }
            acl_free(acl);
        }
        s.filter(|s| !s.trim().is_empty())
    }

    pub fn set(path: &Path, text: &str) -> std::io::Result<()> {
        let c = CString::new(path.as_os_str().as_encoded_bytes())?;
        let t = CString::new(text)?;
        let acl = unsafe { acl_from_text(t.as_ptr()) };
        if acl.is_null() {
            return Err(std::io::Error::last_os_error());
        }
        let rc = unsafe { acl_set_file(c.as_ptr(), ACL_TYPE_EXTENDED, acl) };
        unsafe { acl_free(acl) };
        if rc != 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn xattrs_roundtrip_and_exclusion() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("f");
        std::fs::write(&f, b"x").unwrap();
        if xattr::set(&f, "user.test", b"value").is_err() {
            eprintln!("filesystem without user xattrs; skipping");
            return;
        }
        let m = capture(&f, MetaOptions { xattrs: true, acls: true });
        assert!(m.xattrs.contains_key("user.test"));
        let g = d.path().join("g");
        std::fs::write(&g, b"y").unwrap();
        assert!(apply(&g, &m, true).is_empty());
        assert_eq!(xattr::get(&g, "user.test").unwrap().unwrap(), b"value");
        assert!(!has_exclusion_attribute(&g));
        xattr::set(&g, "user.xdg.robots.backup", b"false").unwrap();
        assert!(has_exclusion_attribute(&g));
        xattr::set(&g, "user.xdg.robots.backup", b"true").unwrap();
        assert!(!has_exclusion_attribute(&g));
        let none = capture(&f, MetaOptions { xattrs: false, acls: false });
        assert!(none.is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn advisory_lock_detection() {
        use std::os::fd::AsRawFd;
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("locked");
        std::fs::write(&p, b"x").unwrap();
        let holder = std::fs::File::open(&p).unwrap();
        assert!(!is_advisory_locked(&std::fs::File::open(&p).unwrap()));
        unsafe { libc::flock(holder.as_raw_fd(), libc::LOCK_EX) };
        assert!(is_advisory_locked(&std::fs::File::open(&p).unwrap()));
    }

    #[test]
    fn free_space_and_priority() {
        let d = tempfile::tempdir().unwrap();
        assert!(free_space(&d.path().join("missing/child")).unwrap_or(1) > 0);
        set_background_priority();
    }

    #[cfg(unix)]
    #[test]
    fn symlink_mtime() {
        let d = tempfile::tempdir().unwrap();
        let l = d.path().join("l");
        std::os::unix::fs::symlink("target", &l).unwrap();
        set_symlink_mtime(&l, 1_000_000).unwrap();
        let md = std::fs::symlink_metadata(&l).unwrap();
        assert_eq!(md.modified().unwrap().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs(), 1_000_000);
    }

    #[test]
    fn junction_flag_serialises_only_when_set() {
        assert_eq!(serde_json::to_string(&ExtMeta::default()).unwrap(), "{}");
        let j = ExtMeta { junction: true, ..Default::default() };
        let s = serde_json::to_string(&j).unwrap();
        assert_eq!(s, r#"{"junction":true}"#);
        assert_eq!(serde_json::from_str::<ExtMeta>(&s).unwrap(), j);
        assert!(!is_junction(Path::new("/")));
        #[cfg(not(windows))]
        assert!(create_junction(Path::new("/tmp/x"), "/tmp").is_err());
    }
}
