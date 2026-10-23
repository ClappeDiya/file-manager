//! Security audit logging for sensitive operations.
//!
//! Logs security-relevant events to the `audit_events` table via
//! Repository. Events include: authentication attempts, destructive
//! operations, configuration changes, and credential access.
//!
//! The table is a tamper-evident, append-only hash chain with signed
//! checkpoints ("anchors") — see [`super::audit_chain`] for the format.
//! [`AuditTrail`] owns the anchor signing key and exposes verification,
//! anchoring and export.
//!
//! # Signing key
//!
//! The Ed25519 anchor key is resolved once, in this order:
//! 1. `UFOP_AUDIT_SIGNING_KEY` — a 64-hex-character seed (managed deployments);
//! 2. the OS keychain (`ufop-audit-signing-key`), created on first use; a key
//!    found in the settings store is moved there when a keychain appears;
//! 3. the settings store secret `security.audit.signing_key` (encrypted with
//!    the settings key when one exists);
//! 4. an ephemeral in-memory key (in-memory database fallback only).
//!
//! Losing the key does not lose the log: the hash chain still verifies, but
//! existing anchors report "signed with key X, not the trusted audit key Y".

use super::audit_chain::{self, AnchorVerifier, AuditAnchor, AuditSigner, AuditVerification};
use super::settings_store::SettingsStore;
use super::CredentialStore;
use crate::core::error::AppError;
use crate::core::types::AuditEvent;
use crate::storage::pool::DbPool;
use crate::storage::Repository;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Env var holding the anchor signing seed (64 hex chars).
pub const SIGNING_KEY_ENV: &str = "UFOP_AUDIT_SIGNING_KEY";
const KEYCHAIN_KEY: &str = "ufop-audit-signing-key";
/// Settings-store key for the fallback signing seed. Kept across a
/// database reset (see `migrations::reset_database`).
pub const SETTINGS_KEY: &str = "security.audit.signing_key";
/// How often the desktop app checkpoints new entries.
pub const ANCHOR_INTERVAL: Duration = Duration::from_secs(60 * 60);

/// Where the anchor signing key lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySource {
    Environment,
    OsKeychain,
    SettingsStoreEncrypted,
    SettingsStorePlain,
    Ephemeral,
}

struct ResolvedKey {
    signer: AuditSigner,
    source: KeySource,
}

/// Summary for the settings UI (`AuditLogStatus` in `src/lib/audit-log.ts`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditLogStatus {
    pub entries: i64,
    pub last_sequence: Option<i64>,
    pub last_hash: Option<String>,
    pub anchors: i64,
    pub last_anchor: Option<AuditAnchor>,
    pub unanchored_entries: i64,
    pub append_only_enforced: bool,
    pub key_id: Option<String>,
    pub public_key: Option<String>,
    pub key_source: Option<KeySource>,
}

/// Verification, anchoring and export over the tamper-evident audit log.
#[derive(Clone)]
pub struct AuditTrail {
    pool: DbPool,
    creds: Option<Arc<CredentialStore>>,
    settings: Option<SettingsStore>,
    key: Arc<tokio::sync::OnceCell<Result<ResolvedKey, String>>>,
}

fn signer_from_hex(hexed: &str) -> Result<AuditSigner, AppError> {
    let seed = Zeroizing::new(hex::decode(hexed.trim()).map_err(|_| AppError::Encryption {
        message: "The audit signing key is not valid hex.".into(),
        advice: format!("Set {SIGNING_KEY_ENV} to the original 64-hex-character seed."),
    })?);
    if seed.len() != 32 {
        return Err(AppError::Encryption {
            message: "The audit signing key must be 32 bytes (64 hex characters).".into(),
            advice: format!("Check {SIGNING_KEY_ENV} or the stored audit key."),
        });
    }
    AuditSigner::from_seed(&seed)
}

fn fresh_seed_hex() -> Zeroizing<String> {
    Zeroizing::new(hex::encode(AuditSigner::generate_seed().as_ref()))
}

impl AuditTrail {
    pub fn new(pool: DbPool, creds: Option<Arc<CredentialStore>>, settings: Option<SettingsStore>) -> Self {
        Self { pool, creds, settings, key: Arc::new(tokio::sync::OnceCell::new()) }
    }

    /// Trail with a fixed signing seed (tests, tooling).
    pub fn with_seed(pool: DbPool, seed: &[u8]) -> Result<Self, AppError> {
        let signer = AuditSigner::from_seed(seed)?;
        let cell = tokio::sync::OnceCell::new_with(Some(Ok(ResolvedKey { signer, source: KeySource::Environment })));
        Ok(Self { pool, creds: None, settings: None, key: Arc::new(cell) })
    }

    async fn resolved(&self) -> Result<&ResolvedKey, AppError> {
        let r = self
            .key
            .get_or_init(|| async { self.resolve_key().await.map_err(|e| e.user_message()) })
            .await;
        r.as_ref().map_err(|msg| AppError::Encryption {
            message: format!("The audit signing key is unavailable: {msg}"),
            advice: format!("Restore the key, or set {SIGNING_KEY_ENV} to the original 64-hex-character seed."),
        })
    }

    async fn resolve_key(&self) -> Result<ResolvedKey, AppError> {
        let ok = |signer, source| Ok(ResolvedKey { signer, source });

        if let Some(k) = std::env::var(SIGNING_KEY_ENV).ok().filter(|k| !k.trim().is_empty()) {
            return ok(signer_from_hex(&k)?, KeySource::Environment);
        }
        let settings_seed = match &self.settings {
            Some(s) => s.get_secret(SETTINGS_KEY).await?.map(Zeroizing::new),
            None => None,
        };
        if let Some(creds) = self.creds.as_ref().filter(|c| c.is_keychain_available()) {
            if let Some(k) = creds.retrieve(KEYCHAIN_KEY).await?.map(Zeroizing::new) {
                return ok(signer_from_hex(&k)?, KeySource::OsKeychain);
            }
            // Move a settings-store key into the keychain, or mint one.
            let hexed = settings_seed.clone().unwrap_or_else(fresh_seed_hex);
            let signer = signer_from_hex(&hexed)?;
            match creds.store(KEYCHAIN_KEY, &hexed).await {
                Ok(()) => {
                    if settings_seed.is_some() {
                        // The keychain copy is authoritative now. A failed
                        // delete leaves a redundant copy, not a broken key.
                        if let Some(s) = &self.settings {
                            let _ = s.delete(SETTINGS_KEY).await;
                        }
                    }
                    return ok(signer, KeySource::OsKeychain);
                }
                Err(e) => tracing::warn!("Could not store the audit signing key in the OS keychain: {e}"),
            }
        }
        if let Some(s) = &self.settings {
            let source = if s.is_encrypted() { KeySource::SettingsStoreEncrypted } else { KeySource::SettingsStorePlain };
            if let Some(k) = settings_seed {
                return ok(signer_from_hex(&k)?, source);
            }
            let hexed = fresh_seed_hex();
            s.set_secret(SETTINGS_KEY, &hexed).await?;
            if source == KeySource::SettingsStorePlain {
                tracing::warn!("Audit signing key stored unencrypted; anchors only detect tampering by parties without database access");
            }
            return ok(signer_from_hex(&hexed)?, source);
        }
        tracing::warn!("No persistent store for the audit signing key; anchors made this session are verifiable only until restart");
        ok(AuditSigner::from_seed(AuditSigner::generate_seed().as_ref())?, KeySource::Ephemeral)
    }

    async fn verifier(&self) -> Option<AnchorVerifier> {
        match self.resolved().await {
            Ok(k) => Some(k.signer.verifier().clone()),
            Err(e) => {
                tracing::warn!("{e}");
                None
            }
        }
    }

    /// Recompute the chain, check sequence gaps and verify anchor signatures.
    pub async fn verify(&self) -> Result<AuditVerification, AppError> {
        let v = self.verifier().await;
        self.pool.execute(move |c| audit_chain::verify(c, v.as_ref())).await
    }

    /// Sign a checkpoint over the current tail (on demand).
    pub async fn create_anchor(&self) -> Result<AuditAnchor, AppError> {
        self.resolved().await?;
        let key = self.key.clone();
        self.pool
            .execute(move |c| {
                let k = key
                    .get()
                    .and_then(|r| r.as_ref().ok())
                    .ok_or_else(|| AppError::internal("audit signing key not initialized"))?;
                audit_chain::create_anchor(c, &k.signer)
            })
            .await
    }

    /// Anchor only when entries were added since the last anchor.
    pub async fn anchor_if_new(&self) -> Result<Option<AuditAnchor>, AppError> {
        let pending = self.pool.execute(audit_chain::unanchored_count).await?;
        if pending == 0 {
            return Ok(None);
        }
        self.create_anchor().await.map(Some)
    }

    /// Checkpoint at start-up and then every `every` (desktop app).
    pub async fn run_anchor_loop(self, every: Duration) {
        let mut tick = tokio::time::interval(every);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tick.tick().await;
            match self.anchor_if_new().await {
                Ok(Some(a)) => tracing::info!("Audit log anchored at entry {} (anchor #{})", a.last_sequence, a.anchor_seq),
                Ok(None) => {}
                Err(e) => tracing::warn!("Audit log anchoring failed: {e}"),
            }
        }
    }

    /// Full log as JSON lines (header, entries, anchors) for external verification.
    pub async fn export_jsonl(&self) -> Result<String, AppError> {
        let v = self.verifier().await;
        self.pool.execute(move |c| audit_chain::export_jsonl(c, v.as_ref())).await
    }

    pub async fn status(&self) -> Result<AuditLogStatus, AppError> {
        let key = self.resolved().await.ok().map(|k| (k.signer.verifier().clone(), k.source));
        self.pool
            .execute(move |c| {
                let tail = audit_chain::chain_tail(c)?;
                let last_anchor = audit_chain::latest_anchor(c)?;
                Ok(AuditLogStatus {
                    entries: c.query_row("SELECT COUNT(*) FROM audit_events", [], |r| r.get(0))?,
                    last_sequence: tail.as_ref().map(|t| t.0),
                    last_hash: tail.map(|t| t.1),
                    anchors: last_anchor.as_ref().map_or(0, |a| a.anchor_seq),
                    unanchored_entries: audit_chain::unanchored_count(c)?,
                    last_anchor,
                    append_only_enforced: audit_chain::append_only_enforced(c)?,
                    key_id: key.as_ref().map(|k| k.0.key_id.clone()),
                    public_key: key.as_ref().map(|k| k.0.public_key_hex()),
                    key_source: key.map(|k| k.1),
                })
            })
            .await
    }
}

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

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;
