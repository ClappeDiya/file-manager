//! Which rclone to run, with which config file.
//!
//! Every rclone caller (the rclone connector, the backup store, cloud
//! mounts, the remote manager, diagnostics) goes through [`Rclone`], so the
//! binary is looked up in one place. Resolution order:
//!
//! 1. the binary chosen in Settings (`rclone_settings.binary_path`, which is
//!    also where a managed install points);
//! 2. the `UFOP_RCLONE` environment variable (a path, or a name looked up in
//!    `PATH`);
//! 3. `rclone` on `PATH`.
//!
//! The rclone.conf chosen in Settings (`config_path`) is passed to every call
//! as `--config`; without one rclone uses its own default (`RCLONE_CONFIG` or
//! `~/.config/rclone/rclone.conf`).

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{OnceLock, RwLock};
use std::time::Duration;

/// Environment variable naming the rclone binary (path or command name).
pub const ENV_BINARY: &str = "UFOP_RCLONE";
/// Config key the settings are stored under.
pub const CONFIG_KEY: &str = "rclone_settings";
/// Oldest rclone the app is written against: the non-interactive
/// configuration state machine (`config create --non-interactive`,
/// `--continue --state --result`) arrived in 1.56.
pub const MIN_VERSION: Semver = Semver { major: 1, minor: 56, patch: 0 };

const VERSION_TIMEOUT: Duration = Duration::from_secs(15);

/// rclone settings (Settings → rclone).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RcloneSettings {
    /// rclone binary; empty/`None` = `UFOP_RCLONE`, then `PATH`.
    pub binary_path: Option<String>,
    /// rclone.conf passed as `--config`; empty/`None` = rclone's default.
    pub config_path: Option<String>,
}

impl RcloneSettings {
    fn binary(&self) -> Option<&str> {
        self.binary_path.as_deref().map(str::trim).filter(|s| !s.is_empty())
    }
    fn config(&self) -> Option<&str> {
        self.config_path.as_deref().map(str::trim).filter(|s| !s.is_empty())
    }
}

/// Where the binary came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BinarySource {
    Settings,
    Env,
    Path,
    /// Nothing found: plain `rclone` is tried (and fails to start).
    None,
}

/// The binary and config file every call uses.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Resolved {
    pub path: String,
    pub source: BinarySource,
    /// The binary exists (an executable file).
    pub found: bool,
    pub config: Option<String>,
}

fn active() -> &'static RwLock<RcloneSettings> {
    static A: OnceLock<RwLock<RcloneSettings>> = OnceLock::new();
    A.get_or_init(|| RwLock::new(RcloneSettings::default()))
}

/// Make `settings` the ones every rclone call uses.
pub fn apply(settings: RcloneSettings) {
    *active().write().unwrap_or_else(|e| e.into_inner()) = settings;
}

/// The settings in force.
pub fn current_settings() -> RcloneSettings {
    active().read().unwrap_or_else(|e| e.into_inner()).clone()
}

pub fn is_executable(p: &Path) -> bool {
    let Ok(m) = std::fs::metadata(p) else { return false };
    if !m.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        m.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// `name` looked up in `PATH` (with `.exe` on Windows).
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let names: Vec<String> = if cfg!(windows) && !name.to_ascii_lowercase().ends_with(".exe") { vec![format!("{name}.exe"), name.to_string()] } else { vec![name.to_string()] };
    std::env::split_paths(&path).flat_map(|d| names.iter().map(move |n| d.join(n))).find(|p| is_executable(p))
}

fn has_separator(s: &str) -> bool {
    s.contains('/') || s.contains('\\')
}

/// Resolve with explicit inputs (`which` looks a command name up in PATH).
pub fn resolve_with(settings: &RcloneSettings, env: Option<&str>, which: impl Fn(&str) -> Option<PathBuf>) -> Resolved {
    let config = settings.config().map(str::to_string);
    if let Some(b) = settings.binary() {
        return Resolved { path: b.to_string(), source: BinarySource::Settings, found: is_executable(Path::new(b)), config };
    }
    if let Some(e) = env.map(str::trim).filter(|s| !s.is_empty()) {
        if has_separator(e) {
            return Resolved { path: e.to_string(), source: BinarySource::Env, found: is_executable(Path::new(e)), config };
        }
        return match which(e) {
            Some(p) => Resolved { path: p.to_string_lossy().into_owned(), source: BinarySource::Env, found: true, config },
            None => Resolved { path: e.to_string(), source: BinarySource::Env, found: false, config },
        };
    }
    match which("rclone") {
        Some(p) => Resolved { path: p.to_string_lossy().into_owned(), source: BinarySource::Path, found: true, config },
        None => Resolved { path: "rclone".into(), source: BinarySource::None, found: false, config },
    }
}

/// The binary and config every call uses now.
pub fn resolve() -> Resolved {
    resolve_with(&current_settings(), std::env::var(ENV_BINARY).ok().as_deref(), which)
}

/// `major.minor.patch` of an rclone release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Semver {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl std::fmt::Display for Semver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl Semver {
    /// `1.60.1`, `v1.60.1`, `v1.60.1-DEV`, `1.66.0-beta.7890.abc`.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim().trim_start_matches('v');
        let core = s.split(['-', '+', ' ']).next()?;
        let mut it = core.split('.');
        let major = it.next()?.parse().ok()?;
        let minor = it.next()?.parse().ok()?;
        let patch = it.next().map(|p| p.parse()).transpose().ok()?.unwrap_or(0);
        if it.next().is_some() {
            return None;
        }
        Some(Self { major, minor, patch })
    }
}

/// What `rclone version` reported.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RcloneVersion {
    /// As printed (`v1.60.1-DEV`).
    pub raw: String,
    pub semver: Semver,
}

/// The first line of `rclone version`: `rclone v1.60.1-DEV`.
pub fn parse_version(out: &str) -> Option<RcloneVersion> {
    let line = out.lines().map(str::trim).find(|l| !l.is_empty())?;
    let mut words = line.split_whitespace();
    if !words.next()?.eq_ignore_ascii_case("rclone") {
        return None;
    }
    let raw = words.next()?.to_string();
    Some(RcloneVersion { semver: Semver::parse(&raw)?, raw })
}

fn not_started(e: &std::io::Error) -> AppError {
    AppError::connection(
        format!("Could not start rclone: {e}"),
        "Install rclone (https://rclone.org/install/), choose its path in Settings → rclone, or set UFOP_RCLONE to its path.",
    )
}

/// The last meaningful stderr line, without rclone's log prefix.
pub fn last_error_line(stderr: &str) -> String {
    let line = stderr.lines().rfind(|l| !l.trim().is_empty()).unwrap_or("unknown error");
    let msg = line.split_once(" : ").map(|(_, m)| m).unwrap_or(line).trim();
    // "2026/10/24 13:07:39 Failed to …": drop the timestamp.
    let b = msg.as_bytes();
    let stamped = b.len() > 20 && b[4] == b'/' && b[7] == b'/' && b[10] == b' ' && b[13] == b':' && b[16] == b':' && b[19] == b' ';
    let msg = if stamped { &msg[20..] } else { msg };
    msg.strip_prefix("NOTICE: ").unwrap_or(msg).trim().to_string()
}

/// An rclone binary plus the config file it uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rclone {
    pub program: String,
    pub config: Option<String>,
}

impl Rclone {
    /// The resolved binary and Settings config file.
    pub fn current() -> Self {
        let r = resolve();
        Self { program: r.path, config: r.config }
    }

    pub fn new(program: impl Into<String>, config: Option<String>) -> Self {
        Self { program: program.into(), config: config.filter(|c| !c.trim().is_empty()) }
    }

    /// The same config file with another binary (tests, per-job binaries).
    pub fn with_program(&self, program: &str) -> Self {
        Self { program: program.to_string(), config: self.config.clone() }
    }

    /// Flags every call starts with (`--config <file>`).
    pub fn base_args(&self) -> Vec<String> {
        match &self.config {
            Some(c) => vec!["--config".into(), c.clone()],
            None => Vec::new(),
        }
    }

    /// `rclone [--config file]`, stdin closed, killed when dropped.
    pub fn command(&self) -> tokio::process::Command {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.args(self.base_args()).stdin(std::process::Stdio::null()).kill_on_drop(true);
        cmd
    }

    /// Run with `args`; stdout on success, the last stderr line otherwise.
    pub async fn run(&self, args: &[String], timeout: Duration) -> Result<String, AppError> {
        let mut cmd = self.command();
        cmd.args(args);
        let out = tokio::time::timeout(timeout, cmd.output())
            .await
            .map_err(|_| AppError::connection("rclone timed out", "Check the remote is reachable."))?
            .map_err(|e| not_started(&e))?;
        if !out.status.success() {
            let msg = last_error_line(&String::from_utf8_lossy(&out.stderr));
            return Err(AppError::connection(format!("rclone: {msg}"), "Check the remote with `rclone config` and try again."));
        }
        Ok(String::from_utf8_lossy(&out.stdout).into_owned())
    }

    /// `rclone version`.
    pub async fn version(&self) -> Result<RcloneVersion, AppError> {
        let mut cmd = tokio::process::Command::new(&self.program);
        cmd.arg("version").stdin(std::process::Stdio::null()).kill_on_drop(true);
        let out = tokio::time::timeout(VERSION_TIMEOUT, cmd.output())
            .await
            .map_err(|_| AppError::connection("rclone version timed out", "Check the rclone binary in Settings → rclone."))?
            .map_err(|e| not_started(&e))?;
        let text = String::from_utf8_lossy(&out.stdout);
        parse_version(&text).ok_or_else(|| {
            AppError::connection(
                format!("{} did not report an rclone version", self.program),
                "Choose the rclone executable in Settings → rclone.",
            )
        })
    }

    /// `rclone version` without an async runtime (diagnostics).
    pub fn version_blocking(&self) -> Result<RcloneVersion, String> {
        let mut child = std::process::Command::new(&self.program)
            .arg("version")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Could not start rclone: {e}"))?;
        let deadline = std::time::Instant::now() + VERSION_TIMEOUT;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err("rclone version timed out".into());
                }
            }
        }
        let mut out = String::new();
        if let Some(mut o) = child.stdout.take() {
            use std::io::Read;
            let _ = o.read_to_string(&mut out);
        }
        parse_version(&out).ok_or_else(|| format!("{} did not report an rclone version", self.program))
    }
}

/// What Settings, About and diagnostics show about rclone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RcloneStatus {
    pub path: String,
    pub source: BinarySource,
    pub found: bool,
    /// As printed (`v1.60.1`), when rclone ran.
    pub version: Option<String>,
    /// Whether the version is at least [`MIN_VERSION`]; `None` = unknown.
    pub meets_minimum: Option<bool>,
    pub min_version: String,
    /// `--config` from Settings (`None` = rclone's default).
    pub config_path: Option<String>,
    /// The binary is the app's managed copy.
    pub managed: bool,
    /// Why the version could not be read.
    pub error: Option<String>,
}

/// Status from a resolution and the outcome of `rclone version`.
pub fn status_of(r: &Resolved, version: Result<RcloneVersion, String>) -> RcloneStatus {
    let managed = super::install::is_managed_path(Path::new(&r.path), &super::install::managed_dir());
    let (version, error) = match version {
        Ok(v) => (Some(v), None),
        Err(e) => (None, Some(if r.found || r.source != BinarySource::None { e } else { "rclone is not installed".to_string() })),
    };
    RcloneStatus {
        path: r.path.clone(),
        source: r.source,
        found: r.found || version.is_some(),
        meets_minimum: version.as_ref().map(|v| v.semver >= MIN_VERSION),
        version: version.map(|v| v.raw),
        min_version: MIN_VERSION.to_string(),
        config_path: r.config.clone(),
        managed,
        error,
    }
}

/// Current status (runs `rclone version`).
pub async fn status() -> RcloneStatus {
    let r = resolve();
    let v = Rclone::new(r.path.clone(), r.config.clone()).version().await.map_err(|e| e.user_message());
    status_of(&r, v)
}

/// Current status from synchronous code (diagnostics).
pub fn status_blocking() -> RcloneStatus {
    let r = resolve();
    let v = Rclone::new(r.path.clone(), r.config.clone()).version_blocking();
    status_of(&r, v)
}

/// A binary "Detect" found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Candidate {
    pub path: String,
    /// `env`, `path`, `managed` or `common` (a usual install folder).
    pub origin: String,
    pub version: Option<String>,
    pub meets_minimum: Option<bool>,
    pub error: Option<String>,
}

/// Usual install locations, besides `PATH`.
fn common_locations() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if cfg!(windows) {
        for var in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
            if let Ok(d) = std::env::var(var) {
                v.push(Path::new(&d).join("rclone").join("rclone.exe"));
            }
        }
        if let Some(h) = dirs::home_dir() {
            v.push(h.join("scoop").join("shims").join("rclone.exe"));
        }
    } else {
        v.extend(["/usr/local/bin/rclone", "/usr/bin/rclone", "/opt/homebrew/bin/rclone", "/snap/bin/rclone"].map(PathBuf::from));
        if let Some(h) = dirs::home_dir() {
            v.push(h.join("bin").join("rclone"));
            v.push(h.join(".local").join("bin").join("rclone"));
        }
    }
    v
}

/// Every rclone binary on this machine worth offering, with its version.
pub async fn detect() -> Vec<Candidate> {
    let mut found: Vec<(PathBuf, &str)> = Vec::new();
    if let Ok(e) = std::env::var(ENV_BINARY) {
        let e = e.trim().to_string();
        if !e.is_empty() {
            let p = if has_separator(&e) { Some(PathBuf::from(&e)) } else { which(&e) };
            if let Some(p) = p.filter(|p| is_executable(p)) {
                found.push((p, "env"));
            }
        }
    }
    if let Some(p) = which("rclone") {
        found.push((p, "path"));
    }
    let managed = super::install::managed_binary(&super::install::managed_dir());
    if is_executable(&managed) {
        found.push((managed, "managed"));
    }
    for p in common_locations() {
        if is_executable(&p) {
            found.push((p, "common"));
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for (p, origin) in found {
        let key = std::fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
        if !seen.insert(key) {
            continue;
        }
        let path = p.to_string_lossy().into_owned();
        let v = Rclone::new(path.clone(), None).version().await;
        out.push(match v {
            Ok(v) => Candidate { path, origin: origin.into(), meets_minimum: Some(v.semver >= MIN_VERSION), version: Some(v.raw), error: None },
            Err(e) => Candidate { path, origin: origin.into(), version: None, meets_minimum: None, error: Some(e.user_message()) },
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    fn exe_file(dir: &Path, name: &str, script: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, script).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    #[test]
    fn resolver_precedence_settings_then_env_then_path() {
        let d = tempfile::tempdir().unwrap();
        let mine = exe_file(d.path(), "my-rclone", "#!/bin/sh\n");
        let envb = exe_file(d.path(), "env-rclone", "#!/bin/sh\n");
        let on_path = d.path().join("path-rclone");
        let which_none = |_: &str| None;
        let which_path = |n: &str| (n == "rclone").then(|| on_path.clone());
        let settings = RcloneSettings { binary_path: Some(mine.to_string_lossy().into()), config_path: Some(" /tmp/r.conf ".into()) };

        // 1. Settings beats the environment and PATH.
        let r = resolve_with(&settings, Some(envb.to_str().unwrap()), which_path);
        assert_eq!((r.path.as_str(), r.source, r.found), (mine.to_str().unwrap(), BinarySource::Settings, true));
        assert_eq!(r.config.as_deref(), Some("/tmp/r.conf"));
        // 2. No Settings path: the environment.
        let none = RcloneSettings::default();
        let r = resolve_with(&none, Some(envb.to_str().unwrap()), which_path);
        assert_eq!((r.path.as_str(), r.source, r.found), (envb.to_str().unwrap(), BinarySource::Env, true));
        assert_eq!(r.config, None);
        // A bare name in UFOP_RCLONE is looked up in PATH.
        let r = resolve_with(&none, Some("rclone"), which_path);
        assert_eq!((r.path.as_str(), r.source), (on_path.to_str().unwrap(), BinarySource::Env));
        // 3. Neither: PATH.
        let r = resolve_with(&RcloneSettings { binary_path: Some("  ".into()), config_path: None }, Some(""), which_path);
        assert_eq!((r.path.as_str(), r.source, r.found), (on_path.to_str().unwrap(), BinarySource::Path, true));
        // Nothing anywhere: plain "rclone", not found.
        let r = resolve_with(&none, None, which_none);
        assert_eq!((r.path.as_str(), r.source, r.found), ("rclone", BinarySource::None, false));
        // A Settings path that does not exist is still used (and reported missing).
        let gone = RcloneSettings { binary_path: Some("/no/such/rclone".into()), config_path: None };
        let r = resolve_with(&gone, Some(envb.to_str().unwrap()), which_path);
        assert_eq!((r.source, r.found), (BinarySource::Settings, false));
        // A folder or non-executable file is not a binary.
        let plain = d.path().join("plain");
        std::fs::write(&plain, "x").unwrap();
        assert!(!is_executable(&plain) && !is_executable(d.path()));
    }

    #[test]
    fn rclone_commands_carry_the_config_flag() {
        let r = Rclone::new("rclone", Some("/c/rclone.conf".into()));
        assert_eq!(r.base_args(), vec!["--config", "/c/rclone.conf"]);
        assert!(Rclone::new("rclone", Some(" ".into())).base_args().is_empty());
        assert_eq!(r.with_program("/x/rclone"), Rclone::new("/x/rclone", Some("/c/rclone.conf".into())));
    }

    #[test]
    fn parses_versions() {
        let v = parse_version("rclone v1.60.1-DEV\n- os/version: ubuntu 24.04 (64 bit)\n").unwrap();
        assert_eq!((v.raw.as_str(), v.semver), ("v1.60.1-DEV", Semver { major: 1, minor: 60, patch: 1 }));
        assert_eq!(parse_version("\nrclone v1.69.1\n").unwrap().semver.to_string(), "1.69.1");
        assert_eq!(parse_version("rclone 1.53").unwrap().semver, Semver { major: 1, minor: 53, patch: 0 });
        assert_eq!(parse_version("rclone v1.66.0-beta.7890.abc123").unwrap().semver, Semver { major: 1, minor: 66, patch: 0 });
        assert!(parse_version("").is_none());
        assert!(parse_version("restic 0.16.0").is_none());
        assert!(parse_version("rclone vX.Y").is_none());
        assert!(Semver::parse("1.2.3.4").is_none());
        assert!(Semver::parse("1.55.9").unwrap() < MIN_VERSION && Semver::parse("v1.56.0").unwrap() >= MIN_VERSION);
        assert!(Semver::parse("1.100.0").unwrap() > Semver::parse("1.99.9").unwrap());
    }

    #[test]
    fn status_reports_minimum_and_missing() {
        let r = Resolved { path: "rclone".into(), source: BinarySource::None, found: false, config: None };
        let s = status_of(&r, Err("Could not start rclone: No such file".into()));
        assert_eq!((s.found, s.version.as_deref(), s.error.as_deref()), (false, None, Some("rclone is not installed")));
        assert_eq!(s.min_version, "1.56.0");
        let old = RcloneVersion { raw: "v1.50.2".into(), semver: Semver { major: 1, minor: 50, patch: 2 } };
        let s = status_of(&Resolved { found: true, source: BinarySource::Path, ..r.clone() }, Ok(old));
        assert_eq!((s.found, s.meets_minimum, s.version.as_deref()), (true, Some(false), Some("v1.50.2")));
        // An explicit binary that fails keeps its own error.
        let s = status_of(&Resolved { source: BinarySource::Settings, ..r }, Err("Could not start rclone: denied".into()));
        assert_eq!(s.error.as_deref(), Some("Could not start rclone: denied"));
    }

    #[test]
    fn error_lines_lose_the_log_prefix() {
        assert_eq!(last_error_line("2026/10/24 13:07:39 NOTICE: x\n2026/10/24 13:07:39 ERROR : remote: directory not found\n"), "remote: directory not found");
        assert_eq!(last_error_line("2026/10/24 13:07:39 Failed to create file system: didn't find section in config file\n"), "Failed to create file system: didn't find section in config file");
        assert_eq!(last_error_line(""), "unknown error");
    }

    /// Fake binaries: version parsing, a binary that prints nothing, a missing one.
    #[cfg(unix)]
    #[tokio::test]
    async fn version_of_real_and_fake_binaries() {
        let d = tempfile::tempdir().unwrap();
        let good = exe_file(d.path(), "rclone", "#!/bin/sh\necho 'rclone v1.62.2'\necho '- os/type: linux'\n");
        let bad = exe_file(d.path(), "other", "#!/bin/sh\necho hello\n");
        assert_eq!(Rclone::new(good.to_str().unwrap(), None).version().await.unwrap().raw, "v1.62.2");
        assert_eq!(Rclone::new(good.to_str().unwrap(), None).version_blocking().unwrap().semver.minor, 62);
        assert!(Rclone::new(bad.to_str().unwrap(), None).version().await.unwrap_err().user_message().contains("did not report"));
        assert!(Rclone::new("/no/such/rclone", None).version().await.unwrap_err().user_message().contains("Could not start rclone"));
        assert!(Rclone::new("/no/such/rclone", None).version_blocking().is_err());
        if which("rclone").is_some() {
            let v = Rclone::new("rclone", None).version().await.unwrap();
            assert!(v.semver >= Semver { major: 1, minor: 0, patch: 0 });
        }
    }
}
