//! Share links for files on cloud connections, under a sharing policy
//! (stored in config; enforced before any provider is asked).

use super::connector_commands::connect_if_needed;
use crate::connectors::{ConnectionManager, ConnectorRegistry, ShareLink, ShareOptions, ShareScope};
use crate::core::error::AppError;
use crate::ledger::{LedgerEngine, LedgerStatus, OperationLedger, RecordEvent};
use crate::storage::Repository;
use serde::{Deserialize, Serialize};
use tauri::State;

const CONFIG_KEY: &str = "sharing_policy";

/// What links people may create from this app.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct SharingPolicy {
    /// Creating links at all.
    pub enabled: bool,
    /// Links anyone can open (off = organization links only).
    pub allow_anyone: bool,
    /// Links must expire...
    pub require_expiry: bool,
    /// ...within this many days.
    pub max_expiry_days: Option<u32>,
    /// Anyone-links must have a password.
    pub require_password: bool,
    /// Links may grant editing.
    pub allow_edit: bool,
}

impl Default for SharingPolicy {
    fn default() -> Self {
        Self { enabled: true, allow_anyone: true, require_expiry: false, max_expiry_days: None, require_password: false, allow_edit: true }
    }
}

impl SharingPolicy {
    pub fn check(&self, o: &ShareOptions) -> Result<(), AppError> {
        let refuse = |why: &str| Err(AppError::file_op(format!("Sharing policy: {why}"), "Change the link options, or the policy in Settings → Sharing."));
        if !self.enabled {
            return refuse("creating share links is turned off");
        }
        if o.scope == ShareScope::Anyone && !self.allow_anyone {
            return refuse("links must be limited to your organization");
        }
        if o.allow_edit && !self.allow_edit {
            return refuse("links can't allow editing");
        }
        if o.scope == ShareScope::Anyone && self.require_password && o.password.as_deref().map_or(true, str::is_empty) {
            return refuse("links anyone can open need a password");
        }
        match (o.expires_in_secs, self.max_expiry_days) {
            (None, _) if self.require_expiry || self.max_expiry_days.is_some() => refuse("links must have an expiry date"),
            (Some(s), Some(d)) if s > u64::from(d) * 86_400 => refuse(&format!("links must expire within {d} day{}", if d == 1 { "" } else { "s" })),
            _ => Ok(()),
        }
    }
}

pub async fn load_policy(repo: &Repository) -> Result<SharingPolicy, AppError> {
    Ok(repo.get_config(CONFIG_KEY).await?.and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default())
}

#[tauri::command]
pub async fn sharing_policy_get(repo: State<'_, Repository>) -> Result<SharingPolicy, AppError> {
    load_policy(&repo).await
}

#[tauri::command]
pub async fn sharing_policy_set(policy: SharingPolicy, repo: State<'_, Repository>) -> Result<SharingPolicy, AppError> {
    let json = serde_json::to_string(&policy).map_err(|e| AppError::internal(e.to_string()))?;
    repo.set_config(CONFIG_KEY, &json).await?;
    Ok(policy)
}

/// Create a share link for `path` on a connection. The link itself is
/// not stored; the ledger records that one was made (and with what).
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn connector_create_share_link(
    protocol: String,
    connection_id: Option<String>,
    path: String,
    options: ShareOptions,
    registry: State<'_, ConnectorRegistry>,
    connections: State<'_, ConnectionManager>,
    repo: State<'_, Repository>,
    ledger: State<'_, OperationLedger>,
) -> Result<ShareLink, AppError> {
    load_policy(&repo).await?.check(&options)?;
    // A user action: tried even when the connection is flagged as
    // credentials-rejected (only background reconnects skip those).
    let connector = registry.get(&protocol).ok_or_else(|| AppError::validation(format!("Unsupported protocol '{protocol}'")))?;
    connect_if_needed(connector.as_ref(), &connections, connection_id.as_deref()).await?;
    let result = connector.create_share_link(&path, &options).await;
    let summary = match &result {
        Ok(l) => format!(
            "share_link: {protocol}:{path} ({}{})",
            if l.scope == ShareScope::Anyone { "anyone" } else { "organization" },
            l.expires_at.map(|t| format!(", expires {}", t.to_rfc3339())).unwrap_or_default()
        ),
        Err(e) => format!("share_link failed: {protocol}:{path}: {e}"),
    };
    let status = if result.is_ok() { LedgerStatus::Ok } else { LedgerStatus::Failed };
    ledger.record(RecordEvent::new(LedgerEngine::Connector, "share_link").status(status).summary(summary).subject(format!("{protocol}:{path}"))).await;
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn o(f: impl FnOnce(&mut ShareOptions)) -> ShareOptions {
        let mut x = ShareOptions::default();
        f(&mut x);
        x
    }

    #[test]
    fn default_policy_allows_everything() {
        assert!(SharingPolicy::default().check(&ShareOptions::default()).is_ok());
    }

    #[test]
    fn policy_rules() {
        let p = SharingPolicy { max_expiry_days: Some(7), require_password: true, allow_edit: false, ..Default::default() };
        let ok = o(|x| {
            x.expires_in_secs = Some(86_400);
            x.password = Some("pw".into());
        });
        assert!(p.check(&ok).is_ok());
        let msg = |x: ShareOptions| p.check(&x).unwrap_err().to_string();
        assert!(msg(o(|x| x.password = Some("pw".into()))).contains("expiry"));
        assert!(msg(o(|x| {
            x.expires_in_secs = Some(8 * 86_400);
            x.password = Some("pw".into());
        }))
        .contains("within 7 days"));
        assert!(msg(o(|x| x.expires_in_secs = Some(60))).contains("password"));
        assert!(msg(o(|x| {
            x.expires_in_secs = Some(60);
            x.password = Some("pw".into());
            x.allow_edit = true;
        }))
        .contains("editing"));
        // Organization links don't need a password.
        assert!(p.check(&o(|x| {
            x.scope = ShareScope::Organization;
            x.expires_in_secs = Some(60);
        }))
        .is_ok());
        let org_only = SharingPolicy { allow_anyone: false, ..Default::default() };
        assert!(org_only.check(&ShareOptions::default()).is_err());
        assert!(SharingPolicy { enabled: false, ..Default::default() }.check(&ShareOptions::default()).unwrap_err().to_string().contains("turned off"));
    }

    #[tokio::test]
    async fn policy_persists_with_defaults_for_missing_fields() {
        let repo = Repository::open_in_memory().await.unwrap();
        assert_eq!(load_policy(&repo).await.unwrap(), SharingPolicy::default());
        repo.set_config(CONFIG_KEY, r#"{"require_expiry":true}"#).await.unwrap();
        let p = load_policy(&repo).await.unwrap();
        assert!(p.require_expiry && p.enabled && p.allow_anyone);
    }
}
