pub mod ai_engine;
pub mod commands;
pub mod compat_engine;
pub mod connectors;
pub mod core;
pub mod fs_engine;
pub mod governance;
pub mod security;
pub mod storage;
pub mod sync_engine;
pub mod transfer_engine;

use commands::fs_commands;
use commands::system_commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    tracing::info!("Starting Unified File Operations Platform");

    tauri::Builder::default()
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_os::init())
        .invoke_handler(tauri::generate_handler![
            system_commands::greet,
            system_commands::get_app_version,
            system_commands::get_platform_info,
            fs_commands::list_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
