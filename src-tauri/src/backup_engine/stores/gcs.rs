//! Google Cloud Storage through the JSON API.
//!
//! Credentials: a service-account key (JSON; a signed JWT is exchanged for
//! an access token) or an OAuth access token. Uploads carry CRC32C and MD5
//! checksums that the service verifies; downloads are checked against the
//! returned hashes. Object retention locks are supported.

use super::http::{self, TlsOptions};
use super::{child_within_one, direct_child, join_key, BoxFut, ClassByKind, ObjectStore};
use crate::core::error::AppError;
use base64::Engine;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use ufop_core::backup::backend::LockMode;
use ufop_core::backup::RemoteFile;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct GcsConfig {
    pub bucket: String,
    pub prefix: String,
    /// Project id (only needed to create the bucket).
    pub project: String,
    /// e.g. `NEARLINE`, `COLDLINE`, `ARCHIVE` (empty = bucket default).
    pub storage_class: String,
    /// API endpoint; empty = `https://storage.googleapis.com`.
    pub endpoint: String,
    /// Send CRC32C/MD5 with uploads and verify downloads.
    pub checksums: bool,
    pub tls: TlsOptions,
    /// OAuth scope requested for a service-account key; empty = read-only
    /// scope for read-only connections, else full control.
    pub scope: String,
    /// Storage class per volume kind, overriding `storage_class`.
    pub storage_class_by_kind: ClassByKind,
}

pub const SCOPE_FULL_CONTROL: &str = "https://www.googleapis.com/auth/devstorage.full_control";
pub const SCOPE_READ_ONLY: &str = "https://www.googleapis.com/auth/devstorage.read_only";

impl Default for GcsConfig {
    fn default() -> Self {
        Self {
            bucket: String::new(),
            prefix: String::new(),
            project: String::new(),
            storage_class: String::new(),
            endpoint: String::new(),
            checksums: true,
            tls: TlsOptions::default(),
            scope: String::new(),
            storage_class_by_kind: ClassByKind::default(),
        }
    }
}

#[derive(Deserialize)]
pub(crate) struct ServiceAccount {
    pub client_email: String,
    pub private_key: String,
    #[serde(default = "default_token_uri")]
    pub token_uri: String,
}

fn default_token_uri() -> String {
    "https://oauth2.googleapis.com/token".into()
}

enum Cred {
    Token(String),
    Account(ServiceAccount),
}

pub struct GcsStore {
    cfg: GcsConfig,
    cred: Cred,
    /// OAuth scope asked for (service-account keys).
    scope: String,
    token: Mutex<Option<(String, std::time::Instant)>>,
    http: reqwest::Client,
    base: String,
}

/// CRC32C (Castagnoli), as GCS reports in `x-goog-hash`.
pub fn crc32c(data: &[u8]) -> u32 {
    static TABLE: std::sync::OnceLock<[u32; 256]> = std::sync::OnceLock::new();
    let t = TABLE.get_or_init(|| {
        let mut t = [0u32; 256];
        for (i, e) in t.iter_mut().enumerate() {
            let mut c = i as u32;
            for _ in 0..8 {
                c = if c & 1 != 0 { 0x82F6_3B78 ^ (c >> 1) } else { c >> 1 };
            }
            *e = c;
        }
        t
    });
    let mut crc = !0u32;
    for b in data {
        crc = t[((crc ^ *b as u32) & 0xff) as usize] ^ (crc >> 8);
    }
    !crc
}

fn b64(d: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(d)
}

fn b64url(d: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(d)
}

impl GcsStore {
    /// `secret` = service-account key JSON, or an OAuth access token.
    pub fn new(cfg: GcsConfig, secret: String) -> Result<Self, AppError> {
        Self::with_access(cfg, secret, false)
    }

    /// [`new`](Self::new) for a connection that only reads (`read_only`):
    /// a service-account key then asks for the read-only scope.
    pub fn with_access(cfg: GcsConfig, secret: String, read_only: bool) -> Result<Self, AppError> {
        if cfg.bucket.trim().is_empty() {
            return Err(AppError::validation("Enter the bucket name"));
        }
        let s = secret.trim();
        let cred = if s.starts_with('{') {
            Cred::Account(serde_json::from_str(s).map_err(|e| AppError::validation(format!("Invalid service-account key: {e}")))?)
        } else {
            Cred::Token(s.to_string())
        };
        let base = if cfg.endpoint.trim().is_empty() { "https://storage.googleapis.com".to_string() } else { cfg.endpoint.trim().trim_end_matches('/').to_string() };
        let scope = match cfg.scope.trim() {
            "" if read_only => SCOPE_READ_ONLY.to_string(),
            "" => SCOPE_FULL_CONTROL.to_string(),
            s => s.to_string(),
        };
        Ok(Self { http: http::client(&cfg.tls)?, cfg, cred, scope, token: Mutex::new(None), base })
    }

    /// The OAuth scope this store requests.
    pub fn scope(&self) -> &str {
        &self.scope
    }

    fn jwt(&self, sa: &ServiceAccount) -> Result<String, AppError> {
        service_account_jwt(sa, &self.scope)
    }

    fn folder_prefix(&self) -> String {
        let prefix = self.cfg.prefix.trim_matches('/');
        if prefix.is_empty() { String::new() } else { format!("{prefix}/") }
    }

    /// Objects under `pfx` (all pages) and, with the delimiter, the sub-prefixes.
    async fn list_prefix(&self, pfx: &str, delimiter: bool) -> Result<(Vec<(String, u64)>, Vec<String>), AppError> {
        let (mut items, mut folders) = (Vec::new(), Vec::new());
        let mut page: Option<String> = None;
        loop {
            let mut q = vec![("prefix", pfx.to_string()), ("fields", "items(name,size),prefixes,nextPageToken".to_string())];
            if delimiter {
                q.push(("delimiter", "/".to_string()));
            }
            if let Some(p) = &page {
                q.push(("pageToken", p.clone()));
            }
            let r = self.http.get(format!("{}/storage/v1/b/{}/o", self.base, self.cfg.bucket)).query(&q).bearer_auth(self.bearer().await?).send().await.map_err(|e| http::net("GCS listing", e))?;
            let v: serde_json::Value = http::check(r, "GCS listing").await?.json().await.map_err(|e| http::net("GCS listing", e))?;
            for it in v["items"].as_array().into_iter().flatten() {
                let name = it["name"].as_str().unwrap_or_default().to_string();
                let size = it["size"].as_str().and_then(|s| s.parse().ok()).or_else(|| it["size"].as_u64()).unwrap_or(0);
                items.push((name, size));
            }
            folders.extend(v["prefixes"].as_array().into_iter().flatten().filter_map(|p| p.as_str().map(String::from)));
            page = v["nextPageToken"].as_str().map(String::from);
            if page.is_none() {
                break;
            }
        }
        Ok((items, folders))
    }
}

/// RS256-signed JWT bearer assertion for a service account and scope.
pub(crate) fn service_account_jwt(sa: &ServiceAccount, scope: &str) -> Result<String, AppError> {
    {
        let pem = sa.private_key.replace("\\n", "\n");
        let der: String = pem.lines().filter(|l| !l.starts_with("-----")).collect();
        let der = base64::engine::general_purpose::STANDARD.decode(der.trim()).map_err(|_| AppError::validation("The service-account private key is not valid PEM"))?;
        let key = ring::signature::RsaKeyPair::from_pkcs8(&der).map_err(|_| AppError::validation("The service-account private key is not a PKCS#8 RSA key"))?;
        let now = chrono::Utc::now().timestamp();
        let header = b64url(br#"{"alg":"RS256","typ":"JWT"}"#);
        let claims = serde_json::json!({
            "iss": sa.client_email,
            "scope": scope,
            "aud": sa.token_uri,
            "iat": now,
            "exp": now + 3600,
        });
        let payload = b64url(claims.to_string().as_bytes());
        let input = format!("{header}.{payload}");
        let mut sig = vec![0u8; key.public().modulus_len()];
        key.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), input.as_bytes(), &mut sig)
            .map_err(|_| AppError::internal("JWT signing failed"))?;
        Ok(format!("{input}.{}", b64url(&sig)))
    }
}

impl GcsStore {
    async fn bearer(&self) -> Result<String, AppError> {
        match &self.cred {
            Cred::Token(t) => Ok(t.clone()),
            Cred::Account(sa) => {
                let mut cache = self.token.lock().await;
                if let Some((t, exp)) = cache.as_ref() {
                    if *exp > std::time::Instant::now() {
                        return Ok(t.clone());
                    }
                }
                let jwt = self.jwt(sa)?;
                let r = self
                    .http
                    .post(&sa.token_uri)
                    .form(&[("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"), ("assertion", jwt.as_str())])
                    .send()
                    .await
                    .map_err(|e| http::net("Google token", e))?;
                let v: serde_json::Value = http::check(r, "Google token").await?.json().await.map_err(|e| http::net("Google token", e))?;
                let t = v["access_token"].as_str().ok_or_else(|| AppError::connection("Google returned no access token", "Check the service account."))?.to_string();
                let ttl = v["expires_in"].as_u64().unwrap_or(3600).saturating_sub(120);
                *cache = Some((t.clone(), std::time::Instant::now() + std::time::Duration::from_secs(ttl)));
                Ok(t)
            }
        }
    }

    fn obj_url(&self, key: &str) -> String {
        format!("{}/storage/v1/b/{}/o/{}", self.base, self.cfg.bucket, urlencoding::encode(key))
    }
}

impl ObjectStore for GcsStore {
    fn describe(&self) -> String {
        format!("gs://{}/{}", self.cfg.bucket, self.cfg.prefix.trim_matches('/'))
    }

    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let prefix = self.cfg.prefix.trim_matches('/');
            let (items, _) = self.list_prefix(&self.folder_prefix(), true).await?;
            Ok(items.into_iter().filter_map(|(n, size)| direct_child(prefix, &n).map(|n| RemoteFile { name: n.to_string(), size })).collect())
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
                let (items, _) = self.list_prefix(&pfx, false).await?;
                return Ok(items.into_iter().filter_map(|(n, size)| child_within_one(&prefix, &n).map(|name| RemoteFile { name, size })).collect());
            }
            let (top, folders) = self.list_prefix(&pfx, true).await?;
            let mut out: Vec<RemoteFile> = top.into_iter().filter_map(|(n, size)| direct_child(&prefix, &n).map(|n| RemoteFile { name: n.to_string(), size })).collect();
            let p2 = prefix.clone();
            out.extend(
                super::list_folders(folders, parallelism, |f| {
                    let prefix = p2.clone();
                    async move {
                        let (items, _) = self.list_prefix(&f, true).await?;
                        Ok(items.into_iter().filter_map(|(n, size)| child_within_one(&prefix, &n).map(|name| RemoteFile { name, size })).collect())
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
            let mut meta = serde_json::json!({ "name": key, "contentType": "application/octet-stream" });
            let class = self.cfg.storage_class_by_kind.pick(name, &self.cfg.storage_class);
            if !class.is_empty() {
                meta["storageClass"] = class.into();
            }
            if self.cfg.checksums {
                use md5::Digest;
                meta["crc32c"] = b64(&crc32c(&data).to_be_bytes()).into();
                meta["md5Hash"] = b64(&md5::Md5::digest(&data)).into();
            }
            let boundary = format!("ufop-{}", uuid::Uuid::new_v4().simple());
            let mut body = format!("--{boundary}\r\nContent-Type: application/json; charset=UTF-8\r\n\r\n{meta}\r\n--{boundary}\r\nContent-Type: application/octet-stream\r\n\r\n").into_bytes();
            body.extend_from_slice(&data);
            body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
            let r = self
                .http
                .post(format!("{}/upload/storage/v1/b/{}/o?uploadType=multipart", self.base, self.cfg.bucket))
                .bearer_auth(self.bearer().await?)
                .header("content-type", format!("multipart/related; boundary={boundary}"))
                .body(body)
                .send()
                .await
                .map_err(|e| http::net("GCS upload", e))?;
            http::check(r, "GCS upload").await?;
            Ok(())
        })
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let r = self.http.get(format!("{}?alt=media", self.obj_url(&key))).bearer_auth(self.bearer().await?).send().await.map_err(|e| http::net("GCS download", e))?;
            let r = http::check(r, "GCS download").await?;
            let hash = r.headers().get("x-goog-hash").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
            let data = r.bytes().await.map_err(|e| http::net("GCS download", e))?.to_vec();
            if self.cfg.checksums {
                if let Some(c) = hash.split(',').filter_map(|p| p.trim().strip_prefix("crc32c=")).next() {
                    if c != b64(&crc32c(&data).to_be_bytes()) {
                        return Err(AppError::transfer(format!("{name}: CRC32C mismatch after download"), "Retry; the transfer was corrupted."));
                    }
                }
            }
            Ok(data)
        })
    }

    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let r = self.http.delete(self.obj_url(&key)).bearer_auth(self.bearer().await?).send().await.map_err(|e| http::net("GCS delete", e))?;
            http::check(r, "GCS delete").await?;
            Ok(())
        })
    }

    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            let token = self.bearer().await?;
            let r = self.http.get(format!("{}/storage/v1/b/{}", self.base, self.cfg.bucket)).bearer_auth(&token).send().await.map_err(|e| http::net("GCS bucket check", e))?;
            if r.status().is_success() {
                return Ok(());
            }
            if r.status().as_u16() != 404 {
                http::check(r, "GCS bucket check").await?;
            }
            let r = self
                .http
                .post(format!("{}/storage/v1/b", self.base))
                .query(&[("project", self.cfg.project.as_str())])
                .bearer_auth(&token)
                .json(&serde_json::json!({ "name": self.cfg.bucket }))
                .send()
                .await
                .map_err(|e| http::net("GCS bucket creation", e))?;
            http::check(r, "GCS bucket creation").await?;
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
            let r = self
                .http
                .patch(self.obj_url(&key))
                .bearer_auth(self.bearer().await?)
                .json(&serde_json::json!({ "retention": { "mode": m, "retainUntilTime": until.to_rfc3339() } }))
                .send()
                .await
                .map_err(|e| http::net("GCS retention", e))?;
            http::check(r, "GCS retention").await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32c_reference() {
        // RFC 3720 test vector: 32 bytes of zeros.
        assert_eq!(crc32c(&[0u8; 32]), 0x8A91_36AA);
        assert_eq!(crc32c(b"123456789"), 0xE306_9283);
    }
}
