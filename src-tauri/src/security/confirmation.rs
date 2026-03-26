//! Confirmation token system for destructive operations.
//!
//! Generates single-use, time-limited tokens that must be presented when
//! executing destructive commands (e.g., reset_database, permanent delete).
//! This prevents accidental or XSS-driven destructive operations.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use uuid::Uuid;

/// A pending confirmation request.
#[derive(Debug, Clone)]
struct PendingConfirmation {
    operation: String,
    #[allow(dead_code)]
    details: String, // Stored for future audit logging
    created_at: Instant,
    ttl: Duration,
}

/// Manages confirmation tokens for destructive operations.
#[derive(Debug, Clone)]
pub struct ConfirmationManager {
    pending: Arc<RwLock<HashMap<String, PendingConfirmation>>>,
}

impl ConfirmationManager {
    pub fn new() -> Self {
        Self {
            pending: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Request a confirmation token for a destructive operation.
    /// Returns a UUID token that must be presented within 30 seconds.
    pub async fn request_confirmation(&self, operation: String, details: String) -> String {
        let token = Uuid::new_v4().to_string();
        let confirmation = PendingConfirmation {
            operation,
            details,
            created_at: Instant::now(),
            ttl: Duration::from_secs(30),
        };

        let mut pending = self.pending.write().await;
        // Clean up expired tokens while we're here
        pending.retain(|_, v| v.created_at.elapsed() < v.ttl);
        pending.insert(token.clone(), confirmation);
        token
    }

    /// Validate and consume a confirmation token (single-use).
    /// Returns the operation name if valid, or None if invalid/expired.
    pub async fn validate_token(&self, token: &str) -> Option<String> {
        let mut pending = self.pending.write().await;
        if let Some(confirmation) = pending.remove(token) {
            if confirmation.created_at.elapsed() < confirmation.ttl {
                return Some(confirmation.operation);
            }
        }
        None
    }
}

impl Default for ConfirmationManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_request_and_validate() {
        let mgr = ConfirmationManager::new();
        let token = mgr
            .request_confirmation("reset_database".to_string(), "Wipe all data".to_string())
            .await;

        // Token should be valid
        let op = mgr.validate_token(&token).await;
        assert_eq!(op, Some("reset_database".to_string()));

        // Token is single-use — second validation should fail
        let op2 = mgr.validate_token(&token).await;
        assert_eq!(op2, None);
    }

    #[tokio::test]
    async fn test_invalid_token() {
        let mgr = ConfirmationManager::new();
        let op = mgr.validate_token("nonexistent-token").await;
        assert_eq!(op, None);
    }
}
