//! Tauri IPC commands for settings export/import and cross-installation sync.
//!
//! Feature 1: Full Settings Export/Import
//! - Export all settings as JSON bundle (optionally AES-256-GCM encrypted)
//! - Import settings from bundle with validation
//! - Settings summary for UI display
//!
//! Feature 2: Site Synchronization Across Installations
//! - Set/get a shared directory for settings sync (e.g., Dropbox folder)
//! - Sync settings to/from the shared directory
//! - Sync status tracking (last sync time, conflicts)

use crate::connectors::ConnectionManager;
use crate::core::error::AppError;
use crate::core::traits::StorageOperations;
use crate::security::encryption;
use crate::security::kdf;
use crate::storage::Repository;
use crate::sync_engine::SyncManager;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::State;

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

/// Magic header for encrypted settings bundles.
const SETTINGS_MAGIC: &[u8; 8] = b"UFOPSET\x01";

/// Complete settings bundle format.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsBundle {
    pub version: u32,
    pub exported_at: DateTime<Utc>,
    pub connections: Vec<serde_json::Value>,
    pub connection_groups: Vec<serde_json::Value>,
    pub sync_pairs: Vec<serde_json::Value>,
    pub configs: Vec<ConfigEntry>,
    pub workspace_state: serde_json::Value,
}

/// A key-value config entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigEntry {
    pub key: String,
    pub value: String,
}

/// Summary of current settings for display.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsSummary {
    pub num_connections: u32,
    pub num_sync_pairs: u32,
    pub num_configs: u32,
}

/// Result of importing settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsImportResult {
    pub imported_connections: u32,
    pub imported_connection_groups: u32,
    pub imported_sync_pairs: u32,
    pub imported_configs: u32,
    pub imported_workspace: bool,
}

/// Status of settings sync.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncStatus {
    pub sync_dir: Option<String>,
    pub last_synced: Option<String>,
    pub status: String,
    pub auto_sync_on_startup: bool,
}

// ──────────────────────────────────────────────
// Feature 1: Settings Export/Import
// ──────────────────────────────────────────────

/// Collect all settings into a JSON bundle and optionally encrypt with password.
/// Returns base64-encoded data.
#[tauri::command]
pub async fn export_all_settings(
    password: Option<String>,
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
    sync_mgr: State<'_, SyncManager>,
) -> Result<String, AppError> {
    tracing::info!("Exporting all settings");

    // Collect connections via export (strips credentials)
    let connections_json = connection_mgr.export_connections().await?;
    let connections_export: serde_json::Value =
        serde_json::from_str(&connections_json).unwrap_or(serde_json::Value::Null);

    let connections = connections_export
        .get("connections")
        .cloned()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let connection_groups = connections_export
        .get("groups")
        .cloned()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    // Collect sync pairs
    let sync_pairs_list = {
        use crate::core::traits::SyncOperations;
        sync_mgr.list_pairs().await.unwrap_or_default()
    };
    let sync_pairs: Vec<serde_json::Value> = sync_pairs_list
        .iter()
        .filter_map(|p| serde_json::to_value(p).ok())
        .collect();

    // Collect config key-values
    let configs = collect_known_configs(repo.inner()).await;

    // Collect workspace state
    let workspace_state = StorageOperations::load_workspace_state(repo.inner())
        .await
        .ok()
        .and_then(|ws| serde_json::to_value(ws).ok())
        .unwrap_or(serde_json::Value::Null);

    let bundle = SettingsBundle {
        version: 1,
        exported_at: Utc::now(),
        connections,
        connection_groups,
        sync_pairs,
        configs,
        workspace_state,
    };

    let json_bytes = serde_json::to_string_pretty(&bundle).map_err(|e| {
        AppError::state(
            format!("Failed to serialize settings: {e}"),
            "Try again.",
        )
    })?;

    // Optionally encrypt with password
    let output_bytes = match password {
        Some(pwd) if !pwd.is_empty() => {
            let salt = kdf::Salt::generate();
            let kdf_params = kdf::KdfParams::fast(); // fast for settings encryption
            let key = kdf::derive_key(
                &kdf::Password::new(pwd),
                &salt,
                &kdf_params,
            )?;

            let encrypted = encryption::encrypt(&key, json_bytes.as_bytes(), None)?;
            let encrypted_bytes = encrypted.to_bytes();

            // Wire format: [MAGIC:8][salt:16][nonce+ciphertext]
            let mut out = Vec::with_capacity(
                SETTINGS_MAGIC.len() + salt.bytes.len() + encrypted_bytes.len(),
            );
            out.extend_from_slice(SETTINGS_MAGIC);
            out.extend_from_slice(&salt.bytes);
            out.extend_from_slice(&encrypted_bytes);
            out
        }
        _ => {
            // Unencrypted: just JSON bytes
            json_bytes.into_bytes()
        }
    };

    let encoded = base64::engine::general_purpose::STANDARD.encode(&output_bytes);
    tracing::info!("Settings exported ({} bytes encoded)", encoded.len());
    Ok(encoded)
}

/// Import settings from base64-encoded data. Decrypts if password provided.
/// Returns count of imported items.
#[tauri::command]
pub async fn import_all_settings(
    data: String,
    password: Option<String>,
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
    sync_mgr: State<'_, SyncManager>,
) -> Result<SettingsImportResult, AppError> {
    tracing::info!("Importing settings");

    let raw_bytes = base64::engine::general_purpose::STANDARD
        .decode(&data)
        .map_err(|e| {
            AppError::state(
                format!("Invalid settings data encoding: {e}"),
                "The file may be corrupted. Try re-exporting.",
            )
        })?;

    // Determine if encrypted or plaintext
    let json_str = if raw_bytes.len() >= SETTINGS_MAGIC.len()
        && &raw_bytes[..SETTINGS_MAGIC.len()] == SETTINGS_MAGIC
    {
        // Encrypted format
        let pwd = password.ok_or_else(|| {
            AppError::encryption(
                "Password required",
                "This settings bundle is encrypted. Please provide a password.",
            )
        })?;

        let salt_start = SETTINGS_MAGIC.len();
        let salt_end = salt_start + kdf::SALT_SIZE;
        if raw_bytes.len() < salt_end + encryption::NONCE_SIZE + encryption::TAG_SIZE {
            return Err(AppError::state(
                "Settings data is too short",
                "The file may be corrupted.",
            ));
        }

        let salt = kdf::Salt::from_bytes(raw_bytes[salt_start..salt_end].to_vec())?;
        let kdf_params = kdf::KdfParams::fast();
        let key = kdf::derive_key(&kdf::Password::new(pwd), &salt, &kdf_params)?;

        let encrypted_data =
            encryption::EncryptedData::from_bytes(&raw_bytes[salt_end..])?;
        let decrypted = encryption::decrypt(&key, &encrypted_data, None)?;

        String::from_utf8(decrypted).map_err(|e| {
            AppError::state(
                format!("Decrypted data is not valid UTF-8: {e}"),
                "Wrong password or corrupted data.",
            )
        })?
    } else {
        // Plain JSON
        String::from_utf8(raw_bytes).map_err(|e| {
            AppError::state(
                format!("Settings data is not valid UTF-8: {e}"),
                "The file may be corrupted.",
            )
        })?
    };

    let bundle: SettingsBundle = serde_json::from_str(&json_str).map_err(|e| {
        AppError::state(
            format!("Invalid settings bundle format: {e}"),
            "Check that the file is a valid UFOP settings export.",
        )
    })?;

    let mut result = SettingsImportResult {
        imported_connections: 0,
        imported_connection_groups: 0,
        imported_sync_pairs: 0,
        imported_configs: 0,
        imported_workspace: false,
    };

    // Import connection groups first, then connections (via connection_manager's import)
    let connection_export = serde_json::json!({
        "version": 1,
        "exported_at": bundle.exported_at.to_rfc3339(),
        "connections": bundle.connections,
        "groups": bundle.connection_groups,
    });
    let import_json = serde_json::to_string(&connection_export).unwrap_or_default();
    if let Ok(conn_result) = connection_mgr.import_connections(&import_json).await {
        result.imported_connections = conn_result.imported_connections;
        result.imported_connection_groups = conn_result.imported_groups;
    }

    // Import sync pairs
    for pair_value in &bundle.sync_pairs {
        if let Ok(pair) = serde_json::from_value::<crate::core::types::SyncPair>(pair_value.clone())
        {
            use crate::core::traits::SyncOperations;
            if sync_mgr.create_pair(pair).await.is_ok() {
                result.imported_sync_pairs += 1;
            }
        }
    }

    // Import configs
    for config in &bundle.configs {
        if StorageOperations::set_config(repo.inner(), &config.key, &config.value)
            .await
            .is_ok()
        {
            result.imported_configs += 1;
        }
    }

    // Import workspace state
    if bundle.workspace_state != serde_json::Value::Null {
        if let Ok(ws) =
            serde_json::from_value::<crate::core::types::WorkspaceState>(bundle.workspace_state)
        {
            if StorageOperations::save_workspace_state(repo.inner(), &ws)
                .await
                .is_ok()
            {
                result.imported_workspace = true;
            }
        }
    }

    tracing::info!(
        "Settings imported: {} connections, {} groups, {} sync pairs, {} configs, workspace={}",
        result.imported_connections,
        result.imported_connection_groups,
        result.imported_sync_pairs,
        result.imported_configs,
        result.imported_workspace
    );

    Ok(result)
}

/// Return counts of settings for summary display.
#[tauri::command]
pub async fn get_settings_summary(
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
    sync_mgr: State<'_, SyncManager>,
) -> Result<SettingsSummary, AppError> {
    let connections = connection_mgr
        .list_connections()
        .await
        .map(|v| v.len() as u32)
        .unwrap_or(0);

    let sync_pairs = {
        use crate::core::traits::SyncOperations;
        sync_mgr
            .list_pairs()
            .await
            .map(|v| v.len() as u32)
            .unwrap_or(0)
    };

    let configs = count_configs(repo.inner()).await;

    Ok(SettingsSummary {
        num_connections: connections,
        num_sync_pairs: sync_pairs,
        num_configs: configs,
    })
}

// ──────────────────────────────────────────────
// Feature 2: Settings Sync Across Installations
// ──────────────────────────────────────────────

const SYNC_DIR_CONFIG_KEY: &str = "settings_sync_dir";
const SYNC_LAST_TIME_KEY: &str = "settings_sync_last_time";
const SYNC_AUTO_STARTUP_KEY: &str = "settings_sync_auto_startup";
const SYNC_FILE_NAME: &str = "ufop_settings_sync.json";

/// Set the shared directory for settings synchronization.
#[tauri::command]
pub async fn set_settings_sync_dir(
    path: String,
    repo: State<'_, Repository>,
) -> Result<(), AppError> {
    // Verify directory exists
    let dir_path = std::path::Path::new(&path);
    if !dir_path.is_dir() {
        return Err(AppError::state(
            format!("Directory does not exist: {path}"),
            "Choose an existing directory for settings sync.",
        ));
    }

    StorageOperations::set_config(repo.inner(), SYNC_DIR_CONFIG_KEY, &path).await?;
    tracing::info!("Settings sync directory set to: {path}");
    Ok(())
}

/// Get the current settings sync directory.
#[tauri::command]
pub async fn get_settings_sync_dir(
    repo: State<'_, Repository>,
) -> Result<Option<String>, AppError> {
    StorageOperations::get_config(repo.inner(), SYNC_DIR_CONFIG_KEY).await
}

/// Export (sync) settings to the configured sync directory.
#[tauri::command]
pub async fn sync_settings_to_dir(
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
    sync_mgr: State<'_, SyncManager>,
) -> Result<(), AppError> {
    let sync_dir = StorageOperations::get_config(repo.inner(), SYNC_DIR_CONFIG_KEY)
        .await?
        .ok_or_else(|| {
            AppError::state(
                "No sync directory configured",
                "Set a sync directory first via Settings.",
            )
        })?;

    // Collect settings (reuse export logic, unencrypted for sync)
    let connections_json = connection_mgr.export_connections().await?;
    let connections_export: serde_json::Value =
        serde_json::from_str(&connections_json).unwrap_or(serde_json::Value::Null);

    let connections = connections_export
        .get("connections")
        .cloned()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let connection_groups = connections_export
        .get("groups")
        .cloned()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();

    let sync_pairs_list = {
        use crate::core::traits::SyncOperations;
        sync_mgr.list_pairs().await.unwrap_or_default()
    };
    let sync_pairs: Vec<serde_json::Value> = sync_pairs_list
        .iter()
        .filter_map(|p| serde_json::to_value(p).ok())
        .collect();

    let configs = collect_known_configs(repo.inner()).await;

    let workspace_state = StorageOperations::load_workspace_state(repo.inner())
        .await
        .ok()
        .and_then(|ws| serde_json::to_value(ws).ok())
        .unwrap_or(serde_json::Value::Null);

    let bundle = SettingsBundle {
        version: 1,
        exported_at: Utc::now(),
        connections,
        connection_groups,
        sync_pairs,
        configs,
        workspace_state,
    };

    let json_str = serde_json::to_string_pretty(&bundle).map_err(|e| {
        AppError::state(
            format!("Failed to serialize settings for sync: {e}"),
            "Try again.",
        )
    })?;

    let file_path = std::path::Path::new(&sync_dir).join(SYNC_FILE_NAME);
    tokio::fs::write(&file_path, json_str.as_bytes())
        .await
        .map_err(|e| {
            AppError::state(
                format!("Failed to write sync file: {e}"),
                "Check that you have write permissions to the sync directory.",
            )
        })?;

    // Update last sync time
    let now = Utc::now().to_rfc3339();
    StorageOperations::set_config(repo.inner(), SYNC_LAST_TIME_KEY, &now).await?;

    tracing::info!("Settings synced to: {}", file_path.display());
    Ok(())
}

/// Import (sync) settings from the configured sync directory if newer.
#[tauri::command]
pub async fn sync_settings_from_dir(
    repo: State<'_, Repository>,
    connection_mgr: State<'_, ConnectionManager>,
    sync_mgr: State<'_, SyncManager>,
) -> Result<SettingsImportResult, AppError> {
    let sync_dir = StorageOperations::get_config(repo.inner(), SYNC_DIR_CONFIG_KEY)
        .await?
        .ok_or_else(|| {
            AppError::state(
                "No sync directory configured",
                "Set a sync directory first via Settings.",
            )
        })?;

    let file_path = std::path::Path::new(&sync_dir).join(SYNC_FILE_NAME);
    if !file_path.exists() {
        return Err(AppError::state(
            "No sync file found in the sync directory",
            "Sync settings to this directory from another installation first.",
        ));
    }

    // Check if file is newer than last sync
    let last_synced = StorageOperations::get_config(repo.inner(), SYNC_LAST_TIME_KEY).await?;
    if let Some(ref last_time_str) = last_synced {
        if let Ok(last_dt) = DateTime::parse_from_rfc3339(last_time_str) {
            let file_meta = tokio::fs::metadata(&file_path).await.map_err(|e| {
                AppError::state(
                    format!("Failed to read sync file metadata: {e}"),
                    "Check file permissions.",
                )
            })?;
            if let Ok(modified) = file_meta.modified() {
                let file_dt: DateTime<Utc> = modified.into();
                if file_dt <= last_dt.with_timezone(&Utc) {
                    // File is not newer, nothing to import
                    return Ok(SettingsImportResult {
                        imported_connections: 0,
                        imported_connection_groups: 0,
                        imported_sync_pairs: 0,
                        imported_configs: 0,
                        imported_workspace: false,
                    });
                }
            }
        }
    }

    let json_str = tokio::fs::read_to_string(&file_path).await.map_err(|e| {
        AppError::state(
            format!("Failed to read sync file: {e}"),
            "Check file permissions.",
        )
    })?;

    let bundle: SettingsBundle = serde_json::from_str(&json_str).map_err(|e| {
        AppError::state(
            format!("Invalid sync file format: {e}"),
            "The sync file may be corrupted. Try syncing again from the source.",
        )
    })?;

    let mut result = SettingsImportResult {
        imported_connections: 0,
        imported_connection_groups: 0,
        imported_sync_pairs: 0,
        imported_configs: 0,
        imported_workspace: false,
    };

    // Import connections
    let connection_export = serde_json::json!({
        "version": 1,
        "exported_at": bundle.exported_at.to_rfc3339(),
        "connections": bundle.connections,
        "groups": bundle.connection_groups,
    });
    let import_json = serde_json::to_string(&connection_export).unwrap_or_default();
    if let Ok(conn_result) = connection_mgr.import_connections(&import_json).await {
        result.imported_connections = conn_result.imported_connections;
        result.imported_connection_groups = conn_result.imported_groups;
    }

    // Import sync pairs
    for pair_value in &bundle.sync_pairs {
        if let Ok(pair) = serde_json::from_value::<crate::core::types::SyncPair>(pair_value.clone())
        {
            use crate::core::traits::SyncOperations;
            if sync_mgr.create_pair(pair).await.is_ok() {
                result.imported_sync_pairs += 1;
            }
        }
    }

    // Import configs (skip sync-specific keys to avoid overwriting local sync config)
    for config in &bundle.configs {
        if config.key != SYNC_DIR_CONFIG_KEY
            && config.key != SYNC_LAST_TIME_KEY
            && config.key != SYNC_AUTO_STARTUP_KEY
            && StorageOperations::set_config(repo.inner(), &config.key, &config.value)
                .await
                .is_ok()
            {
                result.imported_configs += 1;
            }
    }

    // Import workspace state
    if bundle.workspace_state != serde_json::Value::Null {
        if let Ok(ws) =
            serde_json::from_value::<crate::core::types::WorkspaceState>(bundle.workspace_state)
        {
            if StorageOperations::save_workspace_state(repo.inner(), &ws)
                .await
                .is_ok()
            {
                result.imported_workspace = true;
            }
        }
    }

    // Update last sync time
    let now = Utc::now().to_rfc3339();
    StorageOperations::set_config(repo.inner(), SYNC_LAST_TIME_KEY, &now).await?;

    tracing::info!(
        "Settings synced from dir: {} connections, {} groups, {} sync pairs, {} configs",
        result.imported_connections,
        result.imported_connection_groups,
        result.imported_sync_pairs,
        result.imported_configs
    );

    Ok(result)
}

/// Get current sync status (last sync time, status indicator).
#[tauri::command]
pub async fn get_settings_sync_status(
    repo: State<'_, Repository>,
) -> Result<SyncStatus, AppError> {
    let sync_dir =
        StorageOperations::get_config(repo.inner(), SYNC_DIR_CONFIG_KEY).await?;
    let last_synced =
        StorageOperations::get_config(repo.inner(), SYNC_LAST_TIME_KEY).await?;
    let auto_sync = StorageOperations::get_config(repo.inner(), SYNC_AUTO_STARTUP_KEY)
        .await?
        .map(|v| v == "true")
        .unwrap_or(false);

    let status = match sync_dir.as_deref() {
        None => "not_configured".to_string(),
        Some(dir) if last_synced.is_some() => {
        let file_path = std::path::Path::new(dir).join(SYNC_FILE_NAME);
        if file_path.exists() {
            if let Some(ref last_time_str) = last_synced {
                if let Ok(last_dt) = DateTime::parse_from_rfc3339(last_time_str) {
                    if let Ok(meta) = std::fs::metadata(&file_path) {
                        if let Ok(modified) = meta.modified() {
                            let file_dt: DateTime<Utc> = modified.into();
                            if file_dt > last_dt.with_timezone(&Utc) {
                                return Ok(SyncStatus {
                                    sync_dir,
                                    last_synced,
                                    status: "pending".to_string(),
                                    auto_sync_on_startup: auto_sync,
                                });
                            }
                        }
                    }
                }
            }
            "synced".to_string()
        } else {
            "synced".to_string()
        }
        }
        Some(_) => "never_synced".to_string(),
    };

    Ok(SyncStatus {
        sync_dir,
        last_synced,
        status,
        auto_sync_on_startup: auto_sync,
    })
}

/// Toggle auto-sync on startup.
#[tauri::command]
pub async fn set_settings_auto_sync(
    enabled: bool,
    repo: State<'_, Repository>,
) -> Result<(), AppError> {
    StorageOperations::set_config(
        repo.inner(),
        SYNC_AUTO_STARTUP_KEY,
        if enabled { "true" } else { "false" },
    )
    .await
}

// ──────────────────────────────────────────────
// Feature 3: Privacy Settings Persistence
// ──────────────────────────────────────────────

/// Privacy settings that control telemetry and update behavior.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PrivacySettings {
    pub auto_update_check: bool,
    pub crash_report_opt_in: bool,
}

impl Default for PrivacySettings {
    fn default() -> Self {
        Self {
            auto_update_check: true,
            crash_report_opt_in: false,
        }
    }
}

/// Get the current privacy settings.
#[tauri::command]
pub async fn get_privacy_settings(
    repo: State<'_, Repository>,
) -> Result<PrivacySettings, AppError> {
    let auto_update = repo
        .get_config("privacy_auto_update_check")
        .await
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(true);
    let crash_report = repo
        .get_config("privacy_crash_report_opt_in")
        .await
        .ok()
        .flatten()
        .map(|v| v == "true")
        .unwrap_or(false);
    Ok(PrivacySettings {
        auto_update_check: auto_update,
        crash_report_opt_in: crash_report,
    })
}

/// Set the privacy settings.
#[tauri::command]
pub async fn set_privacy_settings(
    settings: PrivacySettings,
    repo: State<'_, Repository>,
) -> Result<(), AppError> {
    repo.set_config(
        "privacy_auto_update_check",
        &settings.auto_update_check.to_string(),
    )
    .await?;
    repo.set_config(
        "privacy_crash_report_opt_in",
        &settings.crash_report_opt_in.to_string(),
    )
    .await?;
    tracing::info!(
        "Privacy settings updated: auto_update={}, crash_report={}",
        settings.auto_update_check,
        settings.crash_report_opt_in
    );
    Ok(())
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

/// Well-known config keys to export/import.
const KNOWN_CONFIG_KEYS: &[&str] = &[
    "theme",
    "language",
    "default_sort",
    "show_hidden_files",
    "default_view_mode",
    "transfer_verify_checksums",
    "transfer_checksum_algorithm",
    "transfer_verify_tier",
    "default_conflict_policy",
    "auto_connect_on_start",
    "terminal_default_shell",
    "privacy_auto_update_check",
    "privacy_crash_report_opt_in",
    "completion_run_command",
    "completion_play_sound",
    "completion_open_destination",
    "completion_show_notification",
];

/// Collect known config key-values from the repository.
async fn collect_known_configs(repo: &Repository) -> Vec<ConfigEntry> {
    let mut configs = Vec::new();
    for &key in KNOWN_CONFIG_KEYS {
        if let Ok(Some(value)) = repo.get_config(key).await {
            configs.push(ConfigEntry {
                key: key.to_string(),
                value,
            });
        }
    }
    configs
}

/// Count how many known config keys have values set.
async fn count_configs(repo: &Repository) -> u32 {
    let mut count = 0u32;
    for &key in KNOWN_CONFIG_KEYS {
        if let Ok(Some(_)) = repo.get_config(key).await {
            count += 1;
        }
    }
    count
}
