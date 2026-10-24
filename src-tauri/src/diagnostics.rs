//! Diagnostics bundles: a zip with recent logs, versions and OS details that
//! a user can save or send to the management console. Secrets, addresses
//! and folder names are scrubbed from every file before it is added.

use crate::backup_engine::notify;
use crate::core::error::AppError;
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// Log bytes included (newest first; the oldest part of a file is cut).
pub const MAX_LOG_BYTES: u64 = 6 * 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize)]
pub struct Bundle {
    pub file_name: String,
    pub files: Vec<String>,
    pub log_bytes: u64,
    pub truncated: bool,
    #[serde(skip)]
    pub bytes: Vec<u8>,
}

/// Saved locally by `diagnostics_create`.
#[derive(Debug, Clone, Serialize)]
pub struct SavedBundle {
    pub path: String,
    pub size: u64,
    pub files: Vec<String>,
    pub truncated: bool,
}

static SECRET_ASSIGN: LazyLock<regex::Regex> = LazyLock::new(|| {
    regex::Regex::new(r#"(?i)\b(authorization|password|passwd|passphrase|secret|token|api[_-]?key|access[_-]?key|session[_-]?key|credential)(["']?\s*[:=]\s*["']?)[^\s"',;&]+"#).expect("valid regex")
});
static BEARER: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"(?i)\b(bearer|basic)\s+[A-Za-z0-9._~+/=-]{8,}").expect("valid regex"));
static UFOP_TOKEN: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"\bufop_(agent|reg|claim|pat)_[A-Za-z0-9_-]+").expect("valid regex"));
static URL_USERINFO: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"://[^/\s:@]+:[^/\s@]+@").expect("valid regex"));
static EMAIL: LazyLock<regex::Regex> = LazyLock::new(|| regex::Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").expect("valid regex"));

/// Remove secrets, credentials in URLs, e-mail addresses, the home folder
/// and directory names (file names are kept) from log text.
pub fn scrub(text: &str) -> String {
    let mut s = URL_USERINFO.replace_all(text, "://***@").into_owned();
    s = BEARER.replace_all(&s, "$1 ***").into_owned();
    s = SECRET_ASSIGN.replace_all(&s, "$1$2***").into_owned();
    s = UFOP_TOKEN.replace_all(&s, "ufop_${1}_***").into_owned();
    s = EMAIL.replace_all(&s, "***@***").into_owned();
    if let Some(home) = dirs::home_dir().map(|h| h.to_string_lossy().to_string()).filter(|h| h.len() > 1) {
        s = s.replace(&home, "~");
    }
    notify::redact_paths(&s, &[])
}

/// Versions, OS and settings that matter for support (no secrets).
pub fn system_info(agent_kind: &str) -> Value {
    let info = ufop_core::agent::ClientInfo::detect(agent_kind, env!("CARGO_PKG_VERSION"));
    let proxy = crate::net::current();
    json!({
        "app": { "name": "UFOP", "version": env!("CARGO_PKG_VERSION"), "kind": agent_kind },
        "os": info.os,
        "osVersion": info.os_version,
        "arch": info.arch,
        "family": std::env::consts::FAMILY,
        "cpus": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1),
        "logLevel": std::env::var("RUST_LOG").unwrap_or_default(),
        "retention": crate::logging::Retention::load(&crate::logging::log_dir()),
        "proxy": { "mode": proxy.mode, "managed": proxy.managed, "bypassRules": proxy.no_proxy.len() },
        "filenamePolicyActive": !ufop_core::policy::active().is_empty(),
        "rclone": rclone_info(&crate::rclone_engine::resolver::status_blocking()),
        "createdAt": chrono::Utc::now().to_rfc3339(),
    })
}

/// The rclone engine in use (via the resolver): path with the home folder
/// shortened to `~`, version, minimum check; "not installed" when absent.
pub fn rclone_info(s: &crate::rclone_engine::RcloneStatus) -> Value {
    let home = |p: &str| match dirs::home_dir().map(|h| h.to_string_lossy().to_string()).filter(|h| h.len() > 1) {
        Some(h) => p.replace(&h, "~"),
        None => p.to_string(),
    };
    json!({
        "installed": s.version.is_some(),
        "path": home(&s.path),
        "source": s.source,
        "version": s.version.clone().unwrap_or_else(|| "not installed".into()),
        "meetsMinimum": s.meets_minimum,
        "minVersion": s.min_version,
        "managed": s.managed,
        "configPath": s.config_path.as_deref().map(home),
        "error": s.error,
    })
}

/// The tail of `path` up to `limit` bytes, starting at a line boundary.
fn read_tail(path: &Path, limit: u64) -> std::io::Result<(String, bool)> {
    let bytes = std::fs::read(path)?;
    if bytes.len() as u64 <= limit {
        return Ok((String::from_utf8_lossy(&bytes).into_owned(), false));
    }
    let start = bytes.len() - limit as usize;
    let start = bytes[start..].iter().position(|b| *b == b'\n').map(|i| start + i + 1).unwrap_or(start);
    Ok((String::from_utf8_lossy(&bytes[start..]).into_owned(), true))
}

/// Build a bundle from the log files in `log_dir` (newest first, up to
/// `max_log_bytes`) plus `system.json`.
pub fn build(log_dir: &Path, system: &Value, max_log_bytes: u64) -> Result<Bundle, AppError> {
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let mut b = Bundle { file_name: format!("ufop-diagnostics-{}.zip", chrono::Utc::now().format("%Y%m%d-%H%M%S")), ..Default::default() };
    let mut add = |zip: &mut zip::ZipWriter<_>, name: String, text: &str| -> Result<(), AppError> {
        zip.start_file(name.as_str(), opts).map_err(|e| AppError::internal(format!("Cannot write the bundle: {e}")))?;
        zip.write_all(text.as_bytes())?;
        b.files.push(name);
        Ok(())
    };

    add(&mut zip, "system.json".into(), &serde_json::to_string_pretty(system)?)?;
    let mut budget = max_log_bytes;
    for (path, size, _) in crate::logging::log_files(log_dir) {
        if budget == 0 {
            b.truncated = true;
            break;
        }
        let (text, cut) = match read_tail(&path, budget.min(size.max(1))) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("Skipping {} in diagnostics: {e}", path.display());
                continue;
            }
        };
        b.truncated |= cut;
        // A cut file used up the budget; older files would only add gaps.
        budget = if cut { 0 } else { budget.saturating_sub(text.len() as u64) };
        b.log_bytes += text.len() as u64;
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        add(&mut zip, format!("logs/{name}"), &scrub(&text))?;
    }
    b.bytes = zip.finish().map_err(|e| AppError::internal(format!("Cannot write the bundle: {e}")))?.into_inner();
    Ok(b)
}

/// The small summary the console lists next to an uploaded bundle.
pub fn summary(b: &Bundle, system: &Value) -> Value {
    json!({
        "appVersion": system["app"]["version"],
        "os": system["os"],
        "osVersion": system["osVersion"],
        "arch": system["arch"],
        "files": b.files,
        "logBytes": b.log_bytes,
        "truncated": b.truncated,
    })
}

/// Build the app's bundle (its own log folder).
pub fn collect(agent_kind: &str) -> Result<(Bundle, Value), AppError> {
    let system = system_info(agent_kind);
    let b = build(&crate::logging::log_dir(), &system, MAX_LOG_BYTES)?;
    let s = summary(&b, &system);
    Ok((b, s))
}

/// Write `bundle` to `out` (a folder gets the bundle's own file name).
pub fn save(bundle: &Bundle, out: &Path) -> Result<SavedBundle, AppError> {
    let path: PathBuf = if out.is_dir() { out.join(&bundle.file_name) } else { out.to_path_buf() };
    ufop_core::fsutil::write_private_atomic(&path, &bundle.bytes)?;
    Ok(SavedBundle { path: path.to_string_lossy().to_string(), size: bundle.bytes.len() as u64, files: bundle.files.clone(), truncated: bundle.truncated })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    #[test]
    fn scrub_removes_secrets_addresses_and_folders() {
        let home = dirs::home_dir().unwrap().to_string_lossy().to_string();
        let line = format!(
            "connect https://bob:hunter2@example.com/x password=hunter2 token: \"abc123\" Authorization: Bearer abcdefghijkl api_key=K1 ufop_agent_SeCrEt mail bob@example.com {home}/Documents/taxes/2026.pdf /srv/data/clients/acme.xlsx"
        );
        let s = scrub(&line);
        for leak in ["hunter2", "abc123", "abcdefghijkl", "K1 ", "SeCrEt", "bob@example.com", "taxes", "clients", &home] {
            assert!(!s.contains(leak), "{leak} leaked: {s}");
        }
        assert!(s.contains("https://***@example.com"));
        assert!(s.contains("ufop_agent_***"));
        assert!(s.contains("2026.pdf") && s.contains("acme.xlsx"), "file names are kept: {s}");
    }

    #[test]
    fn rclone_engine_info_says_not_installed_and_hides_home() {
        use crate::rclone_engine::resolver::{status_of, BinarySource, RcloneVersion, Resolved, Semver};
        let missing = status_of(&Resolved { path: "rclone".into(), source: BinarySource::None, found: false, config: None }, Err("Could not start rclone".into()));
        let v = rclone_info(&missing);
        assert_eq!((v["installed"].as_bool(), v["version"].as_str(), v["error"].as_str()), (Some(false), Some("not installed"), Some("rclone is not installed")));
        let home = dirs::home_dir().unwrap().to_string_lossy().to_string();
        let found = status_of(
            &Resolved { path: format!("{home}/bin/rclone"), source: BinarySource::Settings, found: true, config: Some(format!("{home}/.config/rclone/rclone.conf")) },
            Ok(RcloneVersion { raw: "v1.69.1".into(), semver: Semver { major: 1, minor: 69, patch: 1 } }),
        );
        let v = rclone_info(&found);
        assert_eq!((v["installed"].as_bool(), v["version"].as_str(), v["meetsMinimum"].as_bool(), v["source"].as_str()), (Some(true), Some("v1.69.1"), Some(true), Some("settings")));
        if home.len() > 1 {
            assert_eq!(v["path"], "~/bin/rclone");
            assert_eq!(v["configPath"], "~/.config/rclone/rclone.conf");
        }
    }

    #[test]
    fn bundle_has_system_info_and_scrubbed_logs_within_budget() {
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("ufop.2026-10-21.log"), "old line\n".repeat(50)).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(d.path().join("ufop.2026-10-22.log"), format!("{}secret=abc\nlast\n", "first\n".repeat(20))).unwrap();
        std::fs::write(d.path().join("notes.txt"), "not a log").unwrap();

        let b = build(d.path(), &json!({ "app": { "version": "1.2.3" }, "os": "linux" }), 30).unwrap();
        assert_eq!(b.files, vec!["system.json", "logs/ufop.2026-10-22.log"]);
        assert!(b.truncated);
        let mut z = zip::ZipArchive::new(std::io::Cursor::new(b.bytes.clone())).unwrap();
        let mut log = String::new();
        z.by_name("logs/ufop.2026-10-22.log").unwrap().read_to_string(&mut log).unwrap();
        assert!(log.ends_with("secret=***\nlast\n"), "tail starts on a line and is scrubbed: {log:?}");
        assert!(!log.starts_with("irst"));
        let s = summary(&b, &json!({ "app": { "version": "1.2.3" }, "os": "linux" }));
        assert_eq!(s["appVersion"], "1.2.3");
        assert_eq!(s["truncated"], true);

        let all = build(d.path(), &json!({}), MAX_LOG_BYTES).unwrap();
        assert_eq!(all.files.len(), 3);
        assert!(!all.truncated);
        let saved = save(&all, d.path()).unwrap();
        assert!(saved.path.ends_with(&all.file_name));
        assert_eq!(std::fs::metadata(&saved.path).unwrap().len(), saved.size);
    }

    #[test]
    fn system_info_reports_versions_without_secrets() {
        let v = system_info("desktop");
        assert_eq!(v["app"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(v["rclone"]["version"].as_str().is_some_and(|s| !s.is_empty()), "rclone engine reported: {}", v["rclone"]);
        assert!(v["os"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(v["proxy"].get("username").is_none() && v["proxy"].get("url").is_none());
    }
}
