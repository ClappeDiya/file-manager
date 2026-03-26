//! Security audit logging for sensitive operations.
//!
//! Logs security-relevant events to the audit_events table via Repository.
//! Events include: authentication attempts, destructive operations,
//! configuration changes, and credential access.

use crate::core::types::AuditEvent;
use crate::storage::Repository;
use chrono::Utc;
use uuid::Uuid;

/// Security event types for audit logging.
pub struct SecurityAudit;

impl SecurityAudit {
    /// Log a security-relevant event. Non-fatal — failures are logged to tracing only.
    pub async fn log(
        repo: &Repository,
        event_type: &str,
        actor: &str,
        resource: &str,
        details: &str,
    ) {
        let event = AuditEvent {
            id: Uuid::new_v4(),
            timestamp: Utc::now(),
            event_type: format!("security.{event_type}"),
            actor: actor.to_string(),
            resource: resource.to_string(),
            details: details.to_string(),
        };

        if let Err(e) = repo.log_audit_event(event).await {
            tracing::warn!("Failed to log security audit event: {e}");
        }
    }

    /// Log a master password verification attempt.
    pub async fn log_auth_attempt(repo: &Repository, success: bool) {
        let details = if success { "successful" } else { "failed" };
        Self::log(repo, "auth.master_password", "user", "master_password", details).await;
    }

    /// Log a master password change.
    pub async fn log_password_change(repo: &Repository) {
        Self::log(repo, "auth.password_changed", "user", "master_password", "Master password changed").await;
    }

    /// Log app lock/unlock events.
    pub async fn log_lock_event(repo: &Repository, locked: bool) {
        let event_type = if locked { "auth.app_locked" } else { "auth.app_unlocked" };
        Self::log(repo, event_type, "user", "app_session", "").await;
    }

    /// Log a destructive operation (reset, permanent delete).
    pub async fn log_destructive_op(repo: &Repository, operation: &str, details: &str) {
        Self::log(repo, "destructive_operation", "user", operation, details).await;
    }

    /// Log credential access (encrypt/decrypt).
    pub async fn log_credential_access(repo: &Repository, operation: &str) {
        Self::log(repo, "credential.access", "user", operation, "").await;
    }

    /// Log a connection saved/deleted.
    pub async fn log_connection_change(repo: &Repository, action: &str, connection_name: &str) {
        Self::log(repo, "connection.change", "user", action, connection_name).await;
    }

    /// Log rate-limit trigger.
    pub async fn log_rate_limit(repo: &Repository, remaining_lockout_secs: u64) {
        Self::log(
            repo,
            "auth.rate_limited",
            "user",
            "master_password",
            &format!("Rate limited for {remaining_lockout_secs}s"),
        ).await;
    }
}
