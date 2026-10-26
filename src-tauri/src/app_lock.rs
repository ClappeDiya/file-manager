//! App lock (Settings → App Lock): the windows show a lock screen until the
//! master password is entered — at start, after some minutes without use,
//! or on demand (palette, tray).
//!
//! The lock is about the windows, not the work: scheduled backups, syncs,
//! automations and transfers keep running, and the master key used for
//! saved credentials stays available to them. While locked, the IPC gate
//! ([`gate`]) refuses every command but the few the lock screen needs, so a
//! window (or anything injected into one) cannot read or change anything.
//! Passwords are checked by [`crate::security::MasterPasswordManager`], with
//! its attempt limit.

use crate::core::error::AppError;
use crate::storage::Repository;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SETTINGS_KEY: &str = "app.lock";
/// Sent to every window when the app locks or unlocks: `{ "locked": bool }`.
pub const LOCK_EVENT: &str = "ufop://app-lock";

/// What the lock does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppLockSettings {
    pub enabled: bool,
    /// Locked when the app starts.
    pub lock_on_start: bool,
    /// Lock after this many minutes without use (0: never).
    pub idle_minutes: u32,
}

impl Default for AppLockSettings {
    fn default() -> Self {
        Self { enabled: false, lock_on_start: true, idle_minutes: 0 }
    }
}

/// The idle timeouts offered (minutes; 0 is "never").
pub const IDLE_CHOICES: [u32; 7] = [0, 1, 5, 10, 15, 30, 60];

/// What the lock screen and Settings show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppLockStatus {
    pub enabled: bool,
    pub locked: bool,
    pub lock_on_start: bool,
    pub idle_minutes: u32,
    /// A master password exists (needed before the lock can be turned on).
    pub has_password: bool,
}

/// Commands a locked window may still call: what the lock screen needs.
pub const ALLOWED_WHILE_LOCKED: &[&str] = &["app_lock_status", "app_lock_unlock", "set_ui_language", "get_app_version", "get_portable_status", "greet"];

#[derive(Clone, Default)]
pub struct AppLock {
    settings: Arc<RwLock<AppLockSettings>>,
    locked: Arc<AtomicBool>,
    /// Seconds since the epoch of the last use reported by a window.
    last_activity: Arc<AtomicU64>,
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

impl AppLock {
    /// Load the settings; locked at once when the lock is on and set to lock at start.
    pub async fn load(repo: &Repository, has_password: bool) -> Self {
        let settings = repo.get_config(SETTINGS_KEY).await.ok().flatten().and_then(|j| serde_json::from_str::<AppLockSettings>(&j).ok()).unwrap_or_default();
        let lock = Self { settings: Arc::new(RwLock::new(settings)), locked: Arc::default(), last_activity: Arc::new(AtomicU64::new(now_secs())) };
        lock.locked.store(settings.enabled && settings.lock_on_start && has_password, Ordering::SeqCst);
        lock
    }

    pub fn settings(&self) -> AppLockSettings {
        self.settings.read().map(|s| *s).unwrap_or_default()
    }

    pub fn is_locked(&self) -> bool {
        self.locked.load(Ordering::SeqCst)
    }

    pub fn status(&self, has_password: bool) -> AppLockStatus {
        let s = self.settings();
        AppLockStatus { enabled: s.enabled, locked: self.is_locked(), lock_on_start: s.lock_on_start, idle_minutes: s.idle_minutes, has_password }
    }

    /// Lock now; `false` when the lock is off (nothing to unlock with).
    pub fn lock(&self) -> bool {
        if !self.settings().enabled {
            return false;
        }
        self.locked.store(true, Ordering::SeqCst);
        true
    }

    pub fn unlock(&self) {
        self.locked.store(false, Ordering::SeqCst);
        self.touch();
    }

    /// A window was used.
    pub fn touch(&self) {
        self.last_activity.store(now_secs(), Ordering::SeqCst);
    }

    /// Whether the idle time has run out at `now` (seconds since the epoch).
    pub fn idle_expired(&self, now: u64) -> bool {
        let s = self.settings();
        s.enabled && s.idle_minutes > 0 && !self.is_locked() && now.saturating_sub(self.last_activity.load(Ordering::SeqCst)) >= u64::from(s.idle_minutes) * 60
    }

    /// Store new settings. Turning the lock off also unlocks.
    pub async fn save(&self, repo: &Repository, settings: AppLockSettings) -> Result<(), AppError> {
        if !IDLE_CHOICES.contains(&settings.idle_minutes) {
            return Err(AppError::validation(format!("Lock after {} minutes is not offered; choose one of {:?}.", settings.idle_minutes, IDLE_CHOICES)));
        }
        repo.set_config(SETTINGS_KEY, &serde_json::to_string(&settings)?).await?;
        if let Ok(mut s) = self.settings.write() {
            *s = settings;
        }
        if !settings.enabled {
            self.unlock();
        }
        self.touch();
        Ok(())
    }

    /// Whether the IPC gate refuses `command` now.
    pub fn blocks(&self, command: &str) -> bool {
        self.is_locked() && !ALLOWED_WHILE_LOCKED.contains(&command)
    }
}

/// What a refused command answers (the shape of an `AppError`).
pub fn locked_error() -> serde_json::Value {
    serde_json::json!({
        "error_type": "app_locked",
        "message": "UFOP is locked",
        "advice": "Enter your master password on the lock screen to unlock.",
    })
}

/// Wrap the app's command handler: while locked, only
/// [`ALLOWED_WHILE_LOCKED`] commands run.
pub fn gate<R: tauri::Runtime, F>(lock: AppLock, inner: F) -> impl Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static
where
    F: Fn(tauri::ipc::Invoke<R>) -> bool + Send + Sync + 'static,
{
    move |invoke: tauri::ipc::Invoke<R>| {
        if lock.blocks(invoke.message.command()) {
            invoke.resolver.reject(locked_error());
            return true;
        }
        inner(invoke)
    }
}

/// Tell every window.
pub fn announce(app: &tauri::AppHandle, locked: bool) {
    use tauri::Emitter;
    let _ = app.emit(LOCK_EVENT, serde_json::json!({ "locked": locked }));
    crate::tray::set_lock_enabled(app);
}

/// Lock after the idle time: checked every 15 seconds.
pub fn spawn_idle_watch(app: tauri::AppHandle, lock: AppLock) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
            if lock.idle_expired(now_secs()) && lock.lock() {
                tracing::info!("App locked after {} idle minute(s)", lock.settings().idle_minutes);
                announce(&app, true);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn repo() -> Repository {
        Repository::open_in_memory().await.unwrap()
    }

    #[tokio::test]
    async fn starts_locked_only_when_on_set_to_lock_at_start_and_a_password_exists() {
        let r = repo().await;
        assert!(!AppLock::load(&r, true).await.is_locked());
        let lock = AppLock::load(&r, true).await;
        lock.save(&r, AppLockSettings { enabled: true, lock_on_start: true, idle_minutes: 5 }).await.unwrap();
        assert!(AppLock::load(&r, true).await.is_locked());
        // No password to unlock with: never locked (the app would be unusable).
        assert!(!AppLock::load(&r, false).await.is_locked());
        lock.save(&r, AppLockSettings { enabled: true, lock_on_start: false, idle_minutes: 5 }).await.unwrap();
        assert!(!AppLock::load(&r, true).await.is_locked());
    }

    #[tokio::test]
    async fn the_gate_lets_only_the_lock_screen_through_while_locked() {
        let r = repo().await;
        let lock = AppLock::load(&r, true).await;
        assert!(!lock.lock(), "an app with the lock off cannot be locked");
        assert!(!lock.blocks("list_directory"));
        lock.save(&r, AppLockSettings { enabled: true, ..Default::default() }).await.unwrap();
        assert!(lock.lock());
        assert!(lock.blocks("list_directory"));
        assert!(lock.blocks("export_settings"));
        assert!(lock.blocks("app_lock_set_settings"));
        assert!(!lock.blocks("app_lock_unlock"));
        assert!(!lock.blocks("app_lock_status"));
        lock.unlock();
        assert!(!lock.blocks("list_directory"));
        assert_eq!(locked_error()["error_type"], "app_locked");
    }

    #[tokio::test]
    async fn idle_time_counts_from_the_last_use() {
        let r = repo().await;
        let lock = AppLock::load(&r, true).await;
        lock.save(&r, AppLockSettings { enabled: true, lock_on_start: false, idle_minutes: 5 }).await.unwrap();
        let t0 = now_secs();
        assert!(!lock.idle_expired(t0 + 4 * 60));
        assert!(lock.idle_expired(t0 + 5 * 60 + 1));
        lock.touch();
        assert!(!lock.idle_expired(now_secs() + 60));
        // Already locked, or no idle limit: nothing to do.
        lock.lock();
        assert!(!lock.idle_expired(t0 + 3600));
        lock.save(&r, AppLockSettings { enabled: true, lock_on_start: false, idle_minutes: 0 }).await.unwrap();
        lock.unlock();
        assert!(!lock.idle_expired(t0 + 100_000));
    }

    #[tauri::command]
    fn list_directory() -> &'static str {
        "listing"
    }

    #[tauri::command]
    fn app_lock_status() -> &'static str {
        "status"
    }

    /// Through the real IPC path: what a window gets back.
    #[test]
    fn a_locked_app_answers_commands_with_app_locked_through_ipc() {
        let lock = AppLock::default();
        let app = tauri::test::mock_builder()
            .invoke_handler(gate(lock.clone(), tauri::generate_handler![list_directory, app_lock_status]))
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        let webview = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        let call = |cmd: &str| {
            tauri::test::get_ipc_response(
                &webview,
                tauri::webview::InvokeRequest {
                    cmd: cmd.into(),
                    callback: tauri::ipc::CallbackFn(0),
                    error: tauri::ipc::CallbackFn(1),
                    url: "http://tauri.localhost".parse().unwrap(),
                    body: tauri::ipc::InvokeBody::default(),
                    headers: Default::default(),
                    invoke_key: tauri::test::INVOKE_KEY.to_string(),
                },
            )
            .map(|b| b.deserialize::<String>().unwrap())
        };
        assert_eq!(call("list_directory"), Ok("listing".to_string()));
        if let Ok(mut s) = lock.settings.write() {
            s.enabled = true;
        }
        assert!(lock.lock());
        assert_eq!(call("list_directory"), Err(locked_error()));
        assert_eq!(call("app_lock_status"), Ok("status".to_string()));
        lock.unlock();
        assert_eq!(call("list_directory"), Ok("listing".to_string()));
    }

    #[tokio::test]
    async fn turning_the_lock_off_unlocks_and_odd_timeouts_are_refused() {
        let r = repo().await;
        let lock = AppLock::load(&r, true).await;
        lock.save(&r, AppLockSettings { enabled: true, ..Default::default() }).await.unwrap();
        lock.lock();
        lock.save(&r, AppLockSettings { enabled: false, ..Default::default() }).await.unwrap();
        assert!(!lock.is_locked());
        assert!(lock.save(&r, AppLockSettings { enabled: true, lock_on_start: true, idle_minutes: 7 }).await.is_err());
        assert!(!lock.settings().enabled, "a refused change keeps the old settings");
    }
}
