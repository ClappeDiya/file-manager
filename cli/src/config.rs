//! CLI configuration management.
//! Stores credentials and settings in ~/.config/ufop/config.toml
//! (`UFOP_CONFIG` overrides the path).
//!
//! Named profiles (`[profiles.<name>]`) hold their own API server, tokens and
//! output format; saved connections are shared. The active profile is the
//! `--profile` option (or `UFOP_PROFILE`), else `current_profile` from the file
//! (`ufop profile use`), else the top-level settings ("default"). Commands
//! keep using `CliConfig::load()`/`save()`: loading overlays the active
//! profile onto the top-level fields and saving writes them back to it.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub use ufop_core::profiles::{valid_profile_name, Profile, DEFAULT_PROFILE};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CliConfig {
    pub api_url: Option<String>,
    pub token: Option<String>,
    pub default_format: Option<String>,
    /// Absent in a file the desktop app created.
    #[serde(default)]
    pub connections: Vec<SavedConnection>,
    /// `ufop server …`: base URL, access/automation token and refresh token.
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub server_token: Option<String>,
    #[serde(default)]
    pub server_refresh: Option<String>,
    /// CSRF token sent with the refresh cookie (`X-CSRF-Token`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_csrf: Option<String>,
    /// Profile used when `--profile` is not given (`ufop profile use`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_profile: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub profiles: BTreeMap<String, Profile>,
    /// The named profile overlaid on the top-level fields, and those fields as stored.
    #[serde(skip)]
    overlay: Option<(String, Profile)>,
    /// `--api-url` for this run and the value it replaced (restored on save unless changed).
    #[serde(skip)]
    api_override: Option<(String, Option<String>)>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedConnection {
    pub name: String,
    pub uri: String,
    pub protocol: String,
    pub created_at: String,
}

/// Process-wide choices from the command line.
#[derive(Debug, Default)]
struct Selection {
    profile: Option<String>,
    api_url: Option<String>,
}
static SELECTION: OnceLock<Selection> = OnceLock::new();

/// Record `--profile` and `--api-url` for every later `CliConfig::load()`.
pub fn select(profile: Option<String>, api_url: Option<String>) {
    let _ = SELECTION.set(Selection { profile: profile.filter(|p| !p.is_empty()), api_url });
}

impl CliConfig {
    /// Config file path: `UFOP_CONFIG`, else ~/.config/ufop/config.toml
    pub fn path() -> PathBuf {
        ufop_core::profiles::config_path(dirs::config_dir())
    }

    /// Load config from disk with the active profile applied, returning default if not found.
    pub fn load() -> Self {
        let sel = SELECTION.get();
        let mut c = Self::load_raw(&Self::path());
        let name = sel.and_then(|s| s.profile.clone()).or_else(|| c.current_profile.clone());
        if let Some(name) = name.filter(|n| n != DEFAULT_PROFILE) {
            c.activate(&name);
        }
        // --api-url / UFOP_API_URL for this run only (never saved).
        if let Some(url) = sel.and_then(|s| s.api_url.clone()) {
            c.override_api_url(url);
        }
        c
    }

    fn override_api_url(&mut self, url: String) {
        let file = self.api_url.replace(url.clone());
        self.api_override = Some((url, file));
    }

    /// The file as stored, without any profile applied.
    pub fn load_raw(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| toml::from_str(&s).ok()).unwrap_or_default()
    }

    /// The active profile's name.
    pub fn profile_name(&self) -> &str {
        self.overlay.as_ref().map(|(n, _)| n.as_str()).unwrap_or(DEFAULT_PROFILE)
    }

    fn top_level(&self) -> Profile {
        Profile {
            api_url: self.api_url.clone(),
            token: self.token.clone(),
            default_format: self.default_format.clone(),
            server_url: self.server_url.clone(),
            server_token: self.server_token.clone(),
            server_refresh: self.server_refresh.clone(),
            server_csrf: self.server_csrf.clone(),
        }
    }

    fn set_top_level(&mut self, p: Profile) {
        self.api_url = p.api_url;
        self.token = p.token;
        self.default_format = p.default_format;
        self.server_url = p.server_url;
        self.server_token = p.server_token;
        self.server_refresh = p.server_refresh;
        self.server_csrf = p.server_csrf;
    }

    /// Overlay a named profile (empty when it does not exist yet: saving creates it).
    fn activate(&mut self, name: &str) {
        let stored = self.top_level();
        let p = self.profiles.get(name).cloned().unwrap_or_default();
        self.set_top_level(p);
        self.overlay = Some((name.to_string(), stored));
    }

    /// Whether `name` exists ("default" always does).
    pub fn has_profile(&self, name: &str) -> bool {
        name == DEFAULT_PROFILE || self.profiles.contains_key(name)
    }

    /// The settings of `name` regardless of the overlay.
    pub fn profile(&self, name: &str) -> Option<Profile> {
        if name == DEFAULT_PROFILE {
            return Some(self.overlay.as_ref().map(|(_, p)| p.clone()).unwrap_or_else(|| self.top_level()));
        }
        if self.profile_name() == name {
            return Some(self.top_level());
        }
        self.profiles.get(name).cloned()
    }

    /// The file contents: the active profile's fields go back to it.
    fn stored(&self) -> Self {
        let mut out = self.clone();
        if let Some((name, top)) = &self.overlay {
            out.profiles.insert(name.clone(), self.top_level());
            out.set_top_level(top.clone());
            out.overlay = None;
        }
        // --api-url is per run: keep what the file had unless a command changed it.
        if let Some((url, file)) = &self.api_override {
            if self.api_url.as_deref() == Some(url.as_str()) {
                match &self.overlay {
                    Some((name, _)) => out.profiles.entry(name.clone()).or_default().api_url = file.clone(),
                    None => out.api_url = file.clone(),
                }
            }
        }
        out.api_override = None;
        out
    }

    /// Save config to disk.
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.save_to(&Self::path())
    }

    pub fn save_to(&self, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        let content = toml::to_string_pretty(&self.stored())?;
        std::fs::write(path, content)?;
        // Holds credentials: owner-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overlay_round_trips_and_keeps_other_profiles() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("c.toml");
        std::fs::write(
            &f,
            "api_url = \"https://prod\"\ntoken = \"tprod\"\nconnections = []\n[profiles.staging]\napi_url = \"https://staging\"\n[profiles.lab]\nserver_url = \"http://lab:8200\"\n",
        )
        .unwrap();
        let mut c = CliConfig::load_raw(&f);
        c.activate("staging");
        assert_eq!(c.profile_name(), "staging");
        assert_eq!(c.api_url.as_deref(), Some("https://staging"));
        assert_eq!(c.token, None, "profiles do not inherit credentials");
        assert_eq!(c.profile(DEFAULT_PROFILE).unwrap().token.as_deref(), Some("tprod"));
        c.token = Some("tstaging".into());
        c.save_to(&f).unwrap();

        let raw = CliConfig::load_raw(&f);
        assert_eq!(raw.api_url.as_deref(), Some("https://prod"));
        assert_eq!(raw.token.as_deref(), Some("tprod"));
        assert_eq!(raw.profiles["staging"].token.as_deref(), Some("tstaging"));
        assert_eq!(raw.profiles["lab"].server_url.as_deref(), Some("http://lab:8200"));

        // A profile that does not exist yet is created on save.
        let mut n = CliConfig::load_raw(&f);
        n.activate("new");
        assert!(!n.has_profile("missing") && n.has_profile(DEFAULT_PROFILE));
        n.server_url = Some("http://new".into());
        n.save_to(&f).unwrap();
        assert_eq!(CliConfig::load_raw(&f).profiles["new"].server_url.as_deref(), Some("http://new"));
    }

    #[test]
    fn api_url_override_is_not_saved_unless_changed() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("c.toml");
        std::fs::write(&f, "api_url = \"https://file\"\nconnections = []\n").unwrap();
        let mut c = CliConfig::load_raw(&f);
        c.override_api_url("https://flag".into());
        assert_eq!(c.api_url.as_deref(), Some("https://flag"));
        c.token = Some("t".into());
        c.save_to(&f).unwrap();
        assert_eq!(CliConfig::load_raw(&f).api_url.as_deref(), Some("https://file"));
        // `ufop login --server` sets it explicitly: that is saved.
        let mut c = CliConfig::load_raw(&f);
        c.override_api_url("https://flag".into());
        c.api_url = Some("https://login".into());
        c.save_to(&f).unwrap();
        assert_eq!(CliConfig::load_raw(&f).api_url.as_deref(), Some("https://login"));
    }

    /// The desktop app creates the file with just a profile; the CLI must still read it.
    #[test]
    fn reads_a_file_with_only_profiles() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("ufop").join("config.toml");
        ufop_core::profiles::save_server_url(&f, "office", "https://files.example.com").unwrap();
        let mut c = CliConfig::load_raw(&f);
        assert_eq!(c.profiles["office"].server_url.as_deref(), Some("https://files.example.com"));
        c.activate("office");
        assert_eq!(c.server_url.as_deref(), Some("https://files.example.com"));
        c.server_token = Some("t".into());
        c.save_to(&f).unwrap();
        // And the desktop still lists it, now signed in.
        let rows = ufop_core::profiles::list(&f).unwrap();
        assert!(rows.iter().any(|r| r.name == "office" && r.signed_in));
    }

    #[test]
    fn profile_names() {
        assert!(valid_profile_name("prod-eu_1.a"));
        assert!(!valid_profile_name("") && !valid_profile_name("a b") && !valid_profile_name("a/b") && !valid_profile_name(&"x".repeat(65)));
    }
}
