/// Greet command - used to verify IPC bridge is operational.
#[tauri::command]
pub fn greet(name: &str) -> String {
    format!("Hello, {name}! Tauri IPC bridge is operational.")
}

/// Get application version.
/// Portable mode: on/off, the folder and where credentials are kept
/// (Settings → About this installation).
#[tauri::command]
pub fn get_portable_status(creds: tauri::State<'_, std::sync::Arc<crate::security::CredentialStore>>) -> crate::paths::PortableStatus {
    crate::paths::status(creds.backend_name())
}

#[tauri::command]
pub fn get_app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Get platform information.
#[tauri::command]
pub fn get_platform_info() -> PlatformInfo {
    PlatformInfo {
        os: std::env::consts::OS.to_string(),
        arch: std::env::consts::ARCH.to_string(),
        family: std::env::consts::FAMILY.to_string(),
    }
}

#[derive(serde::Serialize)]
pub struct PlatformInfo {
    pub os: String,
    pub arch: String,
    pub family: String,
}
