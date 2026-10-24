//! Amazon S3 Connector (T-026).
//!
//! Features:
//! - Auth: access key, secret key, region, IAM role
//! - Bucket listing as top-level folders
//! - Multipart upload for files > 5MB (S3 minimum part size)
//! - Resume via ListParts + range requests
//! - Presigned URL generation
//! - S3-compatible endpoints (MinIO, Wasabi, Backblaze B2 S3 mode)

use crate::connectors::Connector;
use crate::core::error::AppError;
use crate::core::types::{ConnectionProfile, FileEntry};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use tokio::sync::RwLock;

type HmacSha256 = Hmac<Sha256>;

// S3 multipart upload threshold: 5MB (S3 minimum part size)
const MULTIPART_THRESHOLD: u64 = 5 * 1024 * 1024;
// Default part size: 8MB
const DEFAULT_PART_SIZE: u64 = 8 * 1024 * 1024;
// Maximum number of parts (S3 limit: 10,000)
const MAX_PARTS: u64 = 10_000;

// ── S3 Advanced Object Properties ──

/// S3 Access Control List (ACL) options.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum S3Acl {
    Private,
    PublicRead,
    PublicReadWrite,
    AuthenticatedRead,
    BucketOwnerRead,
    BucketOwnerFullControl,
}

impl S3Acl {
    pub fn as_header_value(&self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::PublicRead => "public-read",
            Self::PublicReadWrite => "public-read-write",
            Self::AuthenticatedRead => "authenticated-read",
            Self::BucketOwnerRead => "bucket-owner-read",
            Self::BucketOwnerFullControl => "bucket-owner-full-control",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "public-read" => Self::PublicRead,
            "public-read-write" => Self::PublicReadWrite,
            "authenticated-read" => Self::AuthenticatedRead,
            "bucket-owner-read" => Self::BucketOwnerRead,
            "bucket-owner-full-control" => Self::BucketOwnerFullControl,
            _ => Self::Private,
        }
    }
}

/// S3 storage class options.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum S3StorageClass {
    Standard,
    ReducedRedundancy,
    StandardIa,
    OnezoneIa,
    IntelligentTiering,
    Glacier,
    GlacierIr,
    DeepArchive,
}

impl S3StorageClass {
    pub fn as_header_value(&self) -> &'static str {
        match self {
            Self::Standard => "STANDARD",
            Self::ReducedRedundancy => "REDUCED_REDUNDANCY",
            Self::StandardIa => "STANDARD_IA",
            Self::OnezoneIa => "ONEZONE_IA",
            Self::IntelligentTiering => "INTELLIGENT_TIERING",
            Self::Glacier => "GLACIER",
            Self::GlacierIr => "GLACIER_IR",
            Self::DeepArchive => "DEEP_ARCHIVE",
        }
    }

    pub fn parse(s: &str) -> Self {
        match s {
            "REDUCED_REDUNDANCY" => Self::ReducedRedundancy,
            "STANDARD_IA" => Self::StandardIa,
            "ONEZONE_IA" => Self::OnezoneIa,
            "INTELLIGENT_TIERING" => Self::IntelligentTiering,
            "GLACIER" => Self::Glacier,
            "GLACIER_IR" => Self::GlacierIr,
            "DEEP_ARCHIVE" => Self::DeepArchive,
            _ => Self::Standard,
        }
    }
}

/// S3 server-side encryption options.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum S3Encryption {
    None,
    SseS3,
    SseKms { key_id: Option<String> },
}

/// Well-known S3-compatible endpoint presets.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S3EndpointPreset {
    pub id: String,
    pub name: String,
    pub endpoint_url: String,
    pub region: String,
    pub path_style: bool,
    pub notes: String,
}

/// Properties of an S3 object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S3ObjectProperties {
    pub key: String,
    pub acl: String,
    pub storage_class: String,
    pub encryption: String,
    pub encryption_key_id: Option<String>,
    pub content_type: Option<String>,
    pub content_length: u64,
    pub last_modified: Option<String>,
    pub etag: Option<String>,
    pub version_id: Option<String>,
}

/// S3 object tag (key-value pair). S3 supports up to 10 tags per object.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S3ObjectTag {
    pub key: String,
    pub value: String,
}

/// Bucket-level settings.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S3BucketSettings {
    pub default_encryption: String,
    pub default_encryption_key_id: Option<String>,
    pub versioning_enabled: bool,
    pub versioning_status: String,
    pub acceleration_enabled: bool,
}

// ── CloudFront CDN Types ──

/// A CloudFront distribution associated with S3 origins.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CloudFrontDistribution {
    pub id: String,
    pub domain_name: String,
    pub status: String,
    pub enabled: bool,
    pub origins: Vec<String>,
    pub comment: Option<String>,
}

/// A CloudFront invalidation request/status.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CloudFrontInvalidation {
    pub id: String,
    pub status: String,
    pub paths: Vec<String>,
    pub created_at: Option<String>,
}

// ── MFA Delete Types ──

/// MFA delete status for an S3 bucket.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MfaDeleteStatus {
    /// Whether MFA delete is enabled on the bucket.
    pub mfa_delete_enabled: bool,
    /// Whether versioning is enabled on the bucket.
    pub versioning_enabled: bool,
}

/// S3 lifecycle rule for automated object transitions and expiration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S3LifecycleRule {
    pub id: String,
    pub prefix: String,
    pub enabled: bool,
    pub transitions: Vec<S3LifecycleTransition>,
    pub expiration_days: Option<u32>,
    pub abort_incomplete_multipart_days: Option<u32>,
}

/// A transition within a lifecycle rule (move to a different storage class after N days).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S3LifecycleTransition {
    pub days: u32,
    pub storage_class: String,
}

/// S3 object version from ListObjectVersions response.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S3ObjectVersion {
    pub version_id: String,
    pub key: String,
    pub last_modified: Option<String>,
    pub size: Option<u64>,
    pub is_latest: bool,
    pub is_delete_marker: bool,
}

/// `host[:port]` of a URL (the port only when it is not the scheme default),
/// i.e. exactly what an HTTP client sends in the `Host` header.
fn url_authority(u: &url::Url) -> Option<String> {
    let host = u.host_str()?;
    Some(match u.port() {
        Some(p) => format!("{host}:{p}"),
        None => host.to_string(),
    })
}

/// S3 authentication credentials.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct S3Credentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub region: String,
    /// Custom endpoint for S3-compatible services (MinIO, Wasabi, B2).
    pub endpoint: Option<String>,
    /// Whether to use path-style addressing (required for some S3-compatible services).
    pub path_style: bool,
    /// Optional session token for temporary credentials / IAM role.
    pub session_token: Option<String>,
}

/// S3 multipart upload state for tracking in-progress uploads.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MultipartUploadState {
    pub upload_id: String,
    pub bucket: String,
    pub key: String,
    pub parts: Vec<CompletedPart>,
    pub part_size: u64,
    pub total_size: u64,
    pub next_part_number: u32,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CompletedPart {
    pub part_number: u32,
    pub etag: String,
    pub size: u64,
}

/// Internal connection state.
struct S3State {
    credentials: S3Credentials,
    http: reqwest::Client,
    /// Changes on every connect, so a role refresher of an earlier
    /// connection stops instead of overwriting the new credentials.
    epoch: u64,
}

static S3_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// Refresh assumed-role credentials this long before they expire.
const ROLE_REFRESH_EARLY_SECS: i64 = 300;
/// Never retry faster than this (tests: immediately).
const ROLE_MIN_WAIT_SECS: i64 = if cfg!(test) { 0 } else { 30 };

/// Amazon S3 connector implementing the Connector trait.
pub struct S3Connector {
    state: Arc<RwLock<Option<S3State>>>,
}

impl S3Connector {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(None)),
        }
    }

    /// Connect using S3 credentials directly (not via ConnectionProfile).
    pub async fn connect_with_credentials(&self, creds: S3Credentials) -> Result<(), AppError> {
        let http = crate::net::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| AppError::Connection {
                message: format!("Failed to create HTTP client: {e}"),
                advice: "Check your system's TLS configuration.".to_string(),
            })?;

        // Validate credentials by listing buckets
        let state = S3State {
            credentials: creds,
            http,
            epoch: S3_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
        };

        let mut lock = self.state.write().await;
        *lock = Some(state);
        Ok(())
    }

    /// Connect with a role assumed from `base` and keep it fresh: a
    /// background task re-assumes the role shortly before the temporary
    /// credentials expire, until the connection is replaced or closed.
    pub async fn connect_with_role(
        &self,
        base: super::aws_credentials::BaseKeys,
        role: crate::core::types::AssumeRoleConfig,
        mut creds: S3Credentials,
        sts_endpoint: Option<String>,
    ) -> Result<(), AppError> {
        let temp = super::aws_credentials::assume_role(&base, &role, &creds.region, sts_endpoint.as_deref()).await?;
        let mut expires = expiry_of(&temp);
        creds.access_key_id = temp.access_key_id;
        creds.secret_access_key = temp.secret_access_key;
        creds.session_token = Some(temp.session_token);
        let region = creds.region.clone();
        self.connect_with_credentials(creds).await?;
        let epoch = self.state.read().await.as_ref().map(|s| s.epoch).unwrap_or(0);
        let state = self.state.clone();
        tokio::spawn(async move {
            loop {
                let wait = (expires - chrono::Utc::now()).num_seconds() - ROLE_REFRESH_EARLY_SECS;
                tokio::time::sleep(std::time::Duration::from_secs(wait.max(ROLE_MIN_WAIT_SECS) as u64)).await;
                if state.read().await.as_ref().map(|s| s.epoch) != Some(epoch) {
                    return; // reconnected or disconnected
                }
                match super::aws_credentials::assume_role(&base, &role, &region, sts_endpoint.as_deref()).await {
                    Ok(t) => {
                        expires = expiry_of(&t);
                        let mut st = state.write().await;
                        match st.as_mut() {
                            Some(s) if s.epoch == epoch => {
                                s.credentials.access_key_id = t.access_key_id;
                                s.credentials.secret_access_key = t.secret_access_key;
                                s.credentials.session_token = Some(t.session_token);
                                tracing::debug!("Refreshed assumed-role S3 credentials");
                            }
                            _ => return,
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Assumed-role refresh failed, retrying: {e}");
                        expires = chrono::Utc::now() + chrono::Duration::seconds(ROLE_REFRESH_EARLY_SECS + 60);
                    }
                }
            }
        });
        Ok(())
    }

    /// List all S3 buckets (appears as top-level folders).
    pub async fn list_buckets(&self) -> Result<Vec<FileEntry>, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let endpoint = self.effective_endpoint(&state.credentials);
        let url = format!("{endpoint}/");

        let headers = self.sign_request(
            &state.credentials,
            "GET",
            "/",
            "",
            &[],
            &now,
        )?;

        let mut req = state.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 list buckets failed: {e}"),
            advice: "Check your credentials and network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(s3_status_error(format!("S3 list buckets failed (HTTP {status}): {body}"), status.as_u16(), &body, "Check your access key and permissions."));
        }

        let body = resp.text().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read S3 response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        // Parse XML response for bucket names
        let entries = parse_list_buckets_xml(&body);
        Ok(entries)
    }

    /// List objects in a bucket with optional prefix (virtual folder browsing).
    ///
    /// Follows `NextContinuationToken` until the listing is complete: a single
    /// ListObjectsV2 call returns at most 1,000 keys, and stopping there made
    /// large folders look silently truncated.
    pub async fn list_objects(
        &self,
        bucket: &str,
        prefix: &str,
        delimiter: Option<&str>,
    ) -> Result<Vec<FileEntry>, AppError> {
        let mut entries = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let page = self.list_objects_page(bucket, prefix, delimiter, token.as_deref()).await?;
            entries.extend(page.entries);
            match page.next_token {
                Some(next) if Some(&next) != token.as_ref() => token = Some(next),
                Some(_) => {
                    return Err(AppError::Connection {
                        message: "S3 returned the same continuation token twice".to_string(),
                        advice: "The server's listing is inconsistent; try again.".to_string(),
                    })
                }
                None => break,
            }
        }
        Ok(entries)
    }

    /// One ListObjectsV2 page.
    async fn list_objects_page(
        &self,
        bucket: &str,
        prefix: &str,
        delimiter: Option<&str>,
        continuation_token: Option<&str>,
    ) -> Result<ListPage, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, path) = self.bucket_url(&state.credentials, bucket);

        let mut query_params = vec![
            ("list-type", "2".to_string()),
            ("max-keys", "1000".to_string()),
        ];
        if !prefix.is_empty() {
            query_params.push(("prefix", prefix.to_string()));
        }
        if let Some(delim) = delimiter {
            query_params.push(("delimiter", delim.to_string()));
        }
        if let Some(t) = continuation_token {
            query_params.push(("continuation-token", t.to_string()));
        }
        let query_string = canonical_query(&query_params);

        let url = format!("{endpoint}{path}?{query_string}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/")
        } else {
            "/".to_string()
        };

        let headers = self.sign_request(
            &state.credentials,
            "GET",
            &canonical_path,
            &query_string,
            &[],
            &now,
        )?;

        let mut req = state.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        // For virtual-hosted style, add Host header with bucket
        if !state.credentials.path_style {
            let host = self.bucket_host(&state.credentials, bucket);
            req = req.header("Host", &host);
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 list objects failed: {e}"),
            advice: "Check your credentials and network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(s3_status_error(format!("S3 list objects failed (HTTP {status}): {body}"), status.as_u16(), &body, "Check your bucket name and permissions."));
        }

        let body = resp.text().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read S3 response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        let truncated = extract_xml_value(&body, "IsTruncated").is_some_and(|v| v.trim() == "true");
        let next_token = if truncated {
            Some(extract_xml_value(&body, "NextContinuationToken").ok_or_else(|| AppError::Connection {
                message: "S3 listing is truncated but has no continuation token".to_string(),
                advice: "The server may not support ListObjectsV2 pagination.".to_string(),
            })?)
        } else {
            None
        };
        Ok(ListPage { entries: parse_list_objects_xml(&body, prefix), next_token })
    }

    /// Upload a file to S3. Uses multipart for files > 5MB.
    pub async fn upload_file(
        &self,
        bucket: &str,
        key: &str,
        data: Vec<u8>,
    ) -> Result<String, AppError> {
        if data.len() as u64 > MULTIPART_THRESHOLD {
            self.multipart_upload(bucket, key, &data).await
        } else {
            self.put_object(bucket, key, &data).await
        }
    }

    /// Simple PUT upload for small files.
    async fn put_object(&self, bucket: &str, key: &str, data: &[u8]) -> Result<String, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        // Calculate content hash
        let content_hash = hex::encode(Sha256::digest(data));

        let headers = self.sign_request_with_payload(
            &state.credentials,
            "PUT",
            &canonical_path,
            "",
            &[("content-length", &data.len().to_string())],
            &content_hash,
            &now,
        )?;

        let mut req = state.http.put(&url).body(data.to_vec());
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Transfer {
            message: format!("S3 upload failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("S3 upload failed (HTTP {status}): {body}"),
                advice: "Check your permissions and bucket policy.".to_string(),
            });
        }

        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        tracing::info!("S3 upload complete: s3://{bucket}/{key} (ETag: {etag})");
        Ok(etag)
    }

    /// Multipart upload for large files (> 5MB).
    async fn multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        data: &[u8],
    ) -> Result<String, AppError> {
        let total_size = data.len() as u64;
        let part_size = calculate_part_size(total_size);

        // Step 1: Initiate multipart upload
        let upload_id = self.initiate_multipart_upload(bucket, key).await?;
        tracing::info!("Initiated multipart upload: {upload_id} for s3://{bucket}/{key}");

        // Step 2: Upload parts
        let mut parts = Vec::new();
        let mut offset = 0u64;
        let mut part_number = 1u32;

        while offset < total_size {
            let end = std::cmp::min(offset + part_size, total_size);
            let part_data = &data[offset as usize..end as usize];

            match self.upload_part(bucket, key, &upload_id, part_number, part_data).await {
                Ok(etag) => {
                    parts.push(CompletedPart {
                        part_number,
                        etag,
                        size: (end - offset),
                    });
                    tracing::debug!(
                        "Uploaded part {part_number}/{} ({} bytes)",
                        total_size.div_ceil(part_size),
                        end - offset
                    );
                }
                Err(e) => {
                    // Abort the multipart upload on failure
                    let _ = self.abort_multipart_upload(bucket, key, &upload_id).await;
                    return Err(e);
                }
            }

            offset = end;
            part_number += 1;
        }

        // Step 3: Complete multipart upload
        let etag = self.complete_multipart_upload(bucket, key, &upload_id, &parts).await?;
        tracing::info!("Completed multipart upload: s3://{bucket}/{key} (ETag: {etag})");
        Ok(etag)
    }

    /// Initiate a multipart upload, returning the upload ID.
    async fn initiate_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<String, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}?uploads");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let headers = self.sign_request(
            &state.credentials,
            "POST",
            &canonical_path,
            "uploads",
            &[],
            &now,
        )?;

        let mut req = state.http.post(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Transfer {
            message: format!("S3 initiate multipart failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("S3 initiate multipart failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        let body = resp.text().await.unwrap_or_default();
        parse_upload_id_xml(&body)
    }

    /// Upload a single part of a multipart upload.
    async fn upload_part(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        part_number: u32,
        data: &[u8],
    ) -> Result<String, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let query = format!("partNumber={part_number}&uploadId={upload_id}");
        let url = format!("{endpoint}{base_path}{encoded_key}?{query}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let content_hash = hex::encode(Sha256::digest(data));

        let headers = self.sign_request_with_payload(
            &state.credentials,
            "PUT",
            &canonical_path,
            &query,
            &[("content-length", &data.len().to_string())],
            &content_hash,
            &now,
        )?;

        let mut req = state.http.put(&url).body(data.to_vec());
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Transfer {
            message: format!("S3 upload part {part_number} failed: {e}"),
            advice: "The upload will be retried.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("S3 upload part {part_number} failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        let etag = resp
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .trim_matches('"')
            .to_string();

        Ok(etag)
    }

    /// Complete a multipart upload by combining all parts.
    async fn complete_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
        parts: &[CompletedPart],
    ) -> Result<String, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let query = format!("uploadId={upload_id}");
        let url = format!("{endpoint}{base_path}{encoded_key}?{query}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        // Build CompleteMultipartUpload XML
        let mut xml = String::from("<CompleteMultipartUpload>");
        for part in parts {
            xml.push_str(&format!(
                "<Part><PartNumber>{}</PartNumber><ETag>\"{}\"</ETag></Part>",
                part.part_number, part.etag
            ));
        }
        xml.push_str("</CompleteMultipartUpload>");
        let xml_bytes = xml.as_bytes();

        let content_hash = hex::encode(Sha256::digest(xml_bytes));

        let headers = self.sign_request_with_payload(
            &state.credentials,
            "POST",
            &canonical_path,
            &query,
            &[
                ("content-type", "application/xml"),
                ("content-length", &xml_bytes.len().to_string()),
            ],
            &content_hash,
            &now,
        )?;

        let mut req = state.http.post(&url).body(xml_bytes.to_vec());
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("Content-Type", "application/xml");

        let resp = req.send().await.map_err(|e| AppError::Transfer {
            message: format!("S3 complete multipart failed: {e}"),
            advice: "The upload may need to be retried.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("S3 complete multipart failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        let body = resp.text().await.unwrap_or_default();
        let etag = extract_xml_value(&body, "ETag").unwrap_or_default();
        Ok(etag)
    }

    /// Abort an in-progress multipart upload.
    async fn abort_multipart_upload(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let query = format!("uploadId={upload_id}");
        let url = format!("{endpoint}{base_path}{encoded_key}?{query}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let headers = self.sign_request(
            &state.credentials,
            "DELETE",
            &canonical_path,
            &query,
            &[],
            &now,
        )?;

        let mut req = state.http.delete(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let _ = req.send().await;
        tracing::info!("Aborted multipart upload: {upload_id}");
        Ok(())
    }

    /// List parts of an in-progress multipart upload (for resume).
    pub async fn list_parts(
        &self,
        bucket: &str,
        key: &str,
        upload_id: &str,
    ) -> Result<Vec<CompletedPart>, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let query = format!("uploadId={upload_id}");
        let url = format!("{endpoint}{base_path}{encoded_key}?{query}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let headers = self.sign_request(
            &state.credentials,
            "GET",
            &canonical_path,
            &query,
            &[],
            &now,
        )?;

        let mut req = state.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 list parts failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("S3 list parts failed (HTTP {status}): {body}"),
                advice: "The upload may have expired. Start a new upload.".to_string(),
            });
        }

        let body = resp.text().await.unwrap_or_default();
        Ok(parse_list_parts_xml(&body))
    }

    /// Download an object from S3.
    pub async fn download_object(
        &self,
        bucket: &str,
        key: &str,
        range: Option<(u64, u64)>,
    ) -> Result<Vec<u8>, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let mut extra_headers = Vec::new();
        let range_header;
        if let Some((start, end)) = range {
            range_header = format!("bytes={start}-{end}");
            extra_headers.push(("range", range_header.as_str()));
        }

        let headers = self.sign_request(
            &state.credentials,
            "GET",
            &canonical_path,
            "",
            &extra_headers,
            &now,
        )?;

        let mut req = state.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if let Some((start, end)) = range {
            req = req.header("Range", format!("bytes={start}-{end}"));
        }

        let resp = req.send().await.map_err(|e| AppError::Transfer {
            message: format!("S3 download failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("S3 download failed (HTTP {status}): {body}"),
                advice: "Check your permissions and object key.".to_string(),
            });
        }

        resp.bytes().await.map(|b| b.to_vec()).map_err(|e| AppError::Transfer {
            message: format!("Failed to read S3 response body: {e}"),
            advice: "Try again.".to_string(),
        })
    }

    /// Delete an object from S3.
    pub async fn delete_object(&self, bucket: &str, key: &str) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let headers = self.sign_request(
            &state.credentials,
            "DELETE",
            &canonical_path,
            "",
            &[],
            &now,
        )?;

        let mut req = state.http.delete(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Transfer {
            message: format!("S3 delete failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() && resp.status().as_u16() != 204 {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("S3 delete failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        Ok(())
    }

    /// Generate a presigned URL for temporary access to an object.
    pub fn generate_presigned_url(
        &self,
        creds: &S3Credentials,
        bucket: &str,
        key: &str,
        expires_in_seconds: u64,
    ) -> Result<String, AppError> {
        let now = Utc::now();
        let date_stamp = now.format("%Y%m%d").to_string();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let credential_scope = format!("{}/{}/s3/aws4_request", date_stamp, creds.region);

        let encoded_key = percent_encode_path(key);
        let host = self.bucket_host(creds, bucket);

        let canonical_path = if creds.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let credential = format!(
            "{}/{}",
            creds.access_key_id, credential_scope
        );

        // Build query string for presigned request
        let mut query_params = vec![
            ("X-Amz-Algorithm", "AWS4-HMAC-SHA256".to_string()),
            ("X-Amz-Credential", credential),
            ("X-Amz-Date", amz_date.clone()),
            ("X-Amz-Expires", expires_in_seconds.to_string()),
            ("X-Amz-SignedHeaders", "host".to_string()),
        ];

        if let Some(ref token) = creds.session_token {
            query_params.push(("X-Amz-Security-Token", token.clone()));
        }

        query_params.sort_by(|a, b| a.0.cmp(b.0));

        let canonical_query = query_params
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}={}",
                    percent_encoding::percent_encode(k.as_bytes(), S3_ENCODE_SET),
                    percent_encoding::percent_encode(v.as_bytes(), S3_ENCODE_SET)
                )
            })
            .collect::<Vec<_>>()
            .join("&");

        // Canonical request
        let canonical_request = format!(
            "GET\n{}\n{}\nhost:{}\n\nhost\nUNSIGNED-PAYLOAD",
            canonical_path, canonical_query, host
        );

        // String to sign
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date,
            credential_scope,
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );

        // Calculate signature
        let signing_key = derive_signing_key(
            &creds.secret_access_key,
            &date_stamp,
            &creds.region,
            "s3",
        )?;
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);

        // Build final URL
        let endpoint = self.effective_endpoint(creds);
        let base = if creds.path_style {
            format!("{endpoint}/{bucket}/{encoded_key}")
        } else {
            format!("https://{host}/{encoded_key}")
        };

        Ok(format!("{base}?{canonical_query}&X-Amz-Signature={signature}"))
    }

    // ── S3 Advanced Object Controls ──

    /// Get object properties via HEAD request.
    pub async fn get_object_properties(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<S3ObjectProperties, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let headers = self.sign_request(
            &state.credentials,
            "HEAD",
            &canonical_path,
            "",
            &[],
            &now,
        )?;

        let mut req = state.http.head(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 HEAD object failed: {e}"),
            advice: "Check your credentials and network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            return Err(AppError::Connection {
                message: format!("S3 HEAD object failed (HTTP {status})"),
                advice: "Check your permissions and object key.".to_string(),
            });
        }

        let hdrs = resp.headers();

        let storage_class = hdrs
            .get("x-amz-storage-class")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("STANDARD")
            .to_string();

        let encryption = hdrs
            .get("x-amz-server-side-encryption")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("None")
            .to_string();

        let encryption_key_id = hdrs
            .get("x-amz-server-side-encryption-aws-kms-key-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let content_type = hdrs
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let content_length = hdrs
            .get("content-length")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(0);

        let last_modified = hdrs
            .get("last-modified")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        let etag = hdrs
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim_matches('"').to_string());

        let version_id = hdrs
            .get("x-amz-version-id")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());

        Ok(S3ObjectProperties {
            key: key.to_string(),
            acl: "private".to_string(), // ACL cannot be read from HEAD; default
            storage_class,
            encryption,
            encryption_key_id,
            content_type,
            content_length,
            last_modified,
            etag,
            version_id,
        })
    }

    /// Set ACL on an object via PUT ?acl.
    pub async fn set_object_acl(
        &self,
        bucket: &str,
        key: &str,
        acl: &str,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}?acl");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let headers = self.sign_request(
            &state.credentials,
            "PUT",
            &canonical_path,
            "acl",
            &[("x-amz-acl", acl)],
            &now,
        )?;

        let mut req = state.http.put(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("x-amz-acl", acl);

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 set ACL failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("S3 set ACL failed (HTTP {status}): {body}"),
                advice: "Check your permissions. Bucket may have ACLs disabled.".to_string(),
            });
        }

        tracing::info!("Set ACL on s3://{bucket}/{key} to {acl}");
        Ok(())
    }

    /// Change storage class by copy-in-place (COPY object to itself with new storage class).
    pub async fn set_storage_class(
        &self,
        bucket: &str,
        key: &str,
        storage_class: &str,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let copy_source = format!("/{bucket}/{encoded_key}");

        let headers = self.sign_request(
            &state.credentials,
            "PUT",
            &canonical_path,
            "",
            &[
                ("x-amz-copy-source", &copy_source),
                ("x-amz-metadata-directive", "COPY"),
                ("x-amz-storage-class", storage_class),
            ],
            &now,
        )?;

        let mut req = state.http.put(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("x-amz-copy-source", &copy_source);
        req = req.header("x-amz-metadata-directive", "COPY");
        req = req.header("x-amz-storage-class", storage_class);

        let resp = req.send().await.map_err(|e| AppError::Transfer {
            message: format!("S3 set storage class failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("S3 set storage class failed (HTTP {status}): {body}"),
                advice: "Check your permissions and object key.".to_string(),
            });
        }

        tracing::info!("Changed storage class of s3://{bucket}/{key} to {storage_class}");
        Ok(())
    }

    /// Get bucket settings: default encryption, versioning, transfer acceleration.
    pub async fn get_bucket_settings(
        &self,
        bucket: &str,
    ) -> Result<S3BucketSettings, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();

        // Get encryption configuration
        let encryption = self.get_bucket_xml_config(state, bucket, "encryption", &now).await;
        let (default_encryption, default_encryption_key_id) = match encryption {
            Ok(body) => {
                let algo = extract_xml_value(&body, "SSEAlgorithm").unwrap_or_else(|| "None".to_string());
                let key_id = extract_xml_value(&body, "KMSMasterKeyID");
                (algo, key_id)
            }
            Err(_) => ("None".to_string(), None),
        };

        // Get versioning configuration
        let versioning = self.get_bucket_xml_config(state, bucket, "versioning", &now).await;
        let (versioning_enabled, versioning_status) = match versioning {
            Ok(body) => {
                let status = extract_xml_value(&body, "Status").unwrap_or_else(|| "Disabled".to_string());
                (status == "Enabled", status)
            }
            Err(_) => (false, "Disabled".to_string()),
        };

        // Get transfer acceleration configuration
        let acceleration = self.get_bucket_xml_config(state, bucket, "accelerate", &now).await;
        let acceleration_enabled = match acceleration {
            Ok(body) => {
                let status = extract_xml_value(&body, "Status").unwrap_or_default();
                status == "Enabled"
            }
            Err(_) => false,
        };

        Ok(S3BucketSettings {
            default_encryption,
            default_encryption_key_id,
            versioning_enabled,
            versioning_status,
            acceleration_enabled,
        })
    }

    /// Toggle transfer acceleration on a bucket.
    pub async fn set_transfer_acceleration(
        &self,
        bucket: &str,
        enabled: bool,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let url = format!("{endpoint}{base_path}?accelerate");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/")
        } else {
            "/".to_string()
        };

        let status = if enabled { "Enabled" } else { "Suspended" };
        let xml_body = format!(
            "<AccelerateConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\"><Status>{status}</Status></AccelerateConfiguration>"
        );
        let xml_bytes = xml_body.as_bytes();
        let content_hash = hex::encode(Sha256::digest(xml_bytes));

        let headers = self.sign_request_with_payload(
            &state.credentials,
            "PUT",
            &canonical_path,
            "accelerate",
            &[
                ("content-type", "application/xml"),
                ("content-length", &xml_bytes.len().to_string()),
            ],
            &content_hash,
            &now,
        )?;

        let mut req = state.http.put(&url).body(xml_bytes.to_vec());
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("Content-Type", "application/xml");

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 set transfer acceleration failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let resp_status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("S3 set transfer acceleration failed (HTTP {resp_status}): {body}"),
                advice: "Check your permissions. Transfer acceleration may not be supported for this bucket.".to_string(),
            });
        }

        tracing::info!("Set transfer acceleration on bucket {bucket} to {status}");
        Ok(())
    }

    /// Helper to GET a bucket sub-resource XML config (e.g., ?encryption, ?versioning, ?accelerate).
    async fn get_bucket_xml_config(
        &self,
        state: &S3State,
        bucket: &str,
        sub_resource: &str,
        now: &DateTime<Utc>,
    ) -> Result<String, AppError> {
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let url = format!("{endpoint}{base_path}?{sub_resource}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/")
        } else {
            "/".to_string()
        };

        let headers = self.sign_request(
            &state.credentials,
            "GET",
            &canonical_path,
            sub_resource,
            &[],
            now,
        )?;

        let mut req = state.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 get bucket {sub_resource} failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("S3 get bucket {sub_resource} failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        resp.text().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read S3 response: {e}"),
            advice: "Try again.".to_string(),
        })
    }

    // ── CloudFront CDN Management ──

    /// Sign a request for the CloudFront API (uses the `cloudfront` service, global endpoint).
    #[allow(clippy::too_many_arguments)]
    fn sign_cloudfront_request(
        &self,
        creds: &S3Credentials,
        method: &str,
        canonical_path: &str,
        query_string: &str,
        extra_headers: &[(&str, &str)],
        payload_hash: &str,
        now: &DateTime<Utc>,
    ) -> Result<Vec<(String, String)>, AppError> {
        let date_stamp = now.format("%Y%m%d").to_string();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let credential_scope = format!("{}/us-east-1/cloudfront/aws4_request", date_stamp);

        let host = "cloudfront.amazonaws.com".to_string();

        let mut headers = vec![
            ("host".to_string(), host.clone()),
            ("x-amz-content-sha256".to_string(), payload_hash.to_string()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];

        if let Some(ref token) = creds.session_token {
            headers.push(("x-amz-security-token".to_string(), token.clone()));
        }

        for (k, v) in extra_headers {
            headers.push((k.to_lowercase(), v.to_string()));
        }

        headers.sort_by(|a, b| a.0.cmp(&b.0));

        let signed_headers: String = headers
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(";");

        let canonical_headers: String = headers
            .iter()
            .map(|(k, v)| format!("{k}:{v}\n"))
            .collect();

        let canonical_request = format!(
            "{method}\n{canonical_path}\n{query_string}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );

        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date,
            credential_scope,
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );

        // CloudFront always uses us-east-1 for signing
        let signing_key = derive_signing_key(
            &creds.secret_access_key,
            &date_stamp,
            "us-east-1",
            "cloudfront",
        )?;
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            creds.access_key_id, credential_scope, signed_headers, signature
        );

        let mut result = vec![
            ("Authorization".to_string(), authorization),
            ("x-amz-date".to_string(), amz_date),
            ("x-amz-content-sha256".to_string(), payload_hash.to_string()),
        ];

        if let Some(ref token) = creds.session_token {
            result.push(("x-amz-security-token".to_string(), token.clone()));
        }

        Ok(result)
    }

    /// List CloudFront distributions associated with the current AWS account.
    ///
    /// Uses the CloudFront API (cloudfront.amazonaws.com) which is separate from S3.
    /// Returns all distributions, including those with non-S3 origins.
    pub async fn list_distributions(&self) -> Result<Vec<CloudFrontDistribution>, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first with valid AWS credentials.".to_string(),
        })?;

        let now = Utc::now();
        let url = "https://cloudfront.amazonaws.com/2020-05-31/distribution";

        let headers = self.sign_cloudfront_request(
            &state.credentials,
            "GET",
            "/2020-05-31/distribution",
            "",
            &[],
            "UNSIGNED-PAYLOAD",
            &now,
        )?;

        let mut req = state.http.get(url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("CloudFront list distributions failed: {e}"),
            advice: "Check your credentials and network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("CloudFront list distributions failed (HTTP {status}): {body}"),
                advice: "Check your IAM permissions for CloudFront.".to_string(),
            });
        }

        let body = resp.text().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read CloudFront response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(parse_cloudfront_distributions_xml(&body))
    }

    /// Create a CloudFront invalidation for the given distribution.
    ///
    /// Invalidates cached objects at the specified paths (e.g., `["/*"]` to invalidate everything,
    /// or `["/images/photo.jpg", "/css/*"]` for specific paths).
    pub async fn create_invalidation(
        &self,
        distribution_id: &str,
        paths: &[String],
    ) -> Result<CloudFrontInvalidation, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first with valid AWS credentials.".to_string(),
        })?;

        let now = Utc::now();
        let api_path = format!("/2020-05-31/distribution/{distribution_id}/invalidation");
        let url = format!("https://cloudfront.amazonaws.com{api_path}");

        // Build InvalidationBatch XML
        let caller_ref = format!("ufop-{}", now.timestamp_millis());
        let paths_xml: String = paths
            .iter()
            .map(|p| format!("<Path>{p}</Path>"))
            .collect();
        let xml_body = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><InvalidationBatch xmlns="http://cloudfront.amazonaws.com/doc/2020-05-31/"><Paths><Quantity>{}</Quantity><Items>{}</Items></Paths><CallerReference>{}</CallerReference></InvalidationBatch>"#,
            paths.len(),
            paths_xml,
            caller_ref,
        );
        let xml_bytes = xml_body.as_bytes();
        let content_hash = hex::encode(Sha256::digest(xml_bytes));

        let headers = self.sign_cloudfront_request(
            &state.credentials,
            "POST",
            &api_path,
            "",
            &[
                ("content-type", "application/xml"),
                ("content-length", &xml_bytes.len().to_string()),
            ],
            &content_hash,
            &now,
        )?;

        let mut req = state.http.post(&url).body(xml_bytes.to_vec());
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("Content-Type", "application/xml");

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("CloudFront create invalidation failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("CloudFront create invalidation failed (HTTP {status}): {body}"),
                advice: "Check your IAM permissions and distribution ID.".to_string(),
            });
        }

        let body = resp.text().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read CloudFront response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        // Parse the invalidation response
        let inv_id = extract_xml_value(&body, "Id").unwrap_or_default();
        let inv_status = extract_xml_value(&body, "Status").unwrap_or_else(|| "InProgress".to_string());
        let created = extract_xml_value(&body, "CreateTime");

        tracing::info!(
            "Created CloudFront invalidation {inv_id} for distribution {distribution_id} ({} paths)",
            paths.len()
        );

        Ok(CloudFrontInvalidation {
            id: inv_id,
            status: inv_status,
            paths: paths.to_vec(),
            created_at: created,
        })
    }

    /// List recent invalidations for a CloudFront distribution.
    pub async fn list_invalidations(
        &self,
        distribution_id: &str,
    ) -> Result<Vec<CloudFrontInvalidation>, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first with valid AWS credentials.".to_string(),
        })?;

        let now = Utc::now();
        let api_path = format!("/2020-05-31/distribution/{distribution_id}/invalidation");
        let url = format!("https://cloudfront.amazonaws.com{api_path}");

        let headers = self.sign_cloudfront_request(
            &state.credentials,
            "GET",
            &api_path,
            "",
            &[],
            "UNSIGNED-PAYLOAD",
            &now,
        )?;

        let mut req = state.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("CloudFront list invalidations failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("CloudFront list invalidations failed (HTTP {status}): {body}"),
                advice: "Check your IAM permissions and distribution ID.".to_string(),
            });
        }

        let body = resp.text().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read CloudFront response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(parse_cloudfront_invalidations_xml(&body))
    }

    // ── MFA Delete Operations ──

    /// Check if MFA delete is enabled on a bucket by inspecting the versioning configuration.
    pub async fn get_mfa_delete_status(
        &self,
        bucket: &str,
    ) -> Result<MfaDeleteStatus, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let body = self.get_bucket_xml_config(state, bucket, "versioning", &now).await?;

        let versioning_status = extract_xml_value(&body, "Status")
            .unwrap_or_else(|| "Disabled".to_string());
        let mfa_delete_status = extract_xml_value(&body, "MfaDelete")
            .or_else(|| extract_xml_value(&body, "MFADelete"))
            .unwrap_or_else(|| "Disabled".to_string());

        Ok(MfaDeleteStatus {
            mfa_delete_enabled: mfa_delete_status == "Enabled",
            versioning_enabled: versioning_status == "Enabled",
        })
    }

    /// Delete an object version with MFA authentication.
    ///
    /// When MFA delete is enabled on a bucket, permanently deleting object versions
    /// requires providing an MFA device serial number and a current token code.
    /// The `x-amz-mfa` header format is: `<serial-number> <token-code>` (space-separated).
    pub async fn delete_with_mfa(
        &self,
        bucket: &str,
        key: &str,
        version_id: &str,
        mfa_serial: &str,
        mfa_token: &str,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let query = format!("versionId={version_id}");
        let url = format!("{endpoint}{base_path}{encoded_key}?{query}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let mfa_value = format!("{mfa_serial} {mfa_token}");

        let headers = self.sign_request(
            &state.credentials,
            "DELETE",
            &canonical_path,
            &query,
            &[("x-amz-mfa", &mfa_value)],
            &now,
        )?;

        let mut req = state.http.delete(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("x-amz-mfa", &mfa_value);

        let resp = req.send().await.map_err(|e| AppError::Transfer {
            message: format!("S3 MFA delete failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() && resp.status().as_u16() != 204 {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Transfer {
                message: format!("S3 MFA delete failed (HTTP {status}): {body}"),
                advice: "Check your MFA serial and token code. Ensure the token has not expired.".to_string(),
            });
        }

        tracing::info!(
            "MFA-authenticated delete of s3://{bucket}/{key}?versionId={version_id}"
        );
        Ok(())
    }

    // ── S3 Object Tags ──

    /// Get tags on an S3 object.
    pub async fn get_object_tags(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Vec<S3ObjectTag>, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}?tagging");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let headers = self.sign_request(
            &state.credentials,
            "GET",
            &canonical_path,
            "tagging",
            &[],
            &now,
        )?;

        let mut req = state.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("Failed to get object tags: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if resp.status().as_u16() == 404 {
            // No tagging configuration = empty tags
            return Ok(vec![]);
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("Failed to get object tags (HTTP {status}): {body}"),
                advice: "Check permissions (s3:GetObjectTagging).".to_string(),
            });
        }

        let body = resp.text().await.unwrap_or_default();
        // Parse XML: <Tagging><TagSet><Tag><Key>k</Key><Value>v</Value></Tag>...</TagSet></Tagging>
        let mut tags = Vec::new();
        for tag_block in body.split("<Tag>").skip(1) {
            let key_val = extract_xml_value(tag_block, "Key").unwrap_or_default();
            let value_val = extract_xml_value(tag_block, "Value").unwrap_or_default();
            if !key_val.is_empty() {
                tags.push(S3ObjectTag {
                    key: key_val,
                    value: value_val,
                });
            }
        }

        Ok(tags)
    }

    /// Set tags on an S3 object (replaces all existing tags).
    pub async fn put_object_tags(
        &self,
        bucket: &str,
        key: &str,
        tags: &[S3ObjectTag],
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        if tags.len() > 10 {
            return Err(AppError::Connection {
                message: "S3 objects support a maximum of 10 tags.".to_string(),
                advice: "Remove some tags before adding new ones.".to_string(),
            });
        }

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}?tagging");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        // Build XML body
        let tag_xml: String = tags
            .iter()
            .map(|t| {
                format!(
                    "<Tag><Key>{}</Key><Value>{}</Value></Tag>",
                    xml_escape(&t.key),
                    xml_escape(&t.value)
                )
            })
            .collect();
        let body = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><Tagging><TagSet>{tag_xml}</TagSet></Tagging>"
        );

        let headers = self.sign_request(
            &state.credentials,
            "PUT",
            &canonical_path,
            "tagging",
            &[],
            &now,
        )?;

        let mut req = state.http.put(&url).body(body);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("Content-Type", "application/xml");

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("Failed to set object tags: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("Failed to set object tags (HTTP {status}): {body}"),
                advice: "Check permissions (s3:PutObjectTagging).".to_string(),
            });
        }

        Ok(())
    }

    /// Delete all tags from an S3 object.
    pub async fn delete_object_tags(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{base_path}{encoded_key}?tagging");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let headers = self.sign_request(
            &state.credentials,
            "DELETE",
            &canonical_path,
            "tagging",
            &[],
            &now,
        )?;

        let mut req = state.http.delete(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("Failed to delete object tags: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() && resp.status().as_u16() != 204 {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("Failed to delete object tags (HTTP {status}): {body}"),
                advice: "Check permissions (s3:DeleteObjectTagging).".to_string(),
            });
        }

        Ok(())
    }

    // ── Object Versioning ──

    /// List all versions of a specific S3 object.
    ///
    /// Requires versioning to be enabled on the bucket.
    /// Uses the `GET /?versions&prefix=key` S3 API.
    pub async fn list_object_versions(
        &self,
        bucket: &str,
        key: &str,
    ) -> Result<Vec<S3ObjectVersion>, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, path) = self.bucket_url(&state.credentials, bucket);

        let query_string = canonical_query(&[("prefix", key.to_string()), ("versions", String::new())]);

        let url = format!("{endpoint}{path}?{query_string}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/")
        } else {
            "/".to_string()
        };

        let headers = self.sign_request(
            &state.credentials,
            "GET",
            &canonical_path,
            &query_string,
            &[],
            &now,
        )?;

        let mut req = state.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        if !state.credentials.path_style {
            let host = self.bucket_host(&state.credentials, bucket);
            req = req.header("Host", &host);
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 list object versions failed: {e}"),
            advice: "Check your credentials and network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("S3 list object versions failed (HTTP {status}): {body}"),
                advice: "Ensure versioning is enabled on this bucket.".to_string(),
            });
        }

        let body = resp.text().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read S3 response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(parse_object_versions_xml(&body, key))
    }

    /// Copy a specific version of an object back as the current version.
    ///
    /// Uses CopyObject with `x-amz-copy-source` including the versionId.
    pub async fn restore_object_version(
        &self,
        bucket: &str,
        key: &str,
        version_id: &str,
    ) -> Result<(), AppError> {
        let copy_source = format!(
            "/{bucket}/{}?versionId={}",
            percent_encode_path(key),
            percent_encoding::percent_encode(version_id.as_bytes(), S3_ENCODE_SET)
        );
        self.put_copy(bucket, key, &copy_source, "S3 version restore", "Check bucket permissions and versioning config.").await?;
        tracing::info!("Restored s3://{bucket}/{key} to version {version_id}");
        Ok(())
    }

    /// Server-side copy of `src_bucket/src_key` to `dst_bucket/dst_key`
    /// (CopyObject; objects up to 5 GB).
    pub async fn copy_object(&self, src_bucket: &str, src_key: &str, dst_bucket: &str, dst_key: &str) -> Result<(), AppError> {
        let copy_source = format!("/{src_bucket}/{}", percent_encode_path(src_key));
        self.put_copy(dst_bucket, dst_key, &copy_source, "S3 copy", "Check you may read the source and write the destination.").await
    }

    /// PUT `bucket/key` with `x-amz-copy-source` (CopyObject).
    async fn put_copy(&self, bucket: &str, key: &str, copy_source: &str, what: &str, advice: &str) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, path) = self.bucket_url(&state.credentials, bucket);

        // Same encoding for the URL and the signed path (`/` kept), or
        // nested keys and keys with spaces fail SigV4.
        let encoded_key = percent_encode_path(key);
        let url = format!("{endpoint}{path}{encoded_key}");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/{encoded_key}")
        } else {
            format!("/{encoded_key}")
        };

        let headers = self.sign_request(
            &state.credentials,
            "PUT",
            &canonical_path,
            "",
            &[],
            &now,
        )?;

        let mut req = state.http.put(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("x-amz-copy-source", copy_source);
        if !state.credentials.path_style {
            let host = self.bucket_host(&state.credentials, bucket);
            req = req.header("Host", &host);
        }

        let resp = req.send().await.map_err(|e| AppError::Transfer {
            message: format!("{what} failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(AppError::Transfer {
                message: format!("{what} failed (HTTP {status}): {body}"),
                advice: advice.to_string(),
            });
        }
        copy_reply_ok(&body).map_err(|reason| AppError::Transfer { message: format!("{what} failed: {reason}"), advice: advice.to_string() })
    }

    // ── Internal helpers ──

    fn effective_endpoint(&self, creds: &S3Credentials) -> String {
        creds.endpoint.clone().unwrap_or_else(|| {
            format!("https://s3.{}.amazonaws.com", creds.region)
        })
    }

    fn bucket_host(&self, creds: &S3Credentials, bucket: &str) -> String {
        if creds.path_style {
            if let Some(ref ep) = creds.endpoint {
                url::Url::parse(ep)
                    .ok()
                    .and_then(|u| url_authority(&u))
                    .unwrap_or_else(|| format!("s3.{}.amazonaws.com", creds.region))
            } else {
                format!("s3.{}.amazonaws.com", creds.region)
            }
        } else if let Some(ref ep) = creds.endpoint {
            let host = url::Url::parse(ep)
                .ok()
                .and_then(|u| url_authority(&u))
                .unwrap_or_else(|| format!("s3.{}.amazonaws.com", creds.region));
            format!("{bucket}.{host}")
        } else {
            format!("{bucket}.s3.{}.amazonaws.com", creds.region)
        }
    }

    fn bucket_url(&self, creds: &S3Credentials, bucket: &str) -> (String, String) {
        if creds.path_style {
            (self.effective_endpoint(creds), format!("/{bucket}/"))
        } else {
            let host = self.bucket_host(creds, bucket);
            (format!("https://{host}"), "/".to_string())
        }
    }

    /// AWS Signature V4 signing for requests with UNSIGNED-PAYLOAD.
    fn sign_request(
        &self,
        creds: &S3Credentials,
        method: &str,
        canonical_path: &str,
        query_string: &str,
        extra_headers: &[(&str, &str)],
        now: &DateTime<Utc>,
    ) -> Result<Vec<(String, String)>, AppError> {
        self.sign_request_with_payload(
            creds,
            method,
            canonical_path,
            query_string,
            extra_headers,
            "UNSIGNED-PAYLOAD",
            now,
        )
    }

    /// AWS Signature V4 signing with explicit payload hash.
    #[allow(clippy::too_many_arguments)]
    fn sign_request_with_payload(
        &self,
        creds: &S3Credentials,
        method: &str,
        canonical_path: &str,
        query_string: &str,
        extra_headers: &[(&str, &str)],
        payload_hash: &str,
        now: &DateTime<Utc>,
    ) -> Result<Vec<(String, String)>, AppError> {
        let date_stamp = now.format("%Y%m%d").to_string();
        let amz_date = now.format("%Y%m%dT%H%M%SZ").to_string();
        let credential_scope = format!("{}/{}/s3/aws4_request", date_stamp, creds.region);

        // Path-style requests (and the bucket listing at "/") go to the endpoint
        // itself, so the signed Host must match it, port included.
        let host = if creds.endpoint.is_some() && (creds.path_style || canonical_path == "/") {
            let endpoint = self.effective_endpoint(creds);
            url::Url::parse(&endpoint)
                .ok()
                .and_then(|u| url_authority(&u))
                .unwrap_or_else(|| format!("s3.{}.amazonaws.com", creds.region))
        } else if canonical_path.starts_with('/') && canonical_path.len() > 1 {
            let endpoint = self.effective_endpoint(creds);
            url::Url::parse(&endpoint)
                .ok()
                .and_then(|u| url_authority(&u))
                .unwrap_or_else(|| format!("s3.{}.amazonaws.com", creds.region))
        } else {
            format!("s3.{}.amazonaws.com", creds.region)
        };

        // Collect all headers
        let mut headers = vec![
            ("host".to_string(), host.clone()),
            ("x-amz-content-sha256".to_string(), payload_hash.to_string()),
            ("x-amz-date".to_string(), amz_date.clone()),
        ];

        if let Some(ref token) = creds.session_token {
            headers.push(("x-amz-security-token".to_string(), token.clone()));
        }

        for (k, v) in extra_headers {
            headers.push((k.to_lowercase(), v.to_string()));
        }

        headers.sort_by(|a, b| a.0.cmp(&b.0));

        let signed_headers: String = headers
            .iter()
            .map(|(k, _)| k.as_str())
            .collect::<Vec<_>>()
            .join(";");

        let canonical_headers: String = headers
            .iter()
            .map(|(k, v)| format!("{k}:{v}\n"))
            .collect();

        // Canonical request
        let canonical_request = format!(
            "{method}\n{canonical_path}\n{query_string}\n{canonical_headers}\n{signed_headers}\n{payload_hash}"
        );

        // String to sign
        let string_to_sign = format!(
            "AWS4-HMAC-SHA256\n{}\n{}\n{}",
            amz_date,
            credential_scope,
            hex::encode(Sha256::digest(canonical_request.as_bytes()))
        );

        // Calculate signature
        let signing_key = derive_signing_key(
            &creds.secret_access_key,
            &date_stamp,
            &creds.region,
            "s3",
        )?;
        let signature = hex::encode(hmac_sha256(&signing_key, string_to_sign.as_bytes())?);

        let authorization = format!(
            "AWS4-HMAC-SHA256 Credential={}/{}, SignedHeaders={}, Signature={}",
            creds.access_key_id, credential_scope, signed_headers, signature
        );

        let mut result = vec![
            ("Authorization".to_string(), authorization),
            ("x-amz-date".to_string(), amz_date),
            ("x-amz-content-sha256".to_string(), payload_hash.to_string()),
        ];

        if let Some(ref token) = creds.session_token {
            result.push(("x-amz-security-token".to_string(), token.clone()));
        }

        Ok(result)
    }

    // ── S3 Lifecycle Rules Management ──

    /// Get lifecycle rules for a bucket via GET /{bucket}?lifecycle.
    pub async fn get_lifecycle_rules(
        &self,
        bucket: &str,
    ) -> Result<Vec<S3LifecycleRule>, AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let url = format!("{endpoint}{base_path}?lifecycle");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/")
        } else {
            "/".to_string()
        };

        let headers = self.sign_request(
            &state.credentials,
            "GET",
            &canonical_path,
            "lifecycle",
            &[],
            &now,
        )?;

        let mut req = state.http.get(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 get lifecycle rules failed: {e}"),
            advice: "Check your credentials and network connection.".to_string(),
        })?;

        // 404 means no lifecycle configuration exists — return empty list
        if resp.status().as_u16() == 404 {
            return Ok(Vec::new());
        }

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("S3 get lifecycle rules failed (HTTP {status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        let body = resp.text().await.map_err(|e| AppError::Connection {
            message: format!("Failed to read S3 response: {e}"),
            advice: "Try again.".to_string(),
        })?;

        Ok(parse_lifecycle_rules_xml(&body))
    }

    /// Put a lifecycle rule on a bucket. Fetches existing rules, merges or replaces
    /// the rule with the same ID, then PUTs the full configuration via PUT /{bucket}?lifecycle.
    pub async fn put_lifecycle_rule(
        &self,
        bucket: &str,
        rule: S3LifecycleRule,
    ) -> Result<(), AppError> {
        // Fetch existing rules so we can merge
        let mut rules = self.get_lifecycle_rules(bucket).await?;

        // Replace existing rule with same ID, or append
        if let Some(pos) = rules.iter().position(|r| r.id == rule.id) {
            rules[pos] = rule;
        } else {
            rules.push(rule);
        }

        self.put_lifecycle_configuration(bucket, &rules).await
    }

    /// Delete a specific lifecycle rule by ID. Fetches all rules, removes the one
    /// matching `rule_id`, then PUTs the remaining configuration back.
    pub async fn delete_lifecycle_rule(
        &self,
        bucket: &str,
        rule_id: &str,
    ) -> Result<(), AppError> {
        let mut rules = self.get_lifecycle_rules(bucket).await?;

        let original_len = rules.len();
        rules.retain(|r| r.id != rule_id);

        if rules.len() == original_len {
            return Err(AppError::Connection {
                message: format!("Lifecycle rule '{rule_id}' not found on bucket '{bucket}'"),
                advice: "Check the rule ID and try again.".to_string(),
            });
        }

        if rules.is_empty() {
            // No rules left — delete the entire lifecycle configuration
            self.delete_lifecycle_configuration(bucket).await
        } else {
            self.put_lifecycle_configuration(bucket, &rules).await
        }
    }

    /// PUT a full lifecycle configuration (all rules) to the bucket.
    async fn put_lifecycle_configuration(
        &self,
        bucket: &str,
        rules: &[S3LifecycleRule],
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let url = format!("{endpoint}{base_path}?lifecycle");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/")
        } else {
            "/".to_string()
        };

        let xml_body = build_lifecycle_xml(rules);
        let xml_bytes = xml_body.as_bytes();
        let content_hash = hex::encode(Sha256::digest(xml_bytes));

        // S3 requires Content-MD5 for PutBucketLifecycleConfiguration
        use digest::Digest as _;
        let md5_hash = md5::Md5::digest(xml_bytes);
        let content_md5 = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            md5_hash.as_slice(),
        );

        let headers = self.sign_request_with_payload(
            &state.credentials,
            "PUT",
            &canonical_path,
            "lifecycle",
            &[
                ("content-type", "application/xml"),
                ("content-length", &xml_bytes.len().to_string()),
                ("content-md5", &content_md5),
            ],
            &content_hash,
            &now,
        )?;

        let mut req = state.http.put(&url).body(xml_bytes.to_vec());
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req = req.header("Content-Type", "application/xml");
        req = req.header("Content-MD5", &content_md5);

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 put lifecycle configuration failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() {
            let resp_status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("S3 put lifecycle configuration failed (HTTP {resp_status}): {body}"),
                advice: "Check your permissions and bucket policy.".to_string(),
            });
        }

        tracing::info!("Updated lifecycle configuration on bucket {bucket} ({} rules)", rules.len());
        Ok(())
    }

    /// DELETE the entire lifecycle configuration from a bucket.
    async fn delete_lifecycle_configuration(
        &self,
        bucket: &str,
    ) -> Result<(), AppError> {
        let state = self.state.read().await;
        let state = state.as_ref().ok_or_else(|| AppError::Connection {
            message: "Not connected to S3".to_string(),
            advice: "Connect first.".to_string(),
        })?;

        let now = Utc::now();
        let (endpoint, base_path) = self.bucket_url(&state.credentials, bucket);
        let url = format!("{endpoint}{base_path}?lifecycle");

        let canonical_path = if state.credentials.path_style {
            format!("/{bucket}/")
        } else {
            "/".to_string()
        };

        let headers = self.sign_request(
            &state.credentials,
            "DELETE",
            &canonical_path,
            "lifecycle",
            &[],
            &now,
        )?;

        let mut req = state.http.delete(&url);
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }

        let resp = req.send().await.map_err(|e| AppError::Connection {
            message: format!("S3 delete lifecycle configuration failed: {e}"),
            advice: "Check your network connection.".to_string(),
        })?;

        if !resp.status().is_success() && resp.status().as_u16() != 204 {
            let resp_status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(AppError::Connection {
                message: format!("S3 delete lifecycle configuration failed (HTTP {resp_status}): {body}"),
                advice: "Check your permissions.".to_string(),
            });
        }

        tracing::info!("Deleted lifecycle configuration from bucket {bucket}");
        Ok(())
    }
}

/// Whether a 2xx CopyObject reply reports a copy. S3 may answer 200 and
/// still fail the copy, with an `<Error>` body; a successful copy always
/// returns `<CopyObjectResult>`.
fn copy_reply_ok(body: &str) -> Result<(), String> {
    if body.contains("<Error>") {
        let field = |tag: &str| body.split(&format!("<{tag}>")).nth(1).and_then(|r| r.split(&format!("</{tag}>")).next()).unwrap_or_default().to_string();
        return Err(format!("S3 reported {} {}", field("Code"), field("Message")).trim_end().to_string());
    }
    if !body.contains("<CopyObjectResult") {
        return Err("S3 did not confirm the copy".to_string());
    }
    Ok(())
}

impl Default for S3Connector {
    fn default() -> Self {
        Self::new()
    }
}

impl Connector for S3Connector {
    /// CopyObject between `/bucket/key` paths of this connection.
    fn server_copy(&self, from: &str, to: &str) -> Pin<Box<dyn Future<Output = Result<bool, AppError>> + Send + '_>> {
        let (from, to) = (from.to_string(), to.to_string());
        Box::pin(async move {
            let (sb, sk) = super::split_bucket_key(&from)?;
            let (db, dk) = super::split_bucket_key(&to)?;
            self.copy_object(&sb, &sk, &db, &dk).await.map(|()| true)
        })
    }

    /// S3 has no rename: a file is copied on the server, then the original
    /// deleted. Folders (prefixes) are moved file by file by the caller.
    fn rename_entry(&self, from: &str, to: &str) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let (from, to) = (from.to_string(), to.to_string());
        Box::pin(async move {
            if from.ends_with('/') {
                return Err(AppError::validation("S3 folders are moved file by file"));
            }
            let (sb, sk) = super::split_bucket_key(&from)?;
            let (db, dk) = super::split_bucket_key(&to)?;
            self.copy_object(&sb, &sk, &db, &dk).await?;
            // Delete the original only once the copy is there.
            self.get_object_properties(&db, &dk).await.map_err(|e| AppError::transfer(format!("The copy of {from} could not be confirmed ({e}); the original was kept."), "Check the destination, then retry."))?;
            self.delete_object(&sb, &sk).await
        })
    }

    fn connect(
        &self,
        profile: &ConnectionProfile,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let profile = profile.clone();
        Box::pin(async move {
            // Extract S3 credentials from the profile
            // For S3, host = endpoint (or region), username = access_key_id
            // password = secret_access_key (stored in credential store)
            let creds = S3Credentials {
                access_key_id: profile.username.clone().unwrap_or_default(),
                secret_access_key: String::new(), // Will be set from credential store
                region: if profile.host.contains('.') {
                    // Custom endpoint
                    "us-east-1".to_string()
                } else {
                    profile.host.clone()
                },
                endpoint: if profile.host.contains('.') {
                    Some(format!("https://{}", profile.host))
                } else {
                    None
                },
                path_style: false,
                session_token: None,
            };
            self.connect_with_credentials(creds).await
        })
    }

    fn disconnect(&self) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        Box::pin(async move {
            let mut state = self.state.write().await;
            *state = None;
            Ok(())
        })
    }

    fn list_remote(
        &self,
        path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<FileEntry>, AppError>> + Send + '_>> {
        let path = path.to_string();
        Box::pin(async move {
            if path.is_empty() || path == "/" {
                // List buckets at root
                self.list_buckets().await
            } else {
                // Parse bucket/prefix from path
                let path = path.trim_start_matches('/');
                let parts: Vec<&str> = path.splitn(2, '/').collect();
                let bucket = parts[0];
                let prefix = if parts.len() > 1 { parts[1] } else { "" };
                // With a "/" delimiter a folder prefix must end in "/", otherwise the
                // listing returns the folder itself instead of its contents.
                let prefix = if prefix.is_empty() || prefix.ends_with('/') { prefix.to_string() } else { format!("{prefix}/") };
                self.list_objects(bucket, &prefix, Some("/")).await
            }
        })
    }

    fn is_connected(&self) -> bool {
        self.state.try_read().map(|s| s.is_some()).unwrap_or(true)
    }

    fn connect_with_secret(
        &self,
        profile: &ConnectionProfile,
        secret: Option<String>,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let profile = profile.clone();
        Box::pin(async move {
            let host = profile.host.trim().trim_end_matches('/').to_string();
            // Host forms: a bare region ("eu-west-1"), an AWS/custom hostname, or a
            // full URL ("http://localhost:9000") for S3-compatible servers.
            let (region, endpoint, path_style) = if host.starts_with("http://") || host.starts_with("https://") {
                ("us-east-1".to_string(), Some(host), true)
            } else if host.contains('.') {
                let aws = host.ends_with("amazonaws.com");
                ("us-east-1".to_string(), Some(format!("https://{host}")), !aws)
            } else {
                (host, None, false)
            };
            let creds = S3Credentials {
                access_key_id: profile.username.clone().unwrap_or_default(),
                secret_access_key: secret.unwrap_or_default(),
                region,
                endpoint,
                path_style,
                session_token: None,
            };
            match profile.extras.assume_role.clone().filter(|r| !r.role_arn.trim().is_empty()) {
                Some(role) => {
                    let base = super::aws_credentials::BaseKeys {
                        access_key_id: creds.access_key_id.clone(),
                        secret_access_key: creds.secret_access_key.clone(),
                        session_token: None,
                    };
                    self.connect_with_role(base, role, creds, None).await?;
                }
                None => self.connect_with_credentials(creds).await?,
            }
            // Fail now (bad keys, unreachable endpoint) rather than on first browse.
            if let Err(e) = self.list_buckets().await {
                *self.state.write().await = None;
                return Err(e);
            }
            Ok(())
        })
    }

    fn upload(
        &self,
        local_path: &str,
        remote_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<u64, AppError>> + Send + '_>> {
        let (l, r) = (local_path.to_string(), remote_path.to_string());
        Box::pin(async move {
            let (bucket, key) = super::split_bucket_key(&r)?;
            let data = tokio::fs::read(&l).await.map_err(|e| AppError::file_op(
                format!("Cannot read {l}: {e}"),
                "Check the file exists and is readable.",
            ))?;
            let n = data.len() as u64;
            self.upload_file(&bucket, &key, data).await?;
            Ok(n)
        })
    }

    fn download(
        &self,
        remote_path: &str,
        local_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<u64, AppError>> + Send + '_>> {
        let (r, l) = (remote_path.to_string(), local_path.to_string());
        Box::pin(async move {
            let (bucket, key) = super::split_bucket_key(&r)?;
            let data = self.download_object(&bucket, &key, None).await?;
            tokio::fs::write(&l, &data).await.map_err(|e| AppError::file_op(
                format!("Cannot write {l}: {e}"),
                "Check the destination folder exists and is writable.",
            ))?;
            Ok(data.len() as u64)
        })
    }

    fn make_directory(
        &self,
        remote_path: &str,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let r = remote_path.trim_end_matches('/').to_string();
        Box::pin(async move {
            // Object stores have no real folders: an empty "key/" object is the convention.
            let (bucket, key) = super::split_bucket_key(&r)?;
            self.put_object(&bucket, &format!("{key}/"), &[]).await.map(|_| ())
        })
    }

    fn remove(
        &self,
        remote_path: &str,
        is_dir: bool,
    ) -> Pin<Box<dyn Future<Output = Result<(), AppError>> + Send + '_>> {
        let r = remote_path.to_string();
        Box::pin(async move {
            let (bucket, key) = super::split_bucket_key(&r)?;
            let key = if is_dir && !key.ends_with('/') { format!("{key}/") } else { key };
            self.delete_object(&bucket, &key).await
        })
    }

    fn create_share_link(
        &self,
        remote_path: &str,
        options: &super::ShareOptions,
    ) -> Pin<Box<dyn Future<Output = Result<super::ShareLink, AppError>> + Send + '_>> {
        let (r, o) = (remote_path.to_string(), options.clone());
        Box::pin(async move {
            let secs = presign_expiry(&o)?;
            let (bucket, key) = super::split_bucket_key(&r)?;
            let state = self.state.read().await;
            let st = state.as_ref().ok_or_else(|| AppError::connection("Not connected to S3", "Connect first."))?;
            let url = self.generate_presigned_url(&st.credentials, &bucket, &key, secs)?;
            Ok(super::ShareLink {
                url,
                expires_at: Some(Utc::now() + chrono::Duration::seconds(secs as i64)),
                scope: super::ShareScope::Anyone,
                provider: "s3".into(),
                note: Some("Pre-signed download link: anyone holding it can download until it expires.".into()),
            })
        })
    }
}

/// When temporary credentials expire (an hour if STS didn't say).
fn expiry_of(c: &super::aws_credentials::AwsCredentials) -> chrono::DateTime<Utc> {
    c.expiration
        .as_deref()
        .and_then(|e| chrono::DateTime::parse_from_rfc3339(e).ok())
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|| Utc::now() + chrono::Duration::hours(1))
}

/// SigV4 pre-signed URLs live at most 7 days; default one day. They are
/// bearer links for downloading only.
pub(crate) const PRESIGN_MAX_SECS: u64 = 7 * 24 * 3600;

pub(crate) fn presign_expiry(o: &super::ShareOptions) -> Result<u64, AppError> {
    if o.password.as_deref().is_some_and(|p| !p.is_empty()) {
        return Err(super::share_unsupported("set a password"));
    }
    if o.allow_edit {
        return Err(super::share_unsupported("allow editing"));
    }
    if o.scope != super::ShareScope::Anyone {
        return Err(super::share_unsupported("limit links to your organization"));
    }
    let secs = o.expires_in_secs.unwrap_or(24 * 3600);
    if secs == 0 || secs > PRESIGN_MAX_SECS {
        return Err(AppError::validation("S3 links must expire within 7 days"));
    }
    Ok(secs)
}

// ── AWS Signature V4 helpers ──

fn derive_signing_key(
    secret: &str,
    date_stamp: &str,
    region: &str,
    service: &str,
) -> Result<Vec<u8>, AppError> {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date_stamp.as_bytes())?;
    let k_region = hmac_sha256(&k_date, region.as_bytes())?;
    let k_service = hmac_sha256(&k_region, service.as_bytes())?;
    hmac_sha256(&k_service, b"aws4_request")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut mac = HmacSha256::new_from_slice(key).map_err(|e| {
        AppError::internal(format!("HMAC key error: {e}"))
    })?;
    mac.update(data);
    Ok(mac.finalize().into_bytes().to_vec())
}

/// AWS S3 URI-encode set: encode everything except unreserved chars (A-Z a-z 0-9 - . _ ~).
/// Per AWS Signature V4 spec, slashes in the path are preserved (not encoded).
const S3_ENCODE_SET: &percent_encoding::AsciiSet = &percent_encoding::NON_ALPHANUMERIC
    .remove(b'-')
    .remove(b'.')
    .remove(b'_')
    .remove(b'~');

/// One page of a ListObjectsV2 response.
struct ListPage {
    entries: Vec<FileEntry>,
    next_token: Option<String>,
}

/// SigV4 canonical query string, also used verbatim as the request query:
/// keys and values RFC 3986-encoded (unreserved characters kept, `/`
/// encoded) and sorted by key. Unsorted or over-encoded queries (e.g. a `-`
/// sent as `%2D`) make S3 reject the signature.
fn canonical_query(params: &[(&str, String)]) -> String {
    let mut pairs: Vec<(String, String)> = params
        .iter()
        .map(|(k, v)| {
            (
                percent_encoding::percent_encode(k.as_bytes(), S3_ENCODE_SET).to_string(),
                percent_encoding::percent_encode(v.as_bytes(), S3_ENCODE_SET).to_string(),
            )
        })
        .collect();
    pairs.sort();
    pairs.iter().map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("&")
}

fn percent_encode_path(path: &str) -> String {
    path.split('/')
        .map(|segment| {
            percent_encoding::percent_encode(
                segment.as_bytes(),
                S3_ENCODE_SET,
            )
            .to_string()
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn calculate_part_size(total_size: u64) -> u64 {
    // Start with default, increase if we'd exceed max parts
    let mut size = DEFAULT_PART_SIZE;
    while total_size / size > MAX_PARTS {
        size *= 2;
    }
    size
}

// ── XML parsing helpers (minimal, no external XML crate) ──

fn extract_xml_value(xml: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = xml.find(&open)? + open.len();
    let end = xml[start..].find(&close)? + start;
    Some(xml_unescape(&xml[start..end]))
}

/// Decode the five predefined XML entities (S3 escapes `&` in keys and
/// tokens, and `"` in ETags).
fn xml_unescape(s: &str) -> String {
    if !s.contains('&') {
        return s.to_string();
    }
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn parse_list_buckets_xml(xml: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();
    let mut pos = 0;

    while let Some(start) = xml[pos..].find("<Bucket>") {
        let start = pos + start;
        let end = match xml[start..].find("</Bucket>") {
            Some(e) => start + e + "</Bucket>".len(),
            None => break,
        };
        let bucket_xml = &xml[start..end];

        if let Some(name) = extract_xml_value(bucket_xml, "Name") {
            let created = extract_xml_value(bucket_xml, "CreationDate")
                .and_then(|d| d.parse::<DateTime<Utc>>().ok());

            entries.push(FileEntry {
                name: name.clone(),
                path: format!("/{name}"),
                is_dir: true,
                is_symlink: false,
                size: 0,
                modified: created,
                created,
                is_hidden: false,
                extension: None,
                permissions: None,
            });
        }

        pos = end;
    }

    entries
}

fn parse_list_objects_xml(xml: &str, prefix: &str) -> Vec<FileEntry> {
    let mut entries = Vec::new();

    // Parse CommonPrefixes (virtual folders)
    let mut pos = 0;
    while let Some(start) = xml[pos..].find("<CommonPrefixes>") {
        let start = pos + start;
        let end = match xml[start..].find("</CommonPrefixes>") {
            Some(e) => start + e + "</CommonPrefixes>".len(),
            None => break,
        };
        let cp_xml = &xml[start..end];

        if let Some(pfx) = extract_xml_value(cp_xml, "Prefix") {
            let display_name = pfx.trim_end_matches('/');
            let display_name = display_name
                .strip_prefix(prefix)
                .unwrap_or(display_name)
                .trim_start_matches('/');

            if !display_name.is_empty() {
                entries.push(FileEntry {
                    name: display_name.to_string(),
                    path: format!("/{pfx}"),
                    is_dir: true,
                    is_symlink: false,
                    size: 0,
                    modified: None,
                    created: None,
                    is_hidden: false,
                    extension: None,
                    permissions: None,
                });
            }
        }

        pos = end;
    }

    // Parse Contents (files)
    pos = 0;
    while let Some(start) = xml[pos..].find("<Contents>") {
        let start = pos + start;
        let end = match xml[start..].find("</Contents>") {
            Some(e) => start + e + "</Contents>".len(),
            None => break,
        };
        let content_xml = &xml[start..end];

        if let Some(key) = extract_xml_value(content_xml, "Key") {
            // Skip the prefix itself if it appears as a key
            if key == prefix || key.ends_with('/') {
                pos = end;
                continue;
            }

            let display_name = key
                .strip_prefix(prefix)
                .unwrap_or(&key)
                .trim_start_matches('/');

            let size = extract_xml_value(content_xml, "Size")
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0);

            let modified = extract_xml_value(content_xml, "LastModified")
                .and_then(|d| d.parse::<DateTime<Utc>>().ok());

            let extension = display_name
                .rsplit('.')
                .next()
                .filter(|ext| ext.len() < 10 && !ext.contains('/'))
                .map(|s| s.to_string());

            entries.push(FileEntry {
                name: display_name.to_string(),
                path: format!("/{key}"),
                is_dir: false,
                is_symlink: false,
                size,
                modified,
                created: None,
                is_hidden: display_name.starts_with('.'),
                extension,
                permissions: None,
            });
        }

        pos = end;
    }

    entries
}

fn parse_upload_id_xml(xml: &str) -> Result<String, AppError> {
    extract_xml_value(xml, "UploadId").ok_or_else(|| AppError::Transfer {
        message: "Failed to parse S3 multipart upload response".to_string(),
        advice: "Try again.".to_string(),
    })
}

fn parse_list_parts_xml(xml: &str) -> Vec<CompletedPart> {
    let mut parts = Vec::new();
    let mut pos = 0;

    while let Some(start) = xml[pos..].find("<Part>") {
        let start = pos + start;
        let end = match xml[start..].find("</Part>") {
            Some(e) => start + e + "</Part>".len(),
            None => break,
        };
        let part_xml = &xml[start..end];

        let part_number = extract_xml_value(part_xml, "PartNumber")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);
        let etag = extract_xml_value(part_xml, "ETag")
            .unwrap_or_default()
            .trim_matches('"')
            .to_string();
        let size = extract_xml_value(part_xml, "Size")
            .and_then(|s| s.parse().ok())
            .unwrap_or(0);

        if part_number > 0 {
            parts.push(CompletedPart {
                part_number,
                etag,
                size,
            });
        }

        pos = end;
    }

    parts
}

// ── CloudFront XML parsing helpers ──

/// Parse the ListDistributions XML response from CloudFront.
fn parse_cloudfront_distributions_xml(xml: &str) -> Vec<CloudFrontDistribution> {
    let mut distributions = Vec::new();
    let mut pos = 0;

    while let Some(start) = xml[pos..].find("<DistributionSummary>") {
        let start = pos + start;
        let end = match xml[start..].find("</DistributionSummary>") {
            Some(e) => start + e + "</DistributionSummary>".len(),
            None => break,
        };
        let dist_xml = &xml[start..end];

        let id = extract_xml_value(dist_xml, "Id").unwrap_or_default();
        let domain_name = extract_xml_value(dist_xml, "DomainName").unwrap_or_default();
        let status = extract_xml_value(dist_xml, "Status").unwrap_or_default();
        let enabled = extract_xml_value(dist_xml, "Enabled")
            .map(|v| v == "true")
            .unwrap_or(false);
        let comment = extract_xml_value(dist_xml, "Comment")
            .filter(|c| !c.is_empty());

        // Parse origins
        let mut origins = Vec::new();
        let mut opos = 0;
        while let Some(ostart) = dist_xml[opos..].find("<DomainName>") {
            let ostart = opos + ostart;
            // Skip the distribution-level DomainName (already captured)
            if ostart == 0 {
                opos = ostart + "<DomainName>".len();
                continue;
            }
            let oend = match dist_xml[ostart..].find("</DomainName>") {
                Some(e) => ostart + e,
                None => break,
            };
            let origin_domain = &dist_xml[ostart + "<DomainName>".len()..oend];
            // Only include origins within <Origin> blocks (they contain S3 bucket domains)
            if !origin_domain.is_empty() && origin_domain != domain_name {
                origins.push(origin_domain.to_string());
            }
            opos = oend + "</DomainName>".len();
        }

        if !id.is_empty() {
            distributions.push(CloudFrontDistribution {
                id,
                domain_name,
                status,
                enabled,
                origins,
                comment,
            });
        }

        pos = end;
    }

    distributions
}

/// Parse the ListInvalidations XML response from CloudFront.
fn parse_cloudfront_invalidations_xml(xml: &str) -> Vec<CloudFrontInvalidation> {
    let mut invalidations = Vec::new();
    let mut pos = 0;

    while let Some(start) = xml[pos..].find("<InvalidationSummary>") {
        let start = pos + start;
        let end = match xml[start..].find("</InvalidationSummary>") {
            Some(e) => start + e + "</InvalidationSummary>".len(),
            None => break,
        };
        let inv_xml = &xml[start..end];

        let id = extract_xml_value(inv_xml, "Id").unwrap_or_default();
        let status = extract_xml_value(inv_xml, "Status").unwrap_or_default();
        let created_at = extract_xml_value(inv_xml, "CreateTime");

        if !id.is_empty() {
            invalidations.push(CloudFrontInvalidation {
                id,
                status,
                paths: Vec::new(), // Summary listing doesn't include paths
                created_at,
            });
        }

        pos = end;
    }

    invalidations
}

// ── Lifecycle XML parsing and building helpers ──

/// Parse the GetBucketLifecycleConfiguration XML response into a list of rules.
fn parse_lifecycle_rules_xml(xml: &str) -> Vec<S3LifecycleRule> {
    let mut rules = Vec::new();
    let mut pos = 0;

    while let Some(start) = xml[pos..].find("<Rule>") {
        let start = pos + start;
        let end = match xml[start..].find("</Rule>") {
            Some(e) => start + e + "</Rule>".len(),
            None => break,
        };
        let rule_xml = &xml[start..end];

        let id = extract_xml_value(rule_xml, "ID").unwrap_or_default();
        let prefix = extract_xml_value(rule_xml, "Prefix").unwrap_or_default();
        let status = extract_xml_value(rule_xml, "Status").unwrap_or_default();
        let enabled = status == "Enabled";

        // Parse transitions
        let transitions = parse_lifecycle_transitions(rule_xml);

        // Parse expiration days
        let expiration_days = extract_xml_value(rule_xml, "Expiration")
            .and_then(|exp_xml| extract_xml_value(&format!("<E>{exp_xml}</E>"), "Days"))
            .and_then(|d| d.parse::<u32>().ok())
            .or_else(|| {
                // Try direct <Days> inside <Expiration>
                extract_lifecycle_sub_value(rule_xml, "Expiration", "Days")
                    .and_then(|d| d.parse::<u32>().ok())
            });

        // Parse abort incomplete multipart upload days
        let abort_incomplete_multipart_days =
            extract_lifecycle_sub_value(rule_xml, "AbortIncompleteMultipartUpload", "DaysAfterInitiation")
                .and_then(|d| d.parse::<u32>().ok());

        rules.push(S3LifecycleRule {
            id,
            prefix,
            enabled,
            transitions,
            expiration_days,
            abort_incomplete_multipart_days,
        });

        pos = end;
    }

    rules
}

/// Parse all <Transition> elements within a rule XML fragment.
fn parse_lifecycle_transitions(rule_xml: &str) -> Vec<S3LifecycleTransition> {
    let mut transitions = Vec::new();
    let mut pos = 0;

    while let Some(start) = rule_xml[pos..].find("<Transition>") {
        let start = pos + start;
        let end = match rule_xml[start..].find("</Transition>") {
            Some(e) => start + e + "</Transition>".len(),
            None => break,
        };
        let transition_xml = &rule_xml[start..end];

        let days = extract_xml_value(transition_xml, "Days")
            .and_then(|d| d.parse::<u32>().ok())
            .unwrap_or(0);
        let storage_class = extract_xml_value(transition_xml, "StorageClass")
            .unwrap_or_else(|| "STANDARD".to_string());

        transitions.push(S3LifecycleTransition {
            days,
            storage_class,
        });

        pos = end;
    }

    transitions
}

/// Extract a value from a sub-element: look for <parent>...<child>VALUE</child>...</parent>.
fn extract_lifecycle_sub_value(xml: &str, parent_tag: &str, child_tag: &str) -> Option<String> {
    let open_parent = format!("<{parent_tag}>");
    let close_parent = format!("</{parent_tag}>");
    let start = xml.find(&open_parent)? + open_parent.len();
    let end = xml[start..].find(&close_parent)? + start;
    let inner = &xml[start..end];
    extract_xml_value(inner, child_tag)
}

/// Build the LifecycleConfiguration XML body from a list of rules.
fn build_lifecycle_xml(rules: &[S3LifecycleRule]) -> String {
    let mut xml = String::from(
        "<LifecycleConfiguration xmlns=\"http://s3.amazonaws.com/doc/2006-03-01/\">"
    );

    for rule in rules {
        xml.push_str("<Rule>");
        xml.push_str(&format!("<ID>{}</ID>", xml_escape(&rule.id)));

        // Filter — use prefix-based filter
        xml.push_str("<Filter>");
        if rule.prefix.is_empty() {
            xml.push_str("<Prefix></Prefix>");
        } else {
            xml.push_str(&format!("<Prefix>{}</Prefix>", xml_escape(&rule.prefix)));
        }
        xml.push_str("</Filter>");

        xml.push_str(&format!(
            "<Status>{}</Status>",
            if rule.enabled { "Enabled" } else { "Disabled" }
        ));

        for transition in &rule.transitions {
            xml.push_str(&format!(
                "<Transition><Days>{}</Days><StorageClass>{}</StorageClass></Transition>",
                transition.days,
                xml_escape(&transition.storage_class)
            ));
        }

        if let Some(days) = rule.expiration_days {
            xml.push_str(&format!("<Expiration><Days>{days}</Days></Expiration>"));
        }

        if let Some(days) = rule.abort_incomplete_multipart_days {
            xml.push_str(&format!(
                "<AbortIncompleteMultipartUpload><DaysAfterInitiation>{days}</DaysAfterInitiation></AbortIncompleteMultipartUpload>"
            ));
        }

        xml.push_str("</Rule>");
    }

    xml.push_str("</LifecycleConfiguration>");
    xml
}

/// Simple XML escape for text content.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// Parse S3 ListObjectVersions XML response into S3ObjectVersion structs.
fn parse_object_versions_xml(xml: &str, key_filter: &str) -> Vec<S3ObjectVersion> {
    let mut versions = Vec::new();

    // Parse <Version> elements
    let mut pos = 0;
    while let Some(start) = xml[pos..].find("<Version>") {
        let abs_start = pos + start;
        if let Some(end) = xml[abs_start..].find("</Version>") {
            let version_xml = &xml[abs_start..abs_start + end + 10];

            let key = extract_xml_value(version_xml, "Key").unwrap_or_default();
            if !key_filter.is_empty() && key != key_filter {
                pos = abs_start + end + 10;
                continue;
            }

            let version_id = extract_xml_value(version_xml, "VersionId")
                .unwrap_or_else(|| "null".to_string());
            let last_modified = extract_xml_value(version_xml, "LastModified");
            let size = extract_xml_value(version_xml, "Size")
                .and_then(|s| s.parse::<u64>().ok());
            let is_latest = extract_xml_value(version_xml, "IsLatest")
                .map(|v| v == "true")
                .unwrap_or(false);

            versions.push(S3ObjectVersion {
                version_id,
                key,
                last_modified,
                size,
                is_latest,
                is_delete_marker: false,
            });

            pos = abs_start + end + 10;
        } else {
            break;
        }
    }

    // Parse <DeleteMarker> elements
    pos = 0;
    while let Some(start) = xml[pos..].find("<DeleteMarker>") {
        let abs_start = pos + start;
        if let Some(end) = xml[abs_start..].find("</DeleteMarker>") {
            let dm_xml = &xml[abs_start..abs_start + end + 15];

            let key = extract_xml_value(dm_xml, "Key").unwrap_or_default();
            if !key_filter.is_empty() && key != key_filter {
                pos = abs_start + end + 15;
                continue;
            }

            let version_id = extract_xml_value(dm_xml, "VersionId")
                .unwrap_or_else(|| "null".to_string());
            let last_modified = extract_xml_value(dm_xml, "LastModified");
            let is_latest = extract_xml_value(dm_xml, "IsLatest")
                .map(|v| v == "true")
                .unwrap_or(false);

            versions.push(S3ObjectVersion {
                version_id,
                key,
                last_modified,
                size: None,
                is_latest,
                is_delete_marker: true,
            });

            pos = abs_start + end + 15;
        } else {
            break;
        }
    }

    // Sort by last_modified descending (latest first)
    versions.sort_by(|a, b| b.last_modified.cmp(&a.last_modified));
    versions
}

/// An S3 error reply: rejected keys (401, or 403 with `InvalidAccessKeyId`,
/// `SignatureDoesNotMatch`, `ExpiredToken`…) are authentication failures,
/// which are not retried; a plain 403 (bucket policy) is not.
pub(crate) fn s3_status_error(message: String, status: u16, body: &str, advice: &str) -> AppError {
    const REJECTED: &[&str] = &["InvalidAccessKeyId", "SignatureDoesNotMatch", "ExpiredToken", "InvalidToken", "TokenRefreshRequired", "InvalidClientTokenId"];
    if status == 401 || (status == 403 && REJECTED.iter().any(|c| body.contains(&format!("<Code>{c}</Code>")))) {
        AppError::auth(message, "Check the access key ID and secret (they were refused).")
    } else {
        AppError::connection(message, advice)
    }
}

#[cfg(test)]
mod auth_error_tests {
    #[test]
    fn rejected_keys_are_auth_failures_but_policy_denials_are_not() {
        let e = super::s3_status_error("x".into(), 403, "<Error><Code>SignatureDoesNotMatch</Code></Error>", "a");
        assert!(e.is_auth_failure());
        assert!(super::s3_status_error("x".into(), 403, "<Error><Code>InvalidAccessKeyId</Code></Error>", "a").is_auth_failure());
        assert!(!super::s3_status_error("x".into(), 403, "<Error><Code>AccessDenied</Code></Error>", "a").is_auth_failure());
        assert!(!super::s3_status_error("x".into(), 500, "", "a").is_auth_failure());
    }
}

#[cfg(test)]
mod share_tests {
    use super::*;
    use crate::connectors::{ShareOptions, ShareScope};

    #[tokio::test]
    async fn assumes_role_with_external_id_and_refreshes() {
        use axum::{routing::post, Router};
        let calls = std::sync::Arc::new(std::sync::Mutex::new(Vec::<(String, String)>::new()));
        let c2 = calls.clone();
        let app = Router::new().route(
            "/",
            post(move |headers: axum::http::HeaderMap, body: String| {
                let calls = c2.clone();
                async move {
                    let n = {
                        let mut v = calls.lock().unwrap();
                        v.push((headers.get("authorization").map(|h| h.to_str().unwrap().to_string()).unwrap_or_default(), body));
                        v.len()
                    };
                    // Expires inside the refresh window, so a refresh follows at once.
                    let exp = (chrono::Utc::now() + chrono::Duration::seconds(10)).to_rfc3339();
                    format!("<AssumeRoleResponse><AssumeRoleResult><Credentials><AccessKeyId>ASIA{n}</AccessKeyId><SecretAccessKey>sk{n}</SecretAccessKey><SessionToken>tok{n}</SessionToken><Expiration>{exp}</Expiration></Credentials></AssumeRoleResult></AssumeRoleResponse>")
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let c = S3Connector::new();
        let base = crate::connectors::aws_credentials::BaseKeys { access_key_id: "AKIDBASE".into(), secret_access_key: "basesecret".into(), session_token: None };
        let role = crate::core::types::AssumeRoleConfig { role_arn: "arn:aws:iam::123456789012:role/Reader".into(), external_id: Some("ext-42".into()), session_name: None, duration_secs: Some(900) };
        let creds = S3Credentials { access_key_id: String::new(), secret_access_key: String::new(), region: "eu-west-1".into(), endpoint: None, path_style: false, session_token: None };
        c.connect_with_role(base, role, creds, Some(format!("http://{addr}"))).await.unwrap();
        {
            let st = c.state.read().await;
            let cr = &st.as_ref().unwrap().credentials;
            assert_eq!((cr.access_key_id.as_str(), cr.session_token.as_deref()), ("ASIA1", Some("tok1")));
        }
        let (auth, body) = calls.lock().unwrap()[0].clone();
        assert!(auth.starts_with("AWS4-HMAC-SHA256 Credential=AKIDBASE/") && auth.contains("/eu-west-1/sts/aws4_request"), "{auth}");
        assert!(body.contains("Action=AssumeRole") && body.contains("ExternalId=ext-42") && body.contains("DurationSeconds=900"), "{body}");
        assert!(body.contains("RoleArn=arn%3Aaws%3Aiam%3A%3A123456789012%3Arole%2FReader"));
        // The refresher swaps in new credentials before expiry.
        for _ in 0..100 {
            if c.state.read().await.as_ref().unwrap().credentials.access_key_id != "ASIA1" {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        assert_ne!(c.state.read().await.as_ref().unwrap().credentials.access_key_id, "ASIA1");
        // Reconnecting stops the old refresher from touching the new state.
        c.connect_with_credentials(S3Credentials { access_key_id: "PLAIN".into(), secret_access_key: "x".into(), region: "eu-west-1".into(), endpoint: None, path_style: false, session_token: None }).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(c.state.read().await.as_ref().unwrap().credentials.access_key_id, "PLAIN");
    }

    #[test]
    fn assume_role_validates_arn() {
        let base = crate::connectors::aws_credentials::BaseKeys { access_key_id: "a".into(), secret_access_key: "b".into(), session_token: Some("t".into()) };
        let bad = crate::core::types::AssumeRoleConfig { role_arn: "Reader".into(), ..Default::default() };
        assert!(crate::connectors::aws_credentials::assume_role_request(&base, &bad, "us-east-1", None, chrono::Utc::now()).is_err());
        let ok = crate::core::types::AssumeRoleConfig { role_arn: "arn:aws:iam::1:role/R".into(), duration_secs: Some(60), ..Default::default() };
        let (url, headers, body) = crate::connectors::aws_credentials::assume_role_request(&base, &ok, "us-east-1", None, chrono::Utc::now()).unwrap();
        assert_eq!(url, "https://sts.us-east-1.amazonaws.com/");
        assert!(body.contains("DurationSeconds=900"), "clamped to the STS minimum");
        assert!(!body.contains("ExternalId"));
        assert_eq!(headers.get("x-amz-security-token").map(String::as_str), Some("t"));
    }

    #[test]
    fn presign_expiry_rules() {
        assert_eq!(presign_expiry(&ShareOptions::default()).unwrap(), 86_400);
        assert_eq!(presign_expiry(&ShareOptions { expires_in_secs: Some(600), ..Default::default() }).unwrap(), 600);
        for bad in [
            ShareOptions { expires_in_secs: Some(PRESIGN_MAX_SECS + 1), ..Default::default() },
            ShareOptions { expires_in_secs: Some(0), ..Default::default() },
            ShareOptions { password: Some("x".into()), ..Default::default() },
            ShareOptions { allow_edit: true, ..Default::default() },
            ShareOptions { scope: ShareScope::Organization, ..Default::default() },
        ] {
            assert!(presign_expiry(&bad).is_err(), "{bad:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_s3_connector_creation() {
        let connector = S3Connector::new();
        assert!(!connector.is_connected());
    }

    #[test]
    fn test_calculate_part_size() {
        // Small file: default part size
        assert_eq!(calculate_part_size(10 * 1024 * 1024), DEFAULT_PART_SIZE);

        // Very large file: part size increases
        let huge = 100 * 1024 * 1024 * 1024; // 100 GB
        let part_size = calculate_part_size(huge);
        assert!(part_size >= DEFAULT_PART_SIZE);
        assert!(huge / part_size <= MAX_PARTS);
    }

    #[test]
    fn test_parse_list_buckets_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <ListAllMyBucketsResult>
            <Buckets>
                <Bucket>
                    <Name>my-bucket</Name>
                    <CreationDate>2024-01-01T00:00:00.000Z</CreationDate>
                </Bucket>
                <Bucket>
                    <Name>other-bucket</Name>
                    <CreationDate>2024-06-15T12:00:00.000Z</CreationDate>
                </Bucket>
            </Buckets>
        </ListAllMyBucketsResult>"#;

        let entries = parse_list_buckets_xml(xml);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "my-bucket");
        assert!(entries[0].is_dir);
        assert_eq!(entries[1].name, "other-bucket");
    }

    #[test]
    fn test_parse_list_objects_xml() {
        let xml = r#"<?xml version="1.0" encoding="UTF-8"?>
        <ListBucketResult>
            <CommonPrefixes>
                <Prefix>photos/</Prefix>
            </CommonPrefixes>
            <Contents>
                <Key>readme.txt</Key>
                <Size>1024</Size>
                <LastModified>2024-03-15T10:00:00.000Z</LastModified>
            </Contents>
            <Contents>
                <Key>data.csv</Key>
                <Size>2048</Size>
                <LastModified>2024-03-14T10:00:00.000Z</LastModified>
            </Contents>
        </ListBucketResult>"#;

        let entries = parse_list_objects_xml(xml, "");
        assert_eq!(entries.len(), 3);

        assert_eq!(entries[0].name, "photos");
        assert!(entries[0].is_dir);

        assert_eq!(entries[1].name, "readme.txt");
        assert!(!entries[1].is_dir);
        assert_eq!(entries[1].size, 1024);

        assert_eq!(entries[2].name, "data.csv");
        assert_eq!(entries[2].size, 2048);
    }

    #[test]
    fn test_parse_list_objects_with_prefix() {
        let xml = r#"<ListBucketResult>
            <Contents>
                <Key>photos/vacation/beach.jpg</Key>
                <Size>5000</Size>
            </Contents>
        </ListBucketResult>"#;

        let entries = parse_list_objects_xml(xml, "photos/vacation/");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "beach.jpg");
    }

    #[test]
    fn test_parse_upload_id() {
        let xml = r#"<InitiateMultipartUploadResult>
            <Bucket>my-bucket</Bucket>
            <Key>test.zip</Key>
            <UploadId>VXBsb2FkIElEIGZvciBlbHZpbmcncyBteS1tb3ZpZS5tMnRzIHVwbG9hZA</UploadId>
        </InitiateMultipartUploadResult>"#;

        let id = parse_upload_id_xml(xml).unwrap();
        assert_eq!(id, "VXBsb2FkIElEIGZvciBlbHZpbmcncyBteS1tb3ZpZS5tMnRzIHVwbG9hZA");
    }

    #[test]
    fn test_parse_list_parts() {
        let xml = r#"<ListPartsResult>
            <Part>
                <PartNumber>1</PartNumber>
                <ETag>"abc123"</ETag>
                <Size>8388608</Size>
            </Part>
            <Part>
                <PartNumber>2</PartNumber>
                <ETag>"def456"</ETag>
                <Size>4194304</Size>
            </Part>
        </ListPartsResult>"#;

        let parts = parse_list_parts_xml(xml);
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].part_number, 1);
        assert_eq!(parts[0].etag, "abc123");
        assert_eq!(parts[0].size, 8388608);
        assert_eq!(parts[1].part_number, 2);
    }

    #[test]
    fn test_presigned_url_generation() {
        let connector = S3Connector::new();
        let creds = S3Credentials {
            access_key_id: "AKIAIOSFODNN7EXAMPLE".to_string(),
            secret_access_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".to_string(),
            region: "us-east-1".to_string(),
            endpoint: None,
            path_style: false,
            session_token: None,
        };

        let url = connector
            .generate_presigned_url(&creds, "my-bucket", "photos/test.jpg", 3600)
            .unwrap();

        assert!(url.contains("my-bucket"));
        assert!(url.contains("test.jpg"));
        assert!(url.contains("X-Amz-Algorithm=AWS4-HMAC-SHA256"));
        assert!(url.contains("X-Amz-Credential="));
        assert!(url.contains("X-Amz-Signature="));
        assert!(url.contains("X-Amz-Expires=3600"));
    }

    #[test]
    fn test_percent_encode_path() {
        assert_eq!(percent_encode_path("simple/path"), "simple/path");
        assert_eq!(
            percent_encode_path("path with spaces/file name.txt"),
            "path%20with%20spaces/file%20name.txt"
        );
    }

    #[test]
    fn test_s3_compatible_endpoint() {
        let connector = S3Connector::new();
        let creds = S3Credentials {
            access_key_id: "minioadmin".to_string(),
            secret_access_key: "minioadmin".to_string(),
            region: "us-east-1".to_string(),
            endpoint: Some("http://localhost:9000".to_string()),
            path_style: true,
            session_token: None,
        };

        assert_eq!(
            connector.effective_endpoint(&creds),
            "http://localhost:9000"
        );

        let (url, path) = connector.bucket_url(&creds, "test-bucket");
        assert_eq!(url, "http://localhost:9000");
        assert_eq!(path, "/test-bucket/");
    }

    #[test]
    fn test_derive_signing_key() {
        // Just verify it doesn't panic
        let key = derive_signing_key("mysecret", "20240101", "us-east-1", "s3").unwrap();
        assert_eq!(key.len(), 32); // HMAC-SHA256 output
    }

    #[test]
    fn test_multipart_threshold() {
        assert_eq!(MULTIPART_THRESHOLD, 5 * 1024 * 1024);
    }

    // ── ListObjectsV2 pagination (regression: listings stopped at 1,000) ──

    /// Mock S3 that checks every request's SigV4 signature against the
    /// reference signer and serves `pages` ListObjectsV2 pages of keys.
    async fn mock_s3(pages: usize) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use axum::http::{HeaderMap, Uri};
        use std::collections::BTreeMap;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        let app = axum::Router::new().fallback(move |uri: Uri, h: HeaderMap| {
            let log = log.clone();
            async move {
                let query = uri.query().unwrap_or("").to_string();
                log.lock().unwrap().push(query.clone());
                let params: Vec<(String, String)> = url::form_urlencoded::parse(query.as_bytes())
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect();
                let mut signed = BTreeMap::new();
                for k in ["host", "x-amz-content-sha256", "x-amz-date"] {
                    signed.insert(k.to_string(), h[k].to_str().unwrap().to_string());
                }
                let expected = crate::backup_engine::stores::s3::sign_v4(
                    "GET", uri.path(), &params, &signed, "UNSIGNED-PAYLOAD", "AKID", "SECRET", "us-east-1", "s3",
                );
                if h["authorization"].to_str().unwrap() != expected {
                    return (axum::http::StatusCode::FORBIDDEN, "SignatureDoesNotMatch".to_string());
                }
                let page: usize = params
                    .iter()
                    .find(|(k, _)| k == "continuation-token")
                    .map(|(_, v)| v.trim_start_matches("tok&").parse().unwrap())
                    .unwrap_or(0);
                let mut xml = String::from("<ListBucketResult>");
                for i in 0..3 {
                    xml.push_str(&format!("<Contents><Key>my-dir/p{page}-k{i}.txt</Key><Size>1</Size></Contents>"));
                }
                if page + 1 < pages {
                    xml.push_str(&format!(
                        "<IsTruncated>true</IsTruncated><NextContinuationToken>tok&amp;{}</NextContinuationToken>",
                        page + 1
                    ));
                } else {
                    xml.push_str("<IsTruncated>false</IsTruncated>");
                }
                xml.push_str("</ListBucketResult>");
                (axum::http::StatusCode::OK, xml)
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (base, seen)
    }

    async fn connected(endpoint: String) -> S3Connector {
        let c = S3Connector::new();
        c.connect_with_credentials(S3Credentials {
            access_key_id: "AKID".into(),
            secret_access_key: "SECRET".into(),
            region: "us-east-1".into(),
            endpoint: Some(endpoint),
            path_style: true,
            session_token: None,
        })
        .await
        .unwrap();
        c
    }

    #[tokio::test]
    async fn list_objects_follows_continuation_tokens() {
        let (base, seen) = mock_s3(3).await;
        let c = connected(base).await;
        let entries = c.list_objects("bucket", "my-dir/", Some("/")).await.unwrap();
        assert_eq!(entries.len(), 9, "all three pages must be returned");
        let queries = seen.lock().unwrap().clone();
        assert_eq!(queries.len(), 3);
        assert!(!queries[0].contains("continuation-token"));
        // XML-escaped token is decoded, then re-encoded for the next request.
        assert!(queries[1].contains("continuation-token=tok%261"), "{}", queries[1]);
        assert!(queries[2].contains("continuation-token=tok%262"));
    }

    /// Regression: the query was unsorted and over-encoded (`-` as `%2D`),
    /// so any listing with a delimiter or a dashed prefix failed SigV4.
    #[tokio::test]
    async fn list_objects_signs_a_canonical_query() {
        let (base, seen) = mock_s3(1).await;
        let c = connected(base).await;
        c.list_objects("bucket", "my-dir/", Some("/")).await.unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            "delimiter=%2F&list-type=2&max-keys=1000&prefix=my-dir%2F"
        );
    }

    #[test]
    fn canonical_query_sorts_and_encodes_like_sigv4() {
        assert_eq!(
            canonical_query(&[("prefix", "a b/c-d_e.f~g".into()), ("versions", String::new())]),
            "prefix=a%20b%2Fc-d_e.f~g&versions="
        );
    }

    #[test]
    fn xml_values_are_unescaped() {
        assert_eq!(extract_xml_value("<Key>a&amp;b&lt;c</Key>", "Key").as_deref(), Some("a&b<c"));
        assert_eq!(extract_xml_value("<ETag>&quot;x&quot;</ETag>", "ETag").as_deref(), Some("\"x\""));
    }

    /// Regression: the URL encoded `/` in nested keys while the signed path
    /// used the raw key, so restoring `dir/my file.txt` failed SigV4.
    #[tokio::test]
    async fn restore_version_signs_nested_keys() {
        use axum::http::{HeaderMap, Method, Uri};
        use std::collections::BTreeMap;
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        let app = axum::Router::new().fallback(move |m: Method, uri: Uri, h: HeaderMap| {
            let log = log.clone();
            async move {
                let mut signed = BTreeMap::new();
                for k in ["host", "x-amz-content-sha256", "x-amz-date"] {
                    signed.insert(k.to_string(), h[k].to_str().unwrap().to_string());
                }
                let expected = crate::backup_engine::stores::s3::sign_v4(
                    m.as_str(), uri.path(), &[], &signed, "UNSIGNED-PAYLOAD", "AKID", "SECRET", "us-east-1", "s3",
                );
                log.lock().unwrap().push(format!("{} {}", uri.path(), h["x-amz-copy-source"].to_str().unwrap()));
                // What S3 answers a CopyObject (an empty 200 is no copy).
                if h["authorization"].to_str().unwrap() == expected {
                    (axum::http::StatusCode::OK, COPY_RESULT)
                } else {
                    (axum::http::StatusCode::FORBIDDEN, "")
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let c = connected(base).await;
        c.restore_object_version("bucket", "dir/my file.txt", "v1").await.unwrap();
        assert_eq!(
            seen.lock().unwrap()[0],
            "/bucket/dir/my%20file.txt /bucket/dir/my%20file.txt?versionId=v1"
        );
    }

    /// Pane copies inside one S3 connection run as CopyObject (signed like
    /// the other requests); a rename is that copy, then a delete.
    #[tokio::test]
    async fn server_copy_and_rename_use_copy_object() {
        use axum::http::{HeaderMap, Method, Uri};
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        let app = axum::Router::new().fallback(move |m: Method, uri: Uri, h: HeaderMap| {
            let log = log.clone();
            async move {
                let src = h.get("x-amz-copy-source").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default();
                log.lock().unwrap().push(format!("{m} {} {src}", uri.path()));
                (axum::http::StatusCode::OK, if src.is_empty() { "" } else { COPY_RESULT })
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let c = connected(base).await;
        assert!(c.server_copy("/bucket/in/a b.txt", "/other/out/a b.txt").await.unwrap());
        c.rename_entry("/bucket/in/x.txt", "/bucket/in/y.txt").await.unwrap();
        assert!(c.rename_entry("/bucket/in/", "/bucket/out/").await.is_err());
        let log = seen.lock().unwrap().clone();
        assert_eq!(log[0], "PUT /other/out/a%20b.txt /bucket/in/a%20b.txt");
        assert_eq!(log[1], "PUT /bucket/in/y.txt /bucket/in/x.txt");
        assert!(log[2].starts_with("HEAD /bucket/in/y.txt"), "the copy is confirmed first: {log:?}");
        assert!(log[3].starts_with("DELETE /bucket/in/x.txt"));
    }

    /// A CopyObject reply S3 sends on success.
    const COPY_RESULT: &str = r#"<?xml version="1.0" encoding="UTF-8"?><CopyObjectResult><LastModified>2026-01-01T00:00:00.000Z</LastModified><ETag>"e"</ETag></CopyObjectResult>"#;

    /// Regression (data loss): S3 can fail a CopyObject after answering
    /// 200, with an `<Error>` body. The copy was taken as done, so a rename
    /// (copy, then delete) deleted the only copy. Now the reply must report
    /// the copy, and a rename confirms the copy before deleting the source.
    #[tokio::test]
    async fn a_copy_failed_inside_a_200_reply_never_deletes_the_source() {
        use axum::http::{HeaderMap, Method, Uri};
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let log = seen.clone();
        let app = axum::Router::new().fallback(move |m: Method, uri: Uri, h: HeaderMap| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(format!("{m} {}", uri.path()));
                match (m.as_str(), h.contains_key("x-amz-copy-source"), uri.path()) {
                    ("PUT", true, p) if p.ends_with("err.txt") => (axum::http::StatusCode::OK, r#"<?xml version="1.0" encoding="UTF-8"?><Error><Code>InternalError</Code><Message>We encountered an internal error. Please try again.</Message></Error>"#),
                    ("PUT", true, p) if p.ends_with("empty.txt") => (axum::http::StatusCode::OK, ""),
                    ("PUT", true, _) => (axum::http::StatusCode::OK, COPY_RESULT),
                    // The copy is not there after all.
                    ("HEAD", _, _) => (axum::http::StatusCode::NOT_FOUND, ""),
                    _ => (axum::http::StatusCode::OK, ""),
                }
            }
        });
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let c = connected(base).await;
        let err = c.rename_entry("/bucket/in/x.txt", "/bucket/in/err.txt").await.unwrap_err();
        assert!(err.to_string().contains("InternalError"), "{err}");
        assert!(c.server_copy("/bucket/in/x.txt", "/bucket/in/err.txt").await.is_err());
        assert!(c.rename_entry("/bucket/in/x.txt", "/bucket/in/empty.txt").await.is_err(), "no CopyObjectResult, no copy");
        assert!(c.rename_entry("/bucket/in/x.txt", "/bucket/in/ok.txt").await.is_err(), "a copy HEAD cannot find is not confirmed");
        let log = seen.lock().unwrap().clone();
        assert!(!log.iter().any(|l| l.starts_with("DELETE")), "the source was never deleted: {log:?}");
    }
}
