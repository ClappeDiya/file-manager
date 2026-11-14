//! Starts and stops the chat transports from the saved settings, keeps
//! their status, and stores the bot tokens in the credential store.

use super::backend::{RepoAudit, TauriBackend};
use super::core::Dialect;
use super::handler::Handler;
use super::{slack, telegram, ChannelState, ChannelStatus, ChatControlSettings, SETTINGS_KEY, SLACK_APP_TOKEN_KEY, SLACK_BOT_TOKEN_KEY, TELEGRAM_TOKEN_KEY};
use crate::core::error::AppError;
use crate::core::traits::StorageOperations;
use crate::security::CredentialStore;
use crate::storage::Repository;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager};

/// Secrets typed in the settings form. `None` leaves a stored secret
/// alone, an empty string removes it. They are never read back to the UI.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SecretUpdate {
    pub telegram_token: Option<String>,
    pub slack_bot_token: Option<String>,
    pub slack_app_token: Option<String>,
}

/// What the settings screen shows.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatControlStatus {
    pub running: bool,
    pub telegram: ChannelState,
    pub slack: ChannelState,
    pub has_telegram_token: bool,
    pub has_slack_bot_token: bool,
    pub has_slack_app_token: bool,
}

struct Running {
    stop: tokio::sync::watch::Sender<bool>,
    tasks: Vec<tauri::async_runtime::JoinHandle<()>>,
}

pub struct ChatControlService {
    app: AppHandle,
    running: tokio::sync::Mutex<Option<Running>>,
    telegram: Arc<ChannelStatus>,
    slack: Arc<ChannelStatus>,
}

/// Token shape checks (a typo is caught when saving, not at the first poll).
pub fn check_secrets(s: &SecretUpdate) -> Result<(), String> {
    let plain = |t: &str| !t.chars().any(char::is_whitespace);
    if let Some(t) = s.telegram_token.as_deref().filter(|t| !t.is_empty()) {
        // `<bot id>:<secret>`
        let ok = t.split_once(':').is_some_and(|(id, secret)| !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) && secret.len() >= 10) && plain(t);
        if !ok {
            return Err("That does not look like a Telegram bot token (123456:ABC…); copy it from @BotFather".into());
        }
    }
    if let Some(t) = s.slack_bot_token.as_deref().filter(|t| !t.is_empty()) {
        if !t.starts_with("xoxb-") || !plain(t) {
            return Err("The Slack bot token starts with xoxb-".into());
        }
    }
    if let Some(t) = s.slack_app_token.as_deref().filter(|t| !t.is_empty()) {
        if !t.starts_with("xapp-") || !plain(t) {
            return Err("The Slack app-level token starts with xapp-".into());
        }
    }
    Ok(())
}

impl ChatControlService {
    pub fn new(app: AppHandle) -> Self {
        Self { app, running: tokio::sync::Mutex::new(None), telegram: Arc::new(ChannelStatus::default()), slack: Arc::new(ChannelStatus::default()) }
    }

    fn creds(&self) -> Arc<CredentialStore> {
        self.app.state::<Arc<CredentialStore>>().inner().clone()
    }

    fn repo(&self) -> Repository {
        self.app.state::<Repository>().inner().clone()
    }

    pub async fn settings(&self) -> ChatControlSettings {
        match StorageOperations::get_config(&self.repo(), SETTINGS_KEY).await {
            Ok(Some(json)) => serde_json::from_str(&json).unwrap_or_else(|e| {
                tracing::warn!("Invalid remote-control settings, using defaults (off): {e}");
                ChatControlSettings::default()
            }),
            _ => ChatControlSettings::default(),
        }
    }

    async fn secret(&self, key: &str) -> Option<String> {
        self.creds().retrieve(key).await.ok().flatten().filter(|s| !s.is_empty())
    }

    pub async fn status(&self) -> ChatControlStatus {
        ChatControlStatus {
            running: self.running.lock().await.is_some(),
            telegram: self.telegram.snapshot(),
            slack: self.slack.snapshot(),
            has_telegram_token: self.secret(TELEGRAM_TOKEN_KEY).await.is_some(),
            has_slack_bot_token: self.secret(SLACK_BOT_TOKEN_KEY).await.is_some(),
            has_slack_app_token: self.secret(SLACK_APP_TOKEN_KEY).await.is_some(),
        }
    }

    /// Save the settings and secrets, then start or stop to match. The
    /// settings are kept even when starting fails (the error says why).
    pub async fn save(&self, settings: ChatControlSettings, secrets: SecretUpdate) -> Result<ChatControlStatus, AppError> {
        settings.validate().map_err(|m| AppError::validation(format!("{m}.")))?;
        check_secrets(&secrets).map_err(AppError::validation)?;
        let creds = self.creds();
        for (key, value) in [(TELEGRAM_TOKEN_KEY, &secrets.telegram_token), (SLACK_BOT_TOKEN_KEY, &secrets.slack_bot_token), (SLACK_APP_TOKEN_KEY, &secrets.slack_app_token)] {
            match value.as_deref() {
                None => {}
                Some("") => creds.delete(key).await?,
                Some(v) => creds.store(key, v).await?,
            }
        }
        StorageOperations::set_config(&self.repo(), SETTINGS_KEY, &serde_json::to_string(&settings)?).await?;
        self.apply(true).await?;
        Ok(self.status().await)
    }

    /// Stop every transport.
    pub async fn stop(&self) {
        if let Some(r) = self.running.lock().await.take() {
            let _ = r.stop.send(true);
            for t in r.tasks {
                let _ = tokio::time::timeout(std::time::Duration::from_secs(5), t).await;
            }
        }
    }

    /// Start the enabled transports from the saved settings (stopping any
    /// running ones first). `strict`: a bot token the service refuses is an
    /// error (when saving); otherwise the transport starts and retries.
    pub async fn apply(&self, strict: bool) -> Result<(), AppError> {
        self.stop().await;
        let settings = self.settings().await;
        if !settings.enabled {
            return Ok(());
        }
        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        let mut tasks = Vec::new();
        let backend = Arc::new(TauriBackend::new(self.app.clone()));
        let audit = Arc::new(RepoAudit::new(self.repo()));
        let http = crate::backup_engine::stores::http::client(&Default::default())?;
        let per_min = settings.rate_limit_per_min as usize;

        if settings.telegram.enabled {
            let token = self.secret(TELEGRAM_TOKEN_KEY).await.ok_or_else(|| AppError::validation("Add the Telegram bot token."))?;
            let cfg = telegram::TelegramConfig { api_base: telegram::base_of(&settings.telegram.api_base), token, poll_secs: 25, http: http.clone() };
            let bot_name = match telegram::get_me(&cfg).await {
                Ok(n) => Some(n).filter(|n| !n.is_empty()),
                Err(e) if strict => return Err(AppError::connection(e, "Check the bot token and the network.")),
                Err(e) => {
                    self.telegram.set_error(e);
                    None
                }
            };
            let handler = Arc::new(Handler::new(Dialect::Telegram, bot_name, &settings.telegram.allowed_user_ids, per_min, backend.clone(), audit.clone()));
            tasks.push(tauri::async_runtime::spawn(telegram::run(cfg, handler, stop_rx.clone(), self.telegram.clone())));
        }
        if settings.slack.enabled {
            let bot_token = self.secret(SLACK_BOT_TOKEN_KEY).await.ok_or_else(|| AppError::validation("Add the Slack bot token."))?;
            let app_token = self.secret(SLACK_APP_TOKEN_KEY).await.ok_or_else(|| AppError::validation("Add the Slack app-level token."))?;
            let tls = Arc::new(crate::security::tls::client_config(false, None, &[])?);
            let cfg = slack::SlackConfig { api_base: slack::base_of(&settings.slack.api_base), app_token, bot_token, http: http.clone(), tls };
            if strict {
                slack::open_connection(&cfg).await.map_err(|e| AppError::connection(e, "Check the app-level token and that Socket Mode is on for the app."))?;
            }
            let handler = Arc::new(Handler::new(Dialect::Slack, None, &settings.slack.allowed_user_ids, per_min, backend, audit));
            tasks.push(tauri::async_runtime::spawn(slack::run(cfg, handler, stop_rx, self.slack.clone())));
        }
        *self.running.lock().await = Some(Running { stop: stop_tx, tasks });
        tracing::info!("Remote control started");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_shapes_are_checked_and_clearing_is_allowed() {
        assert!(check_secrets(&SecretUpdate::default()).is_ok());
        let ok = SecretUpdate { telegram_token: Some("123456789:AAEhBP0av18kyhx1234567890abcdefghij".into()), slack_bot_token: Some("xoxb-1-2-abc".into()), slack_app_token: Some("xapp-1-A0-2-abc".into()) };
        assert!(check_secrets(&ok).is_ok());
        for bad in ["", " ", "nocolon", "abc:secretsecretsecret", "123:short", "123:has space in the secret"] {
            let r = check_secrets(&SecretUpdate { telegram_token: Some(bad.into()), ..Default::default() });
            // An empty string means "remove the stored token" and is accepted.
            assert_eq!(r.is_ok(), bad.is_empty(), "{bad:?}");
        }
        assert!(check_secrets(&SecretUpdate { slack_bot_token: Some("xapp-1-A".into()), ..Default::default() }).is_err(), "tokens swapped");
        assert!(check_secrets(&SecretUpdate { slack_app_token: Some("xoxb-1-2".into()), ..Default::default() }).is_err());
        assert!(check_secrets(&SecretUpdate { slack_app_token: Some("xapp-1 2".into()), ..Default::default() }).is_err());
    }
}
