//! Startup and tray behaviour (Settings → Startup & Tray): what the window
//! does at launch, on minimise and on close, and whether the tray icon shows.
//!
//! "Keep running in the tray when the window is closed" is the existing
//! `app.run_in_background` setting ([`crate::tray::BackgroundMode`], also on
//! the Backups screen) and start-at-login is the autostart plugin's own
//! state; the rest is stored as JSON under `app.startup`. Without a tray icon
//! nothing may hide the window, or the app would run with no way back:
//! [`effective`] turns those choices into minimising (or closing) instead.

use crate::storage::Repository;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};

pub const KEY_STARTUP: &str = "app.startup";

/// The stored choices (start-at-login and close-to-tray live elsewhere, see the module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StartupPrefs {
    /// Every launch starts in the tray (or minimised without a tray icon).
    pub start_minimized: bool,
    /// Launched at login: start in the tray instead of opening the window.
    pub start_hidden_at_login: bool,
    /// Minimising the window hides it to the tray.
    pub minimize_to_tray: bool,
    /// Show the tray icon.
    pub show_tray_icon: bool,
}

impl Default for StartupPrefs {
    fn default() -> Self {
        // As before this setting existed: hidden at login, tray shown.
        Self { start_minimized: false, start_hidden_at_login: true, minimize_to_tray: false, show_tray_icon: true }
    }
}

/// Everything Settings → Startup & Tray shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StartupSettings {
    pub start_at_login: bool,
    pub start_minimized: bool,
    pub start_hidden_at_login: bool,
    pub close_to_tray: bool,
    pub minimize_to_tray: bool,
    pub show_tray_icon: bool,
}

impl StartupSettings {
    pub fn new(prefs: StartupPrefs, start_at_login: bool, close_to_tray: bool) -> Self {
        Self {
            start_at_login,
            start_minimized: prefs.start_minimized,
            start_hidden_at_login: prefs.start_hidden_at_login,
            close_to_tray,
            minimize_to_tray: prefs.minimize_to_tray,
            show_tray_icon: prefs.show_tray_icon,
        }
    }

    pub fn prefs(&self) -> StartupPrefs {
        StartupPrefs { start_minimized: self.start_minimized, start_hidden_at_login: self.start_hidden_at_login, minimize_to_tray: self.minimize_to_tray, show_tray_icon: self.show_tray_icon }
    }
}

/// What the main window does when the app starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchWindow {
    Show,
    /// Hidden: only the tray icon.
    Hide,
    /// In the taskbar / Dock, not on screen.
    Minimize,
}

/// The behaviour the choices amount to, given whether the tray icon is
/// actually there (`tray_available`: shown and created without error).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Effective {
    pub close_to_tray: bool,
    pub minimize_to_tray: bool,
    pub launch: LaunchWindow,
}

pub fn effective(s: &StartupSettings, launched_at_login: bool, tray_available: bool) -> Effective {
    let tray = s.show_tray_icon && tray_available;
    let start_out_of_sight = s.start_minimized || (launched_at_login && s.start_hidden_at_login);
    let launch = match (start_out_of_sight, tray) {
        (false, _) => LaunchWindow::Show,
        (true, true) => LaunchWindow::Hide,
        (true, false) => LaunchWindow::Minimize,
    };
    Effective { close_to_tray: s.close_to_tray && tray, minimize_to_tray: s.minimize_to_tray && tray, launch }
}

/// The stored choices and whether the tray icon exists, shared with the
/// window event handler.
#[derive(Clone, Default)]
pub struct StartupState {
    prefs: Arc<RwLock<StartupPrefs>>,
    /// The tray icon was created (set once at startup).
    tray_created: Arc<AtomicBool>,
}

impl StartupState {
    pub fn get(&self) -> StartupPrefs {
        self.prefs.read().map(|p| *p).unwrap_or_default()
    }

    pub fn set_tray_created(&self, created: bool) {
        self.tray_created.store(created, Ordering::SeqCst);
    }

    pub fn tray_created(&self) -> bool {
        self.tray_created.load(Ordering::SeqCst)
    }

    /// What closing and minimising do now (`close_to_tray` from [`crate::tray::BackgroundMode`]).
    pub fn effective(&self, close_to_tray: bool) -> Effective {
        effective(&StartupSettings::new(self.get(), false, close_to_tray), launched_at_login(), self.tray_created())
    }
}

pub async fn load(repo: &Repository) -> StartupState {
    let prefs = repo.get_config(KEY_STARTUP).await.ok().flatten().and_then(|j| serde_json::from_str::<StartupPrefs>(&j).ok()).unwrap_or_default();
    StartupState { prefs: Arc::new(RwLock::new(prefs)), tray_created: Arc::default() }
}

pub async fn save(repo: &Repository, state: &StartupState, prefs: StartupPrefs) -> Result<(), crate::core::error::AppError> {
    repo.set_config(KEY_STARTUP, &serde_json::to_string(&prefs)?).await?;
    if let Ok(mut p) = state.prefs.write() {
        *p = prefs;
    }
    Ok(())
}

/// Whether this process was started by the login item (autostart plugin).
pub fn launched_at_login() -> bool {
    std::env::args().any(|a| a == "--background")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings(f: impl FnOnce(&mut StartupSettings)) -> StartupSettings {
        let mut s = StartupSettings::new(StartupPrefs::default(), false, false);
        f(&mut s);
        s
    }

    #[test]
    fn defaults_keep_the_old_behaviour() {
        let s = settings(|_| {});
        assert_eq!(effective(&s, false, true), Effective { close_to_tray: false, minimize_to_tray: false, launch: LaunchWindow::Show });
        // Launched at login: hidden in the tray, as `--background` always did.
        assert_eq!(effective(&s, true, true).launch, LaunchWindow::Hide);
    }

    #[test]
    fn start_minimized_hides_every_launch_and_login_can_open_the_window() {
        let s = settings(|s| s.start_minimized = true);
        assert_eq!(effective(&s, false, true).launch, LaunchWindow::Hide);
        let s = settings(|s| s.start_hidden_at_login = false);
        assert_eq!(effective(&s, true, true).launch, LaunchWindow::Show);
    }

    #[test]
    fn without_a_tray_icon_nothing_hides_the_window() {
        let s = settings(|s| {
            s.show_tray_icon = false;
            s.close_to_tray = true;
            s.minimize_to_tray = true;
            s.start_minimized = true;
        });
        assert_eq!(effective(&s, true, true), Effective { close_to_tray: false, minimize_to_tray: false, launch: LaunchWindow::Minimize });
        // The same when the tray could not be created (some Linux desktops).
        let s = settings(|s| {
            s.close_to_tray = true;
            s.minimize_to_tray = true;
        });
        let e = effective(&s, true, false);
        assert!(!e.close_to_tray && !e.minimize_to_tray);
        assert_eq!(e.launch, LaunchWindow::Minimize);
        let e = effective(&s, false, true);
        assert!(e.close_to_tray && e.minimize_to_tray);
    }

    #[test]
    fn stored_prefs_tolerate_missing_and_extra_fields() {
        let p: StartupPrefs = serde_json::from_str(r#"{"minimize_to_tray":true,"later":1}"#).unwrap();
        assert!(p.minimize_to_tray && p.show_tray_icon && p.start_hidden_at_login && !p.start_minimized);
    }

    #[tokio::test]
    async fn prefs_persist() {
        let repo = Repository::open_in_memory().await.unwrap();
        let state = load(&repo).await;
        assert_eq!(state.get(), StartupPrefs::default());
        let prefs = StartupPrefs { start_minimized: true, start_hidden_at_login: false, minimize_to_tray: true, show_tray_icon: false };
        save(&repo, &state, prefs).await.unwrap();
        assert_eq!(state.get(), prefs);
        assert_eq!(load(&repo).await.get(), prefs);
    }
}
