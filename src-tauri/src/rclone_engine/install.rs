//! The managed copy of rclone (Advanced): a pinned release for this OS and
//! architecture, downloaded from downloads.rclone.org, checked against that
//! release's `SHA256SUMS` before anything is unpacked, and installed into
//! the app data folder (`<data>/rclone/rclone[.exe]`).
//!
//! Only that folder is ever written: a binary the user chose elsewhere is
//! never replaced. After installing, Settings points at the managed copy;
//! the user's own binary stays where it is and can be chosen again.

use crate::core::error::AppError;
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The release a managed install downloads.
pub const PINNED_VERSION: &str = "1.69.1";
/// Where releases come from.
pub const DOWNLOAD_BASE: &str = "https://downloads.rclone.org";
/// Largest archive accepted (rclone zips are ~20–30 MB).
const MAX_ARCHIVE: usize = 256 * 1024 * 1024;
/// Largest binary accepted from the archive.
const MAX_BINARY: u64 = 512 * 1024 * 1024;
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// The app's rclone folder.
pub fn managed_dir() -> PathBuf {
    crate::paths::data_dir().join("rclone")
}

fn binary_name() -> &'static str {
    if cfg!(windows) {
        "rclone.exe"
    } else {
        "rclone"
    }
}

/// The managed binary inside `dir`.
pub fn managed_binary(dir: &Path) -> PathBuf {
    dir.join(binary_name())
}

/// Whether `path` lies inside the managed folder `dir`.
/// The entry itself is located (its folder resolved, not a symlink it may be).
pub fn is_managed_path(path: &Path, dir: &Path) -> bool {
    let canon = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let entry = match (path.parent(), path.file_name()) {
        (Some(parent), Some(name)) => canon(parent).join(name),
        _ => return false,
    };
    path.is_absolute() && entry.starts_with(canon(dir)) && entry != canon(dir)
}

/// rclone's names for an OS and CPU (`linux`/`osx`/`windows`, `amd64`/`arm64`/…).
pub fn platform_for(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    let os = match os {
        "linux" => "linux",
        "macos" => "osx",
        "windows" => "windows",
        "freebsd" => "freebsd",
        "openbsd" => "openbsd",
        "netbsd" => "netbsd",
        _ => return None,
    };
    let arch = match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "x86" => "386",
        "arm" if os == "linux" => "arm-v7",
        _ => return None,
    };
    if os == "osx" && arch == "386" {
        return None;
    }
    Some((os, arch))
}

/// This machine's platform in rclone's names.
pub fn platform() -> Option<(&'static str, &'static str)> {
    platform_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// `rclone-v1.69.1-linux-amd64.zip`.
pub fn archive_name(version: &str, os: &str, arch: &str) -> String {
    format!("rclone-v{version}-{os}-{arch}.zip")
}

/// `name → sha256` from a `SHA256SUMS` file (PGP clear-signed or plain:
/// lines of `<64 hex>  <file>`; anything else is ignored).
pub fn parse_sha256sums(text: &str) -> HashMap<String, String> {
    text.lines()
        .filter_map(|l| {
            let mut it = l.split_whitespace();
            let hash = it.next()?;
            let name = it.next()?.trim_start_matches('*');
            (hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) && it.next().is_none()).then(|| (name.to_string(), hash.to_ascii_lowercase()))
        })
        .collect()
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Refuse `bytes` unless their SHA-256 is `expected`.
pub fn verify_checksum(name: &str, bytes: &[u8], expected: &str) -> Result<(), AppError> {
    let actual = sha256_hex(bytes);
    if !actual.eq_ignore_ascii_case(expected.trim()) {
        return Err(AppError::encryption(
            format!("Checksum mismatch for {name}: expected {expected}, got {actual}. The download was refused and nothing was installed."),
            "Retry the download; if it keeps failing, the mirror or a proxy may be altering files. Install rclone yourself from https://rclone.org/downloads/.",
        ));
    }
    Ok(())
}

/// The rclone executable inside a release zip
/// (`rclone-v<ver>-<os>-<arch>/rclone[.exe]`).
pub fn extract_binary(zip_bytes: &[u8], exe_name: &str) -> Result<Vec<u8>, AppError> {
    let bad = |e: String| AppError::file_op(format!("The rclone archive could not be read: {e}"), "Retry the download.");
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes)).map_err(|e| bad(e.to_string()))?;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| bad(e.to_string()))?;
        let Some(path) = f.enclosed_name() else { continue };
        let parts: Vec<_> = path.components().collect();
        if f.is_dir() || parts.len() > 2 || path.file_name().and_then(|n| n.to_str()) != Some(exe_name) {
            continue;
        }
        if f.size() > MAX_BINARY {
            return Err(bad(format!("{exe_name} is too large")));
        }
        let mut out = Vec::with_capacity(f.size() as usize);
        f.by_ref().take(MAX_BINARY + 1).read_to_end(&mut out).map_err(|e| bad(e.to_string()))?;
        if out.len() as u64 > MAX_BINARY {
            return Err(bad(format!("{exe_name} is too large")));
        }
        return Ok(out);
    }
    Err(bad(format!("{exe_name} is missing from the archive")))
}

/// Write the binary into `dir` (and only there): a fresh temp file, made
/// executable, then renamed over the previous managed copy.
pub fn place_binary(dir: &Path, bytes: &[u8]) -> Result<PathBuf, AppError> {
    let io = |e: std::io::Error| AppError::file_op(format!("Could not install rclone into {}: {e}", dir.display()), "Check the app data folder is writable.");
    std::fs::create_dir_all(dir).map_err(io)?;
    let target = managed_binary(dir);
    if !is_managed_path(&target, dir) {
        return Err(AppError::validation("Refusing to write rclone outside the app's rclone folder"));
    }
    let tmp = dir.join(format!(".{}.download-{}", binary_name(), uuid::Uuid::new_v4().simple()));
    let result = (|| {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))?;
        }
        // A symlink in our folder is replaced, never written through.
        if std::fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink()) {
            std::fs::remove_file(&target)?;
        }
        std::fs::rename(&tmp, &target)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        return Err(io(e));
    }
    Ok(target)
}

/// What a managed install did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InstallResult {
    pub path: String,
    /// As reported by the installed binary (`v1.69.1`).
    pub version: String,
    pub archive: String,
    pub sha256: String,
}

async fn fetch(client: &reqwest::Client, url: &str, limit: usize) -> Result<Vec<u8>, AppError> {
    let failed = |e: String| AppError::connection(format!("Could not download {url}: {e}"), "Check the internet connection and proxy settings, then retry.");
    let resp = tokio::time::timeout(DOWNLOAD_TIMEOUT, client.get(url).send()).await.map_err(|_| failed("timed out".into()))?.map_err(|e| failed(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(failed(format!("HTTP {}", resp.status())));
    }
    if resp.content_length().is_some_and(|n| n as usize > limit) {
        return Err(failed("the file is larger than expected".into()));
    }
    let bytes = tokio::time::timeout(DOWNLOAD_TIMEOUT, resp.bytes()).await.map_err(|_| failed("timed out".into()))?.map_err(|e| failed(e.to_string()))?;
    if bytes.len() > limit {
        return Err(failed("the file is larger than expected".into()));
    }
    Ok(bytes.to_vec())
}

/// Download `version` for `os`/`arch` from `base`, verify it against the
/// release's `SHA256SUMS`, and install it into `dir`. A missing entry or a
/// mismatching checksum installs nothing.
pub async fn install_from(client: &reqwest::Client, base: &str, version: &str, os: &str, arch: &str, dir: &Path) -> Result<InstallResult, AppError> {
    if super::resolver::Semver::parse(version).is_none() {
        return Err(AppError::validation(format!("Invalid rclone version {version}")));
    }
    let base = base.trim_end_matches('/');
    let archive = archive_name(version, os, arch);
    let sums_text = fetch(client, &format!("{base}/v{version}/SHA256SUMS"), 1024 * 1024).await?;
    let sums = parse_sha256sums(&String::from_utf8_lossy(&sums_text));
    let expected = sums.get(&archive).ok_or_else(|| {
        AppError::connection(format!("{archive} is not listed in the release's SHA256SUMS"), "This platform may have no rclone build; install rclone yourself from https://rclone.org/downloads/.")
    })?;
    let zip = fetch(client, &format!("{base}/v{version}/{archive}"), MAX_ARCHIVE).await?;
    verify_checksum(&archive, &zip, expected)?;
    let exe = if os == "windows" { "rclone.exe" } else { "rclone" };
    let bytes = extract_binary(&zip, exe)?;
    let path = place_binary(dir, &bytes)?;
    let reported = super::resolver::Rclone::new(path.to_string_lossy(), None).version().await?;
    Ok(InstallResult { path: path.to_string_lossy().into_owned(), version: reported.raw, archive, sha256: expected.clone() })
}

/// Install the pinned release for this machine into the app's folder.
pub async fn install_pinned() -> Result<InstallResult, AppError> {
    let (os, arch) = platform().ok_or_else(|| {
        AppError::validation(format!("No rclone build is published for {}/{}", std::env::consts::OS, std::env::consts::ARCH))
    })?;
    install_from(&crate::net::client(), DOWNLOAD_BASE, PINNED_VERSION, os, arch, &managed_dir()).await
}

#[cfg(test)]
mod tests {
    use super::*;

    const SUMS_FIXTURE: &str = include_str!("fixtures/SHA256SUMS");

    #[test]
    fn platforms_and_names() {
        assert_eq!(platform_for("linux", "x86_64"), Some(("linux", "amd64")));
        assert_eq!(platform_for("macos", "aarch64"), Some(("osx", "arm64")));
        assert_eq!(platform_for("windows", "x86"), Some(("windows", "386")));
        assert_eq!(platform_for("linux", "arm"), Some(("linux", "arm-v7")));
        assert_eq!(platform_for("macos", "x86"), None);
        assert_eq!(platform_for("haiku", "x86_64"), None);
        assert_eq!(archive_name("1.69.1", "osx", "arm64"), "rclone-v1.69.1-osx-arm64.zip");
    }

    #[test]
    fn parses_clearsigned_sha256sums() {
        let sums = parse_sha256sums(SUMS_FIXTURE);
        assert_eq!(sums.len(), 4, "{sums:?}");
        assert_eq!(sums["rclone-v1.69.1-linux-amd64.zip"], "8e2e1b5a7a4b9a1d6c4f0e3b2a1c9d8e7f6a5b4c3d2e1f0a9b8c7d6e5f4a3b2c");
        assert!(sums.contains_key("rclone-v1.69.1-windows-amd64.zip"));
        // PGP armour and signature lines are not entries.
        assert!(!sums.keys().any(|k| k.contains("PGP") || k.contains("Hash")));
    }

    #[test]
    fn checksum_verification_accepts_and_refuses() {
        let data = b"rclone release bytes";
        let good = sha256_hex(data);
        verify_checksum("x.zip", data, &good).unwrap();
        verify_checksum("x.zip", data, &good.to_uppercase()).unwrap();
        let e = verify_checksum("x.zip", data, &"0".repeat(64)).unwrap_err().user_message();
        assert!(e.contains("Checksum mismatch for x.zip") && e.contains("refused"), "{e}");
    }

    fn release_zip(top: &str, exe: &str, body: &[u8]) -> Vec<u8> {
        let mut z = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        z.add_directory(format!("{top}/"), o).unwrap();
        z.start_file(format!("{top}/README.txt"), o).unwrap();
        z.write_all(b"readme").unwrap();
        z.start_file(format!("{top}/sub/{exe}"), o).unwrap();
        z.write_all(b"decoy").unwrap();
        z.start_file(format!("{top}/{exe}"), o).unwrap();
        z.write_all(body).unwrap();
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn extracts_only_the_top_level_binary() {
        let zip = release_zip("rclone-v1.69.1-linux-amd64", "rclone", b"BIN");
        assert_eq!(extract_binary(&zip, "rclone").unwrap(), b"BIN");
        assert!(extract_binary(&zip, "rclone.exe").unwrap_err().user_message().contains("missing"));
        assert!(extract_binary(b"not a zip", "rclone").is_err());
    }

    #[test]
    fn places_binary_only_inside_the_managed_folder() {
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("rclone");
        let user_bin = d.path().join("user-rclone");
        std::fs::write(&user_bin, "USER OWNED").unwrap();
        let p = place_binary(&dir, b"one").unwrap();
        assert_eq!(p, managed_binary(&dir));
        assert!(is_managed_path(&p, &dir) && !is_managed_path(&user_bin, &dir));
        assert_eq!(std::fs::read(&p).unwrap(), b"one");
        // A symlink planted in our folder is replaced, not written through.
        #[cfg(unix)]
        {
            std::fs::remove_file(&p).unwrap();
            std::os::unix::fs::symlink(&user_bin, &p).unwrap();
            place_binary(&dir, b"two").unwrap();
            assert_eq!(std::fs::read(&p).unwrap(), b"two");
            assert!(!std::fs::symlink_metadata(&p).unwrap().file_type().is_symlink());
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o111, 0o111);
        }
        assert_eq!(std::fs::read_to_string(&user_bin).unwrap(), "USER OWNED");
        // No temp files left behind.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    }

    /// A local stand-in for downloads.rclone.org.
    async fn mirror(files: Vec<(String, Vec<u8>)>) -> String {
        let files = std::sync::Arc::new(files.into_iter().collect::<HashMap<_, _>>());
        let app = axum::Router::new().fallback(move |uri: axum::http::Uri| {
            let files = files.clone();
            async move {
                match files.get(uri.path()) {
                    Some(b) => (axum::http::StatusCode::OK, b.clone()),
                    None => (axum::http::StatusCode::NOT_FOUND, b"missing".to_vec()),
                }
            }
        });
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        base
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn installs_a_verified_release_and_refuses_a_tampered_one() {
        let top = "rclone-v1.69.1-linux-amd64";
        let zip = release_zip(top, "rclone", b"#!/bin/sh\necho 'rclone v1.69.1'\n");
        let sums = format!("-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA1\n\n{}  {top}.zip\n-----BEGIN PGP SIGNATURE-----\nabc\n-----END PGP SIGNATURE-----\n", sha256_hex(&zip));
        let base = mirror(vec![("/v1.69.1/SHA256SUMS".into(), sums.into_bytes()), (format!("/v1.69.1/{top}.zip"), zip.clone())]).await;
        let client = reqwest::Client::new();
        let d = tempfile::tempdir().unwrap();
        let dir = d.path().join("rclone");
        let r = install_from(&client, &base, "1.69.1", "linux", "amd64", &dir).await.unwrap();
        assert_eq!((r.version.as_str(), r.archive.as_str()), ("v1.69.1", "rclone-v1.69.1-linux-amd64.zip"));
        assert_eq!(r.path, managed_binary(&dir).to_string_lossy());
        assert_eq!(r.sha256, sha256_hex(&zip));

        // Tampered archive: refused, the installed copy is untouched.
        let mut bad = zip.clone();
        let n = bad.len();
        bad[n / 2] ^= 0xff;
        let sums = format!("{}  {top}.zip\n", sha256_hex(&zip));
        let base = mirror(vec![("/v1.69.1/SHA256SUMS".into(), sums.into_bytes()), (format!("/v1.69.1/{top}.zip"), bad)]).await;
        let other = d.path().join("other");
        let e = install_from(&client, &base, "1.69.1", "linux", "amd64", &other).await.unwrap_err().user_message();
        assert!(e.contains("Checksum mismatch"), "{e}");
        assert!(!managed_binary(&other).exists(), "nothing installed on mismatch");
        // Not listed: refused before the archive is fetched.
        let base = mirror(vec![("/v1.69.1/SHA256SUMS".into(), b"deadbeef  other.zip\n".to_vec())]).await;
        let e = install_from(&client, &base, "1.69.1", "linux", "amd64", &other).await.unwrap_err().user_message();
        assert!(e.contains("not listed"), "{e}");
        // Server errors surface; bad versions are rejected up front.
        let base = mirror(vec![]).await;
        assert!(install_from(&client, &base, "1.69.1", "linux", "amd64", &other).await.unwrap_err().user_message().contains("HTTP 404"));
        assert!(install_from(&client, &base, "../x", "linux", "amd64", &other).await.is_err());
    }
}
