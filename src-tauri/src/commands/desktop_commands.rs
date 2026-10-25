//! Desktop shell settings: the language of the shell's own texts (tray),
//! startup and tray behaviour.

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
