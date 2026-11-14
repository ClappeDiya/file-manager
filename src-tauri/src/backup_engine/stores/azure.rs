//! Azure Blob Storage, including the Azurite emulator. Sign-in with the
//! account key, a SAS token, or Microsoft Entra ID (service principal with
//! a secret or certificate, federated workload identity, managed identity).
//!
//! Large volumes go up as blocks of a configurable size; access tiers (also
//! per volume kind), archive rehydration, version-level immutability
//! policies and sharded layouts (`dir/name` blob prefixes) are supported.

use super::http::{self, TlsOptions};
use super::{child_within_one, direct_child, join_key, BoxFut, ClassByKind, ObjectStore};
use crate::connectors::aws_chain::ChainEnv;
use crate::connectors::azure_ad::{AadCredential, AadTokenSource, STORAGE_SCOPE};
use crate::connectors::azure_rest::{AzureClient, ClientAuth, Service};
use crate::core::error::AppError;
use base64::Engine;
use serde::{Deserialize, Serialize};
use ufop_core::backup::backend::LockMode;
use ufop_core::backup::RemoteFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AzureConfig {
    pub account: String,
    pub container: String,
    pub prefix: String,
    /// Blob endpoint; empty = `https://<account>.blob.core.windows.net`.
    /// Azurite: `http://127.0.0.1:10000/<account>`.
    pub endpoint: String,
    /// `Hot`, `Cool`, `Cold` or `Archive` (empty = account default).
    pub access_tier: String,
    /// Upload volumes larger than this in blocks of this size (MiB).
    pub block_size_mb: u32,
    /// Rehydration priority for archived blobs: `Standard` or `High`.
    pub rehydrate_priority: String,
    pub tls: TlsOptions,
    /// How to sign in (default: account key or SAS from the stored secret).
    pub auth: AzureAuth,
    /// Entra ID authority; empty = `https://login.microsoftonline.com`.
    pub authority_host: String,
    /// Access tier per volume kind, overriding `access_tier` (`Archive` refused).
    pub access_tier_by_kind: ClassByKind,
}

/// Azure sign-in. The secret in the credential store holds the account key,
/// SAS token, client secret or PFX password, depending on the method.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AzureAuth {
    /// Account key (a SAS token is recognised too, as before).
    #[default]
    SharedKey,
    Sas,
    /// App registration with a client secret.
    ServicePrincipalSecret {
        #[serde(default)]
        tenant: String,
        #[serde(default)]
        client_id: String,
    },
    /// App registration with a certificate: a PEM (private key + certificate)
    /// or PFX file; the secret is the PFX password, if any.
    ServicePrincipalCertificate {
        #[serde(default)]
        tenant: String,
        #[serde(default)]
        client_id: String,
        #[serde(default)]
        cert_path: String,
    },
    /// Workload identity federation: a token file used as client assertion.
    FederatedToken {
        #[serde(default)]
        tenant: String,
        #[serde(default)]
        client_id: String,
        #[serde(default)]
        token_file: String,
    },
    /// The machine's managed identity (user-assigned: its client id).
    ManagedIdentity {
        #[serde(default)]
        client_id: Option<String>,
    },
}

impl AzureAuth {
    /// Whether this method reads a secret from the credential store
    /// (always for keys and client secrets; optional for certificates).
    pub fn needs_secret(&self) -> bool {
        matches!(self, Self::SharedKey | Self::Sas | Self::ServicePrincipalSecret { .. })
    }
}

/// Access tiers whose blobs must be rehydrated before reading.
pub const ARCHIVE_TIERS: &[&str] = &["Archive"];

impl Default for AzureConfig {
    fn default() -> Self {
        Self {
            account: String::new(),
            container: String::new(),
            prefix: String::new(),
            endpoint: String::new(),
            access_tier: String::new(),
            block_size_mb: 4,
            rehydrate_priority: "Standard".into(),
            tls: TlsOptions::default(),
            auth: AzureAuth::SharedKey,
            authority_host: String::new(),
            access_tier_by_kind: ClassByKind::default(),
        }
    }
}

pub struct AzureStore {
    cfg: AzureConfig,
    client: AzureClient,
}

impl AzureStore {
    /// `secret` = base64 account key, or a SAS token (`sv=…` / `?sv=…`).
    pub fn new(cfg: AzureConfig, secret: String) -> Result<Self, AppError> {
        Self::with_env(cfg, Some(secret), ChainEnv::process())
    }

    /// Open with the sign-in method of `cfg.auth`; `env` is read for
    /// managed-identity endpoints and the authority host.
    pub fn with_env(cfg: AzureConfig, secret: Option<String>, env: ChainEnv) -> Result<Self, AppError> {
        if cfg.account.trim().is_empty() || cfg.container.trim().is_empty() {
            return Err(AppError::validation("Enter the storage account and container"));
        }
        cfg.access_tier_by_kind.refuse(ARCHIVE_TIERS, "Access tier")?;
        let secret = secret.unwrap_or_default();
        let aad = |c: AadCredential| -> Result<ClientAuth, AppError> {
            let src = AadTokenSource::new(c, Some(cfg.authority_host.as_str()), http::client(&cfg.tls)?, env.clone())?;
            Ok(ClientAuth::Bearer(std::sync::Arc::new(src), STORAGE_SCOPE.into()))
        };
        let auth = match &cfg.auth {
            AzureAuth::SharedKey => ClientAuth::Auto(secret),
            AzureAuth::Sas => ClientAuth::Sas(secret),
            AzureAuth::ServicePrincipalSecret { tenant, client_id } => aad(AadCredential::Secret { tenant: tenant.clone(), client_id: client_id.clone(), secret })?,
            AzureAuth::ServicePrincipalCertificate { tenant, client_id, cert_path } => {
                aad(AadCredential::Certificate { tenant: tenant.clone(), client_id: client_id.clone(), path: cert_path.clone(), password: Some(secret).filter(|s| !s.is_empty()) })?
            }
            AzureAuth::FederatedToken { tenant, client_id, token_file } => aad(AadCredential::Federated { tenant: tenant.clone(), client_id: client_id.clone(), token_file: token_file.clone() })?,
            AzureAuth::ManagedIdentity { client_id } => aad(AadCredential::Managed { client_id: client_id.clone() })?,
        };
        let client = AzureClient::with_auth(&cfg.account, &cfg.endpoint, Service::Blob, auth, &cfg.tls)?;
        Ok(Self { cfg, client })
    }

    async fn list_prefix(&self, pfx: &str, delimiter: bool) -> Result<(Vec<(String, u64)>, Vec<String>), AppError> {
        let (mut blobs, mut folders) = (Vec::new(), Vec::new());
        let mut marker = String::new();
        loop {
            let mut q = vec![("restype", "container".to_string()), ("comp", "list".to_string()), ("prefix", pfx.to_string())];
            if delimiter {
                q.push(("delimiter", "/".to_string()));
            }
            if !marker.is_empty() {
                q.push(("marker", marker.clone()));
            }
            let r = http::check(self.send(reqwest::Method::GET, None, &q, &[], Vec::new()).await?, "Azure listing").await?;
            let xml = r.text().await.map_err(|e| http::net("Azure listing", e))?;
            for b in http::xml_all(&xml, "Blob") {
                let name = http::xml_unescape(&http::xml_one(&b, "Name").unwrap_or_default());
                blobs.push((name, http::xml_one(&b, "Content-Length").and_then(|s| s.parse().ok()).unwrap_or(0)));
            }
            for p in http::xml_all(&xml, "BlobPrefix") {
                if let Some(n) = http::xml_one(&p, "Name") {
                    folders.push(http::xml_unescape(&n));
                }
            }
            marker = http::xml_one(&xml, "NextMarker").unwrap_or_default();
            if marker.is_empty() {
                break;
            }
        }
        Ok((blobs, folders))
    }

    fn folder_prefix(&self) -> String {
        let prefix = self.cfg.prefix.trim_matches('/');
        if prefix.is_empty() { String::new() } else { format!("{prefix}/") }
    }

    async fn send(&self, method: reqwest::Method, blob: Option<&str>, query: &[(&str, String)], extra: &[(&str, String)], body: Vec<u8>) -> Result<reqwest::Response, AppError> {
        let path = match blob {
            Some(b) => format!("/{}/{}", self.cfg.container, super::s3::uri_encode(b, true)),
            None => format!("/{}", self.cfg.container),
        };
        self.client.send(method, &path, query, extra, body).await
    }

    fn md5(data: &[u8]) -> String {
        use md5::Digest;
        base64::engine::general_purpose::STANDARD.encode(md5::Md5::digest(data))
    }
}

impl ObjectStore for AzureStore {
    fn describe(&self) -> String {
        format!("azure://{}/{}/{}", self.cfg.account, self.cfg.container, self.cfg.prefix.trim_matches('/'))
    }

    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let prefix = self.cfg.prefix.trim_matches('/');
            let (blobs, _) = self.list_prefix(&self.folder_prefix(), true).await?;
            Ok(blobs.into_iter().filter_map(|(n, size)| direct_child(prefix, &n).map(|n| RemoteFile { name: n.to_string(), size })).collect())
        })
    }

    fn supports_subfolders(&self) -> bool {
        true
    }

    fn list_recursive(&self, parallelism: usize) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let prefix = self.cfg.prefix.trim_matches('/').to_string();
            let pfx = self.folder_prefix();
            if parallelism <= 1 {
                let (blobs, _) = self.list_prefix(&pfx, false).await?;
                return Ok(blobs.into_iter().filter_map(|(n, size)| child_within_one(&prefix, &n).map(|name| RemoteFile { name, size })).collect());
            }
            let (top, folders) = self.list_prefix(&pfx, true).await?;
            let mut out: Vec<RemoteFile> = top.into_iter().filter_map(|(n, size)| direct_child(&prefix, &n).map(|n| RemoteFile { name: n.to_string(), size })).collect();
            let p2 = prefix.clone();
            out.extend(
                super::list_folders(folders, parallelism, |f| {
                    let prefix = p2.clone();
                    async move {
                        let (blobs, _) = self.list_prefix(&f, true).await?;
                        Ok(blobs.into_iter().filter_map(|(n, size)| child_within_one(&prefix, &n).map(|name| RemoteFile { name, size })).collect())
                    }
                })
                .await?,
            );
            Ok(out)
        })
    }

    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let block = (self.cfg.block_size_mb.max(1) as usize) << 20;
            let mut tier = Vec::new();
            let t = self.cfg.access_tier_by_kind.pick(name, &self.cfg.access_tier);
            if !t.is_empty() {
                tier.push(("x-ms-access-tier", t));
            }
            if data.len() <= block {
                let mut h = vec![("x-ms-blob-type", "BlockBlob".to_string()), ("content-md5", Self::md5(&data)), ("content-type", "application/octet-stream".to_string())];
                h.extend(tier);
                http::check(self.send(reqwest::Method::PUT, Some(&key), &[], &h, data).await?, "Azure upload").await?;
                return Ok(());
            }
            let mut ids = Vec::new();
            for (i, chunk) in data.chunks(block).enumerate() {
                // Block ids must have equal length within a blob.
                let id = base64::engine::general_purpose::STANDARD.encode(format!("block-{i:08}"));
                http::check(
                    self.send(reqwest::Method::PUT, Some(&key), &[("comp", "block".into()), ("blockid", id.clone())], &[("content-md5", Self::md5(chunk))], chunk.to_vec()).await?,
                    "Azure block upload",
                )
                .await?;
                ids.push(id);
            }
            let body: String = ids.iter().map(|i| format!("<Latest>{i}</Latest>")).collect();
            let body = format!("<?xml version=\"1.0\" encoding=\"utf-8\"?><BlockList>{body}</BlockList>").into_bytes();
            let mut h = vec![("content-type", "application/xml".to_string())];
            h.extend(tier);
            http::check(self.send(reqwest::Method::PUT, Some(&key), &[("comp", "blocklist".into())], &h, body).await?, "Azure block list").await?;
            Ok(())
        })
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let r = self.send(reqwest::Method::GET, Some(&key), &[], &[], Vec::new()).await?;
            if r.status().as_u16() == 409 {
                let body = r.text().await.unwrap_or_default();
                if body.contains("BlobArchived") {
                    // Cold storage: ask Azure to rehydrate the blob to Hot.
                    let h = [("x-ms-access-tier", "Hot".to_string()), ("x-ms-rehydrate-priority", self.cfg.rehydrate_priority.clone())];
                    let _ = self.send(reqwest::Method::PUT, Some(&key), &[("comp", "tier".into())], &h, Vec::new()).await;
                    return Err(AppError::connection(format!("{name} is in the Archive tier; rehydration was requested"), "Retry after rehydration completes (up to 15 hours)."));
                }
                return Err(AppError::connection(format!("Azure download failed: {body}"), "Check the destination."));
            }
            let r = http::check(r, "Azure download").await?;
            Ok(r.bytes().await.map_err(|e| http::net("Azure download", e))?.to_vec())
        })
    }

    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            http::check(self.send(reqwest::Method::DELETE, Some(&key), &[], &[], Vec::new()).await?, "Azure delete").await?;
            Ok(())
        })
    }

    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            let r = self.send(reqwest::Method::PUT, None, &[("restype", "container".into())], &[], Vec::new()).await?;
            if r.status().as_u16() == 409 {
                return Ok(()); // already exists
            }
            http::check(r, "Azure container creation").await?;
            Ok(())
        })
    }

    fn supports_lock(&self) -> bool {
        true
    }

    fn lock<'a>(&'a self, name: &'a str, until: chrono::DateTime<chrono::Utc>, mode: LockMode) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let m = match mode {
                LockMode::Governance => "Unlocked",
                LockMode::Compliance => "Locked",
            };
            let h = [("x-ms-immutability-policy-until-date", until.format("%a, %d %b %Y %H:%M:%S GMT").to_string()), ("x-ms-immutability-policy-mode", m.to_string())];
            http::check(self.send(reqwest::Method::PUT, Some(&key), &[("comp", "immutabilityPolicies".into())], &h, Vec::new()).await?, "Azure immutability policy").await?;
            Ok(())
        })
    }
}
