//! Engine services available to automation actions.
//!
//! Actions such as "run a sync pair" or "upload to a connection" are carried
//! out by other managers that live in Tauri's managed state. The executor
//! only sees this trait, so it stays testable without a Tauri app: tests pass
//! a recording mock, the app passes [`tauri_services::TauriActionServices`].
//!
//! When no services are attached (unit tests, headless callers), these
//! actions fail with an explanation instead of reporting a success that
//! never happened.

use crate::core::traits::BoxFuture;

/// Error text used when an action needs an engine that isn't attached.
pub const SERVICES_UNAVAILABLE: &str =
    "This action needs the desktop app's engines, which are not available in this context";

/// Engines an automation action can delegate to. Every method returns a
/// short human-readable outcome (shown in the run history) or an error.
pub trait ActionServices: Send + Sync {
    /// Run a saved sync pair to completion.
    fn run_sync(&self, pair_id: &str) -> BoxFuture<'_, Result<String, String>>;

    /// Upload `local_path` into `remote_dir` on a saved connection,
    /// connecting first when needed. Returns the remote path written.
    fn transfer_to_connector(
        &self,
        protocol: &str,
        connection_id: &str,
        local_path: &str,
        remote_dir: &str,
    ) -> BoxFuture<'_, Result<String, String>>;

    /// Run a saved custom command against `files`. Unattended runs never
    /// bypass the command allowlist.
    fn run_custom_command(
        &self,
        command_id: &str,
        files: Vec<String>,
    ) -> BoxFuture<'_, Result<String, String>>;

    /// Show an OS notification.
    fn notify(&self, title: &str, body: &str) -> BoxFuture<'_, Result<(), String>>;

    /// Deliver a message through a configured channel (email, Slack,
    /// Teams, Telegram, XMPP).
    fn send_message(
        &self,
        notification: &crate::backup_engine::notify::Notification,
        title: &str,
        body: &str,
    ) -> BoxFuture<'_, Result<(), String>>;

    /// Another rule by id (workflow steps that run a nested rule).
    fn rule<'a>(&'a self, _id: &'a str) -> BoxFuture<'a, Option<super::AutomationRule>> {
        Box::pin(async { None })
    }
}

/// Join a remote directory and a file name with exactly one `/`.
pub fn remote_join(remote_dir: &str, file_name: &str) -> String {
    let dir = remote_dir.trim_end_matches('/');
    if dir.is_empty() {
        format!("/{file_name}")
    } else {
        format!("{dir}/{file_name}")
    }
}

#[cfg(test)]
pub mod mock {
    //! Recording mock used by executor and manager tests.
    use super::*;
    use std::sync::Mutex;

    #[derive(Default)]
    pub struct RecordingServices {
        pub calls: Mutex<Vec<String>>,
        /// When set, every call fails with this message.
        pub fail_with: Option<String>,
        /// Rules nested workflow steps can run.
        pub rules: Mutex<Vec<crate::automation_engine::AutomationRule>>,
    }

    impl RecordingServices {
        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }

        fn record(&self, call: String) -> Result<(), String> {
            self.calls.lock().unwrap().push(call);
            match &self.fail_with {
                Some(e) => Err(e.clone()),
                None => Ok(()),
            }
        }
    }

    impl ActionServices for RecordingServices {
        fn run_sync(&self, pair_id: &str) -> BoxFuture<'_, Result<String, String>> {
            let r = self.record(format!("sync:{pair_id}"));
            Box::pin(async move { r.map(|_| "sync done".to_string()) })
        }

        fn transfer_to_connector(
            &self,
            protocol: &str,
            connection_id: &str,
            local_path: &str,
            remote_dir: &str,
        ) -> BoxFuture<'_, Result<String, String>> {
            let name = std::path::Path::new(local_path)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();
            let remote = remote_join(remote_dir, &name);
            let r = self.record(format!("transfer:{protocol}:{connection_id}:{local_path}->{remote}"));
            Box::pin(async move { r.map(|_| remote) })
        }

        fn run_custom_command(
            &self,
            command_id: &str,
            files: Vec<String>,
        ) -> BoxFuture<'_, Result<String, String>> {
            let r = self.record(format!("command:{command_id}:{}", files.join(",")));
            Box::pin(async move { r.map(|_| "exit 0".to_string()) })
        }

        fn notify(&self, title: &str, body: &str) -> BoxFuture<'_, Result<(), String>> {
            let r = self.record(format!("notify:{title}:{body}"));
            Box::pin(async move { r })
        }

        fn send_message(
            &self,
            notification: &crate::backup_engine::notify::Notification,
            title: &str,
            body: &str,
        ) -> BoxFuture<'_, Result<(), String>> {
            let kind = serde_json::to_value(&notification.channel).ok().and_then(|v| v["type"].as_str().map(str::to_string)).unwrap_or_default();
            let r = self.record(format!("message:{kind}:{title}:{body}"));
            Box::pin(async move { r })
        }

        fn rule<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Option<crate::automation_engine::AutomationRule>> {
            let r = self.rules.lock().unwrap().iter().find(|r| r.id == id).cloned();
            Box::pin(async move { r })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_join_normalises_slashes() {
        assert_eq!(remote_join("/inbox/", "a.txt"), "/inbox/a.txt");
        assert_eq!(remote_join("/inbox", "a.txt"), "/inbox/a.txt");
        assert_eq!(remote_join("", "a.txt"), "/a.txt");
        assert_eq!(remote_join("/", "a.txt"), "/a.txt");
    }
}
