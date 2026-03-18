//! Tauri commands for connection management (T-019).

use crate::connectors::connection_manager::{ConnectionTestResult, ImportResult};
use crate::connectors::ConnectionManager;
use crate::core::error::AppError;
use crate::core::types::{
    ConnectionGroup, ConnectionProfile, ConnectionProtocol, ProxyConfig,
};
use crate::storage::Repository;
use chrono::Utc;
use tauri::State;
use uuid::Uuid;

/// Save a connection profile.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn save_connection(
    name: String,
    protocol: String,
    host: String,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    remote_path: Option<String>,
    group_id: Option<String>,
    bandwidth_limit_bps: Option<u64>,
    verify_checksums: Option<bool>,
    connection_id: Option<String>,
    // Proxy fields
    proxy_type: Option<String>,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    proxy_username: Option<String>,
    proxy_password: Option<String>,
    // Default paths
    default_local_dir: Option<String>,
    default_remote_dir: Option<String>,
    // Charset
    charset: Option<String>,
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionProfile, AppError> {
    let id = connection_id
        .and_then(|s| Uuid::parse_str(&s).ok())
        .unwrap_or_else(Uuid::new_v4);

    let profile = ConnectionProfile {
        id,
        name,
        protocol: ConnectionProtocol::from_str_lossy(&protocol),
        host,
        port,
        username,
        credential_ref: None,
        remote_path: remote_path.unwrap_or_else(|| "/".to_string()),
        created_at: Utc::now(),
        last_used: None,
        group_id: group_id.and_then(|s| Uuid::parse_str(&s).ok()),
        bandwidth_limit_bps: bandwidth_limit_bps.unwrap_or(0),
        conflict_policy: None,
        retry_policy: None,
        verify_checksums: verify_checksums.unwrap_or(false),
        checksum_algorithm: None,
        proxy_type,
        proxy_host,
        proxy_port,
        proxy_username,
        proxy_password,
        default_local_dir,
        default_remote_dir,
        charset,
        default_file_mode: None,
        default_dir_mode: None,
        ftp_use_mlsd: None,
        ftp_force_passive_ip: None,
        ftp_post_login_commands: None,
        symlink_policy: None,
    };

    manager.save_connection(profile, password).await
}

/// List all saved connections.
#[tauri::command]
pub async fn list_connections(
    manager: State<'_, ConnectionManager>,
) -> Result<Vec<ConnectionProfile>, AppError> {
    manager.list_connections().await
}

/// Get a single connection.
#[tauri::command]
pub async fn get_connection(
    connection_id: String,
    manager: State<'_, ConnectionManager>,
) -> Result<Option<ConnectionProfile>, AppError> {
    let id = Uuid::parse_str(&connection_id).map_err(|_| AppError::Connection {
        message: format!("Invalid connection ID: {connection_id}"),
        advice: "Provide a valid UUID.".to_string(),
    })?;
    manager.get_connection(id).await
}

/// Delete a connection.
#[tauri::command]
pub async fn delete_connection(
    connection_id: String,
    manager: State<'_, ConnectionManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&connection_id).map_err(|_| AppError::Connection {
        message: format!("Invalid connection ID: {connection_id}"),
        advice: "Provide a valid UUID.".to_string(),
    })?;
    manager.delete_connection(id).await
}

/// Test a connection.
#[tauri::command]
pub async fn test_connection(
    connection_id: String,
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionTestResult, AppError> {
    let id = Uuid::parse_str(&connection_id).map_err(|_| AppError::Connection {
        message: format!("Invalid connection ID: {connection_id}"),
        advice: "Provide a valid UUID.".to_string(),
    })?;
    let profile = manager
        .get_connection(id)
        .await?
        .ok_or_else(|| AppError::Connection {
            message: format!("Connection {connection_id} not found."),
            advice: "Check the connection ID.".to_string(),
        })?;
    manager.test_connection(&profile).await
}

/// Search connections for quick-connect autocomplete.
#[tauri::command]
pub async fn search_connections(
    query: String,
    manager: State<'_, ConnectionManager>,
) -> Result<Vec<ConnectionProfile>, AppError> {
    manager.search_connections(&query).await
}

/// Create a connection group.
#[tauri::command]
pub async fn create_connection_group(
    name: String,
    parent_id: Option<String>,
    manager: State<'_, ConnectionManager>,
) -> Result<ConnectionGroup, AppError> {
    let parent = parent_id.and_then(|s| Uuid::parse_str(&s).ok());
    manager.create_group(name, parent).await
}

/// List all connection groups.
#[tauri::command]
pub async fn list_connection_groups(
    manager: State<'_, ConnectionManager>,
) -> Result<Vec<ConnectionGroup>, AppError> {
    manager.list_groups().await
}

/// Delete a connection group.
#[tauri::command]
pub async fn delete_connection_group(
    group_id: String,
    manager: State<'_, ConnectionManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&group_id).map_err(|_| AppError::Connection {
        message: format!("Invalid group ID: {group_id}"),
        advice: "Provide a valid UUID.".to_string(),
    })?;
    manager.delete_group(id).await
}

/// Export connections as JSON.
#[tauri::command]
pub async fn export_connections(
    manager: State<'_, ConnectionManager>,
) -> Result<String, AppError> {
    manager.export_connections().await
}

/// Import connections from JSON.
#[tauri::command]
pub async fn import_connections(
    json: String,
    manager: State<'_, ConnectionManager>,
) -> Result<ImportResult, AppError> {
    manager.import_connections(&json).await
}

/// Set global proxy configuration (stored in config table).
#[tauri::command]
pub async fn set_global_proxy(
    proxy_type: Option<String>,
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    repo: State<'_, Repository>,
) -> Result<(), AppError> {
    let config = ProxyConfig {
        proxy_type,
        host,
        port,
        username,
        password,
    };
    let json = serde_json::to_string(&config).map_err(|e| AppError::internal(format!("Failed to serialize proxy config: {e}")))?;
    repo.set_config("global_proxy", &json).await
}

/// Get global proxy configuration.
#[tauri::command]
pub async fn get_global_proxy(
    repo: State<'_, Repository>,
) -> Result<ProxyConfig, AppError> {
    match repo.get_config("global_proxy").await? {
        Some(json) => serde_json::from_str(&json).map_err(|e| {
            AppError::internal(format!("Failed to parse global proxy config: {e}"))
        }),
        None => Ok(ProxyConfig::default()),
    }
}

/// Resolve a host alias from ~/.ssh/config, returning the resolved
/// HostName, User, Port, IdentityFile, ProxyJump, IdentityAgent, and ForwardAgent.
#[tauri::command]
pub async fn resolve_ssh_config(
    host_alias: String,
) -> Result<crate::connectors::sftp::ssh_config::SshConfigResolved, AppError> {
    use crate::connectors::sftp::ssh_config;

    let entry = ssh_config::resolve_host(&host_alias).ok_or_else(|| AppError::Connection {
        message: format!("No SSH config entry found for host alias '{host_alias}'."),
        advice: "Check that the host is defined in ~/.ssh/config.".to_string(),
    })?;

    Ok(ssh_config::entry_to_resolved(&entry))
}

/// List all Host entries from ~/.ssh/config for autocomplete.
/// Returns alias, hostname, user, and port for each entry.
#[tauri::command]
pub async fn list_ssh_config_hosts(
) -> Result<Vec<crate::connectors::sftp::ssh_config::SshConfigHostEntry>, AppError> {
    use crate::connectors::sftp::ssh_config;

    Ok(ssh_config::list_hosts())
}

/// Discover available SSH agents on the system.
///
/// Returns a list of detected SSH agents including OpenSSH, 1Password, Bitwarden, and Pageant.
/// Each entry indicates whether the agent is currently available.
#[tauri::command]
pub async fn list_ssh_agents(
) -> Result<Vec<crate::connectors::sftp::SshAgentInfo>, AppError> {
    Ok(crate::connectors::sftp::discover_ssh_agents())
}

/// Import connection profiles from a third-party file manager.
///
/// Supports FileZilla (sitemanager.xml), Cyberduck (.duck), WinSCP (.ini),
/// and Transmit (Favorites.xml). The `source_app` parameter is optional;
/// when omitted, the format is auto-detected from the file content and extension.
///
/// Each imported profile is converted to a [`ConnectionProfile`] and saved
/// through the connection manager. Passwords (when available) are stored
/// in the OS keychain.
#[tauri::command]
pub async fn import_from_third_party(
    file_path: String,
    source_app: Option<String>,
    manager: State<'_, ConnectionManager>,
) -> Result<crate::connectors::import_profiles::ThirdPartyImportResult, AppError> {
    use crate::connectors::import_profiles;
    use std::path::Path;

    let path = Path::new(&file_path);

    // Parse the file using the appropriate parser
    let imported = if let Some(ref app) = source_app {
        import_profiles::parse_with_hint(path, app)?
    } else {
        import_profiles::detect_and_parse(path)?
    };

    let source = source_app.clone().unwrap_or_else(|| {
        imported
            .first()
            .map(|p| p.source_app.clone())
            .unwrap_or_else(|| "unknown".to_string())
    });

    let mut imported_count = 0u32;
    let mut skipped = 0u32;
    let mut errors = Vec::new();

    for entry in &imported {
        // Convert ImportedProfile -> ConnectionProfile
        let profile = ConnectionProfile {
            id: Uuid::new_v4(),
            name: entry.name.clone(),
            protocol: ConnectionProtocol::from_str_lossy(&entry.protocol),
            host: entry.host.clone(),
            port: entry.port,
            username: entry.username.clone(),
            credential_ref: None,
            remote_path: entry.remote_path.clone().unwrap_or_else(|| "/".to_string()),
            created_at: Utc::now(),
            last_used: None,
            group_id: None,
            bandwidth_limit_bps: 0,
            conflict_policy: None,
            retry_policy: None,
            verify_checksums: false,
            checksum_algorithm: None,
            proxy_type: None,
            proxy_host: None,
            proxy_port: None,
            proxy_username: None,
            proxy_password: None,
            default_local_dir: None,
            default_remote_dir: None,
            charset: None,
            default_file_mode: None,
            default_dir_mode: None,
            ftp_use_mlsd: None,
            ftp_force_passive_ip: None,
            ftp_post_login_commands: None,
            symlink_policy: None,
        };

        let password = entry.password.clone();

        match manager.save_connection(profile, password).await {
            Ok(_) => {
                imported_count += 1;
            }
            Err(e) => {
                let msg = format!("Failed to import '{}': {e}", entry.name);
                tracing::warn!("{msg}");
                errors.push(msg);
                skipped += 1;
            }
        }
    }

    tracing::info!(
        "Third-party import complete: {} imported, {} skipped from {}",
        imported_count,
        skipped,
        source
    );

    Ok(import_profiles::ThirdPartyImportResult {
        imported_connections: imported_count,
        skipped,
        errors,
        source_app: source,
    })
}
