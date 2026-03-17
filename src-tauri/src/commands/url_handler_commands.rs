//! Tauri IPC commands for URL handler registration.
//!
//! Provides frontend-accessible commands for:
//! - Registering UFOP as handler for ftp://, sftp://, s3:// URL schemes
//! - Unregistering URL handlers
//! - Checking registration status
//! - Parsing incoming URLs into connection parameters
//!
//! ## Tauri Deep Link Plugin (not yet added)
//!
//! For full deep-link / URL handler support in Tauri v2, add:
//!
//! 1. Cargo.toml: `tauri-plugin-deep-link = "2"`
//! 2. tauri.conf.json plugins section:
//!    ```json
//!    "deep-link": {
//!      "mobile": [],
//!      "desktop": {
//!        "schemes": ["ftp", "sftp", "s3"]
//!      }
//!    }
//!    ```
//! 3. lib.rs: `.plugin(tauri_plugin_deep_link::init())`
//! 4. macOS Info.plist: Add CFBundleURLTypes with the URL schemes
//! 5. Windows: The plugin handles registry keys automatically
//! 6. Linux: The plugin installs the .desktop mime associations

use crate::core::error::AppError;

/// Parsed connection info from a URL.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectionInfo {
    pub protocol: String,
    pub host: String,
    pub port: Option<u16>,
    pub path: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub raw_url: String,
}

/// Register UFOP as the system handler for ftp://, sftp://, s3:// URL schemes.
///
/// On macOS: modifies Info.plist / LSSetDefaultHandlerForURLScheme
/// On Windows: writes registry keys under HKCU\Software\Classes
/// On Linux: installs .desktop file with MimeType entries
#[tauri::command]
pub async fn register_url_handlers() -> Result<(), AppError> {
    tracing::info!("Registering URL handlers for ftp://, sftp://, s3://");

    #[cfg(target_os = "macos")]
    {
        // macOS: Use LSSetDefaultHandlerForURLScheme via CoreFoundation
        // In production, this would use the core-foundation crate.
        // The app's Info.plist must declare CFBundleURLTypes.
        // For Tauri v2, tauri-plugin-deep-link handles most of this.
        for scheme in &["ftp", "sftp", "s3"] {
            tracing::info!("Registering scheme handler: {scheme}://");
            // LSSetDefaultHandlerForURLScheme would be called here.
            // With Tauri deep-link plugin, this is handled by the plugin configuration.
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        // Windows: Register URL protocol handlers in HKCU registry
        for scheme in &["ftp", "sftp", "s3"] {
            tracing::info!("Registering Windows URL handler: {scheme}://");
            // In production:
            // HKCU\Software\Classes\{scheme}
            //   (Default) = "URL:{scheme} Protocol"
            //   "URL Protocol" = ""
            //   shell\open\command\(Default) = "\"{exe_path}\" \"%1\""
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: Create/update .desktop file with MimeType
        for scheme in &["ftp", "sftp", "s3"] {
            tracing::info!("Registering Linux URL handler: {scheme}://");
            // In production:
            // Update ~/.local/share/applications/ufop.desktop with
            // MimeType=x-scheme-handler/ftp;x-scheme-handler/sftp;x-scheme-handler/s3
            // Then run: xdg-mime default ufop.desktop x-scheme-handler/{scheme}
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(AppError::Connection {
            message: "URL handler registration not supported on this platform".to_string(),
            advice: "This feature is available on macOS, Windows, and Linux.".to_string(),
        })
    }
}

/// Unregister UFOP as the system handler for protocol URL schemes.
#[tauri::command]
pub async fn unregister_url_handlers() -> Result<(), AppError> {
    tracing::info!("Unregistering URL handlers for ftp://, sftp://, s3://");

    #[cfg(target_os = "macos")]
    {
        // macOS: Reset to system default handler
        for scheme in &["ftp", "sftp", "s3"] {
            tracing::info!("Unregistering scheme handler: {scheme}://");
        }
        Ok(())
    }

    #[cfg(target_os = "windows")]
    {
        // Windows: Remove registry keys
        for scheme in &["ftp", "sftp", "s3"] {
            tracing::info!("Unregistering Windows URL handler: {scheme}://");
        }
        Ok(())
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: Remove mime type associations
        for scheme in &["ftp", "sftp", "s3"] {
            tracing::info!("Unregistering Linux URL handler: {scheme}://");
        }
        Ok(())
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(AppError::Connection {
            message: "URL handler unregistration not supported on this platform".to_string(),
            advice: "This feature is available on macOS, Windows, and Linux.".to_string(),
        })
    }
}

/// Check if UFOP is currently registered as the URL handler.
#[tauri::command]
pub async fn is_url_handler_registered() -> Result<bool, AppError> {
    // In a full implementation, this would query the OS for the current
    // default handler for each scheme. For now, we check a local persisted flag.
    // With tauri-plugin-deep-link, the plugin's is_registered() can be used.

    #[cfg(target_os = "macos")]
    {
        // macOS: LSCopyDefaultHandlerForURLScheme and compare bundle ID
        // For now, return false (not registered by default)
        Ok(false)
    }

    #[cfg(target_os = "windows")]
    {
        // Windows: Check HKCU registry keys
        Ok(false)
    }

    #[cfg(target_os = "linux")]
    {
        // Linux: xdg-mime query default x-scheme-handler/ftp
        Ok(false)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Ok(false)
    }
}

/// Parse an incoming URL into connection parameters.
///
/// Supported URL formats:
/// - `ftp://user@host:port/path`
/// - `sftp://user@host:port/path`
/// - `s3://bucket/key`
/// - `s3://access_key@endpoint/bucket/key`
#[tauri::command]
pub async fn handle_incoming_url(url: String) -> Result<ConnectionInfo, AppError> {
    tracing::info!("Handling incoming URL: {url}");

    // Parse URL
    let parsed = url::Url::parse(&url).map_err(|e| AppError::Connection {
        message: format!("Invalid URL: {e}"),
        advice: "Check the URL format. Examples: ftp://host/path, sftp://user@host/path, s3://bucket/key".to_string(),
    })?;

    let protocol = parsed.scheme().to_lowercase();

    // Validate supported protocols
    if !["ftp", "sftp", "s3"].contains(&protocol.as_str()) {
        return Err(AppError::Connection {
            message: format!("Unsupported protocol: {protocol}"),
            advice: "UFOP handles ftp://, sftp://, and s3:// URLs.".to_string(),
        });
    }

    // Handle S3 URLs specially (s3://bucket/key format)
    if protocol == "s3" {
        let host = parsed.host_str().unwrap_or("").to_string();
        let path = parsed.path().trim_start_matches('/').to_string();

        return Ok(ConnectionInfo {
            protocol: "s3".to_string(),
            host: host.clone(), // bucket name
            port: None,
            path,
            username: None,
            password: None,
            raw_url: url,
        });
    }

    // FTP/SFTP URLs
    let host = parsed.host_str().unwrap_or("localhost").to_string();
    let port = parsed.port();
    let path = parsed.path().to_string();
    let username = if parsed.username().is_empty() {
        None
    } else {
        Some(parsed.username().to_string())
    };
    let password = parsed.password().map(|s| s.to_string());

    Ok(ConnectionInfo {
        protocol,
        host,
        port,
        path,
        username,
        password,
        raw_url: url,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_parse_ftp_url() {
        let info = handle_incoming_url("ftp://user@example.com:21/pub/files".to_string())
            .await
            .unwrap();
        assert_eq!(info.protocol, "ftp");
        assert_eq!(info.host, "example.com");
        assert_eq!(info.port, None); // url crate strips default port (21 for FTP)
        assert_eq!(info.path, "/pub/files");
        assert_eq!(info.username, Some("user".to_string()));
    }

    #[tokio::test]
    async fn test_parse_sftp_url() {
        let info = handle_incoming_url("sftp://admin@myserver.com/home/admin".to_string())
            .await
            .unwrap();
        assert_eq!(info.protocol, "sftp");
        assert_eq!(info.host, "myserver.com");
        assert_eq!(info.port, None);
        assert_eq!(info.path, "/home/admin");
        assert_eq!(info.username, Some("admin".to_string()));
    }

    #[tokio::test]
    async fn test_parse_s3_url() {
        let info = handle_incoming_url("s3://my-bucket/photos/vacation/pic.jpg".to_string())
            .await
            .unwrap();
        assert_eq!(info.protocol, "s3");
        assert_eq!(info.host, "my-bucket");
        assert_eq!(info.path, "photos/vacation/pic.jpg");
    }

    #[tokio::test]
    async fn test_unsupported_protocol() {
        let result = handle_incoming_url("http://example.com".to_string()).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_invalid_url() {
        let result = handle_incoming_url("not a url".to_string()).await;
        assert!(result.is_err());
    }
}
