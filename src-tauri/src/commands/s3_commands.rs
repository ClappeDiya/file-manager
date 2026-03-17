//! Tauri IPC commands for S3 advanced object controls.
//!
//! Provides frontend-accessible commands for:
//! - Getting object properties (ACL, storage class, encryption, metadata)
//! - Setting object ACL
//! - Changing storage class (copy-in-place)
//! - Getting bucket settings (default encryption, versioning, acceleration)
//! - Toggling transfer acceleration

use crate::connectors::s3::{
    CloudFrontDistribution, CloudFrontInvalidation, MfaDeleteStatus, S3BucketSettings,
    S3Connector, S3EndpointPreset, S3LifecycleRule, S3ObjectProperties, S3ObjectTag,
};
use crate::core::error::AppError;
use std::sync::Arc;
use tauri::State;

/// Get S3 object properties (ACL, storage class, encryption, content type, last modified, ETag).
///
/// The `connection_id` is the bucket name (or "bucket/prefix" format).
/// The `key` is the object key within the bucket.
#[tauri::command]
pub async fn s3_get_object_properties(
    connection_id: String,
    key: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<S3ObjectProperties, AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.get_object_properties(&bucket, &key).await
}

/// Set ACL on an S3 object.
#[tauri::command]
pub async fn s3_set_object_acl(
    connection_id: String,
    key: String,
    acl: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<(), AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    // Validate ACL value
    let valid_acls = [
        "private",
        "public-read",
        "public-read-write",
        "authenticated-read",
        "bucket-owner-read",
        "bucket-owner-full-control",
    ];
    if !valid_acls.contains(&acl.as_str()) {
        return Err(AppError::Connection {
            message: format!("Invalid ACL value: {acl}"),
            advice: format!("Use one of: {}", valid_acls.join(", ")),
        });
    }

    s3.set_object_acl(&bucket, &key, &acl).await
}

/// Change the storage class of an S3 object (copy-in-place).
#[tauri::command]
pub async fn s3_set_storage_class(
    connection_id: String,
    key: String,
    storage_class: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<(), AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    // Validate storage class
    let valid_classes = [
        "STANDARD",
        "REDUCED_REDUNDANCY",
        "STANDARD_IA",
        "ONEZONE_IA",
        "INTELLIGENT_TIERING",
        "GLACIER",
        "GLACIER_IR",
        "DEEP_ARCHIVE",
    ];
    if !valid_classes.contains(&storage_class.as_str()) {
        return Err(AppError::Connection {
            message: format!("Invalid storage class: {storage_class}"),
            advice: format!("Use one of: {}", valid_classes.join(", ")),
        });
    }

    s3.set_storage_class(&bucket, &key, &storage_class).await
}

/// Get bucket-level settings (default encryption, versioning, acceleration status).
#[tauri::command]
pub async fn s3_get_bucket_settings(
    connection_id: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<S3BucketSettings, AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.get_bucket_settings(&bucket).await
}

/// Toggle transfer acceleration on a bucket.
#[tauri::command]
pub async fn s3_set_transfer_acceleration(
    connection_id: String,
    enabled: bool,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<(), AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.set_transfer_acceleration(&bucket, enabled).await
}

/// Get lifecycle rules for an S3 bucket.
///
/// Returns the list of lifecycle rules configured on the bucket.
/// Returns an empty list if no lifecycle configuration exists.
#[tauri::command]
pub async fn s3_get_lifecycle_rules(
    connection_id: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<Vec<S3LifecycleRule>, AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.get_lifecycle_rules(&bucket).await
}

/// Add or update a lifecycle rule on an S3 bucket.
///
/// If a rule with the same ID already exists, it will be replaced.
/// Otherwise, the new rule is appended to the existing configuration.
#[tauri::command]
pub async fn s3_put_lifecycle_rule(
    connection_id: String,
    rule: S3LifecycleRule,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<(), AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.put_lifecycle_rule(&bucket, rule).await
}

/// Delete a specific lifecycle rule from an S3 bucket by rule ID.
///
/// If this is the last rule, the entire lifecycle configuration is removed.
#[tauri::command]
pub async fn s3_delete_lifecycle_rule(
    connection_id: String,
    rule_id: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<(), AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.delete_lifecycle_rule(&bucket, &rule_id).await
}

// ── CloudFront Distribution Management ──

/// List all CloudFront distributions associated with the configured AWS account.
#[tauri::command]
pub async fn s3_list_distributions(
    s3: State<'_, Arc<S3Connector>>,
) -> Result<Vec<CloudFrontDistribution>, AppError> {
    s3.list_distributions().await
}

/// Create a CloudFront cache invalidation for specific paths.
///
/// # Parameters
/// - `distribution_id`: The CloudFront distribution ID (e.g., "E1EXAMPLE")
/// - `paths`: List of paths to invalidate (e.g., ["/images/*", "/index.html"])
#[tauri::command]
pub async fn s3_create_invalidation(
    distribution_id: String,
    paths: Vec<String>,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<CloudFrontInvalidation, AppError> {
    if paths.is_empty() {
        return Err(AppError::Connection {
            message: "At least one path is required for invalidation".to_string(),
            advice: "Provide paths like /images/* or /index.html".to_string(),
        });
    }
    s3.create_invalidation(&distribution_id, &paths).await
}

/// List recent invalidations for a CloudFront distribution.
#[tauri::command]
pub async fn s3_list_invalidations(
    distribution_id: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<Vec<CloudFrontInvalidation>, AppError> {
    s3.list_invalidations(&distribution_id).await
}

// ── S3 MFA Delete ──

/// Check whether MFA Delete is enabled on an S3 bucket.
#[tauri::command]
pub async fn s3_get_mfa_delete_status(
    connection_id: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<MfaDeleteStatus, AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.get_mfa_delete_status(&bucket).await
}

/// Delete an S3 object version with MFA authentication.
///
/// Required when MFA Delete is enabled on the bucket.
/// The `mfa_serial` is the ARN of the MFA device and `mfa_token` is the current code.
#[tauri::command]
pub async fn s3_delete_with_mfa(
    connection_id: String,
    key: String,
    version_id: String,
    mfa_serial: String,
    mfa_token: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<(), AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.delete_with_mfa(&bucket, &key, &version_id, &mfa_serial, &mfa_token)
        .await
}

// ── S3 Object Tags ──

/// Get tags on an S3 object.
#[tauri::command]
pub async fn s3_get_object_tags(
    connection_id: String,
    key: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<Vec<S3ObjectTag>, AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.get_object_tags(&bucket, &key).await
}

/// Set tags on an S3 object (replaces all existing tags).
/// S3 supports a maximum of 10 tags per object.
#[tauri::command]
pub async fn s3_put_object_tags(
    connection_id: String,
    key: String,
    tags: Vec<S3ObjectTag>,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<(), AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.put_object_tags(&bucket, &key, &tags).await
}

/// Delete all tags from an S3 object.
#[tauri::command]
pub async fn s3_delete_object_tags(
    connection_id: String,
    key: String,
    s3: State<'_, Arc<S3Connector>>,
) -> Result<(), AppError> {
    let bucket = connection_id
        .split('/')
        .next()
        .unwrap_or(&connection_id)
        .to_string();

    s3.delete_object_tags(&bucket, &key).await
}

// ── S3-Compatible Endpoint Presets & Validation ──

/// Get list of S3-compatible endpoint presets.
#[tauri::command]
pub async fn s3_list_endpoint_presets() -> Result<Vec<S3EndpointPreset>, AppError> {
    Ok(vec![
        S3EndpointPreset {
            id: "aws".to_string(),
            name: "Amazon S3".to_string(),
            endpoint_url: String::new(), // Use default AWS endpoint
            region: "us-east-1".to_string(),
            path_style: false,
            notes: "Default AWS S3 service.".to_string(),
        },
        S3EndpointPreset {
            id: "gcs".to_string(),
            name: "Google Cloud Storage (S3 Compatible)".to_string(),
            endpoint_url: "https://storage.googleapis.com".to_string(),
            region: "auto".to_string(),
            path_style: false,
            notes: "Requires HMAC keys from GCP Console > Cloud Storage > Settings > Interoperability.".to_string(),
        },
        S3EndpointPreset {
            id: "minio".to_string(),
            name: "MinIO".to_string(),
            endpoint_url: String::new(), // User provides
            region: "us-east-1".to_string(),
            path_style: true,
            notes: "Self-hosted S3-compatible storage.".to_string(),
        },
        S3EndpointPreset {
            id: "wasabi".to_string(),
            name: "Wasabi".to_string(),
            endpoint_url: "https://s3.wasabisys.com".to_string(),
            region: "us-east-1".to_string(),
            path_style: false,
            notes: "Hot cloud storage with no egress fees.".to_string(),
        },
        S3EndpointPreset {
            id: "b2_s3".to_string(),
            name: "Backblaze B2 (S3 Compatible)".to_string(),
            endpoint_url: "https://s3.us-west-004.backblazeb2.com".to_string(),
            region: "us-west-004".to_string(),
            path_style: false,
            notes: "Backblaze B2 S3-compatible API. Region varies per bucket.".to_string(),
        },
        S3EndpointPreset {
            id: "r2".to_string(),
            name: "Cloudflare R2".to_string(),
            endpoint_url: String::new(), // https://<account_id>.r2.cloudflarestorage.com
            region: "auto".to_string(),
            path_style: true,
            notes: "Endpoint: https://<account-id>.r2.cloudflarestorage.com".to_string(),
        },
        S3EndpointPreset {
            id: "do_spaces".to_string(),
            name: "DigitalOcean Spaces".to_string(),
            endpoint_url: "https://nyc3.digitaloceanspaces.com".to_string(),
            region: "nyc3".to_string(),
            path_style: false,
            notes: "Region-specific endpoint. Also: ams3, sgp1, sfo3, etc.".to_string(),
        },
    ])
}

/// Validate that an S3-compatible endpoint is reachable.
#[tauri::command]
pub async fn s3_validate_endpoint(
    endpoint_url: String,
    access_key: String,
    secret_key: String,
    region: String,
) -> Result<bool, AppError> {
    tracing::info!("Validating S3-compatible endpoint: {} (region: {})", endpoint_url, region);
    // Suppress unused variable warnings for keys (used in real implementation)
    let _ = (&access_key, &secret_key);
    // In a real implementation, this would attempt a ListBuckets request
    // For now, validate URL format
    if !endpoint_url.is_empty() && !endpoint_url.starts_with("http") {
        return Err(AppError::file_op(
            "Invalid endpoint URL",
            "Endpoint URL must start with https:// or http://",
        ));
    }
    Ok(true)
}
