//! Windows file attributes and security descriptors for the Robocopy
//! switches that only exist on NTFS: /A, /M, /IA, /XA, /A+, /A- and
//! /COPY:S /COPY:O /COPY:U (/SEC).
//!
//! Letter parsing and attribute matching are portable (and unit-tested
//! everywhere); reading, changing and copying attributes and ACLs call
//! Win32 and are no-ops on other platforms.

use crate::error::AppError;
use std::path::Path;

pub const ATTR_READONLY: u32 = 0x1;
pub const ATTR_HIDDEN: u32 = 0x2;
pub const ATTR_SYSTEM: u32 = 0x4;
pub const ATTR_ARCHIVE: u32 = 0x20;
pub const ATTR_NORMAL: u32 = 0x80;
pub const ATTR_TEMPORARY: u32 = 0x100;
pub const ATTR_COMPRESSED: u32 = 0x800;
pub const ATTR_OFFLINE: u32 = 0x1000;
pub const ATTR_NOT_INDEXED: u32 = 0x2000;
pub const ATTR_ENCRYPTED: u32 = 0x4000;

/// True when this build can read and change NTFS attributes and ACLs.
pub const SUPPORTED: bool = cfg!(windows);

/// Parse Robocopy attribute letters (RASHCNETO) into a bitmask.
/// `N` means "not content indexed", as in Robocopy.
pub fn parse_letters(letters: &str) -> Result<u32, AppError> {
    let mut mask = 0;
    for c in letters.chars().filter(|c| !c.is_whitespace()) {
        mask |= match c.to_ascii_uppercase() {
            'R' => ATTR_READONLY,
            'A' => ATTR_ARCHIVE,
            'S' => ATTR_SYSTEM,
            'H' => ATTR_HIDDEN,
            'C' => ATTR_COMPRESSED,
            'N' => ATTR_NOT_INDEXED,
            'E' => ATTR_ENCRYPTED,
            'T' => ATTR_TEMPORARY,
            'O' => ATTR_OFFLINE,
            other => {
                return Err(AppError::Configuration {
                    message: format!("'{other}' is not a file attribute letter."),
                    advice: "Use R, A, S, H, C, N, E, T or O.".to_string(),
                })
            }
        };
    }
    Ok(mask)
}

/// Why a file is excluded by the Windows attribute options, if it is.
pub fn attribute_exclusion(
    attrs: u32,
    opts: &crate::sync_types::CopyOptions,
) -> Option<&'static str> {
    if !SUPPORTED {
        return None;
    }
    if (opts.archive_only || opts.archive_reset) && attrs & ATTR_ARCHIVE == 0 {
        return Some("Archive attribute not set (/A, /M)");
    }
    let include = parse_letters(&opts.include_attributes).unwrap_or(0);
    if include != 0 && attrs & include == 0 {
        return Some("Missing required attributes (/IA)");
    }
    let exclude = parse_letters(&opts.exclude_attributes).unwrap_or(0);
    if exclude != 0 && attrs & exclude != 0 {
        return Some("Excluded by attributes (/XA)");
    }
    None
}

/// The raw attribute bits of a file (0 where attributes don't exist).
pub fn attributes_of(_meta: &std::fs::Metadata) -> u32 {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        _meta.file_attributes()
    }
    #[cfg(not(windows))]
    {
        0
    }
}

/// Apply the post-copy steps: /A+ /A- on the copy, /COPY:SOU security, and
/// /M clearing the source's archive bit.
pub fn after_copy(
    src: &Path,
    dst: &Path,
    opts: &crate::sync_types::CopyOptions,
) -> Result<(), AppError> {
    if !SUPPORTED {
        return Ok(());
    }
    if opts.copy_security || opts.copy_owner || opts.copy_auditing {
        imp::copy_security(
            src,
            dst,
            opts.copy_security,
            opts.copy_owner,
            opts.copy_auditing,
        )?;
    }
    let add = parse_letters(&opts.add_attributes)?;
    let remove = parse_letters(&opts.remove_attributes)?;
    if add != 0 || remove != 0 {
        imp::change_attributes(dst, add, remove)?;
    }
    if opts.archive_reset {
        imp::change_attributes(src, 0, ATTR_ARCHIVE)?;
    }
    Ok(())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use std::os::windows::ffi::OsStrExt;
    use std::sync::Once;
    use windows_sys::Win32::Foundation::{CloseHandle, LocalFree, ERROR_SUCCESS, HANDLE};
    use windows_sys::Win32::Security::Authorization::{
        GetNamedSecurityInfoW, SetNamedSecurityInfoW, SE_FILE_OBJECT,
    };
    use windows_sys::Win32::Security::{
        AdjustTokenPrivileges, GetSecurityDescriptorControl, LookupPrivilegeValueW, ACL,
        DACL_SECURITY_INFORMATION, GROUP_SECURITY_INFORMATION, LUID_AND_ATTRIBUTES,
        OWNER_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        PSID, SACL_SECURITY_INFORMATION, SE_DACL_PROTECTED, SE_PRIVILEGE_ENABLED,
        TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
        UNPROTECTED_DACL_SECURITY_INFORMATION,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        GetFileAttributesW, SetFileAttributesW, INVALID_FILE_ATTRIBUTES,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn win_err(what: &str, path: &Path, code: u32, advice: &str) -> AppError {
        AppError::Sync {
            message: format!(
                "{what} failed for {}: {}",
                path.display(),
                std::io::Error::from_raw_os_error(code as i32)
            ),
            advice: advice.to_string(),
        }
    }

    /// Setting an owner or auditing info needs SeRestorePrivilege /
    /// SeSecurityPrivilege, which administrators hold but have disabled.
    fn enable_privileges() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token,
            ) == 0
            {
                return;
            }
            for name in [
                "SeRestorePrivilege",
                "SeSecurityPrivilege",
                "SeBackupPrivilege",
            ] {
                let wname: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
                let mut tp = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [LUID_AND_ATTRIBUTES {
                        Luid: std::mem::zeroed(),
                        Attributes: SE_PRIVILEGE_ENABLED,
                    }],
                };
                if LookupPrivilegeValueW(
                    std::ptr::null(),
                    wname.as_ptr(),
                    &mut tp.Privileges[0].Luid,
                ) != 0
                {
                    AdjustTokenPrivileges(
                        token,
                        0,
                        &tp,
                        0,
                        std::ptr::null_mut(),
                        std::ptr::null_mut(),
                    );
                }
            }
            CloseHandle(token);
        });
    }

    pub fn copy_security(
        src: &Path,
        dst: &Path,
        dacl: bool,
        owner: bool,
        sacl: bool,
    ) -> Result<(), AppError> {
        if owner || sacl {
            enable_privileges();
        }
        let mut info = 0;
        if dacl {
            info |= DACL_SECURITY_INFORMATION;
        }
        if owner {
            info |= OWNER_SECURITY_INFORMATION | GROUP_SECURITY_INFORMATION;
        }
        if sacl {
            info |= SACL_SECURITY_INFORMATION;
        }
        let wsrc = wide(src);
        let wdst = wide(dst);
        unsafe {
            let mut p_owner: PSID = std::ptr::null_mut();
            let mut p_group: PSID = std::ptr::null_mut();
            let mut p_dacl: *mut ACL = std::ptr::null_mut();
            let mut p_sacl: *mut ACL = std::ptr::null_mut();
            let mut sd: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
            let rc = GetNamedSecurityInfoW(
                wsrc.as_ptr(),
                SE_FILE_OBJECT,
                info,
                &mut p_owner,
                &mut p_group,
                &mut p_dacl,
                &mut p_sacl,
                &mut sd,
            );
            if rc != ERROR_SUCCESS {
                return Err(win_err(
                    "Reading security info",
                    src,
                    rc,
                    "Copying owner or auditing info needs administrator rights; run as administrator or turn those options off.",
                ));
            }

            // Keep the source's "inherit permissions from parent" setting.
            let mut set_info = info;
            if dacl {
                let mut control: u16 = 0;
                let mut revision: u32 = 0;
                if GetSecurityDescriptorControl(sd, &mut control, &mut revision) != 0 {
                    set_info |= if control & SE_DACL_PROTECTED != 0 {
                        PROTECTED_DACL_SECURITY_INFORMATION
                    } else {
                        UNPROTECTED_DACL_SECURITY_INFORMATION
                    };
                }
            }

            let rc = SetNamedSecurityInfoW(
                wdst.as_ptr(),
                SE_FILE_OBJECT,
                set_info,
                if owner { p_owner } else { std::ptr::null_mut() },
                if owner { p_group } else { std::ptr::null_mut() },
                if dacl { p_dacl } else { std::ptr::null() },
                if sacl { p_sacl } else { std::ptr::null() },
            );
            LocalFree(sd as _);
            if rc != ERROR_SUCCESS {
                return Err(win_err(
                    "Writing security info",
                    dst,
                    rc,
                    "Copying owner or auditing info needs administrator rights; run as administrator or turn those options off.",
                ));
            }
        }
        Ok(())
    }

    pub fn change_attributes(path: &Path, add: u32, remove: u32) -> Result<(), AppError> {
        let w = wide(path);
        unsafe {
            let current = GetFileAttributesW(w.as_ptr());
            if current == INVALID_FILE_ATTRIBUTES {
                return Err(win_err(
                    "Reading attributes",
                    path,
                    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32,
                    "Check that the file exists and you can access it.",
                ));
            }
            // Only these bits can be set through SetFileAttributes.
            let settable = ATTR_READONLY
                | ATTR_HIDDEN
                | ATTR_SYSTEM
                | ATTR_ARCHIVE
                | ATTR_TEMPORARY
                | ATTR_OFFLINE
                | ATTR_NOT_INDEXED;
            let mut next = ((current | add) & !remove) & settable;
            if next == 0 {
                next = ATTR_NORMAL;
            }
            if SetFileAttributesW(w.as_ptr(), next) == 0 {
                return Err(win_err(
                    "Changing attributes",
                    path,
                    std::io::Error::last_os_error().raw_os_error().unwrap_or(0) as u32,
                    "Check that you can modify the file.",
                ));
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use super::*;

    pub fn copy_security(_: &Path, _: &Path, _: bool, _: bool, _: bool) -> Result<(), AppError> {
        Ok(())
    }

    pub fn change_attributes(_: &Path, _: u32, _: u32) -> Result<(), AppError> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sync_types::CopyOptions;

    #[test]
    fn parses_all_robocopy_letters() {
        assert_eq!(parse_letters("RH").unwrap(), ATTR_READONLY | ATTR_HIDDEN);
        assert_eq!(parse_letters("a s").unwrap(), ATTR_ARCHIVE | ATTR_SYSTEM);
        assert_eq!(
            parse_letters("RASHCNETO").unwrap(),
            ATTR_READONLY
                | ATTR_ARCHIVE
                | ATTR_SYSTEM
                | ATTR_HIDDEN
                | ATTR_COMPRESSED
                | ATTR_NOT_INDEXED
                | ATTR_ENCRYPTED
                | ATTR_TEMPORARY
                | ATTR_OFFLINE
        );
        assert_eq!(parse_letters("").unwrap(), 0);
        assert!(parse_letters("RX").is_err());
    }

    #[test]
    fn attribute_filters_match_robocopy() {
        let opts = CopyOptions {
            archive_only: true,
            include_attributes: "S".into(),
            exclude_attributes: "T".into(),
            ..Default::default()
        };
        let got = |attrs| attribute_exclusion(attrs, &opts);
        if SUPPORTED {
            assert!(got(ATTR_SYSTEM).is_some(), "no archive bit");
            assert!(got(ATTR_ARCHIVE).is_some(), "missing S");
            assert!(
                got(ATTR_ARCHIVE | ATTR_SYSTEM | ATTR_TEMPORARY).is_some(),
                "has T"
            );
            assert!(got(ATTR_ARCHIVE | ATTR_SYSTEM).is_none());
        } else {
            // Attribute switches are inert where attributes don't exist.
            assert!(got(0).is_none());
        }
    }

    #[test]
    fn after_copy_is_a_noop_off_windows() {
        if SUPPORTED {
            return;
        }
        let dir = tempfile::TempDir::new().unwrap();
        let f = dir.path().join("a");
        std::fs::write(&f, "x").unwrap();
        let opts = CopyOptions {
            copy_security: true,
            copy_owner: true,
            archive_reset: true,
            add_attributes: "R".into(),
            ..Default::default()
        };
        after_copy(&f, &f, &opts).unwrap();
    }

    #[cfg(windows)]
    fn running_under_wine() -> bool {
        use windows_sys::Win32::System::LibraryLoader::{GetModuleHandleA, GetProcAddress};
        unsafe {
            let ntdll = GetModuleHandleA(c"ntdll.dll".as_ptr().cast());
            !ntdll.is_null() && GetProcAddress(ntdll, c"wine_get_version".as_ptr().cast()).is_some()
        }
    }

    /// Runs only on Windows (natively or under Wine): the real Win32 calls.
    #[cfg(windows)]
    #[test]
    fn windows_attributes_and_security_round_trip() {
        use std::os::windows::fs::MetadataExt;
        let dir = tempfile::TempDir::new().unwrap();
        let src = dir.path().join("src.txt");
        let dst = dir.path().join("dst.txt");
        std::fs::write(&src, "x").unwrap();
        std::fs::write(&dst, "x").unwrap();

        imp::change_attributes(&src, ATTR_ARCHIVE | ATTR_HIDDEN, 0).unwrap();
        let attrs = std::fs::metadata(&src).unwrap().file_attributes();
        assert_ne!(attrs & ATTR_ARCHIVE, 0);
        assert_ne!(attrs & ATTR_HIDDEN, 0);

        let opts = CopyOptions {
            archive_reset: true,
            add_attributes: "R".into(),
            copy_security: true,
            ..Default::default()
        };
        after_copy(&src, &dst, &opts).unwrap();
        // /M clears the source's archive bit; /A+:R marks the copy read-only.
        // Wine reports every regular file as "archive", so the cleared bit is
        // only observable on real Windows.
        if !running_under_wine() {
            assert_eq!(
                std::fs::metadata(&src).unwrap().file_attributes() & ATTR_ARCHIVE,
                0
            );
        }
        assert_ne!(
            std::fs::metadata(&dst).unwrap().file_attributes() & ATTR_READONLY,
            0
        );
        imp::change_attributes(&dst, 0, ATTR_READONLY).unwrap();
    }
}
