//! Mount a local WebDAV URL (such as a backup-version drive) as a folder or
//! drive letter with the tools each OS ships:
//!
//! - Linux: `gio mount dav://…` (GVfs, the GNOME/KDE file managers' backend);
//!   the files appear under `/run/user/<uid>/gvfs/`.
//! - macOS: `mount_webdav` into a folder the app creates.
//! - Windows: `net use * http://…` (the WebClient service); the next free
//!   drive letter.
//!
//! When the tool is missing or fails, callers fall back to showing the URL
//! (and opening it in the file manager), so mounting is best effort.

use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::process::Command;

/// How a URL was mounted (what unmounting needs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OsMount {
    pub mount_point: String,
    /// `gio`, `mount_webdav` or `net use`.
    pub tool: String,
    url: String,
}

const MOUNT_TIMEOUT: Duration = Duration::from_secs(30);

async fn run(cmd: &mut Command) -> Result<String, String> {
    cmd.kill_on_drop(true);
    match tokio::time::timeout(MOUNT_TIMEOUT, cmd.output()).await {
        Err(_) => Err("the mount command did not finish within 30 seconds".into()),
        Ok(Err(e)) => Err(format!("cannot run the mount command: {e}")),
        Ok(Ok(o)) if o.status.success() => Ok(String::from_utf8_lossy(&o.stdout).into_owned()),
        Ok(Ok(o)) => {
            let err = String::from_utf8_lossy(&o.stderr).trim().to_string();
            Err(if err.is_empty() { String::from_utf8_lossy(&o.stdout).trim().to_string() } else { err })
        }
    }
}

/// `dav://` form of an `http://` URL (GVfs).
pub fn gvfs_url(url: &str) -> String {
    url.replacen("http://", "dav://", 1)
}

/// Where GVfs shows a `dav://host:port/prefix/` mount.
pub fn gvfs_mount_point(runtime_dir: &Path, url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("http://")?;
    let (hostport, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = hostport.rsplit_once(':')?;
    let prefix = format!("/{}", path.trim_end_matches('/'));
    let enc = prefix.replace('%', "%25").replace('/', "%2F");
    Some(runtime_dir.join("gvfs").join(format!("dav:host={host},port={port},prefix={enc}")))
}

/// Drive letter in `net use` output ("Drive Z: is now connected to …").
pub fn parse_net_use_drive(out: &str) -> Option<String> {
    out.split_whitespace().find(|w| w.len() == 2 && w.ends_with(':') && w.chars().next().is_some_and(|c| c.is_ascii_alphabetic())).map(str::to_string)
}

/// Mount `url`; `folder` is where tools that need a mount folder put it.
#[allow(unused_variables)]
pub async fn mount(url: &str, folder: &Path) -> Result<OsMount, String> {
    if cfg!(target_os = "linux") {
        run(Command::new("gio").arg("mount").arg(gvfs_url(url))).await?;
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", current_uid())));
        let mp = gvfs_mount_point(&runtime, url).filter(|p| p.exists()).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|| gvfs_url(url));
        return Ok(OsMount { mount_point: mp, tool: "gio".into(), url: url.into() });
    }
    if cfg!(target_os = "macos") {
        std::fs::create_dir_all(folder).map_err(|e| format!("cannot create {}: {e}", folder.display()))?;
        run(Command::new("mount_webdav").arg("-S").arg(url).arg(folder)).await?;
        return Ok(OsMount { mount_point: folder.to_string_lossy().into_owned(), tool: "mount_webdav".into(), url: url.into() });
    }
    if cfg!(windows) {
        let out = run(Command::new("net").args(["use", "*"]).arg(url).arg("/persistent:no")).await?;
        let drive = parse_net_use_drive(&out).ok_or_else(|| format!("net use did not report a drive letter: {}", out.trim()))?;
        return Ok(OsMount { mount_point: drive, tool: "net use".into(), url: url.into() });
    }
    Err("mounting WebDAV is not supported on this system".into())
}

/// Undo [`mount`]; failures are logged (the drive may already be gone).
pub async fn unmount(m: &OsMount) {
    let r = match m.tool.as_str() {
        "gio" => run(Command::new("gio").args(["mount", "-u"]).arg(gvfs_url(&m.url))).await,
        "mount_webdav" => match run(Command::new("umount").arg(&m.mount_point)).await {
            Ok(o) => {
                let _ = std::fs::remove_dir(&m.mount_point);
                Ok(o)
            }
            Err(e) => Err(e),
        },
        "net use" => run(Command::new("net").args(["use"]).arg(&m.mount_point).args(["/delete", "/y"])).await,
        _ => Ok(String::new()),
    };
    if let Err(e) = r {
        tracing::warn!("Unmounting {} ({}): {e}", m.mount_point, m.tool);
    }
}

/// [`unmount`] without a runtime (app exit).
pub fn unmount_blocking(m: &OsMount) {
    let mut c = match m.tool.as_str() {
        "gio" => {
            let mut c = std::process::Command::new("gio");
            c.args(["mount", "-u"]).arg(gvfs_url(&m.url));
            c
        }
        "mount_webdav" => {
            let mut c = std::process::Command::new("umount");
            c.arg(&m.mount_point);
            c
        }
        "net use" => {
            let mut c = std::process::Command::new("net");
            c.arg("use").arg(&m.mount_point).args(["/delete", "/y"]);
            c
        }
        _ => return,
    };
    if let Err(e) = c.output() {
        tracing::warn!("Unmounting {} ({}): {e}", m.mount_point, m.tool);
    }
}

fn current_uid() -> u32 {
    #[cfg(unix)]
    {
        // SAFETY: getuid has no preconditions.
        return unsafe { libc::getuid() };
    }
    #[allow(unreachable_code)]
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gvfs_names_and_drive_letters() {
        assert_eq!(gvfs_url("http://127.0.0.1:4567/abc/"), "dav://127.0.0.1:4567/abc/");
        assert_eq!(
            gvfs_mount_point(Path::new("/run/user/1000"), "http://127.0.0.1:4567/abc/").unwrap(),
            PathBuf::from("/run/user/1000/gvfs/dav:host=127.0.0.1,port=4567,prefix=%2Fabc")
        );
        assert_eq!(parse_net_use_drive("Drive Z: is now connected to http://127.0.0.1:1/x/.\r\n\r\nThe command completed successfully."), Some("Z:".into()));
        assert_eq!(parse_net_use_drive("The command completed successfully."), None);
    }
}
