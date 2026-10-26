//! Settings → Reset: put preferences back to their defaults.
//!
//! Only preferences are reset, never data: connections, sync pairs, backup
//! jobs, mounts, history, the ledger, keys and saved passwords stay. The
//! integrations someone set up on purpose (local web server, chat control,
//! Prometheus exporter, rclone program, proxy, console enrolment) keep their
//! settings too. Each scope lists exactly the stored keys it removes and
//! puts the running app back on the defaults at once. The appearance and
//! layout scope lives in the web view (`src/lib/reset-settings.ts`).

use crate::core::error::AppError;
use crate::storage::Repository;
use serde::{Deserialize, Serialize};

/// What to reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResetScope {
    /// Startup & tray, desktop notifications.
    Desktop,
    /// Global sync defaults, bandwidth timetable, failed-transfer retention.
    Transfers,
}

impl ResetScope {
    /// The stored keys this scope removes.
    pub fn keys(self) -> &'static [&'static str] {
        match self {
            Self::Desktop => &[crate::startup::KEY_STARTUP, crate::tray::KEY_BACKGROUND, crate::notify_hub::desktop::NOTIFICATION_PREFS_KEY],
            Self::Transfers => &[crate::sync_engine::DEFAULT_OPTIONS_KEY, crate::commands::transfer_commands::BANDWIDTH_SCHEDULE_KEY, crate::transfer_engine::retention::CONFIG_KEY],
        }
    }
}

/// What a reset did.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResetReport {
    /// Stored settings that were removed (they had been changed).
    pub removed: Vec<String>,
}

/// The running parts a reset puts back on their defaults.
pub struct Live<'a> {
    pub startup: &'a crate::startup::StartupState,
    pub background: &'a crate::tray::BackgroundMode,
    pub sync: &'a crate::sync_engine::SyncManager,
}

pub async fn reset(repo: &Repository, scopes: &[ResetScope], live: Live<'_>) -> Result<ResetReport, AppError> {
    let mut report = ResetReport::default();
    for &scope in scopes {
        for key in scope.keys() {
            if repo.delete_config(key).await? {
                report.removed.push((*key).to_string());
            }
        }
        match scope {
            ResetScope::Desktop => {
                crate::startup::save(repo, live.startup, crate::startup::StartupPrefs::default()).await?;
                repo.delete_config(crate::startup::KEY_STARTUP).await?;
                crate::tray::save_background_mode(repo, live.background, false).await?;
                repo.delete_config(crate::tray::KEY_BACKGROUND).await?;
            }
            ResetScope::Transfers => {
                live.sync.reset_default_options(repo).await?;
                ufop_core::throttle::set_schedule(Vec::new())?;
            }
        }
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resets_the_chosen_preferences_and_nothing_else() {
        let repo = Repository::open_in_memory().await.unwrap();
        let startup = crate::startup::load(&repo).await;
        let background = crate::tray::load_background_mode(&repo).await;
        let sync = crate::sync_engine::SyncManager::new();
        crate::startup::save(&repo, &startup, crate::startup::StartupPrefs { start_minimized: true, ..Default::default() }).await.unwrap();
        crate::tray::save_background_mode(&repo, &background, true).await.unwrap();
        repo.set_config(crate::notify_hub::desktop::NOTIFICATION_PREFS_KEY, "{}").await.unwrap();
        repo.set_config(crate::commands::transfer_commands::BANDWIDTH_SCHEDULE_KEY, "[]").await.unwrap();
        // Integrations and data stay.
        repo.set_config(crate::embedded_server::SETTINGS_KEY, "{\"enabled\":true}").await.unwrap();
        repo.set_config(crate::rclone_engine::resolver::CONFIG_KEY, "{}").await.unwrap();

        let live = Live { startup: &startup, background: &background, sync: &sync };
        let report = reset(&repo, &[ResetScope::Desktop], live).await.unwrap();
        let mut removed = report.removed.clone();
        removed.sort();
        assert_eq!(removed, ["app.run_in_background", "app.startup", "notification_prefs"]);
        assert_eq!(startup.get(), crate::startup::StartupPrefs::default());
        assert!(!background.enabled());
        assert_eq!(repo.get_config("app.startup").await.unwrap(), None);
        // Not in this scope:
        assert!(repo.get_config(crate::commands::transfer_commands::BANDWIDTH_SCHEDULE_KEY).await.unwrap().is_some());
        assert!(repo.get_config(crate::embedded_server::SETTINGS_KEY).await.unwrap().is_some());
        assert!(repo.get_config(crate::rclone_engine::resolver::CONFIG_KEY).await.unwrap().is_some());
    }

    #[tokio::test]
    async fn transfers_scope_restores_the_built_in_sync_defaults() {
        let repo = Repository::open_in_memory().await.unwrap();
        let startup = crate::startup::StartupState::default();
        let background = crate::tray::BackgroundMode::default();
        let sync = crate::sync_engine::SyncManager::new();
        let custom = ufop_core::sync_types::CopyOptions { max_depth: 3, ..Default::default() };
        sync.set_default_options(&repo, custom).await.unwrap();
        assert_eq!(sync.default_options().max_depth, 3);
        let report = reset(&repo, &[ResetScope::Transfers], Live { startup: &startup, background: &background, sync: &sync }).await.unwrap();
        assert_eq!(report.removed, ["sync.default_copy_options"]);
        assert_eq!(sync.default_options(), ufop_core::sync_types::CopyOptions::default());
        // Nothing left to remove the second time.
        let again = reset(&repo, &[ResetScope::Transfers], Live { startup: &startup, background: &background, sync: &sync }).await.unwrap();
        assert!(again.removed.is_empty());
    }

    #[test]
    fn scopes_serialize_as_snake_case() {
        assert_eq!(serde_json::to_string(&ResetScope::Desktop).unwrap(), "\"desktop\"");
        assert_eq!(serde_json::from_str::<ResetScope>("\"transfers\"").unwrap(), ResetScope::Transfers);
    }
}
