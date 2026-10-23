//! App settings managed by the organization's console (see
//! `ufop_core::agent::ManagedSettings`): stored locally so they hold while
//! offline, applied to the proxy and AI here, and read by the UI for the
//! update channel, automatic checks and the blackout window.

use crate::ai_engine::AiAssistant;
use crate::automation_engine::AutomationManager;
use crate::backup_engine::BackupManager;
use crate::commands::proxy_commands;
use crate::mount_engine::MountManager;
use crate::core::error::AppError;
use crate::net::{self, ProxyMode, ProxySettings};
use crate::security::CredentialStore;
use crate::storage::repository::Repository;
use ufop_core::agent::{ManagedProxy, ManagedSettings};

const CONFIG_KEY: &str = "managed_settings";

pub async fn load(repo: &Repository) -> Option<ManagedSettings> {
    repo.get_config(CONFIG_KEY).await.ok().flatten().and_then(|j| serde_json::from_str(&j).ok())
}

fn proxy_settings(p: &ManagedProxy) -> Result<ProxySettings, AppError> {
    let mode = match p.mode.as_str() {
        "system" => ProxyMode::System,
        "none" => ProxyMode::None,
        "manual" => ProxyMode::Manual,
        "pac" => ProxyMode::Pac,
        other => return Err(AppError::validation(format!("Unknown proxy mode '{other}'"))),
    };
    Ok(ProxySettings { mode, url: p.url.clone(), pac_url: p.pac_url.clone(), no_proxy: p.no_proxy.clone(), username: String::new(), managed: true })
}

/// The managers managed settings change.
#[derive(Clone, Copy)]
pub struct Targets<'a> {
    pub repo: &'a Repository,
    pub creds: &'a CredentialStore,
    pub ai: &'a AiAssistant,
    pub backups: &'a BackupManager,
    pub mounts: &'a MountManager,
    pub automation: &'a AutomationManager,
}

/// Apply `settings` (or release them with `None`) and store them; `org`
/// names the owner of organization workflows.
pub async fn apply(t: Targets<'_>, org: &str, settings: Option<ManagedSettings>) -> Result<(), AppError> {
    let Targets { repo, creds, ai, backups, mounts, automation } = t;
    if let Some(s) = &settings {
        s.validate().map_err(AppError::validation)?;
    }
    match settings.as_ref().and_then(|s| s.proxy.as_ref()) {
        Some(p) => {
            // A managed proxy has no user credentials: clear any saved password.
            proxy_commands::save(repo, creds, proxy_settings(p)?, Some(None)).await?;
        }
        None if net::current().managed => {
            proxy_commands::save(repo, creds, ProxySettings::default(), None).await?;
        }
        None => {}
    }
    ai.set_managed(settings.as_ref().and_then(|s| s.ai.clone())).await;
    backups.set_managed_caps(settings.as_ref().and_then(|s| s.transfers.clone())).await;
    let mut problems = backups.set_managed_templates(settings.as_ref().and_then(|s| s.backup_templates.as_deref()));
    problems.extend(backups.set_managed_notification_profiles(settings.as_ref().and_then(|s| s.notification_profiles.as_deref())));
    mounts.set_managed(settings.as_ref().and_then(|s| s.mounts.clone())).await;
    // Organization workflows: installed read-only; none listed removes them.
    let workflows = settings.as_ref().and_then(|s| s.workflows.clone()).unwrap_or_default();
    problems.extend(automation.apply_org_workflows(repo, &workflows, org).await?);
    let json = match &settings {
        Some(s) => serde_json::to_string(s)?,
        None => String::new(),
    };
    repo.set_config(CONFIG_KEY, &json).await?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(AppError::validation(format!("Some organization workflows, backup templates or notification profiles were not installed: {}", problems.join("; "))))
    }
}

/// Startup: put the stored limits back in force (the proxy restores itself)
/// and apply the transfer caps.
pub async fn activate_stored(repo: &Repository, ai: &AiAssistant, backups: &BackupManager, mounts: &MountManager) {
    let s = load(repo).await;
    ai.set_managed(s.as_ref().and_then(|s| s.ai.clone())).await;
    backups.set_managed_caps(s.as_ref().and_then(|s| s.transfers.clone())).await;
    for p in backups.set_managed_templates(s.as_ref().and_then(|s| s.backup_templates.as_deref())) {
        tracing::warn!("Organization backup template not installed: {p}");
    }
    for p in backups.set_managed_notification_profiles(s.as_ref().and_then(|s| s.notification_profiles.as_deref())) {
        tracing::warn!("Organization notification profile not installed: {p}");
    }
    mounts.set_managed(s.and_then(|s| s.mounts)).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use ufop_core::agent::{ManagedAi, ManagedMounts, ManagedTransfers, ManagedUpdates, ManagedWorkflow};

    #[tokio::test]
    async fn applies_stores_and_releases() {
        let _lock = crate::net::tests::MODE_LOCK.lock().await;
        let repo = Repository::open_in_memory().await.unwrap();
        let creds = Arc::new(CredentialStore::new());
        let ai = AiAssistant::new();
        let t = tempfile::tempdir().unwrap();
        let backups = BackupManager::with_kdf(repo.pool().clone(), creds.clone(), None, None, t.path().join("dbs"), ufop_core::backup::crypto::KdfParams::insecure_fast());
        let s = ManagedSettings {
            version: 3,
            updates: Some(ManagedUpdates { channel: Some("beta".into()), ..Default::default() }),
            proxy: Some(ManagedProxy { mode: "manual".into(), url: "http://proxy.corp:3128".into(), no_proxy: vec!["intranet".into()], ..Default::default() }),
            ai: Some(ManagedAi { enabled: Some(false), allow_cloud: None }),
            transfers: Some(ManagedTransfers { max_upload_kbps: Some(512), max_download_kbps: None, max_concurrent_runs: Some(3) }),
            mounts: Some(ManagedMounts { read_only: Some(true), cache_mode: Some("full".into()), ..Default::default() }),
            workflows: Some(vec![ManagedWorkflow {
                id: "intake".into(),
                name: "Org intake".into(),
                enabled: false,
                rule: serde_json::to_value(crate::automation_engine::AutomationRule::new("x".into(), crate::automation_engine::RuleTrigger::Manual, crate::automation_engine::RuleAction::CreateFolder { path: "/tmp/org-intake".into() })).unwrap(),
            }]),
            backup_templates: Some(vec![ufop_core::agent::ManagedBackupTemplate {
                id: "std".into(),
                name: "Org standard".into(),
                enforced: true,
                settings: serde_json::json!({ "retention": { "type": "keep_count", "count": 7 } }),
                ..Default::default()
            }]),
            notification_profiles: Some(vec![ufop_core::agent::ManagedNotificationProfile {
                id: "pager".into(),
                name: "Org pager".into(),
                notification: serde_json::json!({ "type": "webhook", "url": "https://events.pagerduty.com/v2/enqueue", "credential_id": "central:pd" }),
                ..Default::default()
            }]),
            server_access: None,
        };
        let automation = AutomationManager::new();
        let mounts = MountManager::with_cache_root(t.path().join("mount-cache"));
        let targets = Targets { repo: &repo, creds: &creds, ai: &ai, backups: &backups, mounts: &mounts, automation: &automation };
        apply(targets, "Acme", Some(s.clone())).await.unwrap();
        let org_rule = automation.get_rule("org-intake").await.expect("org workflow installed");
        assert_eq!((org_rule.name.as_str(), org_rule.managed_by.as_deref()), ("Org intake", Some("Acme")));
        assert!(automation.ensure_editable("org-intake").await.is_err());
        assert_eq!(load(&repo).await, Some(s));
        let p = net::current();
        assert!(p.managed && p.mode == ProxyMode::Manual && p.url == "http://proxy.corp:3128");
        assert!(!ai.get_feature_toggles().await.ai_master_enabled);
        let caps = backups.effective_caps().await;
        assert_eq!((caps.upload_bps, caps.download_bps, caps.concurrency()), (512 * 1024, 0, 3));
        let org = backups.list_templates().await.unwrap();
        assert_eq!(org.len(), 1);
        assert!(org[0].managed && org[0].enforced && org[0].id == "org:std");
        // The enforced template wins over the job's own retention.
        let job = crate::backup_engine::BackupJob { retention: ufop_core::backup::RetentionPolicy::KeepAll, ..Default::default() };
        let r = backups.resolve_job(&job).await.unwrap();
        assert_eq!(r.job.retention, ufop_core::backup::RetentionPolicy::KeepCount { count: 7 });
        let profiles = backups.list_notification_profiles().await.unwrap();
        assert!(profiles.len() == 1 && profiles[0].managed && profiles[0].id == "org:pager");

        // Restart: the AI limit comes back from storage.
        let ai2 = AiAssistant::new();
        assert_eq!(mounts.managed().await.and_then(|m| m.read_only), Some(true));
        let mounts2 = MountManager::with_cache_root(t.path().join("mount-cache"));
        activate_stored(&repo, &ai2, &backups, &mounts2).await;
        assert!(!ai2.get_feature_toggles().await.ai_master_enabled);
        assert_eq!(mounts2.managed().await.and_then(|m| m.cache_mode), Some("full".into()), "the mount profile survives a restart");

        // Invalid settings change nothing.
        let bad = ManagedSettings { version: 4, proxy: Some(ManagedProxy { mode: "pac".into(), ..Default::default() }), ..Default::default() };
        assert!(apply(targets, "Acme", Some(bad)).await.is_err());
        assert_eq!(load(&repo).await.unwrap().version, 3);

        apply(targets, "Acme", None).await.unwrap();
        assert!(automation.get_rule("org-intake").await.is_none(), "org workflows removed when no longer managed");
        assert_eq!(load(&repo).await, None);
        assert!(!net::current().managed, "the managed proxy is released");
        assert!(ai.get_feature_toggles().await.ai_master_enabled);
        assert_eq!(backups.effective_caps().await.concurrency(), 1, "caps released");
        assert!(mounts.managed().await.is_none(), "mount profile released");
        let _ = net::apply(ProxySettings::default(), None);
    }
}
