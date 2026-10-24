//! Tauri IPC commands for protocol connector operations (T-020, T-021, T-022).
//!
//! These commands expose SFTP, FTP/FTPS, and WebDAV connector operations to the frontend.
//! The ConnectorRegistry is used to dispatch operations to the correct connector.

use crate::connectors::{Connector, ConnectionManager, ConnectorRegistry};
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, ConnectionProtocol, FileEntry};
use tauri::State;

/// Connect to a remote server using the appropriate protocol connector.
// Each IPC argument is a named parameter of the command.
#[allow(clippy::too_many_arguments)]
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
    let request = ConnectRequest { protocol, host, port, username, remote_path };
    let (profile, secrets) = resolve_connect(&manager, request, connection_id.as_deref()).await?;
    // The user asked for this connection: try it even when it is flagged,
    // and record the outcome.
    let result = connector.connect_with_secrets(&profile, secrets).await;
    if connection_id.is_some() {
        manager.record_connect_outcome(profile.id, &result).await;
    }
    result
}

/// What the UI asked to connect to.
pub(crate) struct ConnectRequest {
    pub protocol: String,
    pub host: String,
    pub port: Option<u16>,
    pub username: Option<String>,
    pub remote_path: Option<String>,
}

/// The profile and secrets to connect with: the saved connection when
/// `connection_id` names one (so its connector settings - WebDAV mode, SMB
/// domain, OAuth app, scopes - apply), else an ad-hoc profile.
pub(crate) async fn resolve_connect(
    manager: &ConnectionManager,
    req: ConnectRequest,
    connection_id: Option<&str>,
) -> Result<(ConnectionProfile, crate::connectors::ConnectionSecrets), AppError> {
    if let Some(Ok(id)) = connection_id.map(uuid::Uuid::parse_str) {
        let secrets = manager.get_secrets(id).await.unwrap_or_default();
        if let Some(saved) = manager.get_connection(id).await? {
            return Ok((saved, secrets));
        }
        return Ok((adhoc_profile(req), secrets));
    }
    Ok((adhoc_profile(req), Default::default()))
}

fn adhoc_profile(req: ConnectRequest) -> ConnectionProfile {
    ConnectionProfile {
        id: uuid::Uuid::new_v4(),
        name: format!("{}://{}", req.protocol, req.host),
        protocol: ConnectionProtocol::from_str_lossy(&req.protocol),
        host: req.host,
        port: req.port,
        username: req.username,
        credential_ref: None,
        remote_path: req.remote_path.unwrap_or_else(|| "/".to_string()),
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
        extras: Default::default(),
    }
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
            // An automatic reconnect: skip a connection whose credentials
            // were rejected until the user fixes them.
            let secrets = connections.background_secrets(id, &profile.name).await?;
            let result = connector.connect_with_secrets(&profile, secrets).await;
            connections.record_connect_outcome(id, &result).await;
            result.map_err(|e| match e {
                AppError::Authentication { message, advice } => AppError::Authentication { message: format!("Cannot connect to {}: {message}", profile.name), advice },
                e => AppError::connection(format!("Cannot connect to {}: {e}", profile.name), "Check the connection's settings."),
            })?;
        }
    }
    Ok(connector)
}

/// Connect `connector` with the saved connection's secrets when it is not
/// connected (an interactive request: tried even when flagged).
async fn connect_if_needed(connector: &dyn Connector, manager: &ConnectionManager, connection_id: Option<&str>) -> Result<(), AppError> {
    if connector.is_connected() {
        return Ok(());
    }
    let Some(cid) = connection_id.filter(|c| !c.is_empty()) else {
        return require_connected(connector);
    };
    let id = uuid::Uuid::parse_str(cid).map_err(|_| AppError::validation(format!("Invalid connection id '{cid}'")))?;
    let profile = manager.get_connection(id).await?.ok_or_else(|| AppError::not_found(format!("Saved connection {cid} not found")))?;
    let secrets = manager.get_secrets(id).await?;
    let r = connector.connect_with_secrets(&profile, secrets).await;
    manager.record_connect_outcome(id, &r).await;
    r
}

/// Storage quota (total / used / free / trash) of the account behind a
/// connection; `None` when the protocol has none (SFTP, FTP…).
#[tauri::command]
pub async fn connector_quota(
    protocol: String,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<Option<crate::connectors::RemoteQuota>, AppError> {
    let connector = require_connector(&registry, &protocol)?;
    connect_if_needed(connector.as_ref(), &manager, connection_id.as_deref()).await?;
    connector.quota().await
}

fn size_jobs() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, tokio_util::sync::CancellationToken>> {
    static JOBS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, tokio_util::sync::CancellationToken>>> = std::sync::OnceLock::new();
    JOBS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Total size of a remote folder and everything below it: rclone's
/// server-side `size`, else a walk of the listing (8 folders at a time).
/// `job_id` lets `connector_folder_size_cancel` stop it (partial figures,
/// `complete: false`).
#[tauri::command]
pub async fn connector_folder_size(
    protocol: String,
    path: String,
    job_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
) -> Result<crate::connectors::FolderSize, AppError> {
    let connector = require_connector(&registry, &protocol)?;
    require_connected(connector.as_ref())?;
    let cancel = tokio_util::sync::CancellationToken::new();
    if let Some(id) = &job_id {
        size_jobs().insert(id.clone(), cancel.clone());
    }
    let result = crate::connectors::folder_size::folder_size(connector, &path, crate::connectors::folder_size::DEFAULT_CONCURRENCY, cancel).await;
    if let Some(id) = &job_id {
        size_jobs().remove(id);
    }
    result
}

/// Stop a running `connector_folder_size`; `false` when it already ended.
#[tauri::command]
pub async fn connector_folder_size_cancel(job_id: String) -> Result<bool, AppError> {
    Ok(match size_jobs().remove(&job_id) {
        Some(t) => {
            t.cancel();
            true
        }
        None => false,
    })
}

/// rclone remotes from the user's rclone config (`rclone listremotes`).
#[tauri::command]
pub async fn rclone_list_remotes() -> Result<Vec<crate::connectors::rclone::RcloneRemote>, AppError> {
    crate::connectors::rclone::list_remotes().await
}

/// Provider presets for the connection form (S3-compatible stores,
/// hosted SFTP services).
#[tauri::command]
pub async fn connection_presets() -> Result<Vec<crate::connectors::presets::ProviderPreset>, AppError> {
    Ok(crate::connectors::presets::PRESETS.to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::{WebDavAuthMode, WebDavSettings};
    use crate::security::CredentialStore;

    /// Regression: `connector_connect` rebuilt the profile from host/port/
    /// user, dropping the saved connector settings (WebDAV mode, SMB domain,
    /// OAuth app, scopes) and the SFTP key passphrase.
    #[tokio::test]
    async fn connecting_a_saved_connection_uses_its_settings_and_secrets() {
        let repo = crate::storage::Repository::open_in_memory().await.unwrap();
        let mgr = ConnectionManager::new(repo, std::sync::Arc::new(CredentialStore { cache: Default::default(), keychain_available: false, file_vault: None }));
        let mut p = crate::connectors::live_tests::profile(ConnectionProtocol::WebDav, "dav.example", None, "u", "/");
        p.extras.webdav = Some(WebDavSettings { auth: WebDavAuthMode::Bearer, ..Default::default() });
        mgr.save_connection_with_secrets(p.clone(), Some("token".into()), Some("pp".into())).await.unwrap();
        let req = || ConnectRequest { protocol: "webdav".into(), host: "dav.example".into(), port: None, username: Some("u".into()), remote_path: None };
        let (profile, secrets) = resolve_connect(&mgr, req(), Some(&p.id.to_string())).await.unwrap();
        assert_eq!(profile.extras, p.extras);
        assert_eq!((secrets.secret.as_deref(), secrets.key_passphrase.as_deref()), (Some("token"), Some("pp")));
        // Quick connect without a saved connection: an ad-hoc profile, no secrets.
        let (adhoc, none) = resolve_connect(&mgr, req(), None).await.unwrap();
        assert_eq!((adhoc.protocol, adhoc.extras.webdav), (ConnectionProtocol::WebDav, None));
        assert_eq!(none, crate::connectors::ConnectionSecrets::default());
    }
}
