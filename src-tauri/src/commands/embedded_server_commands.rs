//! Settings > Local web server (Advanced): the web server inside the app,
//! and the list of remote `ufop-server` addresses shared with the `ufop` CLI.

use crate::core::error::AppError;
use crate::embedded_server::{EmbeddedServer, EmbeddedSettings, EmbeddedStatus};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tauri::State;
use ufop_core::profiles::{self, ProfileSummary};

#[tauri::command]
pub async fn embedded_server_status(server: State<'_, EmbeddedServer>) -> Result<EmbeddedStatus, AppError> {
    Ok(server.status().await)
}

/// Save the port and network settings (refused while the server runs).
#[tauri::command]
pub async fn embedded_server_save_settings(settings: EmbeddedSettings, server: State<'_, EmbeddedServer>) -> Result<EmbeddedStatus, AppError> {
    server.save_settings(settings).await?;
    Ok(server.status().await)
}

/// Start the server. It will not start without a password.
#[tauri::command]
pub async fn embedded_server_start(server: State<'_, EmbeddedServer>) -> Result<EmbeddedStatus, AppError> {
    server.start().await
}

#[tauri::command]
pub async fn embedded_server_stop(server: State<'_, EmbeddedServer>) -> Result<EmbeddedStatus, AppError> {
    server.stop().await
}

/// Set the password of the server's `admin` account (at least 8 characters).
/// Signs out every session of the server.
#[tauri::command]
pub async fn embedded_server_set_password(password: String, server: State<'_, EmbeddedServer>) -> Result<EmbeddedStatus, AppError> {
    server.set_password(&password).await?;
    Ok(server.status().await)
}

/// Open an `http(s)` address in the default browser: the running server, or a saved one.
#[tauri::command]
pub async fn open_server_in_browser(app: tauri::AppHandle, url: String) -> Result<(), AppError> {
    use tauri_plugin_shell::ShellExt;
    let url = url.trim().to_string();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(AppError::validation("Only web addresses (http:// or https://) can be opened"));
    }
    #[allow(deprecated)]
    let opened = app.shell().open(&url, None);
    opened.map_err(|e| AppError::state(format!("Could not open the browser: {e}"), "Copy the address and open it yourself."))
}

// ── Saved remote servers (profiles of the `ufop` CLI) ──

fn config_file() -> PathBuf {
    profiles::config_path(dirs::config_dir())
}

/// The remote servers saved for the `ufop` CLI, never with their tokens.
#[tauri::command]
pub async fn server_profiles_list() -> Result<Vec<ProfileSummary>, AppError> {
    profiles::list(&config_file())
}

/// Save a server under a name, or change its address.
#[tauri::command]
pub async fn server_profile_save(name: String, server_url: String) -> Result<Vec<ProfileSummary>, AppError> {
    let path = config_file();
    profiles::save_server_url(&path, name.trim(), &server_url)?;
    profiles::list(&path)
}

#[tauri::command]
pub async fn server_profile_remove(name: String) -> Result<Vec<ProfileSummary>, AppError> {
    let path = config_file();
    profiles::remove(&path, name.trim())?;
    profiles::list(&path)
}

/// What a connection test found.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProbeResult {
    pub ok: bool,
    pub message: String,
    pub version: Option<String>,
    pub uptime_secs: Option<u64>,
    pub latency_ms: u64,
    /// The failure was an untrusted (for example self-signed) certificate.
    pub untrusted_certificate: bool,
}

/// Ask a server's health endpoint (`/api/v1/health`, which needs no sign-in)
/// whether it answers and what it is.
pub async fn probe(url: &str, accept_untrusted_certificate: bool) -> Result<ProbeResult, AppError> {
    let base = profiles::clean_server_url(url)?;
    let client = crate::backup_engine::stores::http::client(&crate::backup_engine::stores::http::TlsOptions { accept_invalid_certs: accept_untrusted_certificate, ..Default::default() })?;
    let started = Instant::now();
    let latency = || started.elapsed().as_millis() as u64;
    let fail = |message: String, untrusted: bool, ms: u64| ProbeResult { ok: false, message, version: None, uptime_secs: None, latency_ms: ms, untrusted_certificate: untrusted };
    let resp = match client.get(format!("{base}/api/v1/health")).timeout(Duration::from_secs(8)).send().await {
        Ok(r) => r,
        Err(e) => {
            let chain = {
                let mut s = e.to_string();
                let mut src = std::error::Error::source(&e);
                while let Some(inner) = src {
                    s.push_str(&format!(": {inner}"));
                    src = inner.source();
                }
                s
            };
            let lower = chain.to_lowercase();
            return Ok(if lower.contains("certificate") || lower.contains("unknownissuer") || lower.contains("self-signed") || lower.contains("self signed") {
                fail("The server's certificate is not trusted (it may be self-signed).".into(), true, latency())
            } else if e.is_timeout() {
                fail("The server did not answer within 8 seconds.".into(), false, latency())
            } else if e.is_connect() {
                fail(format!("Could not connect to {base}. Check the address, the port and that the server is running."), false, latency())
            } else {
                fail(format!("The request failed: {e}"), false, latency())
            });
        }
    };
    let ms = latency();
    let status = resp.status();
    if !status.is_success() {
        return Ok(fail(format!("{base} answered with HTTP {}. It does not look like a ufop-server.", status.as_u16()), false, ms));
    }
    let body: serde_json::Value = match resp.json().await {
        Ok(b) => b,
        Err(_) => return Ok(fail(format!("{base} answered, but not like a ufop-server."), false, ms)),
    };
    if body.get("status").and_then(|s| s.as_str()) != Some("ok") {
        return Ok(fail(format!("{base} answered, but not like a ufop-server."), false, ms));
    }
    let version = body.get("version").and_then(|v| v.as_str()).map(String::from);
    Ok(ProbeResult {
        ok: true,
        message: format!("Connected to a ufop-server{} in {ms} ms.", version.as_deref().map(|v| format!(" {v}")).unwrap_or_default()),
        version,
        uptime_secs: body.get("uptime_secs").and_then(|u| u.as_u64()),
        latency_ms: ms,
        untrusted_certificate: false,
    })
}

/// Test a server address: does it answer, and is it a `ufop-server`?
#[tauri::command]
pub async fn server_profile_test(server_url: String, accept_untrusted_certificate: Option<bool>) -> Result<ProbeResult, AppError> {
    probe(&server_url, accept_untrusted_certificate.unwrap_or(false)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup_engine::BackupManager;
    use crate::connectors::ConnectionManager;
    use crate::security::settings_store::SettingsStore;
    use crate::security::CredentialStore;
    use crate::storage::Repository;
    use std::sync::Arc;
    use ufop_core::backup::crypto::KdfParams;

    async fn running_server(dir: &std::path::Path) -> (EmbeddedServer, u16) {
        let repo = Repository::open_in_memory().await.unwrap();
        let creds = Arc::new(CredentialStore::new());
        let mgr = BackupManager::with_kdf(repo.pool().clone(), creds.clone(), None, None, dir.join("dbs"), KdfParams::insecure_fast());
        let s = EmbeddedServer::new(repo.clone(), mgr, ConnectionManager::new(repo.clone(), creds), SettingsStore::plain(repo.pool().clone()));
        let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        s.save_settings(EmbeddedSettings { port, ..Default::default() }).await.unwrap();
        s.set_password("correct horse battery").await.unwrap();
        s.start().await.unwrap();
        (s, port)
    }

    #[tokio::test]
    async fn a_ufop_server_passes_the_connection_test_and_says_what_it_is() {
        let d = tempfile::tempdir().unwrap();
        let (s, port) = running_server(d.path()).await;
        let r = probe(&format!("http://127.0.0.1:{port}/"), false).await.unwrap();
        assert!(r.ok, "{}", r.message);
        assert_eq!(r.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
        assert!(r.uptime_secs.is_some() && r.message.contains("ufop-server"));
        s.stop().await.unwrap();

        // Once stopped, the same address fails with an explanation, not a pass.
        let r = probe(&format!("http://127.0.0.1:{port}"), false).await.unwrap();
        assert!(!r.ok && r.message.contains("Could not connect"), "{}", r.message);
    }

    #[tokio::test]
    async fn something_else_on_the_address_is_not_called_a_ufop_server() {
        // A tiny web server that answers every request with a page and 200.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = sock.read(&mut buf).await;
                    let body = "<html>hello</html>";
                    let _ = sock.write_all(format!("HTTP/1.1 200 OK\r\ncontent-type: text/html\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
                });
            }
        });
        let r = probe(&format!("http://127.0.0.1:{port}"), false).await.unwrap();
        assert!(!r.ok && r.message.contains("not like a ufop-server"), "{}", r.message);
    }

    #[tokio::test]
    async fn bad_addresses_are_refused_before_any_request() {
        for url in ["", "ftp://x", "not a url", "http://user:pw@host"] {
            assert!(probe(url, false).await.is_err(), "{url:?}");
        }
    }

    #[tokio::test]
    async fn the_server_list_is_the_cli_config_file_without_tokens() {
        let d = tempfile::tempdir().unwrap();
        let file = d.path().join("ufop").join("config.toml");
        std::env::set_var("UFOP_CONFIG", &file);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "server_token = \"abc\"\nserver_url = \"http://main:8200\"\n").unwrap();

        let rows = server_profile_save(" office ".into(), "https://files.example.com".into()).await.unwrap();
        std::env::remove_var("UFOP_CONFIG");

        assert_eq!(rows.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(), ["default", "office"]);
        assert!(rows[0].signed_in && !rows[1].signed_in);
        assert!(!serde_json::to_string(&rows).unwrap().contains("abc"));
    }
}
