//! Management console ("Managed by console") commands for the settings UI.
//! The protocol and loop live in `ufop_core::agent`; see `fleet_agent`.

use crate::core::error::AppError;
use crate::fleet_agent::FleetAgent;
use tauri::State;
use crate::fleet_agent::loopback::{self, Loopback};
use serde::Serialize;
use std::sync::{LazyLock, Mutex};
use std::time::Duration;
use ufop_core::agent::{parse_tags, AgentStatus, ClaimPollResult, ClaimStatus};

/// How long the browser sign-in may take.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// A browser sign-in in progress (one at a time).
struct PendingConnect {
    console_url: String,
    loopback: Loopback,
    state: String,
    verifier: String,
}

static PENDING_CONNECT: LazyLock<Mutex<Option<PendingConnect>>> = LazyLock::new(|| Mutex::new(None));
static CANCEL_CONNECT: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

fn pending() -> std::sync::MutexGuard<'static, Option<PendingConnect>> {
    PENDING_CONNECT.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Serialize)]
pub struct ConnectStart {
    /// The console page to open (shown too, in case the browser did not open).
    pub url: String,
    pub browser_opened: bool,
}

#[tauri::command]
pub async fn fleet_agent_status(agent: State<'_, FleetAgent>) -> Result<AgentStatus, AppError> {
    Ok(agent.status())
}

/// Register with a registration link token (`ufop_reg_…`). `tags` is comma separated.
#[tauri::command]
pub async fn fleet_agent_register(agent: State<'_, FleetAgent>, console_url: String, registration_token: String, tags: Option<String>) -> Result<AgentStatus, AppError> {
    agent.register(&console_url, &registration_token, tags.as_deref().map(parse_tags).unwrap_or_default()).await
}

/// Start a browser sign-in: listen on a loopback port and open the console's
/// approval page. Finish with `fleet_agent_connect_finish`.
#[tauri::command]
pub async fn fleet_agent_connect_start(app: tauri::AppHandle, agent: State<'_, FleetAgent>, console_url: String) -> Result<ConnectStart, AppError> {
    let lb = Loopback::bind().await?;
    let (state, pkce) = (loopback::new_state(), loopback::pkce());
    let url = agent.connect_url(&console_url, &lb.redirect_uri, &state, &pkce.challenge)?;
    *pending() = Some(PendingConnect { console_url, loopback: lb, state, verifier: pkce.verifier });
    #[allow(deprecated)]
    let browser_opened = {
        use tauri_plugin_shell::ShellExt;
        app.shell().open(&url, None).map_err(|e| tracing::warn!("Could not open the browser: {e}")).is_ok()
    };
    Ok(ConnectStart { url, browser_opened })
}

/// Wait for the browser to come back, then register with the returned code.
#[tauri::command]
pub async fn fleet_agent_connect_finish(agent: State<'_, FleetAgent>, tags: Option<String>) -> Result<AgentStatus, AppError> {
    let PendingConnect { console_url, loopback, state, verifier } =
        pending().take().ok_or_else(|| AppError::state("No browser sign-in is in progress", "Start the sign-in again."))?;
    let redirect_uri = loopback.redirect_uri.clone();
    let code = tokio::select! {
        r = loopback.wait_for_code(&state, CONNECT_TIMEOUT) => r?,
        _ = CANCEL_CONNECT.notified() => return Err(AppError::state("The browser sign-in was cancelled", "Start it again when you are ready.")),
    };
    agent.register_with_code(&console_url, &code, &verifier, &redirect_uri, tags.as_deref().map(parse_tags).unwrap_or_default()).await
}

#[tauri::command]
pub async fn fleet_agent_connect_cancel() -> Result<(), AppError> {
    pending().take();
    CANCEL_CONNECT.notify_waiters();
    Ok(())
}

/// Start claiming: returns the code the administrator enters in the console.
#[tauri::command]
pub async fn fleet_agent_start_claim(agent: State<'_, FleetAgent>, console_url: String) -> Result<ClaimStatus, AppError> {
    agent.start_claim(&console_url).await
}

/// `pending` until the administrator entered the code, then `registered`.
#[tauri::command]
pub async fn fleet_agent_poll_claim(agent: State<'_, FleetAgent>) -> Result<ClaimPollResult, AppError> {
    agent.poll_claim().await
}

#[tauri::command]
pub async fn fleet_agent_cancel_claim(agent: State<'_, FleetAgent>) -> Result<AgentStatus, AppError> {
    agent.cancel_claim();
    Ok(agent.status())
}

/// Leave the console; `force` also when it cannot be reached.
#[tauri::command]
pub async fn fleet_agent_unregister(agent: State<'_, FleetAgent>, force: Option<bool>) -> Result<AgentStatus, AppError> {
    agent.unregister(force.unwrap_or(false)).await
}

#[tauri::command]
pub async fn fleet_agent_set_enabled(agent: State<'_, FleetAgent>, enabled: bool) -> Result<AgentStatus, AppError> {
    Ok(agent.set_enabled(enabled).await)
}

/// Heartbeat, commands and policy sync now.
#[tauri::command]
pub async fn fleet_agent_sync_now(agent: State<'_, FleetAgent>) -> Result<AgentStatus, AppError> {
    agent.sync_now().await
}
