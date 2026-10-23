//! Amazon S3 and S3-compatible object storage (IDrive e2, Storj gateway,
//! Aliyun OSS, Tencent COS, Wasabi, MinIO, Ceph, …) with AWS Signature V4.
//!
//! Supports storage classes, S3 Object Lock retention, restoring objects
//! from Glacier / Deep Archive (cold storage), multipart uploads, path-style
//! or virtual-host addressing and custom TLS trust.

use super::http::{self, TlsOptions};
use super::{direct_child, join_key, BoxFut, ObjectStore};
use crate::core::error::AppError;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use ufop_core::backup::backend::LockMode;
use ufop_core::backup::RemoteFile;

/// Restoring archived (Glacier / Deep Archive) objects before download.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColdStorage {
    /// Days the restored copy stays available.
    pub restore_days: u32,
    /// `Expedited`, `Standard` or `Bulk`.
    pub tier: String,
    /// Wait this long for a restore to finish (0 = request it and fail fast).
    pub wait_minutes: u32,
}

impl Default for ColdStorage {
    fn default() -> Self {
        Self { restore_days: 3, tier: "Standard".into(), wait_minutes: 0 }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct S3Config {
    /// Service preset for the UI (`aws`, `idrive_e2`, `storj`, `aliyun_oss`, `tencent_cos`, `wasabi`, `minio`, `custom`).
    pub preset: String,
    /// `https://host[:port]`; empty = AWS for `region`.
    pub endpoint: String,
    pub region: String,
    pub bucket: String,
    /// Folder inside the bucket.
    pub prefix: String,
    pub access_key_id: String,
    /// `bucket` in the path instead of the host name.
    pub path_style: bool,
    /// e.g. `STANDARD_IA`, `GLACIER_IR`, `DEEP_ARCHIVE` (empty = bucket default).
    pub storage_class: String,
    /// Upload larger volumes in parts of this many MiB (0 = single request).
    pub part_size_mb: u32,
    pub cold_storage: ColdStorage,
    /// When the bucket has to be created, enable S3 Object Lock on it.
    pub create_with_object_lock: bool,
    pub tls: TlsOptions,
}

impl Default for S3Config {
    fn default() -> Self {
        Self {
            preset: "aws".into(),
            endpoint: String::new(),
            region: "us-east-1".into(),
            bucket: String::new(),
            prefix: String::new(),
            access_key_id: String::new(),
            path_style: false,
            storage_class: String::new(),
            part_size_mb: 64,
            cold_storage: ColdStorage::default(),
            create_with_object_lock: false,
            tls: TlsOptions::default(),
        }
    }
}

/// Endpoint for well-known S3-compatible services.
/// Endpoint for a provider preset (the shared catalog in `connectors::presets`).
pub fn preset_endpoint(preset: &str, region: &str) -> Option<String> {
    crate::connectors::presets::s3_endpoint(preset, region)
}

pub struct S3Store {
    cfg: S3Config,
    secret: String,
    session_token: Option<String>,
    http: reqwest::Client,
    scheme: String,
    host: String,
}

type HmacSha256 = Hmac<Sha256>;

fn hmac(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut m = HmacSha256::new_from_slice(key).expect("HMAC accepts any key length");
    m.update(data);
    m.finalize().into_bytes().to_vec()
}

pub(crate) fn sha_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

/// RFC 3986 encoding as SigV4 requires (`/` kept in paths).
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b'/' if keep_slash => out.push('/'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// AWS Signature V4. `headers` must hold every header to sign (lowercase
/// names), including `host` and `x-amz-date`. Returns the Authorization value.
#[allow(clippy::too_many_arguments)]
pub fn sign_v4(method: &str, path: &str, query: &[(String, String)], headers: &BTreeMap<String, String>, payload_hash: &str, access_key: &str, secret: &str, region: &str, service: &str) -> String {
    let amz_date = &headers["x-amz-date"];
    let date = &amz_date[..8];
    let mut q: Vec<(String, String)> = query.iter().map(|(k, v)| (uri_encode(k, false), uri_encode(v, false))).collect();
    q.sort();
    let canonical_query = q.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&");
    let canonical_headers: String = headers.iter().map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
    let signed: String = headers.keys().cloned().collect::<Vec<_>>().join(";");
    let canonical = format!("{method}\n{path}\n{canonical_query}\n{canonical_headers}\n{signed}\n{payload_hash}");
    let scope = format!("{date}/{region}/{service}/aws4_request");
    let sts = format!("AWS4-HMAC-SHA256\n{amz_date}\n{scope}\n{}", sha_hex(canonical.as_bytes()));
    let k = hmac(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k = hmac(&k, region.as_bytes());
    let k = hmac(&k, service.as_bytes());
    let k = hmac(&k, b"aws4_request");
    let sig = hex::encode(hmac(&k, sts.as_bytes()));
    format!("AWS4-HMAC-SHA256 Credential={access_key}/{scope}, SignedHeaders={signed}, Signature={sig}")
}

impl S3Store {
    /// `secret` = secret access key, optionally followed by a newline and a
    /// session token (temporary credentials).
    pub fn new(cfg: S3Config, secret: String) -> Result<Self, AppError> {
        if cfg.bucket.trim().is_empty() || cfg.access_key_id.trim().is_empty() {
            return Err(AppError::validation("Enter the bucket and access key id"));
        }
        let endpoint = if cfg.endpoint.trim().is_empty() {
            preset_endpoint(&cfg.preset, &cfg.region).or_else(|| preset_endpoint("aws", &cfg.region)).unwrap_or_default()
        } else {
            cfg.endpoint.trim().trim_end_matches('/').to_string()
        };
        let u = url::Url::parse(&endpoint).map_err(|e| AppError::validation(format!("Invalid S3 endpoint '{endpoint}': {e}")))?;
        let host = match u.port() {
            Some(p) => format!("{}:{p}", u.host_str().unwrap_or_default()),
            None => u.host_str().unwrap_or_default().to_string(),
        };
        let (secret, session_token) = match secret.split_once('\n') {
            Some((s, t)) => (s.trim().to_string(), Some(t.trim().to_string()).filter(|t| !t.is_empty())),
            None => (secret.trim().to_string(), None),
        };
        Ok(Self { http: http::client(&cfg.tls)?, scheme: u.scheme().to_string(), host, cfg, secret, session_token })
    }

    /// (host header, request path) for an object key ("" = bucket).
    fn locate(&self, key: &str) -> (String, String) {
        let k = uri_encode(key, true);
        if self.cfg.path_style {
            (self.host.clone(), if key.is_empty() { format!("/{}", self.cfg.bucket) } else { format!("/{}/{k}", self.cfg.bucket) })
        } else {
            (format!("{}.{}", self.cfg.bucket, self.host), format!("/{k}"))
        }
    }

    async fn send(&self, method: reqwest::Method, key: &str, query: &[(&str, &str)], extra: &[(&str, String)], body: Vec<u8>) -> Result<reqwest::Response, AppError> {
        let (host, path) = self.locate(key);
        let payload_hash = sha_hex(&body);
        let mut headers: BTreeMap<String, String> = BTreeMap::new();
        headers.insert("host".into(), host.clone());
        headers.insert("x-amz-content-sha256".into(), payload_hash.clone());
        headers.insert("x-amz-date".into(), chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string());
        if let Some(t) = &self.session_token {
            headers.insert("x-amz-security-token".into(), t.clone());
        }
        for (k, v) in extra {
            headers.insert(k.to_lowercase(), v.clone());
        }
        let q: Vec<(String, String)> = query.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        let auth = sign_v4(method.as_str(), &path, &q, &headers, &payload_hash, &self.cfg.access_key_id, &self.secret, &self.cfg.region, "s3");
        let qs = {
            let mut v: Vec<String> = q.iter().map(|(k, v)| if v.is_empty() && *k != "prefix" { uri_encode(k, false) } else { format!("{}={}", uri_encode(k, false), uri_encode(v, false)) }).collect();
            v.sort();
            if v.is_empty() { String::new() } else { format!("?{}", v.join("&")) }
        };
        let url = format!("{}://{host}{path}{qs}", self.scheme);
        let mut req = self.http.request(method, &url).header("authorization", auth);
        for (k, v) in &headers {
            if k != "host" {
                req = req.header(k.as_str(), v.as_str());
            }
        }
        req.body(body).send().await.map_err(|e| http::net("S3 request", e))
    }

    fn md5_b64(data: &[u8]) -> String {
        use md5::Digest as _;
        base64::engine::general_purpose::STANDARD.encode(md5::Md5::digest(data))
    }

    async fn put_single(&self, key: &str, data: Vec<u8>) -> Result<(), AppError> {
        let mut extra = vec![("content-md5", Self::md5_b64(&data)), ("content-type", "application/octet-stream".to_string())];
        if !self.cfg.storage_class.is_empty() {
            extra.push(("x-amz-storage-class", self.cfg.storage_class.clone()));
        }
        http::check(self.send(reqwest::Method::PUT, key, &[], &extra, data).await?, "S3 upload").await?;
        Ok(())
    }

    async fn put_multipart(&self, key: &str, data: Vec<u8>, part: usize) -> Result<(), AppError> {
        let mut extra = vec![("content-type", "application/octet-stream".to_string())];
        if !self.cfg.storage_class.is_empty() {
            extra.push(("x-amz-storage-class", self.cfg.storage_class.clone()));
        }
        let r = http::check(self.send(reqwest::Method::POST, key, &[("uploads", "")], &extra, Vec::new()).await?, "S3 multipart start").await?;
        let xml = r.text().await.map_err(|e| http::net("S3 multipart start", e))?;
        let upload_id = http::xml_one(&xml, "UploadId").ok_or_else(|| AppError::connection("S3 did not return an upload id", "Check the endpoint."))?;
        let result = async {
            let mut etags = Vec::new();
            for (i, chunk) in data.chunks(part).enumerate() {
                let n = (i + 1).to_string();
                let r = self
                    .send(reqwest::Method::PUT, key, &[("partNumber", &n), ("uploadId", &upload_id)], &[("content-md5", Self::md5_b64(chunk))], chunk.to_vec())
                    .await?;
                let r = http::check(r, "S3 part upload").await?;
                let etag = r.headers().get("etag").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
                etags.push(etag);
            }
            let body: String = etags.iter().enumerate().map(|(i, e)| format!("<Part><PartNumber>{}</PartNumber><ETag>{e}</ETag></Part>", i + 1)).collect();
            let body = format!("<CompleteMultipartUpload>{body}</CompleteMultipartUpload>").into_bytes();
            let r = http::check(self.send(reqwest::Method::POST, key, &[("uploadId", &upload_id)], &[("content-type", "application/xml".to_string())], body).await?, "S3 multipart complete").await?;
            // S3 may report an error inside a 200 response.
            let txt = r.text().await.unwrap_or_default();
            if txt.contains("<Error>") {
                return Err(AppError::connection(format!("S3 multipart complete failed: {txt}"), "Retry the backup."));
            }
            Ok(())
        }
        .await;
        if result.is_err() {
            let _ = self.send(reqwest::Method::DELETE, key, &[("uploadId", &upload_id)], &[], Vec::new()).await;
        }
        result
    }

    /// Current Object Lock retention of a volume: (mode, retain-until),
    /// read from the object's HEAD headers.
    pub async fn retention(&self, name: &str) -> Result<Option<(String, String)>, AppError> {
        let key = join_key(&self.cfg.prefix, name);
        let r = http::check(self.send(reqwest::Method::HEAD, &key, &[], &[], Vec::new()).await?, "S3 retention").await?;
        let h = |n: &str| r.headers().get(n).and_then(|v| v.to_str().ok()).map(String::from);
        Ok(h("x-amz-object-lock-mode").zip(h("x-amz-object-lock-retain-until-date")))
    }

    /// Versions of a key (version ids), newest first.
    pub async fn versions(&self, name: &str) -> Result<Vec<String>, AppError> {
        let key = join_key(&self.cfg.prefix, name);
        let r = http::check(self.send(reqwest::Method::GET, "", &[("versions", ""), ("prefix", &key)], &[], Vec::new()).await?, "S3 versions").await?;
        let xml = r.text().await.map_err(|e| http::net("S3 versions", e))?;
        Ok(http::xml_all(&xml, "Version").into_iter().filter(|v| http::xml_one(v, "Key").as_deref() == Some(key.as_str())).filter_map(|v| http::xml_one(&v, "VersionId")).collect())
    }

    /// Permanently delete one version (refused by S3 while it is locked).
    pub async fn delete_version(&self, name: &str, version: &str) -> Result<(), AppError> {
        let key = join_key(&self.cfg.prefix, name);
        http::check(self.send(reqwest::Method::DELETE, &key, &[("versionId", version)], &[], Vec::new()).await?, "S3 version delete").await?;
        Ok(())
    }

    /// Ask S3 to restore an archived object; optionally wait for it.
    async fn restore_cold(&self, key: &str) -> Result<bool, AppError> {
        let c = &self.cfg.cold_storage;
        let body = format!("<RestoreRequest><Days>{}</Days><GlacierJobParameters><Tier>{}</Tier></GlacierJobParameters></RestoreRequest>", c.restore_days.max(1), c.tier);
        let r = self.send(reqwest::Method::POST, key, &[("restore", "")], &[("content-md5", Self::md5_b64(body.as_bytes()))], body.into_bytes()).await?;
        // 409 RestoreAlreadyInProgress is fine.
        if !(r.status().is_success() || r.status().as_u16() == 409) {
            http::check(r, "S3 restore request").await?;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(c.wait_minutes as u64 * 60);
        while std::time::Instant::now() < deadline {
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
            let h = self.send(reqwest::Method::HEAD, key, &[], &[], Vec::new()).await?;
            let state = h.headers().get("x-amz-restore").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
            if state.contains("ongoing-request=\"false\"") {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

impl ObjectStore for S3Store {
    fn describe(&self) -> String {
        format!("s3://{}/{}", self.cfg.bucket, self.cfg.prefix.trim_matches('/'))
    }

    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            let prefix = self.cfg.prefix.trim_matches('/');
            let pfx = if prefix.is_empty() { String::new() } else { format!("{prefix}/") };
            let mut out = Vec::new();
            let mut token: Option<String> = None;
            loop {
                let mut q: Vec<(&str, &str)> = vec![("list-type", "2"), ("prefix", &pfx)];
                if let Some(t) = &token {
                    q.push(("continuation-token", t));
                }
                let r = http::check(self.send(reqwest::Method::GET, "", &q, &[], Vec::new()).await?, "S3 listing").await?;
                let xml = r.text().await.map_err(|e| http::net("S3 listing", e))?;
                for c in http::xml_all(&xml, "Contents") {
                    let key = http::xml_unescape(&http::xml_one(&c, "Key").unwrap_or_default());
                    let size = http::xml_one(&c, "Size").and_then(|s| s.parse().ok()).unwrap_or(0);
                    if let Some(name) = direct_child(prefix, &key) {
                        out.push(RemoteFile { name: name.to_string(), size });
                    }
                }
                if http::xml_one(&xml, "IsTruncated").as_deref() == Some("true") {
                    token = http::xml_one(&xml, "NextContinuationToken");
                    if token.is_none() {
                        break;
                    }
                } else {
                    break;
                }
            }
            Ok(out)
        })
    }

    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let part = (self.cfg.part_size_mb.max(5) as usize) << 20;
            if self.cfg.part_size_mb > 0 && data.len() > part {
                self.put_multipart(&key, data, part).await
            } else {
                self.put_single(&key, data).await
            }
        })
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            for attempt in 0..2 {
                let r = self.send(reqwest::Method::GET, &key, &[], &[], Vec::new()).await?;
                if r.status().as_u16() == 403 {
                    let body = r.text().await.unwrap_or_default();
                    if body.contains("InvalidObjectState") && attempt == 0 {
                        if self.restore_cold(&key).await? {
                            continue;
                        }
                        return Err(AppError::connection(
                            format!("{name} is in archive storage; a restore was requested"),
                            "Retry once the restore completes (hours for Glacier/Deep Archive), or set a wait time.",
                        ));
                    }
                    return Err(AppError::connection(format!("S3 download refused: {body}"), "Check the credentials."));
                }
                let r = http::check(r, "S3 download").await?;
                return Ok(r.bytes().await.map_err(|e| http::net("S3 download", e))?.to_vec());
            }
            Err(AppError::connection(format!("{name} could not be downloaded"), "Retry later."))
        })
    }

    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            http::check(self.send(reqwest::Method::DELETE, &key, &[], &[], Vec::new()).await?, "S3 delete").await?;
            Ok(())
        })
    }

    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move {
            let h = self.send(reqwest::Method::HEAD, "", &[], &[], Vec::new()).await?;
            if h.status().is_success() {
                return Ok(());
            }
            if h.status().as_u16() != 404 {
                http::check(h, "S3 bucket check").await?;
            }
            let body = if self.cfg.region.is_empty() || self.cfg.region == "us-east-1" {
                Vec::new()
            } else {
                format!("<CreateBucketConfiguration><LocationConstraint>{}</LocationConstraint></CreateBucketConfiguration>", self.cfg.region).into_bytes()
            };
            let mut extra = Vec::new();
            if self.cfg.create_with_object_lock {
                extra.push(("x-amz-bucket-object-lock-enabled", "true".to_string()));
            }
            http::check(self.send(reqwest::Method::PUT, "", &[], &extra, body).await?, "S3 bucket creation").await?;
            Ok(())
        })
    }

    fn modified<'a>(&'a self, name: &'a str) -> BoxFut<'a, Option<chrono::DateTime<chrono::Utc>>> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let r = http::check(self.send(reqwest::Method::HEAD, &key, &[], &[], Vec::new()).await?, "S3 file time").await?;
            Ok(r.headers().get("last-modified").and_then(|v| v.to_str().ok()).and_then(|v| chrono::DateTime::parse_from_rfc2822(v).ok()).map(|t| t.with_timezone(&chrono::Utc)))
        })
    }
    fn supports_lock(&self) -> bool {
        true
    }

    fn lock<'a>(&'a self, name: &'a str, until: chrono::DateTime<chrono::Utc>, mode: LockMode) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let key = join_key(&self.cfg.prefix, name);
            let m = match mode {
                LockMode::Governance => "GOVERNANCE",
                LockMode::Compliance => "COMPLIANCE",
            };
            let body = format!(
                "<Retention xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Mode>{m}</Mode><RetainUntilDate>{}</RetainUntilDate></Retention>",
                until.format("%Y-%m-%dT%H:%M:%S%.3fZ")
            );
            let extra = [("content-md5", Self::md5_b64(body.as_bytes())), ("content-type", "application/xml".to_string())];
            http::check(self.send(reqwest::Method::PUT, &key, &[("retention", "")], &extra, body.into_bytes()).await?, "S3 object lock").await?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// AWS SigV4 test suite, "get-vanilla".
    #[test]
    fn sigv4_reference_vector() {
        let mut h = BTreeMap::new();
        h.insert("host".to_string(), "example.amazonaws.com".to_string());
        h.insert("x-amz-date".to_string(), "20150830T123600Z".to_string());
        let auth = sign_v4("GET", "/", &[], &h, &sha_hex(b""), "AKIDEXAMPLE", "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", "us-east-1", "service");
        assert!(auth.ends_with("Signature=5fa00fa31553b73ebf1942676e86291e8372ff2a2260956d9b8aae1d763fbf31"), "{auth}");
        assert!(auth.contains("Credential=AKIDEXAMPLE/20150830/us-east-1/service/aws4_request, SignedHeaders=host;x-amz-date"));
    }

    #[test]
    fn encoding_and_presets() {
        assert_eq!(uri_encode("a b/c+d", true), "a%20b/c%2Bd");
        assert_eq!(uri_encode("a/b", false), "a%2Fb");
        assert_eq!(preset_endpoint("aliyun_oss", "cn-hangzhou").unwrap(), "https://oss-cn-hangzhou.aliyuncs.com");
        assert_eq!(preset_endpoint("storj", "x").unwrap(), "https://gateway.storjshare.io");
        let s = S3Store::new(S3Config { bucket: "b".into(), access_key_id: "k".into(), ..Default::default() }, "sec\ntok".into()).unwrap();
        assert_eq!(s.host, "s3.us-east-1.amazonaws.com");
        assert_eq!(s.session_token.as_deref(), Some("tok"));
        assert_eq!(s.locate("p/x").0, "b.s3.us-east-1.amazonaws.com");
    }
}
