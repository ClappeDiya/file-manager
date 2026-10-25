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
///
/// `connection_id` names the saved connection the caller is browsing: the
/// action then runs signed in as that account even if the shared instance
/// holds another one (see [`instance_for`]).
#[tauri::command]
pub async fn connector_upload(
    protocol: String,
    local_path: String,
    remote_path: String,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<u64, AppError> {
    let connector = user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?;
    connector.upload(&local_path, &remote_path).await
}

/// Download a remote file from the connected server; returns bytes written.
///
/// `connection_id` names the saved connection the caller is browsing: the
/// action then runs signed in as that account even if the shared instance
/// holds another one (see [`instance_for`]).
#[tauri::command]
pub async fn connector_download(
    protocol: String,
    remote_path: String,
    local_path: String,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<u64, AppError> {
    let connector = user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?;
    connector.download(&remote_path, &local_path).await
}

/// Create a directory on the connected server.
///
/// `connection_id` names the saved connection the caller is browsing: the
/// action then runs signed in as that account even if the shared instance
/// holds another one (see [`instance_for`]).
#[tauri::command]
pub async fn connector_make_directory(
    protocol: String,
    remote_path: String,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<(), AppError> {
    let connector = user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?;
    connector.make_directory(&remote_path).await
}

/// Delete a file or empty directory on the connected server.
///
/// `connection_id` names the saved connection the caller is browsing: the
/// action then runs signed in as that account even if the shared instance
/// holds another one (see [`instance_for`]).
#[tauri::command]
pub async fn connector_remove(
    protocol: String,
    remote_path: String,
    is_dir: bool,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<(), AppError> {
    let connector = user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?;
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
///
/// `connection_id` names the saved connection the caller is browsing: the
/// action then runs signed in as that account even if the shared instance
/// holds another one (see [`instance_for`]).
#[tauri::command]
pub async fn connector_list_remote(
    protocol: String,
    path: String,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
    search: State<'_, crate::search_engine::SearchManager>,
) -> Result<Vec<FileEntry>, AppError> {
    let connector = match connection_id.as_deref().filter(|c| !c.is_empty()) {
        Some(_) => user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?,
        None => registry.get(&protocol).ok_or_else(|| AppError::Connection {
            message: format!("Unsupported protocol: {protocol}"),
            advice: "Use one of: sftp, ftp, ftps, webdav, smb, nfs, local.".to_string(),
        })?,
    };

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

/// The connector to run background work (an automation rule) for saved
/// connection `connection_id` on. A connection whose credentials were
/// rejected is skipped until the user fixes them. See [`instance_for`]: the
/// shared instance the user is browsing is never re-signed as another
/// account.
pub async fn ensure_connected(
    registry: &ConnectorRegistry,
    connections: &crate::connectors::ConnectionManager,
    protocol: &str,
    connection_id: Option<&str>,
) -> Result<std::sync::Arc<dyn crate::connectors::Connector>, AppError> {
    let connector = registry
        .get(protocol)
        .ok_or_else(|| AppError::validation(format!("Unsupported protocol '{protocol}'")))?;
    match connection_id.filter(|c| !c.is_empty()) {
        Some(cid) => instance_for(connector, |c| c.detached(), connections, cid, Caller::Background).await,
        None => Ok(connector),
    }
}

fn parse_connection_id(cid: &str) -> Result<uuid::Uuid, AppError> {
    uuid::Uuid::parse_str(cid).map_err(|_| AppError::validation(format!("Invalid connection id '{cid}'")))
}

/// Who wants a connection: the user (tried even when it is flagged as
/// credentials-rejected) or background work (a flagged one is skipped).
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Caller {
    User,
    Background,
}

/// The instance to run an action for saved connection `cid` on:
/// - the shared `connector` when it already serves that connection;
/// - for connectors that track which account they are signed in as
///   (`detach` gives a fresh instance), a dedicated instance signed in as
///   it, leaving the shared one alone: one shared instance serves the
///   Connections panel, so re-signing it would make the user's next
///   delete or upload hit another account;
/// - otherwise the shared one, connected.
///
/// Only an explicit connect (`connector_connect`, a sign-in) changes the
/// account the shared instance holds. The outcome is recorded on the
/// connection (rejected-credentials flag).
pub(crate) async fn instance_for<C: Connector + ?Sized>(
    connector: std::sync::Arc<C>,
    detach: impl FnOnce(&C) -> Option<std::sync::Arc<C>>,
    manager: &ConnectionManager,
    cid: &str,
    caller: Caller,
) -> Result<std::sync::Arc<C>, AppError> {
    let id = parse_connection_id(cid)?;
    if crate::connectors::connected_as(connector.as_ref(), Some(id)) {
        return Ok(connector);
    }
    let profile = manager.get_connection(id).await?.ok_or_else(|| AppError::not_found(format!("Saved connection {cid} not found")))?;
    let secrets = match caller {
        Caller::User => manager.get_secrets(id).await?,
        Caller::Background => manager.background_secrets(id, &profile.name).await?,
    };
    let target = detach(connector.as_ref()).unwrap_or(connector);
    let result = target.connect_with_secrets(&profile, secrets).await;
    manager.record_connect_outcome(id, &result).await;
    match (caller, result) {
        (_, Ok(())) => Ok(target),
        (Caller::User, Err(e)) => Err(e),
        (Caller::Background, Err(AppError::Authentication { message, advice })) => {
            Err(AppError::Authentication { message: format!("Cannot connect to {}: {message}", profile.name), advice })
        }
        (Caller::Background, Err(e)) => Err(AppError::connection(format!("Cannot connect to {}: {e}", profile.name), "Check the connection's settings.")),
    }
}

/// The registry connector for `protocol`, serving saved connection
/// `connection_id` for a user action ([`instance_for`]); with no
/// connection named, the shared instance as it is (must be connected).
pub(crate) async fn user_instance(
    registry: &ConnectorRegistry,
    manager: &ConnectionManager,
    protocol: &str,
    connection_id: Option<&str>,
) -> Result<std::sync::Arc<dyn Connector>, AppError> {
    let connector = require_connector(registry, protocol)?;
    match connection_id.filter(|c| !c.is_empty()) {
        Some(cid) => instance_for(connector, |c| c.detached(), manager, cid, Caller::User).await,
        None => {
            require_connected(connector.as_ref())?;
            Ok(connector)
        }
    }
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
    user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?.quota().await
}

fn size_jobs() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, tokio_util::sync::CancellationToken>> {
    static JOBS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, tokio_util::sync::CancellationToken>>> = std::sync::OnceLock::new();
    JOBS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

/// Total size of a remote folder and everything below it: rclone's
/// server-side `size`, else a walk of the listing (8 folders at a time).
/// `job_id` lets `connector_folder_size_cancel` stop it (partial figures,
/// `complete: false`). `connection_id`: as for `connector_list_remote`.
#[tauri::command]
pub async fn connector_folder_size(
    protocol: String,
    path: String,
    job_id: Option<String>,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<crate::connectors::FolderSize, AppError> {
    let connector = user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?;
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

/// Rename or move `from_path` to `to_path` within one connection, on the
/// server (panes: rename, and moves inside one connection). Returns the
/// item's path afterwards: `to_path`, except where the connector addresses
/// items by id (Google Drive: the new folder's path plus the item's id).
#[tauri::command]
pub async fn connector_rename(
    protocol: String,
    from_path: String,
    to_path: String,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<String, AppError> {
    if from_path.trim_end_matches('/') == to_path.trim_end_matches('/') {
        return Ok(from_path);
    }
    let connector = user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?;
    rename_at(connector.as_ref(), &from_path, &to_path).await
}

/// Refuses a `to` another item already has, as local `rename_file` does:
/// several connectors' renames would replace it (SFTP renames with
/// OVERWRITE, S3 copies over the key, rclone `moveto`, ADLS without
/// If-None-Match, many FTP servers on RNTO).
pub(crate) async fn rename_at(connector: &dyn Connector, from: &str, to: &str) -> Result<String, AppError> {
    if crate::transfer_engine::location::rename_target_taken(connector, from, to).await? {
        let name = to.trim_end_matches('/').rsplit('/').next().unwrap_or(to);
        return Err(AppError::file_op(format!("A file named '{name}' already exists"), "Choose a different name."));
    }
    connector.rename_entry(from, to).await?;
    Ok(connector.renamed_path(from, to))
}

/// The entry at `path` (from its folder's listing); `None` when there is
/// none. Panes use it to validate typed addresses.
#[tauri::command]
pub async fn connector_stat(
    protocol: String,
    path: String,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<Option<FileEntry>, AppError> {
    let connector = user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?;
    stat_entry(connector.as_ref(), &path).await
}

/// The entry at `path` as its folder lists it: its `path` is where a pane
/// opens it. A typed path may name folders by name where the connector
/// addresses them otherwise (Google Drive: `/Reports/Q1` is `/<id>/<id>`),
/// so when the parent cannot be listed as typed, or does not hold the
/// item, the path is resolved folder by folder from the top.
pub(crate) async fn stat_entry(connector: &dyn Connector, path: &str) -> Result<Option<FileEntry>, AppError> {
    let trimmed = path.trim_end_matches('/');
    if trimmed.is_empty() {
        // The root always exists once connected.
        return Ok(Some(FileEntry { name: "/".into(), path: "/".into(), is_dir: true, is_symlink: false, size: 0, modified: None, created: None, is_hidden: false, extension: None, permissions: None }));
    }
    let segments: Vec<&str> = trimmed.split('/').filter(|s| !s.is_empty()).collect();
    let direct_err = match connector.list_remote(&crate::transfer_engine::location::remote_parent(trimmed)).await {
        Ok(listing) => match find_listed(listing, trimmed, segments.last().copied().unwrap_or_default()) {
            Some(entry) => return Ok(Some(entry)),
            None => None,
        },
        Err(e) => Some(e),
    };
    if segments.len() < 2 {
        // A top-level item: its folder (the root) was listed as typed.
        return direct_err.map_or(Ok(None), Err);
    }
    match resolve_by_names(connector, &segments).await {
        Ok(found) => Ok(found),
        // The typed parent was listed fine: the item is simply not there.
        Err(e) => direct_err.map_or(Ok(None), |_| Err(e)),
    }
}

/// Walk `segments` from the root, each one matched by path or by name in
/// its folder's listing.
async fn resolve_by_names(connector: &dyn Connector, segments: &[&str]) -> Result<Option<FileEntry>, AppError> {
    let mut dir = "/".to_string();
    let mut found = None;
    for (i, seg) in segments.iter().enumerate() {
        let typed = format!("/{}", segments[..=i].join("/"));
        let Some(entry) = find_listed(connector.list_remote(&dir).await?, &typed, seg) else {
            return Ok(None);
        };
        if i + 1 < segments.len() && !entry.is_dir {
            return Ok(None);
        }
        dir = entry.path.clone();
        found = Some(entry);
    }
    Ok(found)
}

/// The entry of `listing` at `path`, else the one called `name`: listings
/// may spell paths their own way (OneDrive `/drive/root:/Docs` for a typed
/// `/Docs`, Google Drive ids).
fn find_listed(listing: Vec<FileEntry>, path: &str, name: &str) -> Option<FileEntry> {
    let by_path = listing.iter().position(|e| e.path.trim_end_matches('/') == path);
    let at = by_path.or_else(|| listing.iter().position(|e| e.name == name));
    at.map(|i| listing[i].clone())
}

/// Largest remote file `connector_fetch_preview` downloads.
pub const PREVIEW_MAX_BYTES: u64 = 25 * 1024 * 1024;

/// Download a remote file into the preview cache (the app cache folder) so
/// the local previewers and the default app can open it; returns the local
/// copy's path. `size` is the listed size: larger than `max_bytes`
/// (default [`PREVIEW_MAX_BYTES`]) is refused before anything downloads.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn connector_fetch_preview(
    protocol: String,
    remote_path: String,
    name: String,
    size: Option<u64>,
    max_bytes: Option<u64>,
    connection_id: Option<String>,
    registry: State<'_, ConnectorRegistry>,
    manager: State<'_, ConnectionManager>,
) -> Result<String, AppError> {
    let limit = max_bytes.unwrap_or(PREVIEW_MAX_BYTES).min(PREVIEW_MAX_BYTES);
    if size.unwrap_or(0) > limit {
        return Err(AppError::validation(format!("{name} is too large to open from the connection ({} MB at most)", limit / (1024 * 1024))));
    }
    let connector = user_instance(&registry, &manager, &protocol, connection_id.as_deref()).await?;
    fetch_preview(connector.as_ref(), &remote_path, &name, connection_id.as_deref().unwrap_or(&protocol)).await
}

/// The preview-cache file for `remote_path` of `scope` (a connection id):
/// one folder per remote path, so two files with one name never collide.
/// The cache is in this user's app cache folder (never the shared
/// temporary folder, where another user could plant links).
pub(crate) fn preview_cache_path(scope: &str, remote_path: &str, name: &str) -> std::path::PathBuf {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (scope, remote_path).hash(&mut h);
    let safe: String = name.chars().map(|c| if matches!(c, '/' | '\\' | ':' | '\0') { '_' } else { c }).collect();
    crate::paths::cache_dir().join("remote-previews").join(format!("{:016x}", h.finish())).join(if safe.is_empty() || safe == "." || safe == ".." { "file".into() } else { safe })
}

pub(crate) async fn fetch_preview(connector: &dyn Connector, remote_path: &str, name: &str, scope: &str) -> Result<String, AppError> {
    let local = preview_cache_path(scope, remote_path, name);
    if let Some(dir) = local.parent() {
        tokio::fs::create_dir_all(dir).await.map_err(|e| AppError::file_op(format!("Cannot create the preview folder: {e}"), "Free some disk space and retry."))?;
    }
    let local_str = local.to_string_lossy().to_string();
    let staging = crate::transfer_engine::location::local_staging_path(&local_str);
    // Never write through something already there (a leftover or a link).
    let _ = tokio::fs::remove_file(&staging).await;
    if let Err(e) = connector.download(remote_path, &staging).await {
        let _ = tokio::fs::remove_file(&staging).await;
        return Err(e);
    }
    tokio::fs::rename(&staging, &local).await.map_err(|e| AppError::file_op(format!("Cannot place the preview copy: {e}"), "Retry."))?;
    Ok(local_str)
}

/// Signs the transfer queue in to saved connections: the protocol comes from
/// the saved profile, the instance from [`user_instance`] (the user queued
/// the transfer).
pub struct PaneResolver {
    registry: ConnectorRegistry,
    connections: ConnectionManager,
}

impl PaneResolver {
    pub fn new(registry: ConnectorRegistry, connections: ConnectionManager) -> Self {
        Self { registry, connections }
    }
}

impl crate::transfer_engine::location::RemoteResolver for PaneResolver {
    fn connector(&self, connection_id: uuid::Uuid) -> crate::core::traits::BoxFuture<'_, Result<std::sync::Arc<dyn Connector>, AppError>> {
        Box::pin(async move {
            let profile = self
                .connections
                .get_connection(connection_id)
                .await?
                .ok_or_else(|| AppError::not_found(format!("Saved connection {connection_id} not found")))?;
            user_instance(&self.registry, &self.connections, profile.protocol.as_str(), Some(&connection_id.to_string())).await
        })
    }
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

    /// Regression: a Google Drive address typed by names
    /// (`Work Drive:/Reports/Q1`) failed (Drive paths are folder ids, so
    /// listing "/Reports" is a 404) or, one level deep, validated and then
    /// opened "Reports" as a folder id. It now resolves folder by folder to
    /// the id path the pane opens.
    #[tokio::test(flavor = "multi_thread")]
    async fn stat_resolves_names_to_google_drive_id_paths() {
        let drive = crate::connectors::google_drive::fake_drive::start(&[("r1", "Reports", "root", true), ("q1", "Q1", "r1", true), ("n1", "notes.txt", "q1", false)]).await;
        let c = drive.connector.as_ref();
        assert_eq!(stat_entry(c, "/Reports").await.unwrap().unwrap().path, "/r1");
        let q1 = stat_entry(c, "/Reports/Q1/").await.unwrap().unwrap();
        assert_eq!((q1.path.as_str(), q1.is_dir), ("/r1/q1", true));
        assert_eq!(stat_entry(c, "/Reports/Q1/notes.txt").await.unwrap().unwrap().path, "/r1/q1/n1");
        assert_eq!(stat_entry(c, "/r1/q1").await.unwrap().unwrap().path, "/r1/q1", "id paths resolve as they are");
        assert!(stat_entry(c, "/Reports/Nope").await.unwrap().is_none());
        assert!(stat_entry(c, "/Nope/Q1").await.unwrap().is_none());
    }

    /// A pane rename never replaces an existing item (several connectors'
    /// renames overwrite: SFTP OVERWRITE, S3 copy + delete, rclone moveto),
    /// as local `rename_file` refuses an existing target. A case-only rename
    /// of the item itself stays allowed.
    #[tokio::test]
    async fn rename_refuses_an_existing_target() {
        use crate::transfer_engine::location::tests::MemConnector;
        let c = MemConnector::with(&[("/d/a.txt", "aaa"), ("/d/b.txt", "bbb"), ("/d/sub/x.txt", "x")], &["/d", "/d/sub", "/d/old"]);
        let err = rename_at(&c, "/d/a.txt", "/d/b.txt").await.unwrap_err();
        assert!(err.to_string().contains("A file named 'b.txt' already exists"), "{err}");
        assert_eq!(c.file("/d/a.txt").as_deref(), Some("aaa"), "the source is untouched");
        assert_eq!(c.file("/d/b.txt").as_deref(), Some("bbb"), "the existing item is not overwritten");
        assert!(c.renames.lock().unwrap().is_empty(), "the connector was never asked to rename");
        // Case-insensitive servers (Windows FTP, OneDrive, Dropbox) would
        // replace b.txt with a rename to B.TXT: refused everywhere.
        assert!(rename_at(&c, "/d/a.txt", "/d/B.TXT").await.is_err());
        assert_eq!(c.file("/d/b.txt").as_deref(), Some("bbb"));
        // Onto a folder of that name, too.
        assert!(rename_at(&c, "/d/a.txt", "/d/sub").await.is_err());
        assert!(rename_at(&c, "/d/old", "/d/sub").await.is_err());
        assert_eq!(c.file("/d/sub/x.txt").as_deref(), Some("x"));
        // A free name, and a case-only rename of the same item, still work.
        assert_eq!(rename_at(&c, "/d/a.txt", "/d/A.txt").await.unwrap(), "/d/A.txt");
        assert_eq!(rename_at(&c, "/d/A.txt", "/d/c.txt").await.unwrap(), "/d/c.txt");
        assert_eq!(c.file("/d/c.txt").as_deref(), Some("aaa"));
        assert_eq!(c.file("/d/b.txt").as_deref(), Some("bbb"));
    }

    /// `connector_stat` and `connector_fetch_preview` against an in-memory
    /// connection: entries by path (or by name), the root, missing paths;
    /// previews land under the final name only when complete.
    #[tokio::test]
    async fn stat_and_fetch_preview() {
        use crate::transfer_engine::location::tests::MemConnector;
        let c = MemConnector::with(&[("/docs/a.txt", "hello")], &["/docs"]);
        assert!(stat_entry(&c, "/").await.unwrap().unwrap().is_dir);
        assert_eq!(stat_entry(&c, "/docs/a.txt").await.unwrap().unwrap().size, 5);
        assert!(stat_entry(&c, "/docs").await.unwrap().unwrap().is_dir);
        assert!(stat_entry(&c, "/docs/").await.unwrap().is_some(), "a trailing slash is the same folder");
        assert!(stat_entry(&c, "/docs/b.txt").await.unwrap().is_none());

        let scope = uuid::Uuid::new_v4().to_string();
        let local = fetch_preview(&c, "/docs/a.txt", "a.txt", &scope).await.unwrap();
        assert_eq!(std::fs::read_to_string(&local).unwrap(), "hello");
        assert!(local.ends_with("a.txt"));
        // In this user's app cache, not the shared temporary folder.
        assert!(std::path::Path::new(&local).starts_with(crate::paths::cache_dir()), "{local}");
        assert!(!std::path::Path::new(&local).starts_with(std::env::temp_dir()), "{local}");
        assert_ne!(preview_cache_path(&scope, "/docs/a.txt", "a.txt"), preview_cache_path(&scope, "/other/a.txt", "a.txt"));
        assert!(preview_cache_path(&scope, "/x", "../../etc").file_name().unwrap() == ".._.._etc", "names cannot leave the cache folder");
        for dots in [".", ".."] {
            let p = preview_cache_path(&scope, "/x", dots);
            assert_eq!(p.file_name().and_then(|n| n.to_str()), Some("file"), "{dots:?} names a real file in the cache folder");
            assert_eq!(p.parent(), preview_cache_path(&scope, "/x", "a").parent());
        }
        assert!(fetch_preview(&c, "/docs/missing.txt", "m.txt", &scope).await.is_err());
        let _ = std::fs::remove_dir_all(std::path::Path::new(&local).parent().unwrap());
    }
}
