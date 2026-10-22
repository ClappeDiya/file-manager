//! Proxy settings: stored in config (`proxy_settings`), password in the
//! credential store, applied to every HTTP client via `crate::net`.

use crate::core::error::AppError;
use crate::net::{self, OsProxy, ProxySettings};
#[cfg(test)]
use crate::net::ProxyMode;
use crate::security::CredentialStore;
use crate::storage::Repository;
use serde::Serialize;
use std::sync::Arc;
use tauri::State;

const CONFIG_KEY: &str = "proxy_settings";
const PASSWORD_KEY: &str = "ufop-proxy-password";

#[derive(Debug, Clone, Serialize)]
pub struct ProxyView {
    pub settings: ProxySettings,
    pub has_password: bool,
    /// What System mode currently sees in the OS settings.
    pub os: OsProxyView,
}

#[derive(Debug, Clone, Serialize)]
pub struct OsProxyView {
    pub http: Option<String>,
    pub https: Option<String>,
    pub bypass: Vec<String>,
    pub pac_url: Option<String>,
    pub env: Option<String>,
}

impl From<OsProxy> for OsProxyView {
    fn from(o: OsProxy) -> Self {
        let env = ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy", "ALL_PROXY"].iter().find_map(|n| std::env::var(n).ok());
        Self { http: o.http, https: o.https, bypass: o.bypass, pac_url: o.pac_url, env }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProxyTest {
    pub ok: bool,
    pub status: Option<u16>,
    /// The proxy the request went through (password removed), `None` = direct.
    pub via: Option<String>,
    pub error: Option<String>,
    pub millis: u64,
}

async fn load(repo: &Repository) -> ProxySettings {
    repo.get_config(CONFIG_KEY).await.ok().flatten().and_then(|j| serde_json::from_str(&j).ok()).unwrap_or_default()
}

async fn apply_blocking(settings: ProxySettings, password: Option<String>) -> Result<(), AppError> {
    tokio::task::spawn_blocking(move || net::apply(settings, password))
        .await
        .map_err(|e| AppError::internal(e.to_string()))?
        .map_err(|e| AppError::Connection { message: e, advice: "Check the proxy settings.".into() })
}

/// Apply the stored settings (startup). Failures fall back to System.
pub async fn activate_stored(repo: &Repository, creds: &CredentialStore) {
    let settings = load(repo).await;
    let password = creds.retrieve(PASSWORD_KEY).await.ok().flatten();
    if let Err(e) = apply_blocking(settings, password).await {
        tracing::warn!("Proxy settings not applied, using the system proxy: {e}");
        let _ = apply_blocking(ProxySettings::default(), None).await;
    }
}

/// Save and apply (used by Settings and by fleet-managed policies).
pub async fn save(repo: &Repository, creds: &CredentialStore, settings: ProxySettings, password: Option<Option<String>>) -> Result<ProxySettings, AppError> {
    let pass = match &password {
        Some(p) => p.clone().filter(|p| !p.is_empty()),
        None => creds.retrieve(PASSWORD_KEY).await.ok().flatten(),
    };
    apply_blocking(settings.clone(), pass).await?;
    let json = serde_json::to_string(&settings).map_err(|e| AppError::internal(e.to_string()))?;
    repo.set_config(CONFIG_KEY, &json).await?;
    match password {
        Some(Some(p)) if !p.is_empty() => creds.store(PASSWORD_KEY, &p).await?,
        Some(_) => {
            let _ = creds.delete(PASSWORD_KEY).await;
        }
        None => {}
    }
    Ok(settings)
}

#[tauri::command]
pub async fn proxy_get(repo: State<'_, Repository>, creds: State<'_, Arc<CredentialStore>>) -> Result<ProxyView, AppError> {
    let os = tokio::task::spawn_blocking(net::os_proxy).await.map_err(|e| AppError::internal(e.to_string()))?;
    Ok(ProxyView {
        settings: load(&repo).await,
        has_password: creds.retrieve(PASSWORD_KEY).await.ok().flatten().is_some(),
        os: os.into(),
    })
}

/// `password`: omitted = keep, `null`/"" = remove, string = set.
#[tauri::command]
pub async fn proxy_set(
    settings: ProxySettings,
    password: Option<Option<String>>,
    repo: State<'_, Repository>,
    creds: State<'_, Arc<CredentialStore>>,
) -> Result<ProxySettings, AppError> {
    if net::current().managed && !settings.managed {
        return Err(AppError::validation("The proxy is managed by your organization's console"));
    }
    save(&repo, &creds, ProxySettings { managed: false, ..settings }, password).await
}

/// Fetch `url` through the current settings and say how it went.
#[tauri::command]
pub async fn proxy_test(url: Option<String>) -> Result<ProxyTest, AppError> {
    let url = url.unwrap_or_else(|| "https://www.google.com/generate_204".into());
    let parsed = reqwest::Url::parse(&url).map_err(|e| AppError::validation(format!("Invalid URL: {e}")))?;
    let via = net::proxy_for(&parsed).map(|mut u| {
        let _ = u.set_password(None);
        u.to_string()
    });
    let start = std::time::Instant::now();
    let res = net::builder().timeout(std::time::Duration::from_secs(15)).build().map_err(|e| AppError::internal(e.to_string()))?.get(parsed).send().await;
    let millis = start.elapsed().as_millis() as u64;
    Ok(match res {
        Ok(r) => ProxyTest { ok: r.status().is_success() || r.status().is_redirection(), status: Some(r.status().as_u16()), via, error: None, millis },
        Err(e) => ProxyTest { ok: false, status: None, via, error: Some(e.to_string()), millis },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn saves_applies_and_keeps_password_in_the_credential_store() {
        let repo = Repository::open_in_memory().await.unwrap();
        let creds = CredentialStore::new();
        let s = ProxySettings { mode: ProxyMode::None, no_proxy: vec!["a.example".into()], username: "me".into(), ..Default::default() };
        save(&repo, &creds, s.clone(), Some(Some("pw".into()))).await.unwrap();
        assert_eq!(load(&repo).await, s);
        assert!(!repo.get_config(CONFIG_KEY).await.unwrap().unwrap().contains("pw"), "password not in config");
        assert_eq!(creds.retrieve(PASSWORD_KEY).await.unwrap().as_deref(), Some("pw"));
        // Omitted password keeps it; empty removes it.
        save(&repo, &creds, s.clone(), None).await.unwrap();
        assert!(creds.retrieve(PASSWORD_KEY).await.unwrap().is_some());
        save(&repo, &creds, s.clone(), Some(None)).await.unwrap();
        assert!(creds.retrieve(PASSWORD_KEY).await.unwrap().is_none());
        // Invalid settings are refused and not stored.
        let bad = ProxySettings { mode: ProxyMode::Manual, url: "nope".into(), ..Default::default() };
        assert!(save(&repo, &creds, bad, None).await.is_err());
        assert_eq!(load(&repo).await, s);
        net::apply(ProxySettings::default(), None).unwrap();
    }
}
