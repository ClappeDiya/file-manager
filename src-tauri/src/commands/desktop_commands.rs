//! Desktop shell settings: the language of the shell's own texts (tray).

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
