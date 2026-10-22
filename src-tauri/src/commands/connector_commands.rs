//! Tauri IPC commands for protocol connector operations (T-020, T-021, T-022).
//!
//! These commands expose SFTP, FTP/FTPS, and WebDAV connector operations to the frontend.
//! The ConnectorRegistry is used to dispatch operations to the correct connector.

use crate::connectors::{Connector, ConnectionManager, ConnectorRegistry};
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
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<(), AppError> {
    let connector = registry
        .get(&protocol)
        .ok_or_else(|| AppError::Connection {
            message: format!("Unsupported protocol: {protocol}"),
            advice: "Use one of: sftp, ftp, ftps, webdav, s3, smb, nfs, local.".to_string(),
        })?;

    // The secret (password / access secret / key path) lives in the credential store.
    let secret = match connection_id.as_deref().map(uuid::Uuid::parse_str) {
        Some(Ok(id)) => manager.get_password(id).await.unwrap_or(None),
        _ => None,
    };

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

    connector.connect_with_secret(&profile, secret).await
}

fn require_connector(
    registry: &ConnectorRegistry,
    protocol: &str,
) -> Result<std::sync::Arc<dyn Connector>, AppError> {
    registry.get(protocol).ok_or_else(|| AppError::Connection {
        message: format!("Unsupported protocol: {protocol}"),
        advice: "Use one of: sftp, ftp, ftps, webdav, s3, smb, nfs, local.".to_string(),
    })
}

fn require_connected(connector: &dyn Connector) -> Result<(), AppError> {
    if connector.is_connected() {
        Ok(())
    } else {
        Err(AppError::Connection {
            message: "Not connected to the server.".to_string(),
            advice: "Connect from the Connections panel first.".to_string(),
        })
    }
}

/// Upload a local file to the connected server; returns bytes written.
#[tauri::command]
pub async fn connector_upload(
    protocol: String,
    local_path: String,
    remote_path: String,
    registry: State<'_, ConnectorRegistry>,
) -> Result<u64, AppError> {
    let connector = require_connector(&registry, &protocol)?;
    require_connected(connector.as_ref())?;
    connector.upload(&local_path, &remote_path).await
}

/// Download a remote file from the connected server; returns bytes written.
#[tauri::command]
pub async fn connector_download(
    protocol: String,
    remote_path: String,
    local_path: String,
    registry: State<'_, ConnectorRegistry>,
) -> Result<u64, AppError> {
    let connector = require_connector(&registry, &protocol)?;
    require_connected(connector.as_ref())?;
    connector.download(&remote_path, &local_path).await
}

/// Create a directory on the connected server.
#[tauri::command]
pub async fn connector_make_directory(
    protocol: String,
    remote_path: String,
    registry: State<'_, ConnectorRegistry>,
) -> Result<(), AppError> {
    let connector = require_connector(&registry, &protocol)?;
    require_connected(connector.as_ref())?;
    connector.make_directory(&remote_path).await
}

/// Delete a file or empty directory on the connected server.
#[tauri::command]
pub async fn connector_remove(
    protocol: String,
    remote_path: String,
    is_dir: bool,
    registry: State<'_, ConnectorRegistry>,
) -> Result<(), AppError> {
    let connector = require_connector(&registry, &protocol)?;
    require_connected(connector.as_ref())?;
    connector.remove(&remote_path, is_dir).await
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
    search: State<'_, crate::search_engine::SearchManager>,
) -> Result<Vec<FileEntry>, AppError> {
    let connector = registry
        .get(&protocol)
        .ok_or_else(|| AppError::Connection {
            message: format!("Unsupported protocol: {protocol}"),
            advice: "Use one of: sftp, ftp, ftps, webdav, smb, nfs, local.".to_string(),
        })?;

    let entries = connector.list_remote(&path).await?;
    // Remember what was seen, so global search can find it later. Local
    // "remote" listings are already covered by the folder index.
    if protocol != "local" {
        let listing: Vec<_> = entries
            .iter()
            .map(|e| crate::search_engine::ListingEntry {
                name: e.name.clone(),
                path: e.path.clone(),
                is_dir: e.is_dir,
                size: e.size,
                modified_ms: e.modified.map(|t| t.timestamp_millis()).unwrap_or(0),
            })
            .collect();
        let index = search.index_arc();
        tokio::task::spawn_blocking(move || {
            if let Err(e) = index.record_listing(&protocol, &path, &listing) {
                tracing::debug!("Listing not recorded for search: {e}");
            }
        });
    }
    Ok(entries)
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

/// The connector for `protocol`, connected with the saved connection's
/// secret when it isn't already.
pub async fn ensure_connected(
    registry: &ConnectorRegistry,
    connections: &crate::connectors::ConnectionManager,
    protocol: &str,
    connection_id: Option<&str>,
) -> Result<std::sync::Arc<dyn crate::connectors::Connector>, AppError> {
    let connector = registry
        .get(protocol)
        .ok_or_else(|| AppError::validation(format!("Unsupported protocol '{protocol}'")))?;
    if !connector.is_connected() {
        if let Some(cid) = connection_id.filter(|c| !c.is_empty()) {
            let id = uuid::Uuid::parse_str(cid).map_err(|_| AppError::validation(format!("Invalid connection id '{cid}'")))?;
            let profile = connections
                .get_connection(id)
                .await?
                .ok_or_else(|| AppError::validation(format!("Saved connection {cid} not found")))?;
            let secret = connections.get_password(id).await.unwrap_or(None);
            connector
                .connect_with_secret(&profile, secret)
                .await
                .map_err(|e| AppError::connection(format!("Cannot connect to {}: {e}", profile.name), "Check the connection's settings."))?;
        }
    }
    Ok(connector)
}
