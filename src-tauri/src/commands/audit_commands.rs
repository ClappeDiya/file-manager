//! IPC commands for the tamper-evident security audit log (#357).
//!
//! The log itself is written by `security::audit::SecurityAudit`; these
//! commands only read it, checkpoint it and export it. There is
//! intentionally no command that edits, prunes or deletes audit entries.

use crate::core::error::AppError;
use crate::security::audit::{AuditLogStatus, AuditTrail};
use crate::security::audit_chain::{AuditAnchor, AuditVerification};
use tauri::State;

/// Recompute the hash chain, check for sequence gaps and verify every
/// signed anchor.
#[tauri::command]
pub async fn verify_audit_log(audit: State<'_, AuditTrail>) -> Result<AuditVerification, AppError> {
    audit.verify().await
}

/// Export the whole log (header with public key, entries with hashes,
/// anchors) as JSON lines for verification by an external party.
#[tauri::command]
pub async fn export_audit_log(audit: State<'_, AuditTrail>) -> Result<String, AppError> {
    audit.export_jsonl().await
}

/// Sign a checkpoint over the current end of the log.
#[tauri::command]
pub async fn create_audit_anchor(audit: State<'_, AuditTrail>) -> Result<AuditAnchor, AppError> {
    audit.create_anchor().await
}

/// Entry/anchor counts, signing key fingerprint and trigger status.
#[tauri::command]
pub async fn get_audit_log_status(audit: State<'_, AuditTrail>) -> Result<AuditLogStatus, AppError> {
    audit.status().await
}
