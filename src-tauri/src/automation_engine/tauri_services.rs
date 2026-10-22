//! [`ActionServices`] backed by the desktop app's managed engines.
//!
//! Resolves managers from Tauri's state at call time, so it can be built in
//! `setup` and handed to the automation manager before any rule fires.

use super::services::{remote_join, ActionServices};
use crate::connectors::{ConnectionManager, ConnectorRegistry};
use crate::core::traits::BoxFuture;
use crate::ledger::OperationLedger;
use crate::storage::Repository;
use crate::sync_engine::SyncManager;
use std::path::Path;
use tauri::{AppHandle, Manager};
use tauri_plugin_notification::NotificationExt;

pub struct TauriActionServices {
    app: AppHandle,
}

impl TauriActionServices {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ActionServices for TauriActionServices {
    fn run_sync(&self, pair_id: &str) -> BoxFuture<'_, Result<String, String>> {
        let pair_id = pair_id.to_string();
        Box::pin(async move {
            let manager = self.app.state::<SyncManager>();
            let ledger = self.app.state::<OperationLedger>();
            let repo = self.app.state::<Repository>();
            let report = crate::commands::sync_commands::run_sync_pair(&manager, &ledger, &repo, &pair_id)
                .await
                .map_err(|e| format!("Sync failed: {e}"))?;
            Ok(format!(
                "{:?}: +{} ~{} -{}",
                report.status, report.files_added, report.files_modified, report.files_deleted
            ))
        })
    }

    fn transfer_to_connector(
        &self,
        protocol: &str,
        connection_id: &str,
        local_path: &str,
        remote_dir: &str,
    ) -> BoxFuture<'_, Result<String, String>> {
        let (protocol, connection_id, local_path, remote_dir) = (
            protocol.to_string(),
            connection_id.to_string(),
            local_path.to_string(),
            remote_dir.to_string(),
        );
        Box::pin(async move {
            let connector = crate::commands::connector_commands::ensure_connected(
                &self.app.state::<ConnectorRegistry>(),
                &self.app.state::<ConnectionManager>(),
                &protocol,
                Some(&connection_id),
            )
            .await
            .map_err(|e| e.user_message())?;
            let name = Path::new(&local_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .ok_or("Cannot determine file name")?;
            let remote = remote_join(&remote_dir, &name);
            connector
                .upload(&local_path, &remote)
                .await
                .map_err(|e| format!("Upload failed: {e}"))?;
            Ok(remote)
        })
    }

    fn run_custom_command(
        &self,
        command_id: &str,
        files: Vec<String>,
    ) -> BoxFuture<'_, Result<String, String>> {
        let command_id = command_id.to_string();
        Box::pin(async move {
            // Unattended: never pass user confirmation, so only allowlisted
            // commands run.
            let result = crate::commands::custom_commands::execute_custom_command(
                command_id, files, None, Some(false),
            )
            .await
            .map_err(|e| e.to_string())?;
            match result.exit_code {
                Some(0) => Ok("exit 0".to_string()),
                code => Err(format!(
                    "Command exited with {}: {}",
                    code.map(|c| c.to_string()).unwrap_or_else(|| "a signal".into()),
                    result.stderr.trim()
                )),
            }
        })
    }

    fn notify(&self, title: &str, body: &str) -> BoxFuture<'_, Result<(), String>> {
        let (title, body) = (title.to_string(), body.to_string());
        Box::pin(async move {
            self.app
                .notification()
                .builder()
                .title(&title)
                .body(&body)
                .show()
                .map_err(|e| format!("Notification failed: {e}"))
        })
    }

    fn send_message(
        &self,
        notification: &crate::backup_engine::notify::Notification,
        title: &str,
        body: &str,
    ) -> BoxFuture<'_, Result<(), String>> {
        let (n, title, body) = (notification.clone(), title.to_string(), body.to_string());
        Box::pin(async move {
            self.app
                .state::<crate::backup_engine::BackupManager>()
                .send_message(n, &title, &body)
                .await
                .map_err(|e| e.user_message())
        })
    }
}
