//! System tray: shows backup state (and the next scheduled sync or
//! automation) in the tooltip and a status line, and
//! offers Open, Backups…, Pause for an hour, Resume and Quit. With
//! "Keep running in the tray" enabled, closing the window hides it so
//! scheduled backups keep running. A "Mounts" submenu lists the saved mounts
//! with a check mark on the mounted ones (click to mount or unmount) and
//! "Unmount all"; it is rebuilt whenever a mount changes.

use crate::backup_engine::BackupManager;
use crate::commands::mount_commands::mount_saved_and_open;
use crate::connectors::ConnectionManager;
use crate::mount_engine::{MountInfo, MountManager, MountStatus};
use crate::storage::Repository;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use uuid::Uuid;

pub const OPEN_BACKUPS_EVENT: &str = "ufop://open-backups";
pub const KEY_BACKGROUND: &str = "app.run_in_background";

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

// ──────────────────────────────────────────────
// Mounts submenu
// ──────────────────────────────────────────────

const MOUNT_ITEM_PREFIX: &str = "mount:";
const UNMOUNT_ALL_ID: &str = "unmount_all";

/// One saved mount in the tray: a check item, checked while mounted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MountMenuItem {
    pub id: String,
    pub label: String,
    pub mounted: bool,
}

/// What the "Mounts" submenu shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MountsMenu {
    pub items: Vec<MountMenuItem>,
    pub can_unmount_all: bool,
}

/// What a click in the "Mounts" submenu asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MountAction {
    /// Mount a saved mount, or unmount it if it is mounted.
    Toggle(Uuid),
    UnmountAll,
}

/// The submenu for the current mounts (already in the order the list shows).
pub fn mounts_menu(mounts: &[MountInfo]) -> MountsMenu {
    MountsMenu {
        items: mounts
            .iter()
            .map(|m| {
                let state = if m.status == MountStatus::Error { " (failed)" } else { "" };
                MountMenuItem { id: format!("{MOUNT_ITEM_PREFIX}{}", m.id), label: format!("{} — {}{state}", m.connection_name, m.mount_point), mounted: m.status == MountStatus::Mounted }
            })
            .collect(),
        can_unmount_all: mounts.iter().any(|m| m.status == MountStatus::Mounted),
    }
}

/// The action behind a menu item id (`None` for ids that are not mount items).
pub fn parse_mount_action(id: &str) -> Option<MountAction> {
    if id == UNMOUNT_ALL_ID {
        return Some(MountAction::UnmountAll);
    }
    id.strip_prefix(MOUNT_ITEM_PREFIX).and_then(|u| Uuid::parse_str(u).ok()).map(MountAction::Toggle)
}

/// Do what a click asked for; the text says what happened.
pub async fn run_mount_action(action: MountAction, mounts: &MountManager, repo: &Repository, conns: &ConnectionManager) -> Result<String, crate::core::error::AppError> {
    match action {
        MountAction::UnmountAll => {
            let r = mounts.unmount_all(repo).await;
            if r.failed.is_empty() {
                Ok(format!("Unmounted {} drive(s)", r.unmounted))
            } else {
                Err(crate::core::error::AppError::file_op(format!("Could not unmount: {}", r.failed.join("; ")), "Close files that are open on the drive and try again."))
            }
        }
        MountAction::Toggle(id) => {
            if mounts.get_mount_status(id).await == MountStatus::Mounted {
                mounts.unmount_remote(id, repo).await?;
                Ok("Unmounted".into())
            } else {
                Ok(mount_saved_and_open(mounts, id, repo, conns).await?.message)
            }
        }
    }
}

fn rebuild_mounts_submenu(app: &AppHandle, sub: &Submenu<tauri::Wry>, model: &MountsMenu) -> tauri::Result<()> {
    for item in sub.items()? {
        sub.remove(&item)?;
    }
    let text = crate::tray_labels::current();
    if model.items.is_empty() {
        sub.append(&MenuItem::with_id(app, "mounts_none", text.no_mounts, false, None::<&str>)?)?;
    }
    for it in &model.items {
        sub.append(&CheckMenuItem::with_id(app, &it.id, &it.label, true, it.mounted, None::<&str>)?)?;
    }
    sub.append(&PredefinedMenuItem::separator(app)?)?;
    sub.append(&MenuItem::with_id(app, UNMOUNT_ALL_ID, text.unmount_all, model.can_unmount_all, None::<&str>)?)?;
    Ok(())
}

/// Keep the submenu in step with the mounts: rebuilt on every change, and
/// every few seconds for drives that vanish without telling us.
fn spawn_mounts_menu(app: AppHandle, sub: Submenu<tauri::Wry>) {
    tauri::async_runtime::spawn(async move {
        let (mounts, repo, conns) = (app.state::<MountManager>().inner().clone(), app.state::<Repository>().inner().clone(), app.state::<ConnectionManager>().inner().clone());
        let mut changes = mounts.subscribe();
        let mut shown: Option<MountsMenu> = None;
        let mut redraw = true;
        loop {
            let model = mounts_menu(&mounts.list_mounts(&repo, &conns).await.unwrap_or_default());
            if redraw || shown.as_ref() != Some(&model) {
                if let Err(e) = rebuild_mounts_submenu(&app, &sub, &model) {
                    tracing::warn!("Tray mounts menu: {e}");
                }
                shown = Some(model);
            }
            // A change redraws even when the list looks the same (a click flips a check mark by itself).
            redraw = tokio::select! {
                r = changes.changed() => r.is_ok(),
                _ = tokio::time::sleep(std::time::Duration::from_secs(5)) => false,
            };
        }
    });
}

fn handle_mount_click(app: &AppHandle, action: MountAction) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let (mounts, repo, conns) = (app.state::<MountManager>(), app.state::<Repository>(), app.state::<ConnectionManager>());
        match run_mount_action(action, &mounts, &repo, &conns).await {
            Ok(msg) => tracing::info!("Tray mounts: {msg}"),
            Err(e) => {
                tracing::warn!("Tray mounts: {e}");
                // The clicked check mark flipped by itself; redraw the true state.
                mounts.notify();
                let _ = app.notification().builder().title("Mount failed").body(e.user_message()).show();
            }
        }
    });
}

fn show_main(app: &AppHandle) {
    let windows = app.webview_windows();
    if let Some(w) = windows.get("main").or_else(|| windows.values().next()) {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// One log line for "Back up all now".
pub fn run_all_summary(r: &crate::backup_engine::runs::RunAllReport) -> String {
    let mut s = format!("Back up all now: {} backup(s) queued", r.started.len());
    if !r.skipped.is_empty() {
        let names: Vec<String> = r.skipped.iter().map(|k| format!("{} ({})", if k.job_name.is_empty() { &k.job_id } else { &k.job_name }, k.reason)).collect();
        s.push_str(&format!(", skipped {}", names.join(", ")));
    }
    s
}

/// The tray items whose text follows the app's language.
pub struct TrayMenuItems {
    open: MenuItem<tauri::Wry>,
    new_window: MenuItem<tauri::Wry>,
    backups: MenuItem<tauri::Wry>,
    run_all: MenuItem<tauri::Wry>,
    pause: MenuItem<tauri::Wry>,
    resume: MenuItem<tauri::Wry>,
    lock: MenuItem<tauri::Wry>,
    quit: MenuItem<tauri::Wry>,
    mounts: Submenu<tauri::Wry>,
}

impl TrayMenuItems {
    /// Apply the current language's texts (the mounts submenu is rebuilt by its own task).
    pub fn relabel(&self) {
        let t = crate::tray_labels::current();
        let _ = self.open.set_text(t.open);
        let _ = self.new_window.set_text(t.new_window);
        let _ = self.backups.set_text(t.backups);
        let _ = self.run_all.set_text(t.run_all);
        let _ = self.pause.set_text(t.pause);
        let _ = self.resume.set_text(t.resume);
        let _ = self.quit.set_text(t.quit);
        let _ = self.lock.set_text(t.lock);
        let _ = self.mounts.set_text(t.mounts);
    }
}

/// "Lock UFOP" is offered while the app lock is on and the app is unlocked.
pub fn set_lock_enabled(app: &AppHandle) {
    if let (Some(items), Some(lock)) = (app.try_state::<TrayMenuItems>(), app.try_state::<crate::app_lock::AppLock>()) {
        let _ = items.lock.set_enabled(lock.settings().enabled && !lock.is_locked());
    }
}

/// The web view's language changed: relabel the tray and its mounts submenu.
pub fn apply_language(app: &AppHandle) {
    if let Some(items) = app.try_state::<TrayMenuItems>() {
        items.relabel();
        app.state::<MountManager>().notify();
    }
}

pub fn setup(app: &tauri::App, mgr: BackupManager) -> tauri::Result<()> {
    let text = crate::tray_labels::current();
    let status_item = MenuItem::with_id(app, "status", "Loading…", false, None::<&str>)?;
    // Shown only while a sync pair or automation rule has a schedule.
    let next_item = MenuItem::with_id(app, "next_other", "", false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", text.open, true, None::<&str>)?;
    let new_window = MenuItem::with_id(app, "new_window", text.new_window, true, None::<&str>)?;
    let backups = MenuItem::with_id(app, "backups", text.backups, true, None::<&str>)?;
    let run_all = MenuItem::with_id(app, "run_all", text.run_all, true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", text.pause, true, None::<&str>)?;
    let resume = MenuItem::with_id(app, "resume", text.resume, true, None::<&str>)?;
    let lock = MenuItem::with_id(app, "lock", text.lock, false, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", text.quit, true, None::<&str>)?;
    let mounts_menu = Submenu::with_id(app, "mounts", text.mounts, true)?;
    rebuild_mounts_submenu(app.handle(), &mounts_menu, &MountsMenu::default())?;
    let menu = Menu::with_items(
        app,
        &[&status_item, &PredefinedMenuItem::separator(app)?, &open, &new_window, &backups, &mounts_menu, &PredefinedMenuItem::separator(app)?, &run_all, &pause, &resume, &PredefinedMenuItem::separator(app)?, &lock, &quit],
    )?;
    let m2 = mgr.clone();
    let mut builder = TrayIconBuilder::with_id("ufop").tooltip("UFOP").menu(&menu).show_menu_on_left_click(true).on_menu_event(move |app, ev| match ev.id().as_ref() {
        "open" => show_main(app),
        "new_window" => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                if let Err(e) = crate::commands::desktop_commands::open_new_window(None, app).await {
                    tracing::warn!("New window: {}", e.user_message());
                }
            });
        }
        "backups" => {
            show_main(app);
            let _ = app.emit_to("main", OPEN_BACKUPS_EVENT, ());
        }
        "run_all" => {
            let m = m2.clone();
            tauri::async_runtime::spawn(async move {
                match m.run_all(Default::default()).await {
                    Ok(r) => tracing::info!("{}", run_all_summary(&r)),
                    Err(e) => tracing::warn!("Back up all now: {e}"),
                }
            });
        }
        "pause" => m2.pause(Some(60)),
        "resume" => m2.resume(),
        "lock" => {
            let lock = app.state::<crate::app_lock::AppLock>();
            if lock.lock() {
                crate::app_lock::announce(app, true);
            }
        }
        "quit" => app.exit(0),
        other => {
            if let Some(action) = parse_mount_action(other) {
                handle_mount_click(app, action);
            }
        }
    });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    let tray = builder.build(app)?;
    app.manage(TrayMenuItems { open, new_window, backups, run_all, pause: pause.clone(), resume: resume.clone(), lock, quit, mounts: mounts_menu.clone() });
    set_lock_enabled(app.handle());
    spawn_mounts_menu(app.handle().clone(), mounts_menu);

    // Refresh the status lines and tooltip.
    let handle = app.handle().clone();
    tauri::async_runtime::spawn(async move {
        let mut last = (String::new(), None::<String>);
        let mut next_shown = false;
        loop {
            let status = mgr.status().await;
            let s = describe(&status);
            let upcoming = crate::schedule_library::upcoming::upcoming(&handle.state::<crate::sync_engine::SyncManager>(), &handle.state::<crate::automation_engine::AutomationManager>()).await;
            let other = crate::schedule_library::upcoming::describe(&upcoming);
            if (s.clone(), other.clone()) != last {
                let tip = match &other {
                    Some(o) => format!("UFOP — {s}\n{o}"),
                    None => format!("UFOP — {s}"),
                };
                let _ = tray.set_tooltip(Some(tip));
                let _ = status_item.set_text(&s);
                let _ = pause.set_enabled(!status.paused);
                let _ = resume.set_enabled(status.paused);
                match &other {
                    Some(o) => {
                        let _ = next_item.set_text(o);
                        if !next_shown {
                            next_shown = menu.insert(&next_item, 1).is_ok();
                        }
                    }
                    None if next_shown => next_shown = menu.remove(&next_item).is_err(),
                    None => {}
                }
                last = (s, other);
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
    fn run_all_summary_names_skipped_jobs() {
        use crate::backup_engine::runs::{RunAllReport, SkippedRun, StartedRun};
        let r = RunAllReport {
            started: vec![StartedRun { job_id: "1".into(), job_name: "Docs".into(), task_id: "t".into() }],
            skipped: vec![SkippedRun { job_id: "2".into(), job_name: "Archive".into(), reason: "read-only".into() }],
        };
        assert_eq!(run_all_summary(&r), "Back up all now: 1 backup(s) queued, skipped Archive (read-only)");
    }

    #[test]
    fn describes_states() {
        let mut s = SchedulerStatus { running: true, paused: false, paused_until: None, tasks: vec![], next_runs: vec![], next_maintenance: vec![] };
        assert_eq!(describe(&s), "No scheduled backups");
        s.next_runs.push(NextRun { job_id: "1".into(), job_name: "Docs".into(), next_run_at: "2026-03-10T02:00:00Z".into(), operation: "backup".into() });
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

    fn mount_info(name: &str, mount_point: &str, status: MountStatus) -> MountInfo {
        MountInfo {
            id: Uuid::new_v4(),
            connection_id: Uuid::new_v4(),
            connection_name: name.into(),
            protocol: "rclone".into(),
            mount_point: mount_point.into(),
            status,
            auto_mount: false,
            created_at: chrono::Utc::now(),
            error_message: None,
            options: Default::default(),
            policy_notes: vec![],
        }
    }

    #[test]
    fn mounts_menu_lists_saved_mounts_with_their_state() {
        assert_eq!(mounts_menu(&[]), MountsMenu::default());
        let a = mount_info("Work", "/mnt/work", MountStatus::Mounted);
        let b = mount_info("Photos", "/mnt/photos", MountStatus::Unmounted);
        let c = mount_info("Old", "/mnt/old", MountStatus::Error);
        let m = mounts_menu(&[a.clone(), b.clone(), c.clone()]);
        assert!(m.can_unmount_all);
        let rows: Vec<(String, bool)> = m.items.iter().map(|i| (i.label.clone(), i.mounted)).collect();
        assert_eq!(rows, [("Work — /mnt/work".to_string(), true), ("Photos — /mnt/photos".to_string(), false), ("Old — /mnt/old (failed)".to_string(), false)]);
        assert_eq!(m.items[0].id, format!("mount:{}", a.id));
        // "Unmount all" has nothing to do when nothing is mounted.
        assert!(!mounts_menu(&[b, c]).can_unmount_all);
    }

    #[test]
    fn menu_ids_map_back_to_actions() {
        let id = Uuid::new_v4();
        assert_eq!(parse_mount_action(&format!("mount:{id}")), Some(MountAction::Toggle(id)));
        assert_eq!(parse_mount_action("unmount_all"), Some(MountAction::UnmountAll));
        for other in ["open", "backups", "mount:not-a-uuid", "mounts_none", ""] {
            assert_eq!(parse_mount_action(other), None, "{other}");
        }
        // Every id the model produces parses.
        let m = mounts_menu(&[mount_info("A", "/a", MountStatus::Mounted)]);
        assert!(matches!(parse_mount_action(&m.items[0].id), Some(MountAction::Toggle(_))));
    }

    #[tokio::test]
    async fn clicking_unmount_all_unmounts_and_reports() {
        let repo = Repository::open_in_memory().await.unwrap();
        let conns = ConnectionManager::new(repo.clone(), std::sync::Arc::new(crate::security::CredentialStore::new()));
        let mounts = MountManager::new();
        assert_eq!(run_mount_action(MountAction::UnmountAll, &mounts, &repo, &conns).await.unwrap(), "Unmounted 0 drive(s)");
        // Toggling a mount that no longer exists is an error the user is told about.
        assert!(run_mount_action(MountAction::Toggle(Uuid::new_v4()), &mounts, &repo, &conns).await.is_err());
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
