//! NTFS USN change journal (Windows): find the folders whose contents
//! changed since the previous backup, so only those are rescanned.
//!
//! State per volume (journal id + next USN) is stored in the job database
//! after each successful backup. When the journal was reset, wrapped past
//! the stored position, or no state exists, the caller falls back to a full
//! scan.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Saved journal position per volume root (e.g. `C:\`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsnState {
    pub volumes: BTreeMap<String, (u64, i64)>,
}

/// Result of a journal query.
#[derive(Debug, Clone, Default)]
pub struct UsnChanges {
    /// Folders whose direct children changed (rescan shallowly).
    pub changed_dirs: Vec<String>,
    /// Journal positions to store once the backup succeeds.
    pub next_state: UsnState,
    /// Volumes that needed a full scan (no or stale state).
    pub full_scan_volumes: Vec<String>,
}

/// Volume root of a Windows path (`C:\…` → `C:\`).
pub fn volume_root(path: &str) -> Option<String> {
    let b = path.as_bytes();
    (b.len() >= 2 && b[1] == b':' && b[0].is_ascii_alphabetic()).then(|| format!("{}:\\", (b[0] as char).to_ascii_uppercase()))
}

#[cfg(not(windows))]
pub fn changes(_sources: &[String], _prev: &UsnState) -> Result<UsnChanges, AppError> {
    Err(AppError::validation("The NTFS change journal is only available on Windows"))
}

#[cfg(windows)]
pub fn changes(sources: &[String], prev: &UsnState) -> Result<UsnChanges, AppError> {
    let mut out = UsnChanges::default();
    let mut roots: Vec<String> = sources.iter().filter_map(|s| volume_root(s)).collect();
    roots.sort();
    roots.dedup();
    if roots.is_empty() {
        return Err(AppError::validation("No NTFS volumes among the sources"));
    }
    for root in roots {
        let v = win::Volume::open(&root)?;
        let (journal_id, first_usn, next_usn) = v.query()?;
        out.next_state.volumes.insert(root.clone(), (journal_id, next_usn));
        match prev.volumes.get(&root) {
            Some((jid, usn)) if *jid == journal_id && *usn >= first_usn => {
                for dir in v.changed_dirs(journal_id, *usn, next_usn)? {
                    out.changed_dirs.push(dir);
                }
            }
            _ => out.full_scan_volumes.push(root),
        }
    }
    out.changed_dirs.sort();
    out.changed_dirs.dedup();
    Ok(out)
}

#[cfg(windows)]
mod win {
    use crate::error::AppError;
    use std::collections::HashSet;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GENERIC_READ, HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFinalPathNameByHandleW, OpenFileById, FILE_FLAG_BACKUP_SEMANTICS, FILE_ID_DESCRIPTOR, FILE_ID_DESCRIPTOR_0, FILE_SHARE_DELETE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, FileIdType, FILE_NAME_NORMALIZED,
    };
    use windows_sys::Win32::System::Ioctl::{FSCTL_QUERY_USN_JOURNAL, FSCTL_READ_USN_JOURNAL, READ_USN_JOURNAL_DATA_V0, USN_JOURNAL_DATA_V0};
    use windows_sys::Win32::System::IO::DeviceIoControl;

    fn os_err(what: &str) -> AppError {
        AppError::file_op(format!("{what}: {}", std::io::Error::last_os_error()), "The change journal needs administrator rights on an NTFS volume.")
    }

    pub struct Volume {
        h: HANDLE,
    }

    impl Drop for Volume {
        fn drop(&mut self) {
            unsafe { CloseHandle(self.h) };
        }
    }

    impl Volume {
        pub fn open(root: &str) -> Result<Self, AppError> {
            let dev = format!("\\\\.\\{}", root.trim_end_matches('\\'));
            let w: Vec<u16> = std::ffi::OsStr::new(&dev).encode_wide().chain(Some(0)).collect();
            let h = unsafe {
                CreateFileW(w.as_ptr(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, std::ptr::null(), OPEN_EXISTING, 0, std::ptr::null_mut())
            };
            if h == INVALID_HANDLE_VALUE {
                return Err(os_err("Opening the volume"));
            }
            Ok(Self { h })
        }

        /// (journal id, first USN, next USN)
        pub fn query(&self) -> Result<(u64, i64, i64), AppError> {
            let mut d: USN_JOURNAL_DATA_V0 = unsafe { std::mem::zeroed() };
            let mut ret = 0u32;
            let ok = unsafe {
                DeviceIoControl(self.h, FSCTL_QUERY_USN_JOURNAL, std::ptr::null(), 0, &mut d as *mut _ as *mut _, std::mem::size_of::<USN_JOURNAL_DATA_V0>() as u32, &mut ret, std::ptr::null_mut())
            };
            if ok == 0 {
                return Err(os_err("Querying the change journal"));
            }
            Ok((d.UsnJournalID, d.FirstUsn, d.NextUsn))
        }

        /// Folders (parents of changed entries) between `from` and `to`.
        pub fn changed_dirs(&self, journal_id: u64, from: i64, to: i64) -> Result<Vec<String>, AppError> {
            let mut parents: HashSet<u64> = HashSet::new();
            let mut start = from;
            let mut buf = vec![0u8; 64 * 1024];
            while start < to {
                let req = READ_USN_JOURNAL_DATA_V0 { StartUsn: start, ReasonMask: 0xFFFF_FFFF, ReturnOnlyOnClose: 0, Timeout: 0, BytesToWaitFor: 0, UsnJournalID: journal_id };
                let mut ret = 0u32;
                let ok = unsafe {
                    DeviceIoControl(self.h, FSCTL_READ_USN_JOURNAL, &req as *const _ as *const _, std::mem::size_of::<READ_USN_JOURNAL_DATA_V0>() as u32, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut ret, std::ptr::null_mut())
                };
                if ok == 0 {
                    return Err(os_err("Reading the change journal"));
                }
                if ret <= 8 {
                    break;
                }
                let next = i64::from_le_bytes(buf[..8].try_into().unwrap());
                let mut off = 8usize;
                while off + 60 <= ret as usize {
                    let len = u32::from_le_bytes(buf[off..off + 4].try_into().unwrap()) as usize;
                    if len == 0 {
                        break;
                    }
                    let major = u16::from_le_bytes(buf[off + 4..off + 6].try_into().unwrap());
                    if major == 2 {
                        let parent = u64::from_le_bytes(buf[off + 16..off + 24].try_into().unwrap());
                        parents.insert(parent);
                    }
                    off += len;
                }
                if next <= start {
                    break;
                }
                start = next;
            }
            let mut out = Vec::new();
            for frn in parents {
                if let Some(p) = self.path_of(frn) {
                    out.push(p);
                }
            }
            Ok(out)
        }

        fn path_of(&self, frn: u64) -> Option<String> {
            let desc = FILE_ID_DESCRIPTOR { dwSize: std::mem::size_of::<FILE_ID_DESCRIPTOR>() as u32, Type: FileIdType, Anonymous: FILE_ID_DESCRIPTOR_0 { FileId: frn as i64 } };
            let h = unsafe { OpenFileById(self.h, &desc, 0, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE, std::ptr::null(), FILE_FLAG_BACKUP_SEMANTICS) };
            if h == INVALID_HANDLE_VALUE {
                return None;
            }
            let mut buf = vec![0u16; 1024];
            let n = unsafe { GetFinalPathNameByHandleW(h, buf.as_mut_ptr(), buf.len() as u32, FILE_NAME_NORMALIZED) };
            unsafe { CloseHandle(h) };
            if n == 0 || n as usize >= buf.len() {
                return None;
            }
            let s = String::from_utf16_lossy(&buf[..n as usize]);
            Some(s.strip_prefix("\\\\?\\").unwrap_or(&s).to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn volume_roots() {
        assert_eq!(volume_root(r"c:\Users\me").as_deref(), Some(r"C:\"));
        assert_eq!(volume_root("/home/me"), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn unsupported_off_windows() {
        assert!(changes(&["/x".into()], &UsnState::default()).is_err());
    }
}
