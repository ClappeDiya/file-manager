//! Connection Manager (T-019).
//!
//! - Save connections: name, type, host/endpoint, credentials, per-connection settings
//! - Connection groups (named folders)
//! - One-click connection test
//! - Bookmarks per connection
//! - Quick-connect bar with autocomplete
//! - Import/export connections (encrypted JSON)
//! - Credentials stored in OS keychain via CredentialStore

use crate::core::error::AppError;
use crate::core::types::{ConnectionGroup, ConnectionProfile, ConnectionProtocol};
use crate::security::CredentialStore;
use crate::storage::Repository;
use chrono::Utc;
use std::sync::Arc;
use uuid::Uuid;

/// Connection manager for managing saved connections and their credentials.
#[derive(Clone)]
pub struct ConnectionManager {
    repo: Repository,
    credential_store: Arc<CredentialStore>,
}

impl ConnectionManager {
    pub fn new(repo: Repository, credential_store: Arc<CredentialStore>) -> Self {
        Self {
            repo,
            credential_store,
        }
    }

    /// Save a connection profile (credentials go to keychain).
    pub async fn save_connection(
        &self,
        profile: ConnectionProfile,
        password: Option<String>,
    ) -> Result<ConnectionProfile, AppError> {
        self.save_connection_with_secrets(profile, password, None).await
    }

    /// Save a connection and its secrets. `key_passphrase`: `None` keeps the
    /// stored one, `Some("")` removes it, anything else replaces it.
    pub async fn save_connection_with_secrets(
        &self,
        mut profile: ConnectionProfile,
        password: Option<String>,
        key_passphrase: Option<String>,
    ) -> Result<ConnectionProfile, AppError> {
        // Store password in keychain if provided
        if let Some(pwd) = password {
            let cred_key = CredentialStore::credential_key(&profile.id);
            self.credential_store.store(&cred_key, &pwd).await?;
            profile.credential_ref = Some(cred_key);
        }
        match key_passphrase {
            Some(p) if p.is_empty() => self.credential_store.delete(&CredentialStore::passphrase_key(&profile.id)).await?,
            Some(p) => self.credential_store.store(&CredentialStore::passphrase_key(&profile.id), &p).await?,
            None => {}
        }

        self.repo.save_connection(&profile).await?;
        // Editing a connection is how the user fixes rejected credentials.
        self.repo.set_credentials_rejected(profile.id, None).await?;
        tracing::info!("Connection saved: {} ({})", profile.name, profile.id);
        Ok(profile)
    }

    /// Star or unstar a connection.
    pub async fn set_starred(&self, id: Uuid, starred: bool) -> Result<(), AppError> {
        if !self.repo.set_connection_starred(id, starred).await? {
            return Err(AppError::not_found(format!("Connection {id} not found")));
        }
        Ok(())
    }

    /// Starred / credentials-rejected flags of every connection.
    pub async fn list_status(&self) -> Result<Vec<crate::core::types::ConnectionStatus>, AppError> {
        self.repo.list_connection_status().await
    }

    /// Clear the credentials-rejected flag (the user clicked Retry).
    pub async fn clear_credentials_rejected(&self, id: Uuid) -> Result<(), AppError> {
        self.repo.set_credentials_rejected(id, None).await
    }

    /// Remember how a connect attempt went: rejected credentials flag the
    /// connection, a success clears the flag, other failures (network,
    /// server down) leave it as it was.
    pub async fn record_connect_outcome<T>(&self, id: Uuid, outcome: &Result<T, AppError>) {
        let r = match outcome {
            Ok(_) => match self.repo.get_connection_status(id).await {
                Ok(Some(s)) if s.credentials_rejected_at.is_some() => self.repo.set_credentials_rejected(id, None).await,
                _ => Ok(()),
            },
            Err(e) if e.is_auth_failure() => {
                tracing::warn!("Credentials of connection {id} were rejected; automatic reconnects are paused: {}", e.user_message());
                self.repo.set_credentials_rejected(id, Some(e.user_message())).await
            }
            Err(_) => Ok(()),
        };
        if let Err(e) = r {
            tracing::warn!("Could not record the connect outcome of {id}: {e}");
        }
    }

    /// Secrets for an automatic (background) connect: sync, automation,
    /// backup, auto-mount. A connection whose credentials were rejected is
    /// skipped with a clear error instead of being retried with the same
    /// credentials (which can lock the account).
    pub async fn background_secrets(&self, id: Uuid, name: &str) -> Result<super::ConnectionSecrets, AppError> {
        self.ensure_not_rejected(id, name).await?;
        self.get_secrets(id).await
    }

    /// `Err` when the connection is flagged as having rejected credentials.
    pub async fn ensure_not_rejected(&self, id: Uuid, name: &str) -> Result<(), AppError> {
        if let Some(s) = self.repo.get_connection_status(id).await? {
            if let Some(at) = s.credentials_rejected_at {
                return Err(AppError::auth(
                    format!(
                        "Skipped '{name}': its credentials were rejected on {} ({})",
                        at.format("%Y-%m-%d %H:%M UTC"),
                        s.credentials_rejected_reason.unwrap_or_else(|| "sign-in refused".into())
                    ),
                    "Edit the connection with the current credentials, or click Retry in Connections.",
                ));
            }
        }
        Ok(())
    }

    /// List all saved connections.
    pub async fn list_connections(&self) -> Result<Vec<ConnectionProfile>, AppError> {
        self.repo.list_connections().await
    }

    /// Get a single connection by ID.
    pub async fn get_connection(&self, id: Uuid) -> Result<Option<ConnectionProfile>, AppError> {
        self.repo.get_connection(id).await
    }

    /// Delete a connection and its stored credentials.
    pub async fn delete_connection(&self, id: Uuid) -> Result<(), AppError> {
        // Delete credentials from keychain
        let cred_key = CredentialStore::credential_key(&id);
        self.credential_store.delete(&cred_key).await?;
        self.credential_store.delete(&CredentialStore::passphrase_key(&id)).await?;

        self.repo.delete_connection(id).await?;
        tracing::info!("Connection deleted: {id}");
        Ok(())
    }

    /// Retrieve the password for a connection.
    pub async fn get_password(&self, connection_id: Uuid) -> Result<Option<String>, AppError> {
        let cred_key = CredentialStore::credential_key(&connection_id);
        self.credential_store.retrieve(&cred_key).await
    }

    /// Every stored secret of a connection (password / key path / token and
    /// the SFTP key passphrase).
    pub async fn get_secrets(&self, connection_id: Uuid) -> Result<super::ConnectionSecrets, AppError> {
        Ok(super::ConnectionSecrets {
            secret: self.get_password(connection_id).await?,
            key_passphrase: self.credential_store.retrieve(&CredentialStore::passphrase_key(&connection_id)).await?,
        })
    }

    /// Store a new primary secret (e.g. rotated OAuth tokens) for a saved connection.
    pub async fn set_password(&self, connection_id: Uuid, secret: &str) -> Result<(), AppError> {
        self.credential_store.store(&CredentialStore::credential_key(&connection_id), secret).await
    }

    /// Forget a connection's primary secret (e.g. an OAuth sign-in).
    pub async fn delete_password(&self, connection_id: Uuid) -> Result<(), AppError> {
        self.credential_store.delete(&CredentialStore::credential_key(&connection_id)).await
    }

    /// The credential store (OAuth connectors persist rotated tokens through it).
    pub fn credential_store(&self) -> Arc<CredentialStore> {
        self.credential_store.clone()
    }

    /// Test a connection (checks if we can reach the host).
    pub async fn test_connection(&self, profile: &ConnectionProfile) -> Result<ConnectionTestResult, AppError> {
        let start = std::time::Instant::now();

        let result: Result<bool, AppError> = match profile.protocol {
            ConnectionProtocol::Local => {
                // Local connections always succeed
                Ok(true)
            }
            _ => {
                // For remote connections, try a TCP connect to the host:port
                let host = profile.host.clone();
                let port = profile.port.unwrap_or(Self::default_port(profile.protocol));

                tokio::task::spawn_blocking(move || {
                    let addr = format!("{host}:{port}");
                    match std::net::TcpStream::connect_timeout(
                        &addr.parse().map_err(|e| AppError::Connection {
                            message: format!("Invalid address {addr}: {e}"),
                            advice: "Check the host and port.".to_string(),
                        })?,
                        std::time::Duration::from_secs(5),
                    ) {
                        Ok(_) => Ok(true),
                        Err(e) => {
                            tracing::warn!("Connection test failed for {addr}: {e}");
                            Ok(false)
                        }
                    }
                })
                .await
                .map_err(|e| AppError::internal(format!("Connection test task panicked: {e}")))?
            }
        };

        let latency_ms = start.elapsed().as_millis() as u64;
        let reachable = result?;

        Ok(ConnectionTestResult {
            reachable,
            latency_ms,
            error: if reachable {
                None
            } else {
                Some("Host is not reachable.".to_string())
            },
        })
    }

    /// Get default port for a protocol.
    fn default_port(protocol: ConnectionProtocol) -> u16 {
        match protocol {
            ConnectionProtocol::Sftp => 22,
            ConnectionProtocol::Ftp => 21,
            ConnectionProtocol::Ftps => 990,
            ConnectionProtocol::WebDav => 443,
            ConnectionProtocol::Smb => 445,
            ConnectionProtocol::Nfs => 2049,
            ConnectionProtocol::S3 => 443,
            ConnectionProtocol::GoogleDrive => 443,
            ConnectionProtocol::Dropbox => 443,
            ConnectionProtocol::OneDrive => 443,
            ConnectionProtocol::AzureBlob | ConnectionProtocol::AdlsGen2 | ConnectionProtocol::AzureFiles => 443,
            ConnectionProtocol::Swift => 5000,
            ConnectionProtocol::Local => 0,
            ConnectionProtocol::Rclone => 0,
            ConnectionProtocol::B2 => 443,
        }
    }

    /// Update the last_used timestamp for a connection.
    pub async fn touch_connection(&self, id: Uuid) -> Result<(), AppError> {
        if let Some(mut profile) = self.repo.get_connection(id).await? {
            profile.last_used = Some(Utc::now());
            self.repo.save_connection(&profile).await?;
        }
        Ok(())
    }

    // -- Connection Groups --

    /// Create a connection group (folder).
    pub async fn create_group(
        &self,
        name: String,
        parent_id: Option<Uuid>,
    ) -> Result<ConnectionGroup, AppError> {
        let group = ConnectionGroup {
            id: Uuid::new_v4(),
            name,
            parent_id,
            sort_order: 0,
            created_at: Utc::now(),
        };
        self.repo.save_connection_group(&group).await?;
        Ok(group)
    }

    /// List all connection groups.
    pub async fn list_groups(&self) -> Result<Vec<ConnectionGroup>, AppError> {
        self.repo.list_connection_groups().await
    }

    /// Delete a connection group.
    pub async fn delete_group(&self, id: Uuid) -> Result<(), AppError> {
        self.repo.delete_connection_group(id).await
    }

    // -- Import/Export --

    /// Export connections as JSON (credentials stripped).
    pub async fn export_connections(&self) -> Result<String, AppError> {
        let connections = self.repo.list_connections().await?;
        let groups = self.repo.list_connection_groups().await?;

        let mut export_data = ConnectionExport {
            version: 1,
            exported_at: Utc::now(),
            connections,
            groups,
        };

        // Never export credentials
        for conn in &mut export_data.connections {
            conn.credential_ref = None;
        }

        serde_json::to_string_pretty(&export_data).map_err(|e| {
            AppError::Connection {
                message: format!("Failed to export connections: {e}"),
                advice: "Try again.".to_string(),
            }
        })
    }

    /// Import connections from JSON.
    pub async fn import_connections(&self, json: &str) -> Result<ImportResult, AppError> {
        let export: ConnectionExport = serde_json::from_str(json).map_err(|e| {
            AppError::Connection {
                message: format!("Invalid import data: {e}"),
                advice: "Check that the file is a valid connection export.".to_string(),
            }
        })?;

        let mut imported_connections = 0u32;
        let mut imported_groups = 0u32;

        // Import groups first
        for group in &export.groups {
            self.repo.save_connection_group(group).await?;
            imported_groups += 1;
        }

        // Import connections
        for conn in &export.connections {
            self.repo.save_connection(conn).await?;
            imported_connections += 1;
        }

        Ok(ImportResult {
            imported_connections,
            imported_groups,
        })
    }

    /// Search connections for quick-connect autocomplete.
    pub async fn search_connections(&self, query: &str) -> Result<Vec<ConnectionProfile>, AppError> {
        let all = self.repo.list_connections().await?;
        let query_lower = query.to_lowercase();

        let mut matches: Vec<ConnectionProfile> = all
            .into_iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&query_lower)
                    || c.host.to_lowercase().contains(&query_lower)
                    || c.protocol.as_str().contains(&query_lower)
            })
            .collect();

        // Sort by last_used (most recent first), then by name
        matches.sort_by(|a, b| {
            b.last_used
                .cmp(&a.last_used)
                .then_with(|| a.name.cmp(&b.name))
        });

        Ok(matches)
    }
}

/// Result of a connection test.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectionTestResult {
    pub reachable: bool,
    pub latency_ms: u64,
    pub error: Option<String>,
}

/// Connection export format.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ConnectionExport {
    pub version: u32,
    pub exported_at: chrono::DateTime<Utc>,
    pub connections: Vec<ConnectionProfile>,
    pub groups: Vec<ConnectionGroup>,
}

/// Result of importing connections.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImportResult {
    pub imported_connections: u32,
    pub imported_groups: u32,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::types::*;

    async fn setup() -> ConnectionManager {
        let repo = Repository::open_in_memory().await.unwrap();
        let cred_store = Arc::new(CredentialStore {
            cache: Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new())),
            keychain_available: false,
            file_vault: None,
        });
        ConnectionManager::new(repo, cred_store)
    }

    /// Regression: upload modes, FTP tuning and symlink policy had no
    /// columns and were silently dropped on save; extras persist too.
    #[tokio::test]
    async fn advanced_settings_and_extras_survive_a_reload() {
        let repo = Repository::open_in_memory().await.unwrap();
        let mut p = make_test_profile();
        p.default_file_mode = Some("640".into());
        p.default_dir_mode = Some("750".into());
        p.ftp_use_mlsd = Some(true);
        p.ftp_force_passive_ip = Some(false);
        p.ftp_post_login_commands = Some(vec!["OPTS UTF8 ON".into()]);
        p.symlink_policy = Some("skip".into());
        p.extras.assume_role = Some(crate::core::types::AssumeRoleConfig {
            role_arn: "arn:aws:iam::123456789012:role/Reader".into(),
            external_id: Some("ext-42".into()),
            session_name: None,
            duration_secs: Some(1800),
        });
        p.extras.preset = Some("r2".into());
        repo.save_connection(&p).await.unwrap();
        let back = repo.get_connection(p.id).await.unwrap().unwrap();
        assert_eq!(back.default_file_mode.as_deref(), Some("640"));
        assert_eq!(back.default_dir_mode.as_deref(), Some("750"));
        assert_eq!((back.ftp_use_mlsd, back.ftp_force_passive_ip), (Some(true), Some(false)));
        assert_eq!(back.ftp_post_login_commands, p.ftp_post_login_commands);
        assert_eq!(back.symlink_policy.as_deref(), Some("skip"));
        assert_eq!(back.extras, p.extras);
        assert_eq!(repo.list_connections().await.unwrap()[0].extras, p.extras);
    }

    fn make_test_profile() -> ConnectionProfile {
        ConnectionProfile {
            id: Uuid::new_v4(),
            name: "Test Server".to_string(),
            protocol: ConnectionProtocol::Sftp,
            host: "test.example.com".to_string(),
            port: Some(22),
            username: Some("admin".to_string()),
            credential_ref: None,
            remote_path: "/".to_string(),
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
            extras: Default::default(),
        }
    }

    #[tokio::test]
    async fn test_save_and_list_connections() {
        let mgr = setup().await;
        let profile = make_test_profile();

        mgr.save_connection(profile.clone(), Some("password123".to_string()))
            .await
            .unwrap();

        let connections = mgr.list_connections().await.unwrap();
        assert_eq!(connections.len(), 1);
        assert_eq!(connections[0].name, "Test Server");
        assert!(connections[0].credential_ref.is_some());
    }

    #[tokio::test]
    async fn test_get_connection() {
        let mgr = setup().await;
        let profile = make_test_profile();
        let id = profile.id;

        mgr.save_connection(profile, None).await.unwrap();

        let conn = mgr.get_connection(id).await.unwrap();
        assert!(conn.is_some());
        assert_eq!(conn.unwrap().name, "Test Server");
    }

    #[tokio::test]
    async fn test_delete_connection() {
        let mgr = setup().await;
        let profile = make_test_profile();
        let id = profile.id;

        mgr.save_connection(profile, Some("pass".to_string())).await.unwrap();
        mgr.delete_connection(id).await.unwrap();

        let connections = mgr.list_connections().await.unwrap();
        assert!(connections.is_empty());

        // Credential should also be deleted
        let password = mgr.get_password(id).await.unwrap();
        assert!(password.is_none());
    }

    #[tokio::test]
    async fn test_connection_groups() {
        let mgr = setup().await;

        let group = mgr.create_group("Work Servers".to_string(), None).await.unwrap();
        assert_eq!(group.name, "Work Servers");

        let groups = mgr.list_groups().await.unwrap();
        assert_eq!(groups.len(), 1);

        mgr.delete_group(group.id).await.unwrap();
        let groups = mgr.list_groups().await.unwrap();
        assert!(groups.is_empty());
    }

    #[tokio::test]
    async fn test_export_import_connections() {
        let mgr = setup().await;

        let mut profile = make_test_profile();
        profile.name = "Export Test".to_string();
        mgr.save_connection(profile, None).await.unwrap();

        let _group = mgr.create_group("Test Group".to_string(), None).await.unwrap();

        // Export
        let json = mgr.export_connections().await.unwrap();
        assert!(json.contains("Export Test"));
        assert!(json.contains("Test Group"));

        // Import into a fresh manager
        let mgr2 = setup().await;
        let result = mgr2.import_connections(&json).await.unwrap();
        assert_eq!(result.imported_connections, 1);
        assert_eq!(result.imported_groups, 1);

        let connections = mgr2.list_connections().await.unwrap();
        assert_eq!(connections.len(), 1);
    }

    #[tokio::test]
    async fn test_search_connections() {
        let mgr = setup().await;

        let mut p1 = make_test_profile();
        p1.name = "Production Server".to_string();
        p1.host = "prod.example.com".to_string();
        mgr.save_connection(p1, None).await.unwrap();

        let mut p2 = make_test_profile();
        p2.id = Uuid::new_v4();
        p2.name = "Staging Server".to_string();
        p2.host = "staging.example.com".to_string();
        mgr.save_connection(p2, None).await.unwrap();

        let results = mgr.search_connections("prod").await.unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Production Server");

        let results = mgr.search_connections("example").await.unwrap();
        assert_eq!(results.len(), 2);
    }

    #[tokio::test]
    async fn test_test_connection_local() {
        let mgr = setup().await;
        let mut profile = make_test_profile();
        profile.protocol = ConnectionProtocol::Local;

        let result = mgr.test_connection(&profile).await.unwrap();
        assert!(result.reachable);
    }

    /// Regression: an SFTP key passphrase had nowhere to be stored.
    #[tokio::test]
    async fn key_passphrase_is_stored_kept_and_removed() {
        let mgr = setup().await;
        let p = make_test_profile();
        let id = p.id;
        mgr.save_connection_with_secrets(p.clone(), Some("/keys/id_ed25519".into()), Some("pp".into())).await.unwrap();
        let s = mgr.get_secrets(id).await.unwrap();
        assert_eq!((s.secret.as_deref(), s.key_passphrase.as_deref()), (Some("/keys/id_ed25519"), Some("pp")));
        // Editing without touching the secrets keeps both.
        mgr.save_connection_with_secrets(p.clone(), None, None).await.unwrap();
        assert_eq!(mgr.get_secrets(id).await.unwrap().key_passphrase.as_deref(), Some("pp"));
        // An empty passphrase removes it.
        mgr.save_connection_with_secrets(p, None, Some(String::new())).await.unwrap();
        assert_eq!(mgr.get_secrets(id).await.unwrap().key_passphrase, None);
        mgr.save_connection_with_secrets(make_test_profile(), None, Some("x".into())).await.unwrap();
        mgr.delete_connection(id).await.unwrap();
        assert_eq!(mgr.get_secrets(id).await.unwrap(), crate::connectors::ConnectionSecrets::default());
    }

    #[tokio::test]
    async fn starred_connections_are_listed_first() {
        let mgr = setup().await;
        let mut a = make_test_profile();
        a.name = "Alpha".into();
        let mut b = make_test_profile();
        b.id = Uuid::new_v4();
        b.name = "Beta".into();
        mgr.save_connection(a.clone(), None).await.unwrap();
        mgr.save_connection(b.clone(), None).await.unwrap();
        assert_eq!(mgr.list_connections().await.unwrap()[0].name, "Alpha");
        mgr.set_starred(b.id, true).await.unwrap();
        assert_eq!(mgr.list_connections().await.unwrap()[0].name, "Beta");
        let st = mgr.list_status().await.unwrap();
        assert_eq!((st[0].id, st[0].starred, st[1].starred), (b.id, true, false));
        // Editing keeps the star.
        mgr.save_connection(b.clone(), None).await.unwrap();
        assert!(mgr.list_status().await.unwrap()[0].starred);
        mgr.set_starred(b.id, false).await.unwrap();
        assert_eq!(mgr.list_connections().await.unwrap()[0].name, "Alpha");
        assert!(mgr.set_starred(Uuid::new_v4(), true).await.is_err());
    }

    /// Rejected credentials flag the connection; background reconnects are
    /// refused until the profile is edited or the user retries.
    #[tokio::test]
    async fn rejected_credentials_pause_background_reconnects() {
        let mgr = setup().await;
        let p = make_test_profile();
        let id = p.id;
        mgr.save_connection(p.clone(), Some("old".into())).await.unwrap();
        assert_eq!(mgr.background_secrets(id, "Test").await.unwrap().secret.as_deref(), Some("old"));
        // A network failure does not flag it.
        mgr.record_connect_outcome::<()>(id, &Err(AppError::connection("timed out", "retry"))).await;
        assert!(mgr.background_secrets(id, "Test").await.is_ok());
        // A rejected sign-in does.
        mgr.record_connect_outcome::<()>(id, &Err(AppError::auth("HTTP 401", "check"))).await;
        let e = mgr.background_secrets(id, "Test").await.unwrap_err();
        assert!(e.is_auth_failure() && e.user_message().contains("Skipped 'Test'"), "{e}");
        let st = mgr.list_status().await.unwrap();
        assert!(st[0].credentials_rejected_at.is_some() && st[0].credentials_rejected_reason.as_deref().unwrap().contains("HTTP 401"));
        // Retry clears it…
        mgr.clear_credentials_rejected(id).await.unwrap();
        assert!(mgr.background_secrets(id, "Test").await.is_ok());
        // …and so does editing the profile, and a successful connect.
        mgr.record_connect_outcome::<()>(id, &Err(AppError::auth("HTTP 403", "check"))).await;
        mgr.save_connection(p.clone(), Some("new".into())).await.unwrap();
        assert_eq!(mgr.background_secrets(id, "Test").await.unwrap().secret.as_deref(), Some("new"));
        mgr.record_connect_outcome::<()>(id, &Err(AppError::auth("HTTP 401", "check"))).await;
        mgr.record_connect_outcome(id, &Ok(())).await;
        assert!(mgr.background_secrets(id, "Test").await.is_ok());
    }

    #[tokio::test]
    async fn test_password_storage_and_retrieval() {
        let mgr = setup().await;
        let profile = make_test_profile();
        let id = profile.id;

        mgr.save_connection(profile, Some("my_secret_password".to_string()))
            .await
            .unwrap();

        let password = mgr.get_password(id).await.unwrap();
        assert_eq!(password, Some("my_secret_password".to_string()));
    }
}
