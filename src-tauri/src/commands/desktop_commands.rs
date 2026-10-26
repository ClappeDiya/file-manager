//! Desktop shell settings: the language of the shell's own texts (tray),
//! startup and tray behaviour, reset, cloud-synced folders.

use crate::core::error::AppError;

/// The web view's language (Settings → Appearance → Language): the tray
/// menu follows it.
#[tauri::command]
pub async fn set_ui_language(locale: String, app: tauri::AppHandle) -> Result<(), AppError> {
    if !crate::tray_labels::set_current(&locale) {
        return Err(AppError::validation(format!("Unsupported language '{locale}'. Choose one of the languages listed in Settings → Appearance.")));
    }
    crate::tray::apply_language(&app);
    Ok(())
}

/// Settings → Startup & Tray.
#[tauri::command]
pub async fn get_startup_settings(
    app: tauri::AppHandle,
    startup: tauri::State<'_, crate::startup::StartupState>,
    background: tauri::State<'_, crate::tray::BackgroundMode>,
) -> Result<crate::startup::StartupSettings, AppError> {
    use tauri_plugin_autostart::ManagerExt;
    Ok(crate::startup::StartupSettings::new(startup.get(), app.autolaunch().is_enabled().unwrap_or(false), background.enabled()))
}

/// Save Settings → Startup & Tray: the login item, close-to-tray (shared
/// with the Backups screen), the rest of the choices, and the tray icon.
#[tauri::command]
pub async fn set_startup_settings(
    settings: crate::startup::StartupSettings,
    app: tauri::AppHandle,
    startup: tauri::State<'_, crate::startup::StartupState>,
    background: tauri::State<'_, crate::tray::BackgroundMode>,
    repo: tauri::State<'_, crate::storage::Repository>,
) -> Result<crate::startup::StartupSettings, AppError> {
    use tauri_plugin_autostart::ManagerExt;
    let al = app.autolaunch();
    if al.is_enabled().unwrap_or(false) != settings.start_at_login {
        let res = if settings.start_at_login { al.enable() } else { al.disable() };
        res.map_err(|e| AppError::state(format!("Could not change start-at-login: {e}"), "Check that the app may register itself to start at login."))?;
    }
    crate::tray::save_background_mode(&repo, &background, settings.close_to_tray).await?;
    crate::startup::save(&repo, &startup, settings.prefs()).await?;
    if let Some(tray) = app.tray_by_id("ufop") {
        let _ = tray.set_visible(settings.show_tray_icon);
    }
    get_startup_settings(app, startup, background).await
}

/// Settings → Reset: put the chosen preferences back to their defaults.
/// Start-at-login is turned off with the Desktop scope. Data, saved
/// passwords and integrations are never touched (see `reset`).
#[tauri::command]
pub async fn reset_settings(
    scopes: Vec<crate::reset::ResetScope>,
    app: tauri::AppHandle,
    startup: tauri::State<'_, crate::startup::StartupState>,
    background: tauri::State<'_, crate::tray::BackgroundMode>,
    sync: tauri::State<'_, crate::sync_engine::SyncManager>,
    repo: tauri::State<'_, crate::storage::Repository>,
) -> Result<crate::reset::ResetReport, AppError> {
    let live = crate::reset::Live { startup: &startup, background: &background, sync: &sync };
    let report = crate::reset::reset(&repo, &scopes, live).await?;
    if scopes.contains(&crate::reset::ResetScope::Desktop) {
        use tauri_plugin_autostart::ManagerExt;
        let al = app.autolaunch();
        if al.is_enabled().unwrap_or(false) {
            if let Err(e) = al.disable() {
                tracing::warn!("Reset: could not turn off start-at-login: {e}");
            }
        }
        if let Some(tray) = app.tray_by_id("ufop") {
            let _ = tray.set_visible(true);
        }
    }
    crate::security::audit::SecurityAudit::log(repo.inner(), "settings.reset", "user", "settings", &format!("{scopes:?}")).await;
    Ok(report)
}

/// Folders on this computer kept in sync by a cloud service's own app
/// (Dropbox, OneDrive, Google Drive, iCloud Drive…), for the sidebar.
#[tauri::command]
pub async fn detect_cloud_folders() -> Result<Vec<crate::cloud_folders::CloudFolder>, AppError> {
    tokio::task::spawn_blocking(|| crate::cloud_folders::Probe::current().map(|p| crate::cloud_folders::detect(&p)).unwrap_or_default())
        .await
        .map_err(|e| AppError::internal(format!("Cloud folder detection failed: {e}")))
}

/// The page a new window loads: its label (so it keeps its own panes) and
/// the folder to open, percent-encoded.
pub fn new_window_url(label: &str, path: Option<&str>) -> String {
    let mut url = format!("index.html?window={}", urlencoding::encode(label));
    if let Some(p) = path.filter(|p| !p.is_empty()) {
        url.push_str("&path=");
        url.push_str(&urlencoding::encode(p));
    }
    url
}

/// Labels of the windows this command opens (`main` is the first window).
pub const SECONDARY_WINDOW_PREFIX: &str = "win-";

/// Open another app window, optionally at a folder (a local path or a
/// `conn://` location). Each window has its own panes and tabs.
#[tauri::command]
pub async fn open_new_window(path: Option<String>, app: tauri::AppHandle) -> Result<String, AppError> {
    let label = format!("{SECONDARY_WINDOW_PREFIX}{}", &uuid::Uuid::new_v4().simple().to_string()[..12]);
    let cfg = app.config().app.windows.first().cloned().ok_or_else(|| AppError::internal("No window configuration"))?;
    let mut builder = tauri::WebviewWindowBuilder::new(&app, &label, tauri::WebviewUrl::App(new_window_url(&label, path.as_deref()).into()))
        .title(cfg.title.clone())
        .inner_size(cfg.width, cfg.height)
        .resizable(cfg.resizable);
    if let (Some(w), Some(h)) = (cfg.min_width, cfg.min_height) {
        builder = builder.min_inner_size(w, h);
    }
    // Portable mode: the same web data store as the main window (shared settings).
    if let Some(id) = crate::paths::webview_store_identifier(crate::paths::dirs()) {
        builder = builder.data_store_identifier(id);
    }
    builder.build().map_err(|e| AppError::state(format!("Could not open a new window: {e}"), "Close a window and try again."))?;
    Ok(label)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_window_urls_carry_the_label_and_an_encoded_folder() {
        assert_eq!(new_window_url("win-1", None), "index.html?window=win-1");
        assert_eq!(new_window_url("win-1", Some("")), "index.html?window=win-1");
        assert_eq!(new_window_url("win-1", Some("/home/me/A & B?#")), "index.html?window=win-1&path=%2Fhome%2Fme%2FA%20%26%20B%3F%23");
        assert_eq!(new_window_url("win-2", Some("conn://6f1c/Docs")), "index.html?window=win-2&path=conn%3A%2F%2F6f1c%2FDocs");
    }
}
