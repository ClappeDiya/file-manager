//! Tauri IPC commands for protocol connector operations (T-020, T-021, T-022).
//!
//! These commands expose SFTP, FTP/FTPS, and WebDAV connector operations to the frontend.
//! The ConnectorRegistry is used to dispatch operations to the correct connector.

use crate::connectors::ConnectorRegistry;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, ConnectionProtocol, FileEntry};
use tauri::State;

/// Connect to a remote server using the appropriate protocol connector.
#[tauri::command]
pub async fn connector_connect(
    protocol: String,
    host: String,
    port: Option<u16>,
    username: Option<String>,
    remote_path: Option<String>,
    registry: State<'_, ConnectorRegistry>,
) -> Result<(), AppError> {
    let connector = registry
        .get(&protocol)
        .ok_or_else(|| AppError::Connection {
            message: format!("Unsupported protocol: {protocol}"),
            advice: "Use one of: sftp, ftp, ftps, webdav, smb, nfs, local.".to_string(),
        })?;

    let profile = ConnectionProfile {
        id: uuid::Uuid::new_v4(),
        name: format!("{protocol}://{host}"),
        protocol: ConnectionProtocol::from_str_lossy(&protocol),
        host,
        port,
        username,
        credential_ref: None,
        remote_path: remote_path.unwrap_or_else(|| "/".to_string()),
        created_at: chrono::Utc::now(),
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

    connector.connect(&profile).await
}

/// Disconnect from a remote server.
#[tauri::command]
pub async fn connector_disconnect(
    protocol: String,
    registry: State<'_, ConnectorRegistry>,
) -> Result<(), AppError> {
    let connector = registry
        .get(&protocol)
        .ok_or_else(|| AppError::Connection {
            message: format!("Unsupported protocol: {protocol}"),
            advice: "Use one of: sftp, ftp, ftps, webdav, smb, nfs, local.".to_string(),
        })?;

    connector.disconnect().await
}

/// List remote directory contents.
#[tauri::command]
pub async fn connector_list_remote(
    protocol: String,
    path: String,
    registry: State<'_, ConnectorRegistry>,
) -> Result<Vec<FileEntry>, AppError> {
    let connector = registry
        .get(&protocol)
        .ok_or_else(|| AppError::Connection {
            message: format!("Unsupported protocol: {protocol}"),
            advice: "Use one of: sftp, ftp, ftps, webdav, smb, nfs, local.".to_string(),
        })?;

    connector.list_remote(&path).await
}

/// Check if a protocol connector is connected.
#[tauri::command]
pub async fn connector_is_connected(
    protocol: String,
    registry: State<'_, ConnectorRegistry>,
) -> Result<bool, AppError> {
    let connector = registry
        .get(&protocol)
        .ok_or_else(|| AppError::Connection {
            message: format!("Unsupported protocol: {protocol}"),
            advice: "Use one of: sftp, ftp, ftps, webdav, smb, nfs, local.".to_string(),
        })?;

    Ok(connector.is_connected())
}

/// List all registered protocols.
#[tauri::command]
pub async fn connector_list_protocols(
    registry: State<'_, ConnectorRegistry>,
) -> Result<Vec<String>, AppError> {
    Ok(registry.protocols())
}
