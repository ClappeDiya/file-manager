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
