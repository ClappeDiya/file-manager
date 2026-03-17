//! Tauri IPC commands for AWS credential chain, SSO, and STS federation.
//!
//! Provides frontend-accessible commands for:
//! - Listing all AWS profiles from `~/.aws/`
//! - Resolving a specific profile (merged credentials + config)
//! - Starting IAM Identity Center SSO device auth flow
//! - Polling for SSO token completion
//! - Getting SSO role credentials
//! - STS AssumeRoleWithWebIdentity (OIDC federation)

use crate::connectors::aws_credentials::{
    self, AwsCredentials, AwsProfile, SsoAuthStart, SsoTokenResponse,
};
use crate::core::error::AppError;

/// List all AWS profiles found in `~/.aws/credentials` and `~/.aws/config`.
///
/// Returns resolved profiles (merged from both files) sorted by name.
#[tauri::command]
pub async fn list_aws_profiles() -> Result<Vec<AwsProfile>, AppError> {
    let names = aws_credentials::list_all_profile_names();
    let profiles: Vec<AwsProfile> = names
        .into_iter()
        .filter_map(|name| aws_credentials::resolve_profile(&name))
        .collect();
    Ok(profiles)
}

/// Resolve a specific AWS profile by name, merging credentials and config.
///
/// Returns the merged profile or an error if the profile is not found.
#[tauri::command]
pub async fn resolve_aws_profile(profile_name: String) -> Result<AwsProfile, AppError> {
    aws_credentials::resolve_profile(&profile_name).ok_or_else(|| {
        AppError::connection(
            format!("AWS profile '{profile_name}' not found"),
            "Check that the profile exists in ~/.aws/credentials or ~/.aws/config.",
        )
    })
}

/// Start the IAM Identity Center (SSO) device authorization flow.
///
/// Returns a verification URI and user code that the user must open in a
/// browser to complete authentication, plus a device code for polling.
///
/// The `client_id` and `client_secret` in the response must be passed to
/// `poll_sso_token` along with the `device_code`.
#[tauri::command]
pub async fn start_sso_auth(
    start_url: String,
    region: String,
) -> Result<SsoAuthStart, AppError> {
    aws_credentials::start_sso_auth(&start_url, &region).await
}

/// Poll the SSO OIDC service to check if the user has completed browser auth.
///
/// Call this periodically (every 5 seconds) after `start_sso_auth`.
/// - Returns `Ok(SsoTokenResponse)` when auth is complete.
/// - Returns `Err` with message "authorization_pending" if still waiting.
/// - Returns `Err` with message "slow_down" if polling too fast.
/// - Returns `Err` with message about expiration if the device code expired.
#[tauri::command]
pub async fn poll_sso_token(
    device_code: String,
    client_id: String,
    client_secret: String,
    region: String,
) -> Result<SsoTokenResponse, AppError> {
    aws_credentials::poll_sso_token(&device_code, &client_id, &client_secret, &region).await
}

/// Get temporary AWS credentials for an SSO role using an access token.
///
/// After obtaining an SSO access token via `poll_sso_token`, use this to
/// fetch temporary IAM credentials for a specific account and role.
#[tauri::command]
pub async fn get_sso_role_credentials(
    access_token: String,
    account_id: String,
    role_name: String,
    region: String,
) -> Result<AwsCredentials, AppError> {
    aws_credentials::get_sso_role_credentials(&access_token, &account_id, &role_name, &region)
        .await
}

/// Assume an IAM role using a web identity (OIDC) token via STS.
///
/// Exchanges an OIDC token (from providers like GitHub Actions, Cognito,
/// Google, etc.) for temporary AWS credentials.
///
/// # Parameters
/// - `role_arn`: The ARN of the IAM role to assume (e.g. `arn:aws:iam::123456789012:role/MyRole`)
/// - `web_identity_token`: The JWT/OIDC token from the identity provider
/// - `region`: AWS region for the STS endpoint
/// - `session_name`: Optional session name (defaults to "ufop-session")
#[tauri::command]
pub async fn assume_role_with_web_identity(
    role_arn: String,
    web_identity_token: String,
    region: String,
    session_name: Option<String>,
) -> Result<AwsCredentials, AppError> {
    aws_credentials::assume_role_with_web_identity(
        &role_arn,
        &web_identity_token,
        &region,
        session_name.as_deref(),
    )
    .await
}
