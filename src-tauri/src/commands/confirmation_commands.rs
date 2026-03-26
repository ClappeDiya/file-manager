//! IPC commands for the confirmation token system.
//!
//! Frontend must call `request_destructive_confirmation` before executing
//! destructive operations, then pass the returned token to the command.

use crate::core::error::AppError;
use crate::security::confirmation::ConfirmationManager;
use serde::{Deserialize, Serialize};
use tauri::State;

/// Token returned by the confirmation request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfirmationToken {
    pub token: String,
    pub operation: String,
    pub expires_in_seconds: u64,
}

/// Request a confirmation token for a destructive operation.
/// The token is valid for 30 seconds and single-use.
#[tauri::command]
pub async fn request_destructive_confirmation(
    operation: String,
    details: String,
    confirmation_mgr: State<'_, ConfirmationManager>,
) -> Result<ConfirmationToken, AppError> {
    let token = confirmation_mgr
        .request_confirmation(operation.clone(), details)
        .await;

    Ok(ConfirmationToken {
        token,
        operation,
        expires_in_seconds: 30,
    })
}

/// Validate a confirmation token. Used internally by destructive commands.
/// Not exposed as an IPC command — commands validate tokens directly.
pub async fn validate_confirmation(
    token: &str,
    expected_operation: &str,
    confirmation_mgr: &ConfirmationManager,
) -> Result<(), AppError> {
    match confirmation_mgr.validate_token(token).await {
        Some(op) if op == expected_operation => Ok(()),
        Some(_) => Err(AppError::file_op(
            "Confirmation token was issued for a different operation",
            "Request a new confirmation token for this operation.",
        )),
        None => Err(AppError::file_op(
            "Invalid or expired confirmation token",
            "Request a new confirmation token. Tokens expire after 30 seconds.",
        )),
    }
}
