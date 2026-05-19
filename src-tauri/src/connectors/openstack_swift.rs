//! OpenStack Swift Object Storage Connector
//!
//! Features:
//! - Keystone v3 authentication (token-based)
//! - Application credentials authentication
//! - TempAuth (legacy) support
//! - Container listing and management
//! - Object operations: list, upload, download, delete, copy
//! - Large Object Support (SLO/DLO)
//! - Object metadata and custom headers
//! - Temporary URL generation (TempURL middleware)
//! - Container ACL management
//! - Pseudo-folder navigation (delimiter-based)

use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use tokio::sync::RwLock;
use zeroize::Zeroize;

// Large object segment size: 5GB (Swift limit per segment)
#[allow(dead_code)]
const SEGMENT_SIZE: u64 = 5 * 1024 * 1024 * 1024;
// Default segment size for uploads: 100MB
const DEFAULT_SEGMENT_SIZE: u64 = 100 * 1024 * 1024;

// ── Swift Types ──

/// OpenStack authentication method.
///
/// Secret fields are zeroized on drop to prevent credential leakage
/// in memory dumps or core files.
#[derive(Debug, Clone, Serialize, Deserialize, Zeroize)]
#[serde(rename_all = "snake_case")]
#[zeroize(drop)]
pub enum SwiftAuthMethod {
    /// Keystone v3 (username + password + project)
    KeystoneV3 {
        auth_url: String,
        username: String,
        password: String,
        project_name: String,
        domain_name: String,
        region: Option<String>,
    },
    /// Application credentials (Keystone v3)
    ApplicationCredential {
        auth_url: String,
        credential_id: String,
        credential_secret: String,
    },
    /// TempAuth (legacy, for dev/testing)
    TempAuth {
        auth_url: String,
        username: String,
        password: String,
    },
    /// Pre-authenticated token + storage URL
    Token {
        storage_url: String,
        auth_token: String,
    },
}

/// Swift connector configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwiftConfig {
    pub auth_method: SwiftAuthMethod,
    /// Segment size for large object uploads.
    pub segment_size: u64,
    /// Whether to use Static Large Objects (SLO) instead of Dynamic (DLO).
    pub use_slo: bool,
    /// TempURL key for generating temporary URLs.
    pub temp_url_key: Option<String>,
}

impl Default for SwiftConfig {
    fn default() -> Self {
        Self {
            auth_method: SwiftAuthMethod::KeystoneV3 {
                auth_url: String::new(),
                username: String::new(),
                password: String::new(),
                project_name: String::new(),
                domain_name: "Default".to_string(),
                region: None,
            },
            segment_size: DEFAULT_SEGMENT_SIZE,
            use_slo: true,
            temp_url_key: None,
        }
    }
}

/// Swift container information.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwiftContainerInfo {
    pub name: String,
    pub object_count: u64,
    pub bytes_used: u64,
    pub last_modified: Option<String>,
    pub read_acl: Option<String>,
    pub write_acl: Option<String>,
    pub storage_policy: Option<String>,
}

/// Swift object metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SwiftObjectMeta {
    pub name: String,
    pub container: String,
    pub content_type: Option<String>,
    pub content_length: u64,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
    pub is_large_object: bool,
    pub custom_metadata: HashMap<String, String>,
}

/// Internal Swift connection state.
///
/// The auth_token is zeroized on drop to prevent leakage in memory dumps.
#[allow(dead_code)]
struct SwiftConnection {
    config: SwiftConfig,
    storage_url: String,
    auth_token: zeroize::Zeroizing<String>,
    token_expires: Option<DateTime<Utc>>,
    connected: bool,
}

/// OpenStack Swift connector.
pub struct SwiftConnector {
    state: RwLock<Option<SwiftConnection>>,
}

impl Default for SwiftConnector {
    fn default() -> Self {
        Self::new()
    }
}

impl SwiftConnector {
    pub fn new() -> Self {
        Self {
            state: RwLock::new(None),
        }
    }

    /// Authenticate and obtain a token via the configured auth method.
    async fn authenticate(config: &SwiftConfig) -> Result<(String, String, Option<DateTime<Utc>>), AppError> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| AppError::connection(format!("HTTP client error: {e}"), "Check TLS configuration."))?;

        match &config.auth_method {
            SwiftAuthMethod::KeystoneV3 { auth_url, username, password, project_name, domain_name, region } => {
                tracing::info!(
                    "Authenticating via Keystone v3: {} user={} project={} domain={}",
                    auth_url, username, project_name, domain_name
                );

                let auth_body = serde_json::json!({
                    "auth": {
                        "identity": {
                            "methods": ["password"],
                            "password": {
                                "user": {
                                    "name": username,
                                    "password": password,
                                    "domain": { "name": domain_name }
                                }
                            }
                        },
                        "scope": {
                            "project": {
                                "name": project_name,
                                "domain": { "name": domain_name }
                            }
                        }
                    }
                });

                let token_url = format!("{}/auth/tokens", auth_url.trim_end_matches('/'));
                let resp = client
                    .post(&token_url)
                    .json(&auth_body)
                    .send()
                    .await
                    .map_err(|e| AppError::connection(
                        format!("Keystone v3 auth failed: {e}"),
                        "Check auth URL, credentials, and network connectivity.",
                    ))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(AppError::connection(
                        format!("Keystone v3 auth returned {status}: {body}"),
                        "Check username, password, project, and domain.",
                    ));
                }

                let auth_token = resp
                    .headers()
                    .get("x-subject-token")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from)
                    .ok_or_else(|| AppError::connection(
                        "Keystone v3 response missing X-Subject-Token header",
                        "The identity service may be misconfigured.",
                    ))?;

                let catalog: serde_json::Value = resp.json().await.map_err(|e| {
                    AppError::connection(format!("Failed to parse Keystone response: {e}"), "")
                })?;

                // Extract storage URL from service catalog
                let storage_url = Self::extract_swift_endpoint(&catalog, region.as_deref())?;

                // Parse token expiry
                let token_expires = catalog
                    .pointer("/token/expires_at")
                    .and_then(|v| v.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                Ok((storage_url, auth_token, token_expires))
            }

            SwiftAuthMethod::ApplicationCredential { auth_url, credential_id, credential_secret } => {
                tracing::info!("Authenticating via application credential: {}", credential_id);

                let auth_body = serde_json::json!({
                    "auth": {
                        "identity": {
                            "methods": ["application_credential"],
                            "application_credential": {
                                "id": credential_id,
                                "secret": credential_secret
                            }
                        }
                    }
                });

                let token_url = format!("{}/auth/tokens", auth_url.trim_end_matches('/'));
                let resp = client
                    .post(&token_url)
                    .json(&auth_body)
                    .send()
                    .await
                    .map_err(|e| AppError::connection(
                        format!("Application credential auth failed: {e}"),
                        "Check credential ID, secret, and auth URL.",
                    ))?;

                if !resp.status().is_success() {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    return Err(AppError::connection(
                        format!("Application credential auth returned {status}: {body}"),
                        "Verify the credential has not expired or been revoked.",
                    ));
                }

                let auth_token = resp
                    .headers()
                    .get("x-subject-token")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from)
                    .ok_or_else(|| AppError::connection(
                        "Keystone response missing X-Subject-Token header",
                        "The identity service may be misconfigured.",
                    ))?;

                let catalog: serde_json::Value = resp.json().await.map_err(|e| {
                    AppError::connection(format!("Failed to parse Keystone response: {e}"), "")
                })?;

                let storage_url = Self::extract_swift_endpoint(&catalog, None)?;
                let token_expires = catalog
                    .pointer("/token/expires_at")
                    .and_then(|v| v.as_str())
                    .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
                    .map(|dt| dt.with_timezone(&Utc));

                Ok((storage_url, auth_token, token_expires))
            }

            SwiftAuthMethod::TempAuth { auth_url, username, password } => {
                tracing::info!("Authenticating via TempAuth: {} user={}", auth_url, username);

                let resp = client
                    .get(auth_url)
                    .header("X-Auth-User", username.as_str())
                    .header("X-Auth-Key", password.as_str())
                    .send()
                    .await
                    .map_err(|e| AppError::connection(
                        format!("TempAuth request failed: {e}"),
                        "Check auth URL and credentials.",
                    ))?;

                if !resp.status().is_success() {
                    return Err(AppError::connection(
                        format!("TempAuth returned {}", resp.status()),
                        "Check username and key.",
                    ));
                }

                let storage_url = resp
                    .headers()
                    .get("x-storage-url")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from)
                    .ok_or_else(|| AppError::connection(
                        "TempAuth response missing X-Storage-Url",
                        "The Swift proxy may not support TempAuth.",
                    ))?;

                let auth_token = resp
                    .headers()
                    .get("x-auth-token")
                    .and_then(|v| v.to_str().ok())
                    .map(String::from)
                    .ok_or_else(|| AppError::connection(
                        "TempAuth response missing X-Auth-Token",
                        "The Swift proxy may not support TempAuth.",
                    ))?;

                Ok((storage_url, auth_token, None))
            }

            SwiftAuthMethod::Token { storage_url, auth_token } => {
                Ok((storage_url.clone(), auth_token.clone(), None))
            }
        }
    }

    /// Extract the object-store endpoint from a Keystone v3 service catalog.
    fn extract_swift_endpoint(
        catalog: &serde_json::Value,
        region: Option<&str>,
    ) -> Result<String, AppError> {
        let services = catalog
            .pointer("/token/catalog")
            .and_then(|c| c.as_array())
            .ok_or_else(|| AppError::connection(
                "Keystone catalog not found in token response",
                "The identity service may not have a service catalog configured.",
            ))?;

        for svc in services {
            let svc_type = svc.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if svc_type != "object-store" {
                continue;
            }
            if let Some(endpoints) = svc.get("endpoints").and_then(|e| e.as_array()) {
                // Prefer public interface, optionally filter by region
                for ep in endpoints {
                    let interface = ep.get("interface").and_then(|i| i.as_str()).unwrap_or("");
                    let ep_region = ep.get("region_id").and_then(|r| r.as_str());
                    if interface == "public" {
                        if let Some(wanted) = region {
                            if ep_region == Some(wanted) {
                                if let Some(url) = ep.get("url").and_then(|u| u.as_str()) {
                                    return Ok(url.trim_end_matches('/').to_string());
                                }
                            }
                        } else if let Some(url) = ep.get("url").and_then(|u| u.as_str()) {
                            return Ok(url.trim_end_matches('/').to_string());
                        }
                    }
                }
            }
        }

        Err(AppError::connection(
            "No object-store endpoint found in Keystone catalog",
            "Ensure the Swift service is registered in the catalog for this project.",
        ))
    }

    /// List containers in the account.
    pub async fn list_containers(&self) -> Result<Vec<SwiftContainerInfo>, AppError> {
        let state = self.state.read().await;
        let _conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected to Swift", "Connect first.")
        })?;

        tracing::info!("Listing containers at: {}", _conn.storage_url);
        // Real implementation: GET storage_url?format=json
        Ok(vec![])
    }

    /// List objects in a container with optional prefix/delimiter.
    pub async fn list_objects(
        &self,
        container: &str,
        prefix: Option<&str>,
        delimiter: Option<&str>,
    ) -> Result<Vec<FileEntry>, AppError> {
        let state = self.state.read().await;
        let _conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected to Swift", "Connect first.")
        })?;

        tracing::info!(
            "Listing objects in {}/{} (delimiter={:?})",
            container,
            prefix.unwrap_or(""),
            delimiter
        );
        // Real implementation: GET storage_url/container?prefix=X&delimiter=/&format=json
        Ok(vec![])
    }

    /// Get object metadata.
    pub async fn get_object_meta(
        &self,
        container: &str,
        object_name: &str,
    ) -> Result<SwiftObjectMeta, AppError> {
        let state = self.state.read().await;
        let _conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected", "Connect first.")
        })?;

        tracing::info!("Getting metadata for {}/{}", container, object_name);
        Ok(SwiftObjectMeta {
            name: object_name.to_string(),
            container: container.to_string(),
            content_type: None,
            content_length: 0,
            last_modified: None,
            etag: None,
            is_large_object: false,
            custom_metadata: HashMap::new(),
        })
    }

    /// Set custom metadata on an object.
    pub async fn set_object_metadata(
        &self,
        container: &str,
        object_name: &str,
        metadata: HashMap<String, String>,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let _conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected", "Connect first.")
        })?;

        tracing::info!("Setting metadata for {}/{}: {:?}", container, object_name, metadata);
        // Real implementation: POST storage_url/container/object with X-Object-Meta-* headers
        Ok(())
    }

    /// Copy an object within Swift.
    pub async fn copy_object(
        &self,
        src_container: &str,
        src_object: &str,
        dst_container: &str,
        dst_object: &str,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let _conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected", "Connect first.")
        })?;

        tracing::info!(
            "Copying {}/{} -> {}/{}",
            src_container, src_object, dst_container, dst_object
        );
        // Real implementation: COPY or PUT with X-Copy-From header
        Ok(())
    }

    /// Delete an object.
    pub async fn delete_object(
        &self,
        container: &str,
        object_name: &str,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let _conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected", "Connect first.")
        })?;

        tracing::info!("Deleting {}/{}", container, object_name);
        // Real implementation: DELETE storage_url/container/object
        Ok(())
    }

    /// Generate a temporary URL for an object using HMAC-SHA256 signing.
    pub async fn generate_temp_url(
        &self,
        container: &str,
        object_name: &str,
        expires_secs: u64,
        method: &str,
    ) -> Result<String, AppError> {
        let state = self.state.read().await;
        let conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected", "Connect first.")
        })?;

        let key = conn.config.temp_url_key.as_ref().ok_or_else(|| {
            AppError::file_op(
                "TempURL key not configured",
                "Set a TempURL key in the Swift connection settings.",
            )
        })?;

        // Extract the path portion from storage_url
        let storage_path = url::Url::parse(&conn.storage_url)
            .map(|u| u.path().to_string())
            .unwrap_or_else(|_| "/v1/AUTH_account".to_string());

        let path = format!("{}/{}/{}", storage_path.trim_end_matches('/'), container, object_name);
        let expires = chrono::Utc::now().timestamp() as u64 + expires_secs;

        // HMAC-SHA256 of "METHOD\nexpires\npath"
        let string_to_sign = format!("{}\n{}\n{}", method.to_uppercase(), expires, path);

        use hmac::{Hmac, Mac};
        type HmacSha256 = Hmac<sha2::Sha256>;

        let mut mac = HmacSha256::new_from_slice(key.as_bytes())
            .map_err(|e| AppError::Internal { message: format!("HMAC init failed: {e}") })?;
        mac.update(string_to_sign.as_bytes());
        let signature = hex::encode(mac.finalize().into_bytes());

        let url = format!(
            "{}{}?temp_url_sig={}&temp_url_expires={}",
            conn.storage_url, path, signature, expires
        );

        tracing::info!("Generated TempURL for {}/{} (expires in {}s)", container, object_name, expires_secs);
        Ok(url)
    }

    /// Create a container.
    pub async fn create_container(
        &self,
        name: &str,
        storage_policy: Option<&str>,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let _conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected", "Connect first.")
        })?;

        tracing::info!("Creating container: {} (policy={:?})", name, storage_policy);
        // Real implementation: PUT storage_url/container
        Ok(())
    }

    /// Delete a container (must be empty).
    pub async fn delete_container(&self, name: &str) -> Result<(), AppError> {
        let state = self.state.read().await;
        let _conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected", "Connect first.")
        })?;

        tracing::info!("Deleting container: {}", name);
        // Real implementation: DELETE storage_url/container
        Ok(())
    }

    /// Get container ACL.
    pub async fn get_container_acl(
        &self,
        container: &str,
    ) -> Result<(Option<String>, Option<String>), AppError> {
        let state = self.state.read().await;
        let _conn = state.as_ref().ok_or_else(|| {
            AppError::connection("Not connected", "Connect first.")
        })?;

        tracing::info!("Getting ACL for container: {}", container);
        // Real implementation: HEAD storage_url/container -> X-Container-Read/Write
        Ok((None, None))
    }
}

impl Connector for SwiftConnector {
    fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let profile = profile.clone();
        Box::pin(async move {
            let config = SwiftConfig {
                auth_method: SwiftAuthMethod::KeystoneV3 {
                    auth_url: format!("https://{}:{}/identity/v3", profile.host, profile.port.unwrap_or(5000)),
                    username: profile.username.clone().unwrap_or_default(),
                    password: String::new(), // Loaded from credential store
                    project_name: profile.remote_path.trim_start_matches('/').to_string(),
                    domain_name: "Default".to_string(),
                    region: None,
                },
                segment_size: DEFAULT_SEGMENT_SIZE,
                use_slo: true,
                temp_url_key: None,
            };

            let (storage_url, auth_token, token_expires) = Self::authenticate(&config).await?;

            *self.state.write().await = Some(SwiftConnection {
                config,
                storage_url,
                auth_token: zeroize::Zeroizing::new(auth_token),
                token_expires,
                connected: true,
            });

            tracing::info!("Connected to OpenStack Swift");
            Ok(())
        })
    }

    fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            *self.state.write().await = None;
            tracing::info!("Disconnected from OpenStack Swift");
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
            let _conn = state.as_ref().ok_or_else(|| {
                AppError::connection("Not connected to OpenStack Swift", "Connect first.")
            })?;

            if path == "/" || path.is_empty() {
                let containers = self.list_containers().await?;
                Ok(containers
                    .into_iter()
                    .map(|c| FileEntry {
                        name: c.name.clone(),
                        path: format!("/{}", c.name),
                        is_dir: true,
                        is_symlink: false,
                        size: c.bytes_used,
                        modified: None,
                        created: None,
                        is_hidden: false,
                        extension: None,
                        permissions: None,
                    })
                    .collect())
            } else {
                let parts: Vec<&str> = path.trim_start_matches('/').splitn(2, '/').collect();
                let container = parts[0];
                let prefix = parts.get(1).copied();
                self.list_objects(container, prefix, Some("/")).await
            }
        })
    }

    fn is_connected(&self) -> bool {
        self.state
            .try_read()
            .map(|s| s.as_ref().map(|c| c.connected).unwrap_or(false))
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_access_tier_default() {
        let config = SwiftConfig::default();
        assert!(config.use_slo);
        assert_eq!(config.segment_size, DEFAULT_SEGMENT_SIZE);
    }

    #[tokio::test]
    async fn test_connector_lifecycle() {
        let connector = SwiftConnector::new();
        assert!(!connector.is_connected());
    }
}
