//! System tray: shows backup state in the tooltip and a status line, and
//! offers Open, Backups…, Pause for an hour, Resume and Quit. With
//! "Keep running in the tray" enabled, closing the window hides it so
//! scheduled backups keep running.

use crate::backup_engine::BackupManager;
use crate::storage::Repository;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

pub const OPEN_BACKUPS_EVENT: &str = "ufop://open-backups";
const KEY_BACKGROUND: &str = "app.run_in_background";

/// Whether closing the window should hide it instead of exiting.
#[derive(Clone, Default)]
pub struct BackgroundMode(pub Arc<AtomicBool>);

impl BackgroundMode {
    pub fn enabled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

pub async fn load_background_mode(repo: &Repository) -> BackgroundMode {
    let on = repo.get_config(KEY_BACKGROUND).await.ok().flatten().as_deref() == Some("1");
    BackgroundMode(Arc::new(AtomicBool::new(on)))
}

pub async fn save_background_mode(repo: &Repository, mode: &BackgroundMode, on: bool) -> Result<(), crate::core::error::AppError> {
    mode.0.store(on, Ordering::SeqCst);
    repo.set_config(KEY_BACKGROUND, if on { "1" } else { "0" }).await
}

/// Human-readable one-line state for the tray.
pub fn describe(status: &crate::backup_engine::SchedulerStatus) -> String {
    if let Some(t) = status.tasks.iter().find(|t| t.state == "running") {
        let pct = t.progress.as_ref().filter(|p| p.files_total > 0).map(|p| format!(" {}%", p.files_processed * 100 / p.files_total)).unwrap_or_default();
        return format!("Running {} of {}{pct}", t.operation, t.job_name);
    }
    if status.paused {
        return match &status.paused_until {
            Some(u) => format!("Backups paused until {}", chrono::DateTime::parse_from_rfc3339(u).map(|d| d.with_timezone(&chrono::Local).format("%H:%M").to_string()).unwrap_or_default()),
            None => "Backups paused".into(),
        };
    }
    match status.next_runs.first() {
        Some(n) => format!(
            "Next backup: {} at {}",
            n.job_name,
            chrono::DateTime::parse_from_rfc3339(&n.next_run_at).map(|d| d.with_timezone(&chrono::Local).format("%a %H:%M").to_string()).unwrap_or_default()
        ),
        None => "No scheduled backups".into(),
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.webview_windows().values().next() {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn setup(app: &tauri::App, mgr: BackupManager) -> tauri::Result<()> {
    let status_item = MenuItem::with_id(app, "status", "Loading…", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", "Open UFOP", true, None::<&str>)?;
    let backups = MenuItem::with_id(app, "backups", "Backups…", true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", "Pause backups for 1 hour", true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume", "Resume backups", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&status_item, &PredefinedMenuItem::separator(app)?, &open, &backups, &PredefinedMenuItem::separator(app)?, &pause, &resume, &PredefinedMenuItem::separator(app)?, &quit],
    )?;
    let m2 = mgr.clone();
    let mut builder = TrayIconBuilder::with_id("ufop").tooltip("UFOP").menu(&menu).show_menu_on_left_click(true).on_menu_event(move |app, ev| match ev.id().as_ref() {
        "open" => show_main(app),
        "backups" => {
            show_main(app);
            let _ = app.emit(OPEN_BACKUPS_EVENT, ());
        }
        "pause" => m2.pause(Some(60)),
        "resume" => m2.resume(),
        "quit" => app.exit(0),
        _ => {}
    });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = builder.build(app)?;

    // Refresh the status line and tooltip.
    tauri::async_runtime::spawn(async move {
        let mut last = String::new();
        loop {
            let s = describe(&mgr.status().await);
            if s != last {
                let _ = tray.set_tooltip(Some(format!("UFOP — {s}")));
                let _ = status_item.set_text(&s);
                let paused = mgr.status().await.paused;
                let _ = pause.set_enabled(!paused);
                let _ = resume.set_enabled(paused);
                last = s;
            }
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backup_engine::{NextRun, SchedulerStatus, TaskInfo};

    #[test]
    fn describes_states() {
        let mut s = SchedulerStatus { running: true, paused: false, paused_until: None, tasks: vec![], next_runs: vec![] };
        assert_eq!(describe(&s), "No scheduled backups");
        s.next_runs.push(NextRun { job_id: "1".into(), job_name: "Docs".into(), next_run_at: "2026-03-10T02:00:00Z".into() });
        assert!(describe(&s).starts_with("Next backup: Docs at "));
        s.paused = true;
        assert_eq!(describe(&s), "Backups paused");
        s.tasks.push(TaskInfo {
            task_id: "t".into(),
            job_id: "1".into(),
            job_name: "Docs".into(),
            operation: "backup".into(),
            state: "running".into(),
            queued_at: "".into(),
            started_at: None,
            progress: Some(ufop_core::backup::Progress { phase: ufop_core::backup::Phase::Uploading, files_processed: 5, files_total: 10, bytes_processed: 0, current_path: None }),
            elapsed_secs: 0,
            bytes_per_sec: 0,
            eta_secs: None,
        });
        assert_eq!(describe(&s), "Running backup of Docs 50%");
    }

    #[tokio::test]
    async fn background_mode_persists() {
        let repo = Repository::open_in_memory().await.unwrap();
        let m = load_background_mode(&repo).await;
        assert!(!m.enabled());
        save_background_mode(&repo, &m, true).await.unwrap();
        assert!(load_background_mode(&repo).await.enabled());
    }
}
