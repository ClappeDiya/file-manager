//! SMB/CIFS Connector (T-023).
//!
//! Provides SMB2/SMB3 connectivity to Windows shares and NAS devices.
//! Uses system `smbclient` / `mount.cifs` commands with a clean abstraction.
//!
//! Features:
//! - Connection: hostname/IP, share name, username, password, domain
//! - Network discovery of SMB shares on LAN
//! - Kerberos auth for domain environments
//! - SMB2/SMB3 preferred, SMB1 disabled by default
//! - NAS device discovery in sidebar

use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use crate::connectors::Connector;
use chrono::{TimeZone, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::process::Command;
use std::sync::Arc;
use tokio::sync::RwLock;

// ──────────────────────────────────────────────
// SMB Configuration
// ──────────────────────────────────────────────

/// SMB protocol version preference.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SmbVersion {
    /// SMB 2.0 (minimum recommended)
    Smb2,
    /// SMB 2.1
    Smb21,
    /// SMB 3.0
    #[default]
    Smb3,
    /// SMB 3.1.1 (latest, most secure)
    Smb311,
    /// SMB 1.0 (legacy, requires explicit opt-in)
    Smb1,
}

impl SmbVersion {
    pub fn as_mount_option(&self) -> &'static str {
        match self {
            Self::Smb2 => "2.0",
            Self::Smb21 => "2.1",
            Self::Smb3 => "3.0",
            Self::Smb311 => "3.1.1",
            Self::Smb1 => "1.0",
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Smb2 => "smb2",
            Self::Smb21 => "smb2.1",
            Self::Smb3 => "smb3",
            Self::Smb311 => "smb3.1.1",
            Self::Smb1 => "smb1",
        }
    }
}


/// SMB authentication method.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum SmbAuthMethod {
    /// Username/password authentication.
    #[default]
    Password,
    /// Kerberos authentication for domain environments.
    Kerberos,
    /// Guest/anonymous access (no credentials).
    Guest,
    /// NTLM authentication.
    Ntlm,
}


/// Extended SMB connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmbConfig {
    /// The SMB share name (e.g., "SharedDocs")
    pub share_name: String,
    /// Windows domain for domain authentication
    pub domain: Option<String>,
    /// SMB protocol version preference
    pub min_version: SmbVersion,
    /// Maximum SMB version to negotiate
    pub max_version: SmbVersion,
    /// Authentication method
    pub auth_method: SmbAuthMethod,
    /// Allow SMB1 (insecure, disabled by default)
    pub allow_smb1: bool,
    /// Enable encryption (SMB3+)
    pub encryption: bool,
    /// Enable signing
    pub signing: bool,
    /// Mount options (platform-specific)
    pub mount_options: HashMap<String, String>,
}

impl Default for SmbConfig {
    fn default() -> Self {
        Self {
            share_name: String::new(),
            domain: None,
            min_version: SmbVersion::Smb2,
            max_version: SmbVersion::Smb311,
            auth_method: SmbAuthMethod::default(),
            allow_smb1: false,
            encryption: true,
            signing: true,
            mount_options: HashMap::new(),
        }
    }
}

/// A discovered SMB share on the network.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredShare {
    /// Hostname or IP address
    pub host: String,
    /// Share name
    pub share_name: String,
    /// Share type (disk, printer, ipc)
    pub share_type: String,
    /// Optional comment/description
    pub comment: Option<String>,
    /// Whether this appears to be a NAS device
    pub is_nas: bool,
    /// Workgroup or domain
    pub workgroup: Option<String>,
}

/// Result of SMB network discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SmbDiscoveryResult {
    pub shares: Vec<DiscoveredShare>,
    pub scan_duration_ms: u64,
    pub hosts_scanned: u32,
    pub errors: Vec<String>,
}

// ──────────────────────────────────────────────
// SMB Connector
// ──────────────────────────────────────────────

/// Validate that a hostname is a valid IP address or RFC 1123 hostname.
fn validate_hostname(host: &str) -> Result<(), AppError> {
    // Accept valid IP addresses
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(());
    }
    // Validate hostname per RFC 1123
    let valid = host.len() <= 253
        && !host.is_empty()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
                && !label.starts_with('-')
                && !label.ends_with('-')
        });
    if !valid {
        return Err(AppError::connection(
            "Invalid hostname",
            "Use a valid hostname or IP address.",
        ));
    }
    Ok(())
}

/// Validate that a mount credential value does not contain characters
/// that could break or inject into mount option strings.
fn validate_mount_credential(value: &str, field_name: &str) -> Result<(), AppError> {
    if value.contains(',') || value.contains('=') {
        return Err(AppError::connection(
            format!("{field_name} contains invalid characters"),
            "Remove commas and equals signs from the value.",
        ));
    }
    Ok(())
}

/// SMB/CIFS connector using system mount commands.
pub struct SmbConnector {
    /// Current connection state
    state: Arc<RwLock<SmbConnectionState>>,
}

#[derive(Debug, Clone)]
#[derive(Default)]
struct SmbConnectionState {
    connected: bool,
    mount_point: Option<PathBuf>,
    host: String,
    share_name: String,
    config: SmbConfig,
}


impl SmbConnector {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(SmbConnectionState::default())),
        }
    }

    /// Create a connector with specific SMB configuration.
    pub fn with_config(config: SmbConfig) -> Self {
        let state = SmbConnectionState {
            config,
            ..Default::default()
        };
        Self {
            state: Arc::new(RwLock::new(state)),
        }
    }

    /// Validate that SMB1 is not being used unless explicitly allowed.
    fn validate_version(config: &SmbConfig) -> Result<(), AppError> {
        if config.min_version == SmbVersion::Smb1 && !config.allow_smb1 {
            return Err(AppError::Connection {
                message: "SMB1 is disabled for security reasons.".to_string(),
                advice: "Set allow_smb1 to true if you must connect to legacy devices, \
                         or upgrade the target to SMB2+."
                    .to_string(),
            });
        }
        Ok(())
    }

    /// Get the mount point path for a given host and share.
    fn mount_point_path(host: &str, share: &str) -> PathBuf {
        let base = ufop_core::session::user_temp_subdir("smb-mounts");
        let sanitized_host = host.replace(['/', '\\', ':', ' '], "_");
        let sanitized_share = share.replace(['/', '\\', ':', ' '], "_");
        base.join(format!("{sanitized_host}_{sanitized_share}"))
    }

    /// How to mount a share on `os` (pure, so every platform's plan is
    /// tested anywhere). The password never goes on a command line.
    pub(crate) fn plan(
        os: crate::cloud_folders::Os,
        host: &str,
        share: &str,
        username: Option<&str>,
        password: Option<&str>,
        config: &SmbConfig,
    ) -> MountPlan {
        use crate::cloud_folders::Os;
        let user_with_domain = |sep: &str| username.map(|u| config.domain.as_ref().map(|d| format!("{d}{sep}{u}")).unwrap_or_else(|| u.to_string()));
        match os {
            Os::MacOs => {
                let guest = matches!(config.auth_method, SmbAuthMethod::Guest);
                MountPlan::NetFs {
                    url: format!("smb://{host}/{}", urlencoded(share)),
                    user: if guest { None } else { user_with_domain(";") },
                    password: if guest { None } else { password.map(str::to_string) },
                    guest,
                }
            }
            Os::Windows => MountPlan::WNet { remote: format!("\\\\{host}\\{share}"), user: user_with_domain("\\"), password: password.map(str::to_string) },
            Os::Linux => {
                let mut opts = vec![format!("vers={}", config.max_version.as_mount_option())];
                opts.push(
                    match &config.auth_method {
                        SmbAuthMethod::Guest => "guest",
                        SmbAuthMethod::Kerberos => "sec=krb5",
                        SmbAuthMethod::Ntlm => "sec=ntlm",
                        SmbAuthMethod::Password => "sec=ntlmssp",
                    }
                    .to_string(),
                );
                if let Some(user) = username {
                    // Credential values are validated upstream via validate_mount_credential()
                    opts.push(format!("username={user}"));
                }
                if let Some(domain) = &config.domain {
                    opts.push(format!("domain={domain}"));
                }
                if config.encryption {
                    opts.push("seal".to_string());
                }
                for (k, v) in &config.mount_options {
                    opts.push(if v.is_empty() { k.clone() } else { format!("{k}={v}") });
                }
                MountPlan::Cifs { args: vec!["-t".into(), "cifs".into(), format!("//{host}/{share}"), String::new(), "-o".into(), opts.join(",")], password: password.map(str::to_string) }
            }
        }
    }

    /// Mount following `plan` at `mount_point`; the folder the share's files
    /// are then read through (the mount point, or the UNC path on Windows).
    fn execute(plan: &MountPlan, mount_point: &Path) -> Result<PathBuf, AppError> {
        match plan {
            MountPlan::Cifs { args, password } => {
                let mut cmd = Command::new("mount");
                for a in args {
                    // The empty placeholder is the mount point.
                    if a.is_empty() { cmd.arg(mount_point) } else { cmd.arg(a) };
                }
                // mount.cifs reads the password from PASSWD, which keeps it off
                // the process list and allows commas in it.
                if let Some(p) = password {
                    cmd.env("PASSWD", p);
                }
                let output = cmd.output().map_err(|e| AppError::Connection {
                    message: format!("Failed to execute mount command: {e}"),
                    advice: "Ensure SMB client tools are installed on your system.".to_string(),
                })?;
                if !output.status.success() {
                    return Err(smb_mount_error(&String::from_utf8_lossy(&output.stderr)));
                }
                Ok(mount_point.to_path_buf())
            }
            #[cfg(target_os = "macos")]
            MountPlan::NetFs { url, user, password, guest } => {
                super::smb_native::netfs_mount(url, mount_point, user.as_deref(), password.as_deref(), *guest).map_err(|e| smb_mount_error(&e))?;
                Ok(mount_point.to_path_buf())
            }
            #[cfg(windows)]
            MountPlan::WNet { remote, user, password } => {
                super::smb_native::wnet_connect(remote, user.as_deref(), password.as_deref()).map_err(|e| smb_mount_error(&e))?;
                Ok(PathBuf::from(remote))
            }
            #[allow(unreachable_patterns)]
            _ => Err(AppError::Connection { message: "SMB mounting is not supported on this platform.".into(), advice: "Use an SMB connection from macOS, Windows or Linux.".into() }),
        }
    }

    /// Build platform-specific unmount command.
    fn build_unmount_command(mount_point: &Path) -> Command {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let mut cmd = Command::new("umount");
            cmd.arg(mount_point);
            cmd
        }

        #[cfg(target_os = "windows")]
        {
            // Connected without a drive letter: end the UNC connection.
            let mut cmd = Command::new("net");
            cmd.arg("use").arg(mount_point).arg("/delete").arg("/y");
            cmd
        }

        #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
        {
            let mut cmd = Command::new("echo");
            cmd.arg("Unmount not supported");
            cmd
        }
    }

    /// Discover SMB shares on the local network.
    pub async fn discover_shares(
        _timeout_secs: u64,
    ) -> Result<SmbDiscoveryResult, AppError> {
        let start = std::time::Instant::now();

        let result = tokio::task::spawn_blocking(move || {
            let mut shares = Vec::new();
            let mut errors = Vec::new();
            let mut hosts_scanned: u32 = 0;

            // Try smbclient -L (Linux) or smbutil (macOS) for network discovery
            #[cfg(target_os = "macos")]
            {
                // Use dns-sd to find SMB services
                let output = Command::new("dns-sd")
                    .arg("-B")
                    .arg("_smb._tcp")
                    .arg("local.")
                    .output();

                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        for line in stdout.lines() {
                            if line.contains("_smb._tcp.") {
                                // Parse discovery results
                                let parts: Vec<&str> = line.split_whitespace().collect();
                                if parts.len() >= 7 {
                                    let name = parts[6..].join(" ");
                                    shares.push(DiscoveredShare {
                                        host: name.clone(),
                                        share_name: String::new(),
                                        share_type: "disk".to_string(),
                                        comment: None,
                                        is_nas: false,
                                        workgroup: None,
                                    });
                                    hosts_scanned += 1;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(format!("dns-sd discovery failed: {e}"));
                    }
                }

                // Also try smbutil to find shares
                let output = Command::new("smbutil")
                    .arg("lookup")
                    .arg("-a")
                    .output();

                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        for line in stdout.lines() {
                            if !line.is_empty() && !line.starts_with("Looking up") {
                                hosts_scanned += 1;
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(format!("smbutil discovery failed: {e}"));
                    }
                }
            }

            #[cfg(target_os = "linux")]
            {
                // Use avahi-browse for mDNS discovery of SMB services
                let output = Command::new("avahi-browse")
                    .arg("-t")
                    .arg("-r")
                    .arg("_smb._tcp")
                    .output();

                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let mut current_host = None;
                        let mut current_name = None;

                        for line in stdout.lines() {
                            if line.contains("hostname") {
                                current_host =
                                    line.split('=').nth(1).map(|s| s.trim().to_string());
                            }
                            if line.contains("name") {
                                current_name =
                                    line.split('=').nth(1).map(|s| s.trim().to_string());
                            }
                            if line.is_empty() && current_host.is_some() {
                                let host = current_host.take().unwrap_or_default();
                                let name = current_name
                                    .take()
                                    .unwrap_or_else(|| host.clone());
                                shares.push(DiscoveredShare {
                                    host: host.clone(),
                                    share_name: String::new(),
                                    share_type: "disk".to_string(),
                                    comment: Some(name),
                                    is_nas: detect_nas_device(&host),
                                    workgroup: None,
                                });
                                hosts_scanned += 1;
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(format!("avahi-browse discovery failed: {e}"));
                    }
                }

                // Also try smbclient for share listing
                let output = Command::new("smbclient")
                    .arg("-L")
                    .arg("WORKGROUP")
                    .arg("-N")
                    .output();

                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        for line in stdout.lines() {
                            let trimmed = line.trim();
                            if trimmed.starts_with("Disk") || trimmed.contains("Disk") {
                                let parts: Vec<&str> =
                                    trimmed.split_whitespace().collect();
                                if !parts.is_empty() {
                                    shares.push(DiscoveredShare {
                                        host: "WORKGROUP".to_string(),
                                        share_name: parts[0].to_string(),
                                        share_type: "disk".to_string(),
                                        comment: None,
                                        is_nas: false,
                                        workgroup: Some("WORKGROUP".to_string()),
                                    });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(format!("smbclient discovery failed: {e}"));
                    }
                }
            }

            #[cfg(target_os = "windows")]
            {
                // Use "net view" on Windows
                let output = Command::new("net").arg("view").output();

                match output {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        for line in stdout.lines() {
                            let trimmed = line.trim();
                            if trimmed.starts_with("\\\\") {
                                let host = trimmed
                                    .split_whitespace()
                                    .next()
                                    .unwrap_or("")
                                    .trim_start_matches("\\\\");
                                if !host.is_empty() {
                                    shares.push(DiscoveredShare {
                                        host: host.to_string(),
                                        share_name: String::new(),
                                        share_type: "disk".to_string(),
                                        comment: None,
                                        is_nas: detect_nas_device(host),
                                        workgroup: None,
                                    });
                                    hosts_scanned += 1;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        errors.push(format!("net view discovery failed: {e}"));
                    }
                }
            }

            let scan_duration_ms = start.elapsed().as_millis() as u64;

            SmbDiscoveryResult {
                shares,
                scan_duration_ms,
                hosts_scanned,
                errors,
            }
        })
        .await
        .map_err(|e| AppError::Connection {
            message: format!("SMB discovery task failed: {e}"),
            advice: "Try again or check network connectivity.".to_string(),
        })?;

        Ok(result)
    }

    /// List shares available on a specific host.
    pub async fn list_shares(
        host: &str,
        username: Option<&str>,
        password: Option<&str>,
    ) -> Result<Vec<DiscoveredShare>, AppError> {
        let host = host.to_string();
        let username = username.map(|s| s.to_string());
        let password = password.map(|s| s.to_string());

        tokio::task::spawn_blocking(move || {
            let mut shares = Vec::new();

            #[cfg(any(target_os = "macos", target_os = "linux"))]
            {
                let mut cmd = Command::new("smbclient");
                cmd.arg("-L").arg(&host);

                match (&username, &password) {
                    (Some(user), Some(pass)) => {
                        cmd.arg("-U").arg(format!("{user}%{pass}"));
                    }
                    (Some(user), None) => {
                        cmd.arg("-U").arg(user);
                    }
                    _ => {
                        cmd.arg("-N"); // No password
                    }
                }

                match cmd.output() {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        let mut in_share_section = false;

                        for line in stdout.lines() {
                            let trimmed = line.trim();
                            if trimmed.contains("Sharename") && trimmed.contains("Type") {
                                in_share_section = true;
                                continue;
                            }
                            if in_share_section && trimmed.starts_with("---") {
                                continue;
                            }
                            if in_share_section && !trimmed.is_empty() {
                                let parts: Vec<&str> =
                                    trimmed.splitn(3, char::is_whitespace).collect();
                                if parts.len() >= 2 {
                                    let share_name = parts[0].to_string();
                                    let share_type =
                                        parts[1].trim().to_lowercase();
                                    let comment = parts
                                        .get(2)
                                        .map(|s| s.trim().to_string())
                                        .filter(|s| !s.is_empty());

                                    shares.push(DiscoveredShare {
                                        host: host.clone(),
                                        share_name,
                                        share_type,
                                        comment,
                                        is_nas: detect_nas_device(&host),
                                        workgroup: None,
                                    });
                                }
                            }
                            if in_share_section && trimmed.is_empty() {
                                in_share_section = false;
                            }
                        }
                    }
                    Err(e) => {
                        return Err(AppError::Connection {
                            message: format!("Failed to list shares on {host}: {e}"),
                            advice: "Ensure smbclient is installed and the host is reachable."
                                .to_string(),
                        });
                    }
                }
            }

            #[cfg(target_os = "windows")]
            {
                // `net view` has no credential options: authenticate an IPC$
                // session first, list, then drop the session again.
                let ipc = format!("\\\\{host}\\IPC$");
                let authenticated = match (&username, &password) {
                    (Some(user), Some(pass)) => Command::new("net")
                        .args(["use", &ipc, pass, &format!("/user:{user}")])
                        .output()
                        .map(|o| o.status.success())
                        .unwrap_or(false),
                    _ => false,
                };

                let mut cmd = Command::new("net");
                cmd.arg("view").arg(format!("\\\\{host}"));

                let listed = cmd.output();
                if authenticated {
                    let _ = Command::new("net").args(["use", &ipc, "/delete", "/y"]).output();
                }
                match listed {
                    Ok(out) => {
                        let stdout = String::from_utf8_lossy(&out.stdout);
                        for line in stdout.lines() {
                            let trimmed = line.trim();
                            if trimmed.contains("Disk") {
                                let parts: Vec<&str> =
                                    trimmed.split_whitespace().collect();
                                if !parts.is_empty() {
                                    shares.push(DiscoveredShare {
                                        host: host.clone(),
                                        share_name: parts[0].to_string(),
                                        share_type: "disk".to_string(),
                                        comment: None,
                                        is_nas: detect_nas_device(&host),
                                        workgroup: None,
                                    });
                                }
                            }
                        }
                    }
                    Err(e) => {
                        return Err(AppError::Connection {
                            message: format!("Failed to list shares on {host}: {e}"),
                            advice: "Check that the host is reachable.".to_string(),
                        });
                    }
                }
            }

            Ok(shares)
        })
        .await
        .map_err(|e| AppError::Connection {
            message: format!("Share listing task failed: {e}"),
            advice: "Try again.".to_string(),
        })?
    }

    /// Get the current mount point path if connected.
    pub async fn mount_point(&self) -> Option<PathBuf> {
        self.state.read().await.mount_point.clone()
    }
}

impl Default for SmbConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl SmbConnector {
    /// The mount command for `profile`: share from the remote path, the
    /// user, the stored password and the domain. Returns the command, the
    /// local mount point and the share name.
    pub(crate) async fn mount_command_for(&self, profile: &ConnectionProfile, password: Option<&str>) -> Result<(MountPlan, PathBuf, String), AppError> {
        let host = profile.host.clone();
        let username = profile.username.clone().filter(|u| !u.is_empty());
        let mut config = self.state.read().await.config.clone();
        // The saved domain overrides the connector default.
        if let Some(domain) = profile.extras.smb.as_ref().and_then(|s| s.domain.clone()) {
            let domain = domain.trim().to_string();
            config.domain = (!domain.is_empty()).then_some(domain);
        }
        if let Some(d) = &config.domain {
            validate_mount_credential(d, "Domain")?;
            validate_hostname(d).map_err(|_| AppError::validation("The SMB domain may only contain letters, digits, dots and hyphens"))?;
        }
        let password = password.filter(|p| !p.is_empty());
        // Parse share name from remote_path (format: /ShareName/path/to/dir)
        let share_name = profile.remote_path.trim_start_matches('/').split('/').next().unwrap_or("").to_string();
        if share_name.is_empty() {
            return Err(AppError::Connection {
                message: "No share name specified.".to_string(),
                advice: "Set the remote path to /ShareName or /ShareName/subfolder.".to_string(),
            });
        }
        // Validate hostname to prevent command injection
        validate_hostname(&host)?;
        // Validate credentials to prevent mount option injection
        if let Some(ref user) = username {
            validate_mount_credential(user, "Username")?;
        }
        // Validate SMB version
        Self::validate_version(&config)?;
        let mount_point = Self::mount_point_path(&host, &share_name);
        let plan = Self::plan(crate::cloud_folders::Os::current(), &host, &share_name, username.as_deref(), password, &config);
        Ok((plan, mount_point, share_name))
    }
}

impl Connector for SmbConnector {
    fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        self.connect_with_secret(profile, None)
    }

    fn connect_with_secret(
        &self,
        profile: &ConnectionProfile,
        secret: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let profile = profile.clone();
        Box::pin(async move {
            let (plan, mount_point, share_name) = self.mount_command_for(&profile, secret.as_deref()).await?;
            let host = profile.host.clone();

            // Create mount point directory
            tokio::fs::create_dir_all(&mount_point).await.map_err(|e| {
                AppError::Connection {
                    message: format!("Failed to create mount point: {e}"),
                    advice: "Check file permissions.".to_string(),
                }
            })?;

            let mp = mount_point.clone();
            let root = tokio::task::spawn_blocking(move || Self::execute(&plan, &mp))
                .await
                .map_err(|e| AppError::Connection {
                    message: format!("Mount task failed: {e}"),
                    advice: "Try again.".to_string(),
                })??;

            let mut state = self.state.write().await;
            state.connected = true;
            state.mount_point = Some(root);
            state.host = host;
            state.share_name = share_name;

            tracing::info!(
                "SMB connected: //{}/{}",
                state.host,
                state.share_name
            );

            Ok(())
        })
    }

    fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            let mut state = self.state.write().await;

            if !state.connected {
                return Ok(());
            }

            if let Some(mount_point) = &state.mount_point {
                let mp = mount_point.clone();
                let result = tokio::task::spawn_blocking(move || {
                    let mut cmd = Self::build_unmount_command(&mp);
                    let output = cmd.output().map_err(|e| AppError::Connection {
                        message: format!("Failed to unmount: {e}"),
                        advice: "You may need to close files before disconnecting.".to_string(),
                    })?;

                    if !output.status.success() {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        return Err(AppError::Connection {
                            message: format!("Unmount failed: {stderr}"),
                            advice:
                                "Close any open files from this share and try again.".to_string(),
                        });
                    }

                    Ok(())
                })
                .await
                .map_err(|e| AppError::Connection {
                    message: format!("Unmount task failed: {e}"),
                    advice: "Try again.".to_string(),
                })?;

                result?;

                // Clean up mount point directory
                let _ = tokio::fs::remove_dir(&state.mount_point.as_ref().unwrap()).await;
            }

            tracing::info!(
                "SMB disconnected: //{}/{}",
                state.host,
                state.share_name
            );

            state.connected = false;
            state.mount_point = None;

            Ok(())
        })
    }

    fn list_remote(
        &self,
        path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<FileEntry>, AppError>> + Send + '_>> {
        let path = path.to_string();

        Box::pin(async move {
            let state = self.state.read().await;

            if !state.connected {
                return Err(AppError::Connection {
                    message: "Not connected to SMB share.".to_string(),
                    advice: "Connect first before browsing files.".to_string(),
                });
            }

            let mount_point = state.mount_point.as_ref().ok_or_else(|| {
                AppError::Connection {
                    message: "No mount point available.".to_string(),
                    advice: "Reconnect to the SMB share.".to_string(),
                }
            })?;

            // Resolve the path relative to mount point
            let full_path = if path.is_empty() || path == "/" {
                mount_point.clone()
            } else {
                mount_point.join(path.trim_start_matches('/'))
            };

            let entries = list_directory_entries(&full_path).await?;
            Ok(entries)
        })
    }

    fn is_connected(&self) -> bool {
        // Synchronous check - use try_read to avoid blocking
        self.state
            .try_read()
            .map(|s| s.connected)
            .unwrap_or(false)
    }
}

// ──────────────────────────────────────────────
// Helper Functions
// ──────────────────────────────────────────────

/// A failed mount: refused credentials (a rejected sign-in, not retried)
/// are told apart from other failures.
fn smb_mount_error(stderr: &str) -> AppError {
    let message = format!("SMB mount failed: {}", stderr.trim());
    if smb_credentials_rejected(stderr) {
        AppError::auth(message, "Check the user name, password and domain of the connection.")
    } else {
        AppError::connection(message, "Check the hostname, share name, and credentials.")
    }
}

/// Whether mount.cifs / mount_smbfs / `net use` output says the server
/// refused the user name or password.
pub(crate) fn smb_credentials_rejected(stderr: &str) -> bool {
    let s = stderr.to_ascii_lowercase();
    [
        "mount error(13)",          // mount.cifs: EACCES = logon failure
        "status_logon_failure",     // kernel / smbclient
        "status_account_disabled",
        "status_account_locked_out",
        "status_password_expired",
        "status_wrong_password",
        "authentication error",     // mount_smbfs
        "authentication failed",
        "system error 1326",        // net use: user name or password incorrect
        "system error 86",          // net use: network password not correct
        "user name or password is incorrect",
        "logon failure",
    ]
    .iter()
    .any(|k| s.contains(k))
}

/// How a share is mounted on this platform (see `SmbConnector::plan`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum MountPlan {
    /// Linux: `mount -t cifs`, the password in its environment.
    Cifs { args: Vec<String>, password: Option<String> },
    /// macOS: NetFS (no child process; the password is an argument).
    NetFs { url: String, user: Option<String>, password: Option<String>, guest: bool },
    /// Windows: a connection to the UNC path (no child process, no drive letter).
    WNet { remote: String, user: Option<String>, password: Option<String> },
}

impl MountPlan {
    /// What a mount following this plan shows in the process list.
    #[cfg(test)]
    pub(crate) fn visible_command_line(&self) -> Vec<String> {
        match self {
            Self::Cifs { args, .. } => std::iter::once("mount".to_string()).chain(args.iter().cloned()).collect(),
            Self::NetFs { .. } | Self::WNet { .. } => Vec::new(),
        }
    }
}

/// URL-encode a string for SMB URLs.
fn urlencoded(s: &str) -> String {
    s.replace('%', "%25")
        .replace(' ', "%20")
        .replace('@', "%40")
        .replace(':', "%3A")
        .replace('/', "%2F")
        .replace('\\', "%5C")
}

/// Detect if a hostname appears to be a NAS device.
fn detect_nas_device(host: &str) -> bool {
    let lower = host.to_lowercase();
    lower.contains("nas")
        || lower.contains("synology")
        || lower.contains("qnap")
        || lower.contains("netgear")
        || lower.contains("buffalo")
        || lower.contains("drobo")
        || lower.contains("wd")
        || lower.contains("mycloud")
        || lower.contains("freenas")
        || lower.contains("truenas")
        || lower.contains("unraid")
        || lower.contains("asustor")
}

/// List directory entries from a local path (mounted SMB share).
async fn list_directory_entries(path: &Path) -> Result<Vec<FileEntry>, AppError> {
    let path = path.to_path_buf();

    tokio::task::spawn_blocking(move || {
        let mut entries = Vec::new();

        let read_dir = std::fs::read_dir(&path).map_err(|e| AppError::Connection {
            message: format!("Failed to read directory {}: {e}", path.display()),
            advice: "Check that you have permission to access this folder.".to_string(),
        })?;

        for entry in read_dir {
            let entry = entry.map_err(|e| AppError::Connection {
                message: format!("Failed to read entry: {e}"),
                advice: "Some files may not be accessible.".to_string(),
            })?;

            let metadata = entry.metadata().map_err(|e| AppError::Connection {
                message: format!("Failed to read metadata: {e}"),
                advice: "Some files may not be accessible.".to_string(),
            })?;

            let name = entry.file_name().to_string_lossy().to_string();
            let file_path = entry.path().to_string_lossy().to_string();
            let is_hidden = name.starts_with('.');

            let modified = metadata
                .modified()
                .ok()
                .and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .and_then(|d| Utc.timestamp_opt(d.as_secs() as i64, 0).single())
                });

            let created = metadata
                .created()
                .ok()
                .and_then(|t| {
                    t.duration_since(std::time::UNIX_EPOCH)
                        .ok()
                        .and_then(|d| Utc.timestamp_opt(d.as_secs() as i64, 0).single())
                });

            let extension = if metadata.is_file() {
                Path::new(&name)
                    .extension()
                    .map(|e| e.to_string_lossy().to_string())
            } else {
                None
            };

            // Read Unix permissions
            #[cfg(unix)]
            let permissions = {
                use std::os::unix::fs::PermissionsExt;
                Some(format!("{:o}", metadata.permissions().mode() & 0o777))
            };
            #[cfg(not(unix))]
            let permissions = None;

            entries.push(FileEntry {
                name,
                path: file_path,
                is_dir: metadata.is_dir(),
                is_symlink: metadata.file_type().is_symlink(),
                size: metadata.len(),
                modified,
                created,
                is_hidden,
                extension,
                permissions,
            });
        }

        entries.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
        });

        Ok(entries)
    })
    .await
    .map_err(|e| AppError::Connection {
        message: format!("Directory listing task failed: {e}"),
        advice: "Try again.".to_string(),
    })?
}

// ──────────────────────────────────────────────
// Tests
// ──────────────────────────────────────────────

#[cfg(test)]
mod profile_tests {
    use super::*;
    use crate::core::types::{ConnectionProtocol, SmbSettings};

    /// Regression: the configured domain (and the stored password) never
    /// reached the mount command.
    #[tokio::test]
    async fn domain_and_password_reach_the_mount() {
        let mut p = crate::connectors::live_tests::profile(ConnectionProtocol::Smb, "nas.local", None, "alice", "/Share/docs");
        p.extras.smb = Some(SmbSettings { domain: Some("CORP".into()) });
        let c = SmbConnector::new();
        let (plan, _mount_point, share) = c.mount_command_for(&p, Some("s3cret")).await.unwrap();
        assert_eq!(share, "Share");
        match plan {
            MountPlan::Cifs { args, password } => {
                assert_eq!(args[..4], ["-t".to_string(), "cifs".into(), "//nas.local/Share".into(), String::new()]);
                let opts = args.last().unwrap();
                assert!(opts.contains("username=alice") && opts.contains("domain=CORP"), "{opts}");
                assert_eq!(password.as_deref(), Some("s3cret"));
            }
            MountPlan::NetFs { user, password, .. } | MountPlan::WNet { user, password, .. } => {
                assert!(user.unwrap().contains("CORP"));
                assert_eq!(password.as_deref(), Some("s3cret"));
            }
        }
        // A domain that would inject mount options is refused.
        p.extras.smb = Some(SmbSettings { domain: Some("CORP,uid=0".into()) });
        assert!(c.mount_command_for(&p, None).await.is_err());
    }

    /// Review finding (WS-Z1): on macOS the password was in the
    /// `mount_smbfs` URL and on Windows in `net use /password:`, both visible
    /// to every user in the process list. Every platform now keeps it off
    /// the command line.
    #[test]
    fn the_password_is_never_on_a_command_line() {
        use crate::cloud_folders::Os;
        let mut config = SmbConfig { domain: Some("CORP".into()), ..SmbConfig::default() };
        for os in [Os::Linux, Os::MacOs, Os::Windows] {
            let plan = SmbConnector::plan(os, "nas.local", "Team Share", Some("alice"), Some("p@ss:w0rd"), &config);
            let line = plan.visible_command_line().join(" ");
            assert!(!line.contains("p@ss") && !line.contains("w0rd") && !line.contains("p%40ss"), "{os:?}: {line}");
        }
        assert_eq!(
            SmbConnector::plan(Os::Windows, "nas.local", "Team Share", Some("alice"), Some("pw"), &config),
            MountPlan::WNet { remote: "\\\\nas.local\\Team Share".into(), user: Some("CORP\\alice".into()), password: Some("pw".into()) }
        );
        assert_eq!(
            SmbConnector::plan(Os::MacOs, "nas.local", "Team Share", Some("alice"), Some("pw"), &config),
            MountPlan::NetFs { url: "smb://nas.local/Team%20Share".into(), user: Some("CORP;alice".into()), password: Some("pw".into()), guest: false }
        );
        config.auth_method = SmbAuthMethod::Guest;
        assert_eq!(
            SmbConnector::plan(Os::MacOs, "nas.local", "S", Some("alice"), Some("pw"), &config),
            MountPlan::NetFs { url: "smb://nas.local/S".into(), user: None, password: None, guest: true }
        );
    }
}

#[cfg(test)]
mod tests {
    /// Regression: every mount failure was a connection error, so a
    /// refused password was retried and never flagged the connection.
    #[test]
    fn refused_smb_credentials_are_rejected_sign_ins() {
        for stderr in [
            "mount error(13): Permission denied\nRefer to the mount.cifs(8) manual page",
            "CIFS: VFS: \\\\nas Send error in SessSetup = -13 STATUS_LOGON_FAILURE",
            "mount_smbfs: server rejected the connection: Authentication error",
            "System error 1326 has occurred.\r\nThe user name or password is incorrect.",
        ] {
            assert!(smb_mount_error(stderr).is_auth_failure(), "{stderr}");
        }
        for stderr in ["mount error(112): Host is down", "mount error(2): No such file or directory", "mount_smbfs: server connection failed: No route to host"] {
            let e = smb_mount_error(stderr);
            assert!(!e.is_auth_failure() && ufop_core::retry::is_retryable(&e), "{stderr}");
        }
    }

    use super::*;

    #[test]
    fn test_smb_version_mount_option() {
        assert_eq!(SmbVersion::Smb2.as_mount_option(), "2.0");
        assert_eq!(SmbVersion::Smb3.as_mount_option(), "3.0");
        assert_eq!(SmbVersion::Smb311.as_mount_option(), "3.1.1");
        assert_eq!(SmbVersion::Smb1.as_mount_option(), "1.0");
    }

    #[test]
    fn test_smb1_validation() {
        // SMB1 should be rejected by default
        let config = SmbConfig {
            min_version: SmbVersion::Smb1,
            allow_smb1: false,
            ..Default::default()
        };
        assert!(SmbConnector::validate_version(&config).is_err());

        // Allowing SMB1 explicitly should work
        let config = SmbConfig {
            min_version: SmbVersion::Smb1,
            allow_smb1: true,
            ..Default::default()
        };
        assert!(SmbConnector::validate_version(&config).is_ok());

        // SMB2+ should always work
        let config = SmbConfig {
            min_version: SmbVersion::Smb2,
            allow_smb1: false,
            ..Default::default()
        };
        assert!(SmbConnector::validate_version(&config).is_ok());
    }

    #[test]
    fn test_mount_point_path() {
        let mp = SmbConnector::mount_point_path("192.168.1.100", "SharedDocs");
        let mp_str = mp.to_string_lossy();
        // In this user's private temp folder, not a shared /tmp name.
        assert!(mp.starts_with(ufop_core::session::user_temp_dir().unwrap().join("smb-mounts")), "{mp_str}");
        assert!(mp_str.contains("192.168.1.100_SharedDocs"));
    }

    #[test]
    fn test_nas_detection() {
        assert!(detect_nas_device("synology-ds920"));
        assert!(detect_nas_device("QNAP-TS251"));
        assert!(detect_nas_device("my-nas-server"));
        assert!(detect_nas_device("TrueNAS-Home"));
        assert!(!detect_nas_device("my-laptop"));
        assert!(!detect_nas_device("file-server-01"));
    }

    #[test]
    fn test_urlencoded() {
        assert_eq!(urlencoded("user@domain"), "user%40domain");
        assert_eq!(urlencoded("pass word"), "pass%20word");
        assert_eq!(urlencoded("admin"), "admin");
    }

    #[test]
    fn test_smb_config_defaults() {
        let config = SmbConfig::default();
        assert!(!config.allow_smb1);
        assert!(config.encryption);
        assert!(config.signing);
        assert_eq!(config.min_version, SmbVersion::Smb2);
        assert_eq!(config.max_version, SmbVersion::Smb311);
        assert_eq!(config.auth_method, SmbAuthMethod::Password);
    }

    #[test]
    fn test_smb_connector_default() {
        let connector = SmbConnector::new();
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_discovered_share_serialization() {
        let share = DiscoveredShare {
            host: "nas.local".to_string(),
            share_name: "Media".to_string(),
            share_type: "disk".to_string(),
            comment: Some("Media files".to_string()),
            is_nas: true,
            workgroup: Some("WORKGROUP".to_string()),
        };
        let json = serde_json::to_string(&share).unwrap();
        let deserialized: DiscoveredShare = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.share_name, "Media");
        assert!(deserialized.is_nas);
    }

    #[test]
    fn test_smb_config_serialization() {
        let config = SmbConfig {
            share_name: "Documents".to_string(),
            domain: Some("CORP".to_string()),
            min_version: SmbVersion::Smb3,
            max_version: SmbVersion::Smb311,
            auth_method: SmbAuthMethod::Kerberos,
            allow_smb1: false,
            encryption: true,
            signing: true,
            mount_options: HashMap::new(),
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("CORP"));
        assert!(json.contains("kerberos"));
        let deserialized: SmbConfig = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.domain, Some("CORP".to_string()));
    }
}
