//! SMB sign-in through the operating system's own API on macOS and Windows,
//! so the password is a function argument and never appears on a command
//! line (`ps`, Task Manager, process auditing). Linux keeps `mount.cifs`
//! with the password in its environment (`PASSWD`).

/// macOS: mount `url` (`smb://host/share`) at `mount_dir` with NetFS.
#[cfg(target_os = "macos")]
pub fn netfs_mount(url: &str, mount_dir: &std::path::Path, user: Option<&str>, password: Option<&str>, guest: bool) -> Result<(), String> {
    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::CFMutableDictionary;
    use core_foundation::string::CFString;
    use core_foundation::url::CFURL;
    use core_foundation_sys::array::CFArrayRef;
    use core_foundation_sys::base::{kCFAllocatorDefault, CFRelease};
    use core_foundation_sys::dictionary::CFMutableDictionaryRef;
    use core_foundation_sys::string::CFStringRef;
    use core_foundation_sys::url::{CFURLCreateWithString, CFURLRef};

    #[link(name = "NetFS", kind = "framework")]
    extern "C" {
        fn NetFSMountURLSync(
            url: CFURLRef,
            mountpath: CFURLRef,
            user: CFStringRef,
            passwd: CFStringRef,
            open_options: CFMutableDictionaryRef,
            mount_options: CFMutableDictionaryRef,
            mountpoints: *mut CFArrayRef,
        ) -> i32;
    }

    let url_s = CFString::new(url);
    // SAFETY: a valid CFString; the result is owned (create rule) or null.
    let url_ref = unsafe { CFURLCreateWithString(kCFAllocatorDefault, url_s.as_concrete_TypeRef(), std::ptr::null()) };
    if url_ref.is_null() {
        return Err(format!("'{url}' is not a valid address"));
    }
    // SAFETY: non-null and owned by us.
    let url_cf = unsafe { CFURL::wrap_under_create_rule(url_ref) };
    let dir = CFURL::from_path(mount_dir, true).ok_or_else(|| format!("'{}' is not a usable folder", mount_dir.display()))?;
    let user_cf = user.map(CFString::new);
    let pass_cf = password.map(CFString::new);

    // No dialogs; mount exactly at the given folder.
    let mut open: CFMutableDictionary<CFString, CFType> = CFMutableDictionary::new();
    open.set(CFString::new("UIOption"), CFString::new("NoUI").as_CFType());
    if guest {
        open.set(CFString::new("Guest"), CFBoolean::true_value().as_CFType());
    }
    let mut mount: CFMutableDictionary<CFString, CFType> = CFMutableDictionary::new();
    mount.set(CFString::new("MountAtMountDir"), CFBoolean::true_value().as_CFType());
    mount.set(CFString::new("UIOption"), CFString::new("NoUI").as_CFType());

    let mut points: CFArrayRef = std::ptr::null();
    // SAFETY: every reference is valid for the call; `points` receives an
    // owned array (or stays null), released below.
    let rc = unsafe {
        NetFSMountURLSync(
            url_cf.as_concrete_TypeRef(),
            dir.as_concrete_TypeRef(),
            user_cf.as_ref().map_or(std::ptr::null(), |s| s.as_concrete_TypeRef()),
            pass_cf.as_ref().map_or(std::ptr::null(), |s| s.as_concrete_TypeRef()),
            open.as_concrete_TypeRef(),
            mount.as_concrete_TypeRef(),
            &mut points,
        )
    };
    if !points.is_null() {
        // SAFETY: owned by us (NetFS create rule).
        unsafe { CFRelease(points as _) };
    }
    match rc {
        0 => Ok(()),
        80 | 1 | 13 => Err("The server refused the user name or password".into()),
        n => Err(format!("NetFS error {n} ({})", std::io::Error::from_raw_os_error(n))),
    }
}

/// UTF-16, NUL-terminated.
#[cfg(any(windows, test))]
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Windows: sign in to `remote` (`\\host\share`) for this session, without
/// a drive letter (files are opened through the UNC path).
#[cfg(windows)]
pub fn wnet_connect(remote: &str, user: Option<&str>, password: Option<&str>) -> Result<(), String> {
    use windows_sys::Win32::NetworkManagement::WNet::{WNetAddConnection2W, NETRESOURCEW, RESOURCETYPE_DISK};

    let mut remote_w = wide(remote);
    let user_w = user.map(wide);
    let pass_w = password.map(wide);
    // SAFETY: zeroed is a valid NETRESOURCEW (all pointers null).
    let mut res: NETRESOURCEW = unsafe { std::mem::zeroed() };
    res.dwType = RESOURCETYPE_DISK;
    res.lpRemoteName = remote_w.as_mut_ptr();
    // SAFETY: the strings outlive the call and are NUL-terminated.
    let rc = unsafe {
        WNetAddConnection2W(
            &res,
            pass_w.as_ref().map_or(std::ptr::null(), |p| p.as_ptr()),
            user_w.as_ref().map_or(std::ptr::null(), |u| u.as_ptr()),
            0,
        )
    };
    match rc {
        0 => Ok(()),
        // Already connected with these credentials.
        1219 => Err("Windows is already connected to this server with another user name; disconnect it first (net use)".into()),
        86 | 1326 => Err("The server refused the user name or password".into()),
        n => Err(format!("Windows error {n} ({})", std::io::Error::from_raw_os_error(n as i32))),
    }
}

/// Windows: end the session's connection to `remote`.
#[cfg(windows)]
pub fn wnet_disconnect(remote: &str) -> Result<(), String> {
    use windows_sys::Win32::NetworkManagement::WNet::WNetCancelConnection2W;
    let remote_w = wide(remote);
    // SAFETY: NUL-terminated string valid for the call.
    let rc = unsafe { WNetCancelConnection2W(remote_w.as_ptr(), 0, 1) };
    if rc == 0 || rc == 2250 {
        // 2250: not connected (already gone).
        Ok(())
    } else {
        Err(format!("Windows error {rc} ({})", std::io::Error::from_raw_os_error(rc as i32)))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn wide_strings_end_with_nul() {
        assert_eq!(super::wide("ab"), [97, 98, 0]);
    }
}
