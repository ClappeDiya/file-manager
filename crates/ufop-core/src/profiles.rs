//! Named profiles of the `ufop` CLI: an API server, tokens and an output
//! format each, kept in the CLI's config file (`[profiles.<name>]`, with the
//! top-level settings as the "default" profile). The CLI and the desktop app
//! share this type and these file operations, so a server saved in one
//! shows up in the other.
//!
//! The desktop app only ever changes a profile's `server_url`; it never reads
//! or writes tokens (they stay where `ufop server login` put them) and it
//! leaves every other key of the file as it found it.

use crate::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The profile name meaning the top-level settings.
pub const DEFAULT_PROFILE: &str = "default";

/// Settings a profile can carry.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct Profile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_format: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_refresh: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_csrf: Option<String>,
}

/// Valid profile names: letters, digits, `-`, `_`, `.` (1–64).
pub fn valid_profile_name(name: &str) -> bool {
    (1..=64).contains(&name.len()) && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
}

/// The CLI config file: `UFOP_CONFIG`, else `<config dir>/ufop/config.toml`
/// (`config_dir` is the platform's per-user config folder, from `dirs`).
pub fn config_path(config_dir: Option<PathBuf>) -> PathBuf {
    if let Some(p) = std::env::var_os("UFOP_CONFIG").filter(|p| !p.is_empty()) {
        return PathBuf::from(p);
    }
    config_dir.unwrap_or_else(|| PathBuf::from(".")).join("ufop").join("config.toml")
}

/// What a profile list row shows; never a token.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProfileSummary {
    pub name: String,
    pub server_url: Option<String>,
    /// A token for `ufop server …` is stored (the CLI signed in).
    pub signed_in: bool,
    /// The profile `ufop` uses when `--profile` is not given.
    pub current: bool,
}

fn config_err(message: impl Into<String>, advice: impl Into<String>) -> AppError {
    AppError::Configuration { message: message.into(), advice: advice.into() }
}

fn read_table(path: &Path) -> Result<toml::Table, AppError> {
    match std::fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => Ok(toml::Table::new()),
        Ok(text) => text.parse::<toml::Table>().map_err(|e| {
            config_err(format!("The ufop config file {} is not valid TOML: {e}", path.display()), "Fix or remove the file, or set UFOP_CONFIG to another one.")
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml::Table::new()),
        Err(e) => Err(AppError::file_op(format!("Cannot read {}: {e}", path.display()), "Check the file's permissions.")),
    }
}

fn write_table(path: &Path, table: &toml::Table) -> Result<(), AppError> {
    let text = toml::to_string_pretty(table).map_err(|e| AppError::internal(format!("Cannot write the ufop config: {e}")))?;
    crate::fsutil::write_private_atomic(path, text.as_bytes()).map_err(|e| AppError::file_op(format!("Cannot write {}: {e}", path.display()), "Check the file's permissions."))
}

fn str_of(t: &toml::Table, key: &str) -> Option<String> {
    t.get(key).and_then(|v| v.as_str()).map(String::from).filter(|s| !s.is_empty())
}

/// The profiles in the file: "default" first (when the top level has a server
/// or token), then the named ones by name.
pub fn list(path: &Path) -> Result<Vec<ProfileSummary>, AppError> {
    let t = read_table(path)?;
    let current = str_of(&t, "current_profile").unwrap_or_else(|| DEFAULT_PROFILE.to_string());
    let mut out = Vec::new();
    let default = ProfileSummary { name: DEFAULT_PROFILE.into(), server_url: str_of(&t, "server_url"), signed_in: str_of(&t, "server_token").is_some(), current: current == DEFAULT_PROFILE };
    if default.server_url.is_some() || default.signed_in {
        out.push(default);
    }
    if let Some(profiles) = t.get("profiles").and_then(|p| p.as_table()) {
        for (name, p) in profiles {
            let Some(p) = p.as_table() else { continue };
            out.push(ProfileSummary { name: name.clone(), server_url: str_of(p, "server_url"), signed_in: str_of(p, "server_token").is_some(), current: *name == current });
        }
    }
    Ok(out)
}

/// A server address the CLI and the desktop can use: `http(s)://host[:port][/path]`, no
/// user name or password in it, no spaces. Returns it without a trailing slash.
pub fn clean_server_url(url: &str) -> Result<String, AppError> {
    let u = url.trim();
    let rest = u.strip_prefix("https://").or_else(|| u.strip_prefix("http://")).ok_or_else(|| AppError::validation("The server address must start with http:// or https://"))?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || u.chars().any(char::is_whitespace) {
        return Err(AppError::validation("Enter the server address like https://files.example.com:8200"));
    }
    if authority.contains('@') {
        return Err(AppError::validation("Leave the user name and password out of the address; sign in with `ufop server login`"));
    }
    Ok(u.trim_end_matches('/').to_string())
}

/// Create the profile `name`, or change its server address. "default" is the
/// top-level setting. Nothing else in the file changes.
pub fn save_server_url(path: &Path, name: &str, url: &str) -> Result<(), AppError> {
    if name != DEFAULT_PROFILE && !valid_profile_name(name) {
        return Err(AppError::validation(format!("\"{name}\" is not a usable profile name; use letters, digits, '-', '_' and '.'")));
    }
    let url = clean_server_url(url)?;
    let mut t = read_table(path)?;
    if name == DEFAULT_PROFILE {
        t.insert("server_url".into(), toml::Value::String(url));
    } else {
        let profiles = t.entry("profiles").or_insert_with(|| toml::Value::Table(toml::Table::new()));
        let profiles = profiles.as_table_mut().ok_or_else(|| config_err("The ufop config file has a 'profiles' entry that is not a table", "Fix the file by hand."))?;
        let p = profiles.entry(name).or_insert_with(|| toml::Value::Table(toml::Table::new()));
        p.as_table_mut().ok_or_else(|| config_err(format!("Profile '{name}' in the ufop config file is not a table"), "Fix the file by hand."))?.insert("server_url".into(), toml::Value::String(url));
    }
    write_table(path, &t)
}

/// Delete a named profile (and its tokens). "default" cannot be deleted; if the
/// profile was the current one, the CLI goes back to "default".
pub fn remove(path: &Path, name: &str) -> Result<(), AppError> {
    if name == DEFAULT_PROFILE {
        return Err(AppError::validation("The default profile cannot be removed"));
    }
    let mut t = read_table(path)?;
    let removed = t.get_mut("profiles").and_then(|p| p.as_table_mut()).and_then(|p| p.remove(name)).is_some();
    if !removed {
        return Err(AppError::validation(format!("There is no profile named \"{name}\"")));
    }
    if str_of(&t, "current_profile").as_deref() == Some(name) {
        t.remove("current_profile");
    }
    write_table(path, &t)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "api_url = \"https://prod\"\ntoken = \"tprod\"\nserver_url = \"http://main:8200\"\nserver_token = \"s3cret-main\"\ncurrent_profile = \"lab\"\n\n[[connections]]\nname = \"nas\"\nuri = \"sftp://nas/x\"\nprotocol = \"sftp\"\ncreated_at = \"2026-01-01\"\n\n[profiles.lab]\nserver_url = \"http://lab:8200\"\nserver_token = \"s3cret-lab\"\ndefault_format = \"json\"\n\n[profiles.staging]\napi_url = \"https://staging\"\n";

    fn file(dir: &tempfile::TempDir) -> PathBuf {
        let p = dir.path().join("ufop").join("config.toml");
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, FILE).unwrap();
        p
    }

    #[test]
    fn lists_profiles_without_their_tokens() {
        let d = tempfile::tempdir().unwrap();
        let rows = list(&file(&d)).unwrap();
        assert_eq!(
            rows,
            vec![
                ProfileSummary { name: "default".into(), server_url: Some("http://main:8200".into()), signed_in: true, current: false },
                ProfileSummary { name: "lab".into(), server_url: Some("http://lab:8200".into()), signed_in: true, current: true },
                ProfileSummary { name: "staging".into(), server_url: None, signed_in: false, current: false },
            ]
        );
        assert!(!serde_json::to_string(&rows).unwrap().contains("s3cret"), "tokens never leave the file");
    }

    #[test]
    fn a_missing_or_empty_file_has_no_profiles() {
        let d = tempfile::tempdir().unwrap();
        assert!(list(&d.path().join("none.toml")).unwrap().is_empty());
        let p = d.path().join("empty.toml");
        std::fs::write(&p, "").unwrap();
        assert!(list(&p).unwrap().is_empty());
        std::fs::write(&p, "this is = = not toml").unwrap();
        assert!(list(&p).unwrap_err().user_message().contains("not valid TOML"));
    }

    #[test]
    fn saving_a_server_keeps_everything_else_in_the_file() {
        let d = tempfile::tempdir().unwrap();
        let p = file(&d);
        save_server_url(&p, "office", "https://files.example.com:8200/").unwrap();
        save_server_url(&p, "lab", "http://lab2:8200").unwrap();
        save_server_url(&p, "default", "http://main2:8200").unwrap();

        let t: toml::Table = std::fs::read_to_string(&p).unwrap().parse().unwrap();
        assert_eq!(t["profiles"]["office"]["server_url"].as_str(), Some("https://files.example.com:8200"), "trailing slash dropped");
        assert_eq!(t["profiles"]["lab"]["server_url"].as_str(), Some("http://lab2:8200"));
        assert_eq!(t["profiles"]["lab"]["server_token"].as_str(), Some("s3cret-lab"), "a change of address keeps the tokens");
        assert_eq!(t["profiles"]["lab"]["default_format"].as_str(), Some("json"));
        assert_eq!(t["server_url"].as_str(), Some("http://main2:8200"));
        assert_eq!(t["server_token"].as_str(), Some("s3cret-main"));
        assert_eq!(t["token"].as_str(), Some("tprod"));
        assert_eq!(t["connections"][0]["name"].as_str(), Some("nas"), "saved connections survive");
        assert_eq!(t["profiles"]["staging"]["api_url"].as_str(), Some("https://staging"));
        // The CLI's own loader still reads it.
        let _: Profile = t["profiles"]["lab"].clone().try_into().unwrap();
    }

    #[test]
    fn creates_the_file_and_its_folder_when_there_is_none() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("new").join("ufop").join("config.toml");
        save_server_url(&p, "first", "http://127.0.0.1:8200").unwrap();
        assert_eq!(list(&p).unwrap()[0].server_url.as_deref(), Some("http://127.0.0.1:8200"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600, "it may hold tokens");
        }
    }

    #[test]
    fn removing_a_profile_drops_its_tokens_and_the_current_marker() {
        let d = tempfile::tempdir().unwrap();
        let p = file(&d);
        remove(&p, "lab").unwrap();
        let t: toml::Table = std::fs::read_to_string(&p).unwrap().parse().unwrap();
        assert!(t["profiles"].as_table().unwrap().get("lab").is_none());
        assert!(t.get("current_profile").is_none(), "the CLI falls back to the default profile");
        assert!(!std::fs::read_to_string(&p).unwrap().contains("s3cret-lab"));
        assert!(remove(&p, "lab").unwrap_err().user_message().contains("no profile"));
        assert!(remove(&p, "default").is_err());
    }

    #[test]
    fn bad_names_and_addresses_are_refused_with_a_reason() {
        let d = tempfile::tempdir().unwrap();
        let p = file(&d);
        for name in ["", "a b", "a/b", "x".repeat(65).as_str()] {
            assert!(save_server_url(&p, name, "http://h").is_err(), "{name:?}");
        }
        for url in ["", "ftp://h", "files.example.com", "http://", "http://u:p@h", "http://h /x", "https://"] {
            assert!(save_server_url(&p, "ok", url).is_err(), "{url:?}");
        }
        assert_eq!(std::fs::read_to_string(&p).unwrap(), FILE, "a refused change leaves the file alone");
        assert_eq!(clean_server_url(" https://h:1/api/ ").unwrap(), "https://h:1/api");
    }

    #[test]
    fn the_config_path_follows_ufop_config_then_the_platform_folder() {
        let dir = Some(PathBuf::from("/home/me/.config"));
        // Tests share the process environment: only check the fallback when the variable is unset.
        if std::env::var_os("UFOP_CONFIG").is_none() {
            assert_eq!(config_path(dir.clone()), PathBuf::from("/home/me/.config/ufop/config.toml"));
            assert_eq!(config_path(None), PathBuf::from("./ufop/config.toml"));
        }
    }
}
