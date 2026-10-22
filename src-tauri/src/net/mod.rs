//! Outbound HTTP for the whole app: one place that knows the proxy.
//!
//! Every client is built with [`builder`] / [`client`], which install a
//! per-request proxy callback reading the *current* settings, so changing
//! the proxy (Settings, or a fleet policy) applies to clients built long
//! before. Modes:
//! - `system`: `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` / `NO_PROXY`, then
//!   the OS settings (Windows Internet Settings, macOS `scutil --proxy`),
//!   including an OS-configured PAC file;
//! - `none`: always direct;
//! - `manual`: one proxy URL (http, https or socks5) with a bypass list;
//! - `pac`: a PAC file (URL or path), evaluated with QuickJS.

pub mod pac;

use serde::{Deserialize, Serialize};
use std::sync::{Arc, OnceLock, RwLock};
use std::time::Duration;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    System,
    None,
    Manual,
    Pac,
}

/// Stored proxy settings (the password lives in the credential store).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProxySettings {
    pub mode: ProxyMode,
    /// `manual`: `http://host:port`, `https://…` or `socks5://…`.
    pub url: String,
    /// `pac`: `https://…/proxy.pac` or a local path.
    pub pac_url: String,
    /// Hosts that go direct: `example.com`, `.corp.example`, `10.*`, `localhost`.
    pub no_proxy: Vec<String>,
    pub username: String,
    /// Set by the fleet console; Settings shows it read-only.
    pub managed: bool,
}

/// The settings in force plus what they resolve to.
struct Active {
    settings: ProxySettings,
    password: Option<String>,
    /// System mode: the OS proxy, read when settings were applied.
    os: OsProxy,
    pac: Option<Arc<pac::PacScript>>,
}

/// What the OS is configured with (empty = direct).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsProxy {
    pub http: Option<String>,
    pub https: Option<String>,
    pub bypass: Vec<String>,
    pub pac_url: Option<String>,
}

fn active() -> &'static RwLock<Active> {
    static A: OnceLock<RwLock<Active>> = OnceLock::new();
    A.get_or_init(|| RwLock::new(Active { settings: ProxySettings::default(), password: None, os: OsProxy::default(), pac: None }))
}

/// Make `settings` the proxy for every client. Fetches a PAC file and
/// reads OS settings as needed; errors leave the previous settings.
pub fn apply(settings: ProxySettings, password: Option<String>) -> Result<(), String> {
    let os = if settings.mode == ProxyMode::System { os_proxy() } else { OsProxy::default() };
    let pac_source = match settings.mode {
        ProxyMode::Pac => Some(settings.pac_url.trim().to_string()),
        ProxyMode::System if env_proxy("https").is_none() && env_proxy("http").is_none() => os.pac_url.clone(),
        _ => None,
    };
    let pac = match pac_source.filter(|s| !s.is_empty()) {
        Some(src) => Some(Arc::new(pac::PacScript::load(&src)?)),
        None if settings.mode == ProxyMode::Pac => return Err("Enter the PAC file URL or path".into()),
        None => None,
    };
    if settings.mode == ProxyMode::Manual {
        reqwest::Url::parse(settings.url.trim()).map_err(|e| format!("Invalid proxy URL: {e}"))?;
    }
    *active().write().unwrap_or_else(|e| e.into_inner()) = Active { settings, password, os, pac };
    Ok(())
}

pub fn current() -> ProxySettings {
    active().read().unwrap_or_else(|e| e.into_inner()).settings.clone()
}

/// Proxy for a request to `url`, `None` = direct.
pub fn proxy_for(url: &reqwest::Url) -> Option<reqwest::Url> {
    let host = url.host_str().unwrap_or_default();
    if always_direct(host) {
        return None;
    }
    let a = active().read().unwrap_or_else(|e| e.into_inner());
    let raw = match a.settings.mode {
        ProxyMode::None => None,
        ProxyMode::Manual => (!bypassed(host, &a.settings.no_proxy)).then(|| a.settings.url.trim().to_string()),
        ProxyMode::Pac => a.pac.as_ref().and_then(|p| p.find(url.as_str(), host)),
        ProxyMode::System => {
            let scheme = url.scheme();
            let env_no = std::env::var("NO_PROXY").or_else(|_| std::env::var("no_proxy")).unwrap_or_default();
            let env_bypass: Vec<String> = env_no.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
            if let Some(p) = env_proxy(scheme) {
                (!bypassed(host, &env_bypass)).then_some(p)
            } else if let Some(pac) = &a.pac {
                pac.find(url.as_str(), host)
            } else {
                let os = if scheme == "https" { a.os.https.clone().or(a.os.http.clone()) } else { a.os.http.clone() };
                os.filter(|_| !bypassed(host, &a.os.bypass))
            }
        }
    }?;
    let with_scheme = if raw.contains("://") { raw } else { format!("http://{raw}") };
    let mut u = reqwest::Url::parse(&with_scheme).ok()?;
    if !a.settings.username.is_empty() && u.username().is_empty() {
        let _ = u.set_username(&a.settings.username);
        let _ = u.set_password(a.password.as_deref());
    }
    Some(u)
}

/// Loopback and link-local (e.g. the cloud metadata service at
/// 169.254.169.254) never go through a proxy.
pub fn always_direct(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']);
    if h.eq_ignore_ascii_case("localhost") {
        return true;
    }
    match h.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(v4)) => v4.is_loopback() || v4.is_link_local(),
        Ok(std::net::IpAddr::V6(v6)) => v6.is_loopback() || (v6.segments()[0] & 0xffc0) == 0xfe80,
        Err(_) => false,
    }
}

fn env_proxy(scheme: &str) -> Option<String> {
    let names: &[&str] = if scheme == "https" { &["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"] } else { &["HTTP_PROXY", "http_proxy", "ALL_PROXY", "all_proxy"] };
    names.iter().find_map(|n| std::env::var(n).ok()).filter(|v| !v.trim().is_empty())
}

/// Whether `host` matches a bypass entry (`*`, `example.com` also matches
/// subdomains, `.example.com`, `*.example.com`, `10.*`, `<local>`).
pub fn bypassed(host: &str, list: &[String]) -> bool {
    let host = host.trim_matches(['[', ']']).to_ascii_lowercase();
    list.iter().any(|raw| {
        let e = raw.trim().to_ascii_lowercase();
        match e.as_str() {
            "" => false,
            "*" => true,
            "<local>" => !host.contains('.'),
            _ if e.contains('*') => wildcard(&e, &host),
            _ => {
                let d = e.trim_start_matches('.');
                host == d || host.ends_with(&format!(".{d}"))
            }
        }
    })
}

fn wildcard(pattern: &str, s: &str) -> bool {
    let re = format!("^{}$", regex::escape(pattern).replace(r"\*", ".*"));
    regex::Regex::new(&re).is_ok_and(|r| r.is_match(s))
}

/// The OS proxy configuration (best effort; empty when unknown).
pub fn os_proxy() -> OsProxy {
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("scutil").arg("--proxy").output();
        return out.map(|o| parse_scutil(&String::from_utf8_lossy(&o.stdout))).unwrap_or_default();
    }
    #[cfg(windows)]
    {
        return windows_proxy();
    }
    #[allow(unreachable_code)]
    OsProxy::default()
}

/// Parse `scutil --proxy` output.
pub fn parse_scutil(text: &str) -> OsProxy {
    let mut kv = std::collections::HashMap::new();
    let mut bypass = Vec::new();
    let mut in_exceptions = false;
    for line in text.lines() {
        let l = line.trim();
        if l.starts_with("ExceptionsList") {
            in_exceptions = true;
            continue;
        }
        if in_exceptions {
            if l == "}" {
                in_exceptions = false;
            } else if let Some((_, v)) = l.split_once(" : ") {
                bypass.push(v.trim().to_string());
            }
            continue;
        }
        if let Some((k, v)) = l.split_once(" : ") {
            kv.insert(k.trim().to_string(), v.trim().to_string());
        }
    }
    let on = |k: &str| kv.get(k).is_some_and(|v| v == "1");
    let hp = |h: &str, p: &str| match (kv.get(h), kv.get(p)) {
        (Some(h), Some(p)) => Some(format!("http://{h}:{p}")),
        _ => None,
    };
    OsProxy {
        http: on("HTTPEnable").then(|| hp("HTTPProxy", "HTTPPort")).flatten(),
        https: on("HTTPSEnable").then(|| hp("HTTPSProxy", "HTTPSPort")).flatten(),
        bypass,
        pac_url: on("ProxyAutoConfigEnable").then(|| kv.get("ProxyAutoConfigURLString").cloned()).flatten(),
    }
}

/// Parse Windows `ProxyServer` (`host:port` or `http=h:p;https=h:p`) and
/// `ProxyOverride` (`;`-separated, `<local>`).
pub fn parse_windows(enabled: bool, server: &str, overrides: &str, auto_config: &str) -> OsProxy {
    let mut out = OsProxy { pac_url: Some(auto_config.trim().to_string()).filter(|s| !s.is_empty()), ..Default::default() };
    if enabled && !server.trim().is_empty() {
        if server.contains('=') {
            for part in server.split(';') {
                if let Some((k, v)) = part.split_once('=') {
                    let v = format!("http://{}", v.trim());
                    match k.trim() {
                        "http" => out.http = Some(v),
                        "https" => out.https = Some(v),
                        _ => {}
                    }
                }
            }
        } else {
            let v = format!("http://{}", server.trim());
            out.http = Some(v.clone());
            out.https = Some(v);
        }
        out.bypass = overrides.split(';').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    out
}

#[cfg(windows)]
fn windows_proxy() -> OsProxy {
    let reg = |name: &str| -> String {
        std::process::Command::new("reg")
            .args(["query", r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings", "/v", name])
            .output()
            .ok()
            .and_then(|o| String::from_utf8_lossy(&o.stdout).lines().find(|l| l.trim_start().starts_with(name)).map(|l| l.split_whitespace().skip(2).collect::<Vec<_>>().join(" ")))
            .unwrap_or_default()
    };
    parse_windows(reg("ProxyEnable").trim().ends_with('1'), &reg("ProxyServer"), &reg("ProxyOverride"), &reg("AutoConfigURL"))
}

/// A client builder with the app's proxy, timeouts and user agent.
pub fn builder() -> reqwest::ClientBuilder {
    reqwest::Client::builder()
        .proxy(reqwest::Proxy::custom(|url| proxy_for(url)))
        .connect_timeout(Duration::from_secs(30))
        .user_agent(concat!("UFOP/", env!("CARGO_PKG_VERSION")))
}

/// A default client (see [`builder`]).
pub fn client() -> reqwest::Client {
    builder().build().unwrap_or_else(|e| {
        tracing::warn!("HTTP client with proxy failed ({e}); using defaults");
        reqwest::Client::new()
    })
}

#[cfg(test)]
pub(crate) mod tests;
