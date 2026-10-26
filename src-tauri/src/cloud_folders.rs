//! Folders on this computer that a cloud service's own app keeps in sync
//! (Dropbox, OneDrive, Google Drive, iCloud Drive, Box, pCloud, MEGA,
//! Nextcloud, ownCloud, Synology Drive, Proton Drive). They are listed in
//! the sidebar so they open like any local folder; nothing is uploaded by
//! UFOP itself.
//!
//! Detection reads what the apps leave behind and never contacts them:
//! Dropbox's `info.json`, the `OneDrive*` environment variables (Windows),
//! the File Provider folders in `~/Library/CloudStorage` (macOS), the
//! Nextcloud / ownCloud `.cfg` sync folder list, and the default folder
//! names of the others. Only folders that exist are returned, each once.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// One detected folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudFolder {
    /// `dropbox`, `onedrive`, `google_drive`, `icloud`, `box`, `pcloud`,
    /// `mega`, `nextcloud`, `owncloud`, `synology`, `proton`.
    pub provider: String,
    /// What the sidebar shows: "Dropbox", "OneDrive — Contoso", "Google Drive (me@example.com)".
    pub label: String,
    pub path: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
}

impl Os {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Linux
        }
    }
}

/// Where to look: the home folder, the per-user app data folders and the
/// environment (injected so every platform's layout can be tested anywhere).
pub struct Probe {
    pub os: Os,
    pub home: PathBuf,
    /// `%APPDATA%` on Windows, `~/.config` elsewhere.
    pub config_dir: Option<PathBuf>,
    /// `%LOCALAPPDATA%` on Windows.
    pub local_data_dir: Option<PathBuf>,
    pub env: HashMap<String, String>,
    /// Drive roots to try for apps that mount a drive letter (Windows: `G:\`, `P:\`).
    pub drive_roots: Vec<PathBuf>,
}

impl Probe {
    pub fn current() -> Option<Self> {
        let os = Os::current();
        let drive_roots = if os == Os::Windows { ('D'..='Z').map(|c| PathBuf::from(format!("{c}:\\"))).collect() } else { Vec::new() };
        Some(Self {
            os,
            home: dirs::home_dir()?,
            config_dir: dirs::config_dir(),
            local_data_dir: dirs::data_local_dir(),
            env: std::env::vars().collect(),
            drive_roots,
        })
    }
}

/// Every cloud-synced folder found, in a stable order (by provider, then label).
pub fn detect(p: &Probe) -> Vec<CloudFolder> {
    let mut out: Vec<CloudFolder> = Vec::new();
    dropbox(p, &mut out);
    onedrive(p, &mut out);
    if p.os == Os::MacOs {
        cloud_storage(p, &mut out);
    }
    google_drive(p, &mut out);
    icloud(p, &mut out);
    for (provider, label, names) in DEFAULT_FOLDERS {
        for name in *names {
            push(&mut out, provider, label, p.home.join(name));
        }
    }
    if p.os == Os::Windows {
        // pCloud Drive mounts P: by default.
        for root in &p.drive_roots {
            if root.to_string_lossy().starts_with('P') && root.join("pCloud Drive").is_dir() {
                push(&mut out, "pcloud", "pCloud Drive", root.clone());
            }
        }
    }
    sync_client_cfg(p, &mut out, "nextcloud", "Nextcloud", "Nextcloud", "nextcloud.cfg");
    sync_client_cfg(p, &mut out, "owncloud", "ownCloud", "ownCloud", "owncloud.cfg");

    // Each folder once (the first, most specific, label wins).
    let mut seen = HashSet::new();
    out.retain(|f| seen.insert(normalize(&f.path)));
    out.sort_by(|a, b| (provider_rank(&a.provider), &a.label).cmp(&(provider_rank(&b.provider), &b.label)));
    out
}

const PROVIDERS: [&str; 11] = ["dropbox", "onedrive", "google_drive", "icloud", "box", "pcloud", "mega", "nextcloud", "owncloud", "synology", "proton"];

fn provider_rank(p: &str) -> usize {
    PROVIDERS.iter().position(|x| *x == p).unwrap_or(PROVIDERS.len())
}

/// Default folder names, relative to the home folder.
const DEFAULT_FOLDERS: &[(&str, &str, &[&str])] = &[
    ("box", "Box", &["Box", "Box Sync"]),
    ("pcloud", "pCloud Drive", &["pCloud Drive", "pCloudDrive"]),
    ("mega", "MEGA", &["MEGA", "MEGAsync"]),
    ("synology", "Synology Drive", &["SynologyDrive"]),
    ("proton", "Proton Drive", &["Proton Drive"]),
];

fn normalize(path: &str) -> String {
    let p = path.trim_end_matches(['/', '\\']).replace('\\', "/");
    if cfg!(windows) {
        p.to_lowercase()
    } else {
        p
    }
}

fn push(out: &mut Vec<CloudFolder>, provider: &str, label: &str, path: PathBuf) {
    if path.is_dir() {
        out.push(CloudFolder { provider: provider.into(), label: label.into(), path: path.to_string_lossy().into_owned() });
    }
}

// ── Dropbox ──

/// `info.json`: `{"personal": {"path": …}, "business": {"path": …, "team_name"?: …}}`.
fn dropbox(p: &Probe, out: &mut Vec<CloudFolder>) {
    let mut files = vec![p.home.join(".dropbox").join("info.json")];
    if p.os == Os::Windows {
        files.extend(p.config_dir.iter().map(|d| d.join("Dropbox").join("info.json")));
        files.extend(p.local_data_dir.iter().map(|d| d.join("Dropbox").join("info.json")));
    }
    let mut found = false;
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        let Ok(info) = serde_json::from_str::<serde_json::Value>(&text) else { continue };
        for (kind, name) in [("personal", "Dropbox"), ("business", "Dropbox (Business)")] {
            let Some(path) = info.get(kind).and_then(|a| a.get("path")).and_then(|v| v.as_str()) else { continue };
            let team = info.get(kind).and_then(|a| a.get("team_name")).and_then(|v| v.as_str()).filter(|t| !t.is_empty());
            let label = match team {
                Some(t) => format!("Dropbox — {t}"),
                None => name.to_string(),
            };
            let before = out.len();
            push(out, "dropbox", &label, PathBuf::from(path));
            found |= out.len() > before;
        }
    }
    if !found {
        push(out, "dropbox", "Dropbox", p.home.join("Dropbox"));
    }
}

// ── OneDrive ──

fn onedrive(p: &Probe, out: &mut Vec<CloudFolder>) {
    // Windows sets these for the signed-in accounts; the folder name of a
    // work account is "OneDrive - <organisation>".
    for var in ["OneDriveConsumer", "OneDriveCommercial", "OneDrive"] {
        if let Some(path) = p.env.get(var).filter(|v| !v.is_empty()) {
            let path = PathBuf::from(path);
            let label = onedrive_label(&path);
            push(out, "onedrive", &label, path);
        }
    }
    // The Windows default, older macOS clients and the Linux `onedrive` client.
    push(out, "onedrive", "OneDrive", p.home.join("OneDrive"));
}

fn onedrive_label(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match name.split_once(" - ") {
        Some((_, org)) if !org.trim().is_empty() => format!("OneDrive — {}", org.trim()),
        _ => "OneDrive".into(),
    }
}

// ── macOS File Provider folders ──

/// `~/Library/CloudStorage/<Provider>-<account>`: OneDrive-Personal,
/// OneDrive-Contoso, GoogleDrive-me@example.com, Box-Box, Dropbox,
/// ProtonDrive-me@proton.me, pCloudDrive…
fn cloud_storage(p: &Probe, out: &mut Vec<CloudFolder>) {
    let dir = p.home.join("Library").join("CloudStorage");
    let Ok(entries) = std::fs::read_dir(&dir) else { return };
    let mut names: Vec<String> = entries.filter_map(|e| e.ok()).filter(|e| e.path().is_dir()).map(|e| e.file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    for name in names {
        let (prefix, account) = match name.split_once('-') {
            Some((a, b)) => (a, Some(b)),
            None => (name.as_str(), None),
        };
        let path = dir.join(&name);
        match prefix {
            "OneDrive" => {
                let label = match account {
                    Some("Personal") | None => "OneDrive".to_string(),
                    Some(org) => format!("OneDrive — {org}"),
                };
                push(out, "onedrive", &label, path);
            }
            "GoogleDrive" => {
                let label = account.map(|a| format!("Google Drive ({a})")).unwrap_or_else(|| "Google Drive".into());
                // Drive for desktop puts the files under "My Drive"; shared drives beside it.
                let my = path.join("My Drive");
                push(out, "google_drive", &label, if my.is_dir() { my } else { path });
            }
            "Dropbox" => {
                let label = account.map(|a| format!("Dropbox — {a}")).unwrap_or_else(|| "Dropbox".into());
                push(out, "dropbox", &label, path);
            }
            "Box" => push(out, "box", "Box", path),
            "ProtonDrive" => {
                let label = account.map(|a| format!("Proton Drive ({a})")).unwrap_or_else(|| "Proton Drive".into());
                push(out, "proton", &label, path);
            }
            "pCloudDrive" | "pCloud" => push(out, "pcloud", "pCloud Drive", path),
            "SynologyDrive" => push(out, "synology", "Synology Drive", path),
            _ => {}
        }
    }
}

// ── Google Drive ──

fn google_drive(p: &Probe, out: &mut Vec<CloudFolder>) {
    match p.os {
        // Drive for desktop: a virtual drive (G: by default) with "My Drive".
        Os::Windows => {
            for root in &p.drive_roots {
                push(out, "google_drive", "Google Drive", root.join("My Drive"));
            }
            // The older Backup and Sync folder.
            push(out, "google_drive", "Google Drive", p.home.join("Google Drive"));
        }
        // Older clients (Backup and Sync, before File Provider).
        Os::MacOs => push(out, "google_drive", "Google Drive", p.home.join("Google Drive")),
        // No official client; the common third-party folder names.
        Os::Linux => {
            for name in ["Google Drive", "GoogleDrive", "google-drive"] {
                push(out, "google_drive", "Google Drive", p.home.join(name));
            }
        }
    }
}

// ── iCloud Drive ──

fn icloud(p: &Probe, out: &mut Vec<CloudFolder>) {
    match p.os {
        Os::MacOs => push(out, "icloud", "iCloud Drive", p.home.join("Library").join("Mobile Documents").join("com~apple~CloudDocs")),
        Os::Windows => push(out, "icloud", "iCloud Drive", p.home.join("iCloudDrive")),
        Os::Linux => {}
    }
}

// ── Nextcloud / ownCloud ──

/// The desktop client lists its sync folders in a Qt settings file:
/// `0\Folders\1\localPath=/home/me/Nextcloud/` (also `FoldersWithPlaceholders`).
fn sync_client_cfg(p: &Probe, out: &mut Vec<CloudFolder>, provider: &str, label: &str, app_dir: &str, file: &str) {
    let mut files = Vec::new();
    match p.os {
        Os::MacOs => files.push(p.home.join("Library").join("Preferences").join(app_dir).join(file)),
        _ => files.extend(p.config_dir.iter().map(|d| d.join(app_dir).join(file))),
    }
    let mut found = false;
    for f in files {
        let Ok(text) = std::fs::read_to_string(&f) else { continue };
        let paths = cfg_local_paths(&text);
        let many = paths.len() > 1;
        for path in paths {
            let pb = PathBuf::from(&path);
            let name = pb.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let l = if many && !name.is_empty() && name != label { format!("{label} — {name}") } else { label.to_string() };
            let before = out.len();
            push(out, provider, &l, pb);
            found |= out.len() > before;
        }
    }
    if !found {
        push(out, provider, label, p.home.join(app_dir));
    }
}

/// The `localPath` values of a Nextcloud/ownCloud `.cfg`, in file order.
pub fn cfg_local_paths(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some((key, value)) = line.split_once('=') else { continue };
        let parts: Vec<&str> = key.split('\\').collect();
        if parts.len() == 4 && matches!(parts[1], "Folders" | "FoldersWithPlaceholders" | "Multifolders") && parts[3] == "localPath" {
            let v = value.trim().trim_matches('"');
            if !v.is_empty() && !out.iter().any(|x: &String| x == v) {
                out.push(v.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn probe(os: Os, home: &Path) -> Probe {
        Probe { os, home: home.to_path_buf(), config_dir: Some(home.join(".config")), local_data_dir: Some(home.join("AppData/Local")), env: HashMap::new(), drive_roots: Vec::new() }
    }

    fn mkdir(p: &Path) -> String {
        std::fs::create_dir_all(p).unwrap();
        p.to_string_lossy().into_owned()
    }

    fn found(p: &Probe) -> Vec<(String, String)> {
        detect(p).into_iter().map(|f| (f.provider, f.label)).collect()
    }

    #[test]
    fn nothing_installed_finds_nothing() {
        let t = tempfile::tempdir().unwrap();
        for os in [Os::Linux, Os::MacOs, Os::Windows] {
            assert!(detect(&probe(os, t.path())).is_empty());
        }
    }

    #[test]
    fn dropbox_personal_and_business_from_info_json() {
        let t = tempfile::tempdir().unwrap();
        let personal = mkdir(&t.path().join("Dropbox"));
        let business = mkdir(&t.path().join("Dropbox (Contoso)"));
        mkdir(&t.path().join(".dropbox"));
        let info = serde_json::json!({"personal": {"path": personal}, "business": {"path": business, "team_name": "Contoso"}, "other": {"path": "/nope"}});
        std::fs::write(t.path().join(".dropbox/info.json"), info.to_string()).unwrap();
        let f = detect(&probe(Os::Linux, t.path()));
        assert_eq!(f.iter().map(|f| (f.label.as_str(), f.path.as_str())).collect::<Vec<_>>(), [("Dropbox", personal.as_str()), ("Dropbox — Contoso", business.as_str())]);
    }

    #[test]
    fn dropbox_moved_elsewhere_is_found_and_a_missing_folder_is_not() {
        let t = tempfile::tempdir().unwrap();
        let moved = mkdir(&t.path().join("Data/MyDropbox"));
        mkdir(&t.path().join(".dropbox"));
        std::fs::write(t.path().join(".dropbox/info.json"), serde_json::json!({"personal": {"path": moved}, "business": {"path": "/gone"}}).to_string()).unwrap();
        let f = detect(&probe(Os::Linux, t.path()));
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].path, moved);
        // A broken info.json falls back to ~/Dropbox.
        std::fs::write(t.path().join(".dropbox/info.json"), "{not json").unwrap();
        assert!(detect(&probe(Os::Linux, t.path())).is_empty());
        mkdir(&t.path().join("Dropbox"));
        assert_eq!(found(&probe(Os::Linux, t.path())), [("dropbox".into(), "Dropbox".into())]);
    }

    #[test]
    fn onedrive_accounts_from_the_windows_environment() {
        let t = tempfile::tempdir().unwrap();
        let personal = mkdir(&t.path().join("OneDrive"));
        let work = mkdir(&t.path().join("OneDrive - Contoso Ltd"));
        let mut p = probe(Os::Windows, t.path());
        p.env.insert("OneDrive".into(), work.clone());
        p.env.insert("OneDriveCommercial".into(), work.clone());
        p.env.insert("OneDriveConsumer".into(), personal.clone());
        let f = detect(&p);
        assert_eq!(f.iter().map(|f| (f.label.as_str(), f.path.as_str())).collect::<Vec<_>>(), [("OneDrive", personal.as_str()), ("OneDrive — Contoso Ltd", work.as_str())]);
    }

    #[test]
    fn macos_file_provider_folders() {
        let t = tempfile::tempdir().unwrap();
        let cs = t.path().join("Library/CloudStorage");
        mkdir(&cs.join("OneDrive-Personal"));
        mkdir(&cs.join("OneDrive-Contoso"));
        let my_drive = mkdir(&cs.join("GoogleDrive-me@example.com/My Drive"));
        mkdir(&cs.join("Box-Box"));
        mkdir(&cs.join("ProtonDrive-me@proton.me"));
        mkdir(&cs.join("SomethingElse-x"));
        mkdir(&t.path().join("Library/Mobile Documents/com~apple~CloudDocs"));
        let p = probe(Os::MacOs, t.path());
        assert_eq!(
            found(&p),
            [
                ("onedrive".into(), "OneDrive".into()),
                ("onedrive".into(), "OneDrive — Contoso".into()),
                ("google_drive".into(), "Google Drive (me@example.com)".into()),
                ("icloud".into(), "iCloud Drive".into()),
                ("box".into(), "Box".into()),
                ("proton".into(), "Proton Drive (me@proton.me)".into()),
            ]
        );
        assert!(detect(&p).iter().any(|f| f.path == my_drive));
    }

    #[test]
    fn windows_google_drive_letter_and_icloud() {
        let t = tempfile::tempdir().unwrap();
        let g = t.path().join("G");
        mkdir(&g.join("My Drive"));
        mkdir(&t.path().join("iCloudDrive"));
        let mut p = probe(Os::Windows, t.path());
        p.drive_roots = vec![t.path().join("F"), g.clone()];
        assert_eq!(found(&p), [("google_drive".into(), "Google Drive".into()), ("icloud".into(), "iCloud Drive".into())]);
    }

    #[test]
    fn nextcloud_sync_folders_from_its_config() {
        let t = tempfile::tempdir().unwrap();
        let a = mkdir(&t.path().join("Nextcloud"));
        let b = mkdir(&t.path().join("Work Cloud"));
        mkdir(&t.path().join(".config/Nextcloud"));
        std::fs::write(
            t.path().join(".config/Nextcloud/nextcloud.cfg"),
            format!("[General]\nx=1\n\n[Accounts]\n0\\url=https://cloud.example.com\n0\\Folders\\1\\localPath={a}/\n0\\FoldersWithPlaceholders\\2\\localPath={b}\n0\\Folders\\3\\localPath=/gone\n"),
        )
        .unwrap();
        assert_eq!(found(&probe(Os::Linux, t.path())), [("nextcloud".into(), "Nextcloud".into()), ("nextcloud".into(), "Nextcloud — Work Cloud".into())]);
    }

    #[test]
    fn default_folder_names_and_each_folder_once() {
        let t = tempfile::tempdir().unwrap();
        mkdir(&t.path().join("MEGA"));
        mkdir(&t.path().join("SynologyDrive"));
        mkdir(&t.path().join("Box"));
        let od = mkdir(&t.path().join("OneDrive"));
        let mut p = probe(Os::Windows, t.path());
        p.env.insert("OneDrive".into(), od.clone());
        p.env.insert("OneDriveConsumer".into(), format!("{od}/"));
        let f = found(&p);
        assert_eq!(f, [("onedrive".into(), "OneDrive".into()), ("box".into(), "Box".into()), ("mega".into(), "MEGA".into()), ("synology".into(), "Synology Drive".into())]);
    }

    #[test]
    fn cfg_parser_reads_only_local_paths() {
        let text = "0\\Folders\\1\\localPath=/a\n0\\Folders\\1\\targetPath=/\n1\\Multifolders\\4\\localPath=\"C:/Users/me/Cloud/\"\n0\\Folders\\1\\localPath=/a\nlocalPath=/no\n";
        assert_eq!(cfg_local_paths(text), ["/a", "C:/Users/me/Cloud/"]);
    }
}
