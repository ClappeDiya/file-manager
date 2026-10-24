//! rclone: binary and config (Settings → rclone), the managed install
//! (Advanced), the in-app remote manager and crypt tools. The work is done
//! in `crate::rclone_engine`; these commands load and store the settings
//! (config key `rclone_settings`) and pass the resolved rclone along.

use crate::core::error::AppError;
use crate::rclone_engine::config::{self, ConfigStep, ObscureMode, ProviderSchema, RemoteDetails, RemoteTest};
use crate::rclone_engine::crypt::{self, CryptInfo, CryptName};
use crate::rclone_engine::install::{self, InstallResult};
use crate::rclone_engine::resolver::{self, Candidate, RcloneSettings, RcloneStatus, Semver};
use crate::rclone_engine::Rclone;
use crate::storage::Repository;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::Path;
use tauri::State;

/// Event carrying the sign-in address of an rclone OAuth step.
pub const AUTH_URL_EVENT: &str = "rclone-auth-url";

/// The stored settings (defaults when unset or unreadable).
pub async fn load(repo: &Repository) -> RcloneSettings {
    repo.get_config(resolver::CONFIG_KEY).await.ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

/// Apply the stored settings (startup).
pub async fn activate_stored(repo: &Repository) {
    resolver::apply(load(repo).await);
}

fn clean(s: Option<String>) -> Option<String> {
    s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// Validate and store `settings` (the caller applies them).
pub async fn persist(repo: &Repository, settings: RcloneSettings) -> Result<RcloneSettings, AppError> {
    let s = RcloneSettings { binary_path: clean(settings.binary_path), config_path: clean(settings.config_path) };
    if let Some(b) = &s.binary_path {
        if !Path::new(b).is_absolute() || !resolver::is_executable(Path::new(b)) {
            return Err(AppError::validation(format!("{b} is not an executable file; choose the rclone program (or use Detect)")));
        }
    }
    if let Some(c) = &s.config_path {
        let p = Path::new(c);
        if !p.is_absolute() || p.is_dir() || !p.parent().is_some_and(|d| d.is_dir()) {
            return Err(AppError::validation(format!("{c} is not usable as rclone.conf: choose a file in an existing folder")));
        }
    }
    let json = serde_json::to_string(&s).map_err(|e| AppError::internal(e.to_string()))?;
    repo.set_config(resolver::CONFIG_KEY, &json).await?;
    Ok(s)
}

/// Settings → rclone.
#[derive(Debug, Clone, Serialize)]
pub struct RcloneSettingsView {
    pub settings: RcloneSettings,
    pub status: RcloneStatus,
    /// rclone's own config file, used when none is chosen (`rclone config file`).
    pub default_config_path: Option<String>,
    pub managed_dir: String,
    /// The release a managed install downloads.
    pub pinned_version: String,
    /// Version of the managed copy, when installed.
    pub managed_version: Option<String>,
    /// The managed copy is older than the pinned release.
    pub update_available: bool,
    /// A release exists for this OS/CPU.
    pub platform_supported: bool,
}

/// `rclone config file` → the path it prints last.
pub fn parse_config_file(out: &str) -> Option<String> {
    out.lines().map(str::trim).rfind(|l| !l.is_empty() && !l.ends_with(':')).map(str::to_string)
}

async fn view() -> RcloneSettingsView {
    let status = resolver::status().await;
    let rc = Rclone::current();
    let default_config_path = if rc.config.is_none() && status.version.is_some() {
        rc.run(&["config".into(), "file".into()], std::time::Duration::from_secs(15)).await.ok().as_deref().and_then(parse_config_file)
    } else {
        None
    };
    let dir = install::managed_dir();
    let managed = install::managed_binary(&dir);
    let managed_version = if resolver::is_executable(&managed) { Rclone::new(managed.to_string_lossy(), None).version().await.ok() } else { None };
    let pinned = Semver::parse(install::PINNED_VERSION);
    RcloneSettingsView {
        settings: resolver::current_settings(),
        status,
        default_config_path,
        managed_dir: dir.to_string_lossy().into_owned(),
        pinned_version: install::PINNED_VERSION.to_string(),
        update_available: managed_version.as_ref().zip(pinned).is_some_and(|(v, p)| v.semver < p),
        managed_version: managed_version.map(|v| v.raw),
        platform_supported: install::platform().is_some(),
    }
}

/// The rclone in use: path, where it came from, version, minimum check.
#[tauri::command]
pub async fn rclone_status() -> Result<RcloneStatus, AppError> {
    Ok(resolver::status().await)
}

#[tauri::command]
pub async fn rclone_settings_get() -> Result<RcloneSettingsView, AppError> {
    Ok(view().await)
}

/// Save and apply the rclone binary / rclone.conf (empty = default).
#[tauri::command]
pub async fn rclone_settings_set(settings: RcloneSettings, repo: State<'_, Repository>) -> Result<RcloneSettingsView, AppError> {
    let s = persist(&repo, settings).await?;
    resolver::apply(s);
    Ok(view().await)
}

/// rclone binaries found on this machine (UFOP_RCLONE, PATH, the managed
/// copy, usual install folders), each with its version.
#[tauri::command]
pub async fn rclone_detect() -> Result<Vec<Candidate>, AppError> {
    Ok(resolver::detect().await)
}

/// What a managed install did, plus the updated settings.
#[derive(Debug, Clone, Serialize)]
pub struct RcloneInstallOutcome {
    pub result: InstallResult,
    pub view: RcloneSettingsView,
}

/// Advanced: download the pinned rclone release, verify it against the
/// release's SHA256SUMS, install it into the app data folder and use it.
/// A binary outside that folder is never touched.
#[tauri::command]
pub async fn rclone_install_managed(repo: State<'_, Repository>) -> Result<RcloneInstallOutcome, AppError> {
    let result = install::install_pinned().await?;
    let s = persist(&repo, RcloneSettings { binary_path: Some(result.path.clone()), ..resolver::current_settings() }).await?;
    resolver::apply(s);
    tracing::info!("Installed managed rclone {} ({})", result.version, result.archive);
    Ok(RcloneInstallOutcome { result, view: view().await })
}

/// Every backend the installed rclone offers, with its options.
#[tauri::command]
pub async fn rclone_providers() -> Result<Vec<ProviderSchema>, AppError> {
    Ok(config::providers(&Rclone::current()).await?.as_ref().clone())
}

/// A remote's options for editing (secrets are not returned).
#[tauri::command]
pub async fn rclone_remote_get(name: String) -> Result<RemoteDetails, AppError> {
    config::remote_details(&Rclone::current(), &name).await
}

/// `rclone config create <name> <type> key=value… --non-interactive`.
#[tauri::command]
pub async fn rclone_remote_create(name: String, remote_type: String, params: Option<BTreeMap<String, String>>, obscure: Option<ObscureMode>) -> Result<ConfigStep, AppError> {
    config::create(&Rclone::current(), &name, &remote_type, &params.unwrap_or_default(), obscure.unwrap_or_default()).await
}

/// `rclone config update <name> key=value… --non-interactive`.
#[tauri::command]
pub async fn rclone_remote_update(name: String, params: Option<BTreeMap<String, String>>, obscure: Option<ObscureMode>) -> Result<ConfigStep, AppError> {
    config::update(&Rclone::current(), &name, &params.unwrap_or_default(), obscure.unwrap_or_default()).await
}

/// Answer the question of a configuration step.
#[tauri::command]
pub async fn rclone_config_continue(name: String, state: String, result: String, params: Option<BTreeMap<String, String>>) -> Result<ConfigStep, AppError> {
    config::continue_with(&Rclone::current(), &name, &state, &result, &params.unwrap_or_default()).await
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthUrlEvent {
    pub name: String,
    pub url: String,
    /// The system opener started a browser on it.
    pub opened: bool,
}

/// Sign in to an OAuth backend through the browser (the step asked "Use
/// auto config?"): the sign-in address opens in the default browser and is
/// also sent as an `rclone-auth-url` event; returns the next step once the
/// browser returns (up to ten minutes; `rclone_config_cancel` stops it).
#[tauri::command]
pub async fn rclone_config_oauth(app: tauri::AppHandle, name: String, state: String, params: Option<BTreeMap<String, String>>, open_browser: Option<bool>) -> Result<ConfigStep, AppError> {
    let open = open_browser.unwrap_or(true);
    let n = name.trim().trim_end_matches(':').to_string();
    let on_url = move |url: String| {
        use tauri::Emitter;
        let opened = open && {
            #[allow(deprecated)]
            {
                use tauri_plugin_shell::ShellExt;
                app.shell().open(&url, None).map_err(|e| tracing::warn!("Could not open the browser: {e}")).is_ok()
            }
        };
        let _ = app.emit(AUTH_URL_EVENT, AuthUrlEvent { name: n.clone(), url, opened });
    };
    config::oauth_sign_in(&Rclone::current(), &name, &state, &params.unwrap_or_default(), &on_url).await
}

/// Stop a browser sign-in started by `rclone_config_oauth`.
#[tauri::command]
pub async fn rclone_config_cancel(name: String) -> Result<bool, AppError> {
    Ok(config::cancel_sign_in(&name))
}

/// `rclone config delete <name>`.
#[tauri::command]
pub async fn rclone_remote_delete(name: String) -> Result<(), AppError> {
    config::delete(&Rclone::current(), &name).await
}

/// List the root of a remote (one level) and report.
#[tauri::command]
pub async fn rclone_remote_test(name: String) -> Result<RemoteTest, AppError> {
    config::test_remote(&Rclone::current(), &name).await
}

/// Whether a remote (`name:` / `name:path`) is a crypt remote, and what it wraps.
#[tauri::command]
pub async fn rclone_crypt_info(remote: String) -> Result<CryptInfo, AppError> {
    let r = crate::connectors::rclone::normalize_remote(&remote)?;
    crypt::info(&Rclone::current(), &r).await
}

/// Decrypt (or with `reverse`, encrypt) file names with a crypt remote's
/// keys, offline (`rclone cryptdecode [--reverse]`).
#[tauri::command]
pub async fn rclone_cryptdecode(remote: String, names: Vec<String>, reverse: Option<bool>) -> Result<Vec<CryptName>, AppError> {
    let r = crate::connectors::rclone::normalize_remote(&remote)?;
    crypt::decode(&Rclone::current(), &r, &names, reverse.unwrap_or(false)).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_file_output() {
        assert_eq!(parse_config_file("Configuration file is stored at:\n/root/.config/rclone/rclone.conf\n").as_deref(), Some("/root/.config/rclone/rclone.conf"));
        assert_eq!(parse_config_file("Configuration file doesn't exist, but rclone will use this path:\n/home/a/.config/rclone/rclone.conf\n").as_deref(), Some("/home/a/.config/rclone/rclone.conf"));
        assert_eq!(parse_config_file(""), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn settings_are_validated_and_stored() {
        use std::os::unix::fs::PermissionsExt;
        let repo = Repository::open_in_memory().await.unwrap();
        assert_eq!(load(&repo).await, RcloneSettings::default());
        let d = tempfile::tempdir().unwrap();
        let bin = d.path().join("rclone");
        std::fs::write(&bin, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        let conf = d.path().join("rclone.conf");
        let s = persist(&repo, RcloneSettings { binary_path: Some(format!(" {} ", bin.display())), config_path: Some(conf.to_string_lossy().into()) }).await.unwrap();
        assert_eq!(s.binary_path.as_deref(), Some(bin.to_str().unwrap()));
        assert_eq!(load(&repo).await, s);
        // Blank fields mean "default".
        let s = persist(&repo, RcloneSettings { binary_path: Some("".into()), config_path: Some("  ".into()) }).await.unwrap();
        assert_eq!(s, RcloneSettings::default());
        // Not executable, relative, missing folder, a folder as config: refused, nothing stored.
        std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o644)).unwrap();
        assert!(persist(&repo, RcloneSettings { binary_path: Some(bin.to_string_lossy().into()), config_path: None }).await.is_err());
        assert!(persist(&repo, RcloneSettings { binary_path: Some("rclone".into()), config_path: None }).await.is_err());
        assert!(persist(&repo, RcloneSettings { binary_path: None, config_path: Some("/no/such/dir/rclone.conf".into()) }).await.is_err());
        assert!(persist(&repo, RcloneSettings { binary_path: None, config_path: Some(d.path().to_string_lossy().into()) }).await.is_err());
        assert_eq!(load(&repo).await, RcloneSettings::default());
    }
}
