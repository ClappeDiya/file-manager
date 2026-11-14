//! Where the app keeps its files.
//!
//! Every app folder (configuration database, backup job databases, logs,
//! caches including the backup volume cache, thumbnails, sync state, peer
//! identity, editor drafts, temporary previews, the credential vault) is
//! resolved here, so **portable mode** can move all of them at once.
//!
//! Portable mode is on, in this order of precedence, when:
//!
//! 1. `UFOP_PORTABLE_DIR` is set (non-empty): everything lives under that
//!    folder (a relative path is taken relative to the executable's folder);
//! 2. `ufop-portable.json` exists next to the executable: under its
//!    `data_dir` (relative to the executable's folder; default `ufop-data`);
//! 3. a file named `portable` exists next to the executable: under
//!    `ufop-data` next to it.
//!
//! On macOS "next to the executable" also means next to the `.app` bundle.
//! Inside the portable root: `data/` (databases, backup job databases,
//! search index, logs under `data/logs`), `config/` (credential vault),
//! `cache/` (volume cache, thumbnails, mounts) and `tmp/` (temporary
//! files). Otherwise the OS folders of `directories::ProjectDirs` are used,
//! as before.
//!
//! The OS keychain cannot travel with the folder, so in portable mode
//! credentials go to the encrypted vault file `config/credentials.vault`
//! (see `security::credential_store`).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub const PORTABLE_ENV: &str = "UFOP_PORTABLE_DIR";
pub const PORTABLE_MARKER: &str = "portable";
pub const PORTABLE_CONFIG: &str = "ufop-portable.json";
pub const PORTABLE_DEFAULT_DIR: &str = "ufop-data";

/// Why portable mode is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PortableSource {
    /// `UFOP_PORTABLE_DIR`.
    Env,
    /// `ufop-portable.json` next to the executable.
    ConfigFile,
    /// A `portable` file next to the executable.
    Marker,
}

/// The app's folders.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppDirs {
    /// Databases and other durable data.
    pub data: PathBuf,
    /// Machine-local data (logs).
    pub data_local: PathBuf,
    pub config: PathBuf,
    pub cache: PathBuf,
    /// Temporary files (portable mode only; otherwise the OS temp folder).
    pub temp: Option<PathBuf>,
    /// Portable root, when portable mode is on.
    pub portable_root: Option<PathBuf>,
    pub portable_source: Option<PortableSource>,
}

impl AppDirs {
    pub fn is_portable(&self) -> bool {
        self.portable_root.is_some()
    }

    fn portable(root: PathBuf, source: PortableSource) -> Self {
        Self {
            data: root.join("data"),
            data_local: root.join("data"),
            config: root.join("config"),
            cache: root.join("cache"),
            temp: Some(root.join("tmp")),
            portable_root: Some(root),
            portable_source: Some(source),
        }
    }

    /// The OS folders (`directories::ProjectDirs`), or private temp folders
    /// when the OS reports none.
    pub fn os_default() -> Self {
        match directories::ProjectDirs::from("com", "ufop", "unified-file-ops") {
            Some(d) => Self {
                data: d.data_dir().to_path_buf(),
                data_local: d.data_local_dir().to_path_buf(),
                config: d.config_dir().to_path_buf(),
                cache: d.cache_dir().to_path_buf(),
                temp: None,
                portable_root: None,
                portable_source: None,
            },
            None => {
                let base = ufop_core::session::user_temp_subdir("app");
                Self { data: base.join("data"), data_local: base.join("data"), config: base.join("config"), cache: base.join("cache"), temp: None, portable_root: None, portable_source: None }
            }
        }
    }
}

#[derive(Debug, Default, Deserialize)]
struct PortableFile {
    #[serde(default)]
    data_dir: Option<String>,
}

/// Folders an executable at `exe` is "next to": its own folder and, for a
/// macOS bundle (`X.app/Contents/MacOS/x`), the folder holding `X.app`.
pub fn exe_dirs(exe: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(dir) = exe.parent() {
        out.push(dir.to_path_buf());
        let bundle = dir.parent().filter(|c| c.ends_with("Contents")).and_then(Path::parent).filter(|a| a.extension().is_some_and(|e| e == "app"));
        if let Some(outer) = bundle.and_then(Path::parent) {
            out.push(outer.to_path_buf());
        }
    }
    out
}

fn absolute(base: &Path, p: &str) -> PathBuf {
    let p = PathBuf::from(p.trim());
    if p.is_absolute() {
        p
    } else {
        base.join(p)
    }
}

/// Resolve the folders from the environment value of `UFOP_PORTABLE_DIR`,
/// the folders next to the executable, and the OS defaults (pure; see the
/// module docs for the precedence).
pub fn resolve(env: Option<&str>, exe_dirs: &[PathBuf], os: impl FnOnce() -> AppDirs) -> AppDirs {
    let base = exe_dirs.first().cloned().unwrap_or_else(|| PathBuf::from("."));
    if let Some(v) = env.map(str::trim).filter(|v| !v.is_empty()) {
        return AppDirs::portable(absolute(&base, v), PortableSource::Env);
    }
    for dir in exe_dirs {
        let cfg = dir.join(PORTABLE_CONFIG);
        if cfg.is_file() {
            let parsed: PortableFile = std::fs::read_to_string(&cfg).ok().and_then(|t| serde_json::from_str(&t).map_err(|e| tracing::warn!("{} is not valid JSON ({e}); using {PORTABLE_DEFAULT_DIR}", cfg.display())).ok()).unwrap_or_default();
            let sub = parsed.data_dir.filter(|d| !d.trim().is_empty()).unwrap_or_else(|| PORTABLE_DEFAULT_DIR.into());
            return AppDirs::portable(absolute(dir, &sub), PortableSource::ConfigFile);
        }
        if dir.join(PORTABLE_MARKER).is_file() {
            return AppDirs::portable(dir.join(PORTABLE_DEFAULT_DIR), PortableSource::Marker);
        }
    }
    os()
}

static DIRS: OnceLock<AppDirs> = OnceLock::new();

/// This process's folders (resolved once).
pub fn dirs() -> &'static AppDirs {
    DIRS.get_or_init(|| {
        let exe = std::env::current_exe().ok();
        let near = exe.as_deref().map(exe_dirs).unwrap_or_default();
        let d = resolve(std::env::var(PORTABLE_ENV).ok().as_deref(), &near, AppDirs::os_default);
        if let Some(root) = &d.portable_root {
            tracing::info!("Portable mode: app data in {}", root.display());
        }
        d
    })
}

pub fn data_dir() -> PathBuf {
    dirs().data.clone()
}

pub fn data_local_dir() -> PathBuf {
    dirs().data_local.clone()
}

pub fn config_dir() -> PathBuf {
    dirs().config.clone()
}

pub fn cache_dir() -> PathBuf {
    dirs().cache.clone()
}

/// Portable mode state for the Settings screen (`get_portable_status`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortableStatus {
    pub enabled: bool,
    pub root: Option<String>,
    pub source: Option<PortableSource>,
    pub data_dir: String,
    pub config_dir: String,
    pub cache_dir: String,
    /// Where credentials are kept: `keychain`, `vault_file` or `memory`.
    pub credential_store: String,
}

pub fn status(credential_store: &str) -> PortableStatus {
    let d = dirs();
    PortableStatus {
        enabled: d.is_portable(),
        root: d.portable_root.as_ref().map(|p| p.display().to_string()),
        source: d.portable_source,
        data_dir: d.data.display().to_string(),
        config_dir: d.config.display().to_string(),
        cache_dir: d.cache.display().to_string(),
        credential_store: credential_store.to_string(),
    }
}

/// Environment for libraries that find their own folders (the webview's
/// storage, `dirs`-based crates, the temp folder): pointed into the portable
/// root. Call at startup before any thread is spawned.
pub fn process_env(d: &AppDirs) -> Vec<(&'static str, PathBuf)> {
    let Some(root) = &d.portable_root else { return Vec::new() };
    let mut v = Vec::new();
    if let Some(t) = &d.temp {
        if cfg!(windows) {
            v.push(("TMP", t.clone()));
            v.push(("TEMP", t.clone()));
        } else {
            v.push(("TMPDIR", t.clone()));
        }
    }
    if cfg!(windows) {
        // WebView2 keeps cookies, localStorage and caches here.
        v.push(("WEBVIEW2_USER_DATA_FOLDER", root.join("webview")));
    } else if cfg!(all(unix, not(target_os = "macos"))) {
        // WebKitGTK and XDG-aware libraries.
        v.push(("XDG_DATA_HOME", root.join("xdg").join("data")));
        v.push(("XDG_CONFIG_HOME", root.join("xdg").join("config")));
        v.push(("XDG_CACHE_HOME", root.join("xdg").join("cache")));
    }
    v
}

/// macOS portable mode: the web view's own data store. WKWebView cannot be
/// pointed at a folder (`data_directory` is unsupported there), so portable
/// mode gives the window a separate data store whose identifier is derived
/// from the portable folder: each portable copy keeps its web storage
/// (window and panel preferences) apart from the installed app and from
/// other copies, and finds it again on the next start. The store itself
/// stays under `~/Library/WebKit` (Apple decides where). Needs macOS 14 or
/// later; older versions use the default store (as before).
pub fn webview_store_identifier(d: &AppDirs) -> Option<[u8; 16]> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    d.portable_root.as_deref().map(store_identifier_for)
}

/// A stable UUID (RFC 9562 version 8) from a folder path.
pub fn store_identifier_for(root: &Path) -> [u8; 16] {
    use sha2::{Digest, Sha256};
    let path = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let digest = Sha256::digest(format!("ufop-webview-store:{}", path.display()).as_bytes());
    let mut id = [0u8; 16];
    id.copy_from_slice(&digest[..16]);
    id[6] = (id[6] & 0x0f) | 0x80; // version 8 (custom)
    id[8] = (id[8] & 0x3f) | 0x80; // RFC 4122 variant
    id
}

/// Create the portable folders and export [`process_env`] (no-op outside
/// portable mode).
pub fn apply_process_env() {
    let d = dirs();
    if !d.is_portable() {
        return;
    }
    for p in [&d.data, &d.config, &d.cache] {
        let _ = std::fs::create_dir_all(p);
    }
    for (k, v) in process_env(d) {
        let _ = std::fs::create_dir_all(&v);
        std::env::set_var(k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webview_store_identifier_is_stable_per_portable_folder() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        let id = store_identifier_for(a.path());
        assert_eq!(id, store_identifier_for(a.path()), "same folder, same store");
        std::fs::create_dir(a.path().join("x")).unwrap();
        assert_eq!(id, store_identifier_for(&a.path().join("x").join("..")), "the path is canonicalized");
        assert_ne!(id, store_identifier_for(b.path()));
        let u = uuid::Uuid::from_bytes(id);
        assert_eq!((u.get_version_num(), u.get_variant()), (8, uuid::Variant::RFC4122));
        // Only macOS portable mode uses a custom store.
        let mut d = os();
        assert_eq!(webview_store_identifier(&d), None);
        d.portable_root = Some(a.path().into());
        assert_eq!(webview_store_identifier(&d), cfg!(target_os = "macos").then_some(id));
    }

    fn os() -> AppDirs {
        AppDirs { data: "/os/data".into(), data_local: "/os/local".into(), config: "/os/config".into(), cache: "/os/cache".into(), temp: None, portable_root: None, portable_source: None }
    }

    #[test]
    fn env_beats_files_next_to_the_executable() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join(PORTABLE_MARKER), "").unwrap();
        std::fs::write(t.path().join(PORTABLE_CONFIG), r#"{"data_dir":"cfg"}"#).unwrap();
        let exe = [t.path().to_path_buf()];
        let d = resolve(Some("/stick/ufop"), &exe, os);
        assert_eq!((d.portable_root.as_deref(), d.portable_source), (Some(Path::new("/stick/ufop")), Some(PortableSource::Env)));
        assert_eq!(d.data, PathBuf::from("/stick/ufop/data"));
        assert_eq!(d.cache, PathBuf::from("/stick/ufop/cache"));
        assert_eq!(d.config, PathBuf::from("/stick/ufop/config"));
        assert_eq!(d.temp, Some(PathBuf::from("/stick/ufop/tmp")));
        // Relative env paths are taken from the executable's folder.
        assert_eq!(resolve(Some("rel"), &exe, os).portable_root, Some(t.path().join("rel")));
        // An empty value does not turn portable mode on.
        assert_eq!(resolve(Some("  "), &exe, os).portable_source, Some(PortableSource::ConfigFile));
    }

    #[test]
    fn config_file_beats_marker_and_defaults_its_folder() {
        let t = tempfile::tempdir().unwrap();
        let exe = [t.path().to_path_buf()];
        std::fs::write(t.path().join(PORTABLE_MARKER), "").unwrap();
        assert_eq!(resolve(None, &exe, os).portable_source, Some(PortableSource::Marker));
        assert_eq!(resolve(None, &exe, os).portable_root, Some(t.path().join(PORTABLE_DEFAULT_DIR)));
        std::fs::write(t.path().join(PORTABLE_CONFIG), r#"{"data_dir":"Data Folder"}"#).unwrap();
        let d = resolve(None, &exe, os);
        assert_eq!((d.portable_root, d.portable_source), (Some(t.path().join("Data Folder")), Some(PortableSource::ConfigFile)));
        std::fs::write(t.path().join(PORTABLE_CONFIG), "{}").unwrap();
        assert_eq!(resolve(None, &exe, os).portable_root, Some(t.path().join(PORTABLE_DEFAULT_DIR)));
        std::fs::write(t.path().join(PORTABLE_CONFIG), "not json").unwrap();
        assert_eq!(resolve(None, &exe, os).portable_root, Some(t.path().join(PORTABLE_DEFAULT_DIR)));
        std::fs::write(t.path().join(PORTABLE_CONFIG), r#"{"data_dir":"/abs/where"}"#).unwrap();
        assert_eq!(resolve(None, &exe, os).portable_root, Some(PathBuf::from("/abs/where")));
    }

    #[test]
    fn without_markers_the_os_folders_are_used() {
        let t = tempfile::tempdir().unwrap();
        let d = resolve(None, &[t.path().to_path_buf()], os);
        assert_eq!(d, os());
        assert!(!d.is_portable() && process_env(&d).is_empty());
        // A folder named `portable` is not the marker.
        std::fs::create_dir(t.path().join(PORTABLE_MARKER)).unwrap();
        assert!(!resolve(None, &[t.path().to_path_buf()], os).is_portable());
    }

    #[test]
    fn macos_bundles_look_next_to_the_app() {
        let dirs = exe_dirs(Path::new("/Volumes/Stick/UFOP.app/Contents/MacOS/ufop"));
        assert_eq!(dirs, vec![PathBuf::from("/Volumes/Stick/UFOP.app/Contents/MacOS"), PathBuf::from("/Volumes/Stick")]);
        assert_eq!(exe_dirs(Path::new("/usr/bin/ufop")), vec![PathBuf::from("/usr/bin")]);
        let t = tempfile::tempdir().unwrap();
        let macos = t.path().join("UFOP.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        std::fs::write(t.path().join(PORTABLE_MARKER), "").unwrap();
        let d = resolve(None, &exe_dirs(&macos.join("ufop")), os);
        assert_eq!(d.portable_root, Some(t.path().join(PORTABLE_DEFAULT_DIR)));
    }

    #[test]
    fn portable_environment_points_into_the_root() {
        let d = AppDirs::portable("/p".into(), PortableSource::Marker);
        let env: std::collections::HashMap<_, _> = process_env(&d).into_iter().collect();
        if cfg!(windows) {
            assert_eq!(env["TEMP"], PathBuf::from("/p/tmp"));
            assert_eq!(env["WEBVIEW2_USER_DATA_FOLDER"], PathBuf::from("/p/webview"));
        } else {
            assert_eq!(env["TMPDIR"], PathBuf::from("/p/tmp"));
        }
        if cfg!(target_os = "linux") {
            assert_eq!(env["XDG_DATA_HOME"], PathBuf::from("/p/xdg/data"));
        }
    }
}
