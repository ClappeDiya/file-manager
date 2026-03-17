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
        mut profile: ConnectionProfile,
        password: Option<String>,
    ) -> Result<ConnectionProfile, AppError> {
        // Store password in keychain if provided
        if let Some(pwd) = password {
            let cred_key = CredentialStore::credential_key(&profile.id);
            self.credential_store.store(&cred_key, &pwd).await?;
            profile.credential_ref = Some(cred_key);
        }

        self.repo.save_connection(&profile).await?;
        tracing::info!("Connection saved: {} ({})", profile.name, profile.id);
        Ok(profile)
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

        self.repo.delete_connection(id).await?;
        tracing::info!("Connection deleted: {id}");
        Ok(())
    }

    /// Retrieve the password for a connection.
    pub async fn get_password(&self, connection_id: Uuid) -> Result<Option<String>, AppError> {
        let cred_key = CredentialStore::credential_key(&connection_id);
        self.credential_store.retrieve(&cred_key).await
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
            ConnectionProtocol::AzureBlob => 443,
            ConnectionProtocol::Swift => 5000,
            ConnectionProtocol::Local => 0,
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
        });
        ConnectionManager::new(repo, cred_store)
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
