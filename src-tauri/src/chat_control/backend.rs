//! The real backend of the chat commands: the backup, sync and automation
//! managers of the running app, and the tamper-evident audit log.
//!
//! This is the only place a chat command touches an engine, and it calls
//! the same entry points the UI buttons call (`start_backup`,
//! `run_sync_pair`, `run_rule`, the cancel calls). Nothing here takes a path,
//! a program or a script from a message.

use super::core::{JobInfo, JobKind};
use super::handler::{AuditEntry, AuditSink, BotBackend};
use crate::automation_engine::AutomationManager;
use crate::backup_engine::BackupManager;
use crate::core::traits::{BoxFuture, SyncOperations};
use crate::core::types::AuditEvent;
use crate::ledger::OperationLedger;
use crate::storage::Repository;
use crate::sync_engine::SyncManager;
use tauri::{AppHandle, Manager};

pub struct TauriBackend {
    app: AppHandle,
}

impl TauriBackend {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl BotBackend for TauriBackend {
    fn jobs(&self) -> BoxFuture<'_, Vec<JobInfo>> {
        Box::pin(async move {
            let backups = self.app.state::<BackupManager>();
            let sync = self.app.state::<SyncManager>();
            let automation = self.app.state::<AutomationManager>();
            let mut out: Vec<JobInfo> = backups
                .list_jobs()
                .await
                .into_iter()
                .map(|j| {
                    let state = if backups.is_job_busy(&j.id) { "running" } else if j.read_only { "read-only" } else { "idle" };
                    JobInfo { kind: JobKind::Backup, id: j.id, name: j.name, state: state.into() }
                })
                .collect();
            out.extend(sync.list_pairs().await.unwrap_or_default().into_iter().map(|p| {
                let state = if sync.get_progress(p.id).is_some() { "running" } else if p.enabled { "idle" } else { "disabled" };
                JobInfo { kind: JobKind::Sync, id: p.id.to_string(), name: p.name, state: state.into() }
            }));
            out.extend(automation.list_rules().await.into_iter().map(|r| JobInfo { kind: JobKind::Rule, id: r.id, name: r.name, state: if r.enabled { "enabled" } else { "disabled" }.into() }));
            out
        })
    }

    fn run<'a>(&'a self, job: &'a JobInfo) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            match job.kind {
                JobKind::Backup => {
                    self.app.state::<BackupManager>().start_backup(&job.id, false).await.map_err(|e| e.user_message())?;
                    Ok(format!("Started the backup \"{}\".", job.name))
                }
                JobKind::Sync => {
                    let id = uuid::Uuid::parse_str(&job.id).map_err(|_| "Invalid sync pair".to_string())?;
                    let manager = self.app.state::<SyncManager>();
                    let pair = manager.list_pairs().await.map_err(|e| e.user_message())?.into_iter().find(|p| p.id == id).ok_or("That sync pair no longer exists")?;
                    if !pair.enabled {
                        return Err(format!("\"{}\" is disabled", pair.name));
                    }
                    if manager.get_progress(id).is_some() {
                        return Err(format!("\"{}\" is already syncing", pair.name));
                    }
                    let app = self.app.clone();
                    tauri::async_runtime::spawn(async move {
                        let (manager, ledger, repo) = (app.state::<SyncManager>(), app.state::<OperationLedger>(), app.state::<Repository>());
                        if let Err(e) = crate::commands::sync_commands::run_sync_pair(&manager, &ledger, &repo, &id.to_string()).await {
                            tracing::warn!("Remote control: sync {id} failed: {e}");
                        }
                    });
                    Ok(format!("Started syncing \"{}\".", job.name))
                }
                JobKind::Rule => {
                    let app = self.app.clone();
                    let id = job.id.clone();
                    if self.app.state::<AutomationManager>().get_rule(&id).await.is_none() {
                        return Err("That automation rule no longer exists".into());
                    }
                    tauri::async_runtime::spawn(async move {
                        let (mgr, repo) = (app.state::<AutomationManager>(), app.state::<Repository>());
                        if let Err(e) = mgr.run_rule(&repo, &id).await {
                            tracing::warn!("Remote control: rule {id} failed: {e}");
                        }
                    });
                    Ok(format!("Started the rule \"{}\".", job.name))
                }
            }
        })
    }

    fn stop<'a>(&'a self, job: &'a JobInfo) -> BoxFuture<'a, Result<String, String>> {
        Box::pin(async move {
            match job.kind {
                JobKind::Backup => {
                    if self.app.state::<BackupManager>().cancel(&job.id) {
                        Ok(format!("Stopping the backup \"{}\".", job.name))
                    } else {
                        Err(format!("\"{}\" is not running", job.name))
                    }
                }
                JobKind::Sync => {
                    let id = uuid::Uuid::parse_str(&job.id).map_err(|_| "Invalid sync pair".to_string())?;
                    if self.app.state::<SyncManager>().cancel_sync(id).await {
                        Ok(format!("Stopping \"{}\" after the file it is copying.", job.name))
                    } else {
                        Err(format!("\"{}\" is not running", job.name))
                    }
                }
                JobKind::Rule => Err(format!("Automation rules run to the end once started; \"{}\" cannot be stopped from here.", job.name)),
            }
        })
    }

    fn status(&self) -> BoxFuture<'_, String> {
        Box::pin(async move {
            let backups = self.app.state::<BackupManager>().status().await;
            let sync = self.app.state::<SyncManager>();
            let automation = self.app.state::<AutomationManager>();
            let mut lines = Vec::new();
            let running: Vec<String> = backups.tasks.iter().filter(|t| t.state == "running").map(|t| format!("{} of {}", t.operation, t.job_name)).collect();
            lines.push(if running.is_empty() { "Backups: nothing running".to_string() } else { format!("Backups running: {}", running.join(", ")) });
            if backups.paused {
                lines.push("Backups are paused".into());
            }
            let syncing = sync.all_progress().len();
            lines.push(if syncing == 0 { "Sync: nothing running".to_string() } else { format!("Sync: {syncing} running") });
            let rules = automation.list_rules().await;
            let enabled = rules.iter().filter(|r| r.enabled).count();
            let failing: Vec<&str> = rules.iter().filter(|r| r.last_error.is_some()).map(|r| r.name.as_str()).take(5).collect();
            lines.push(format!("Automation: {enabled} of {} rule(s) enabled", rules.len()));
            if !failing.is_empty() {
                lines.push(format!("Rules with an error: {}", failing.join(", ")));
            }
            lines.join("\n")
        })
    }
}

/// Writes accepted commands to the hash-chained audit log.
pub struct RepoAudit {
    repo: Repository,
}

impl RepoAudit {
    pub fn new(repo: Repository) -> Self {
        Self { repo }
    }
}

impl AuditSink for RepoAudit {
    fn record<'a>(&'a self, entry: AuditEntry) -> BoxFuture<'a, Result<(), String>> {
        Box::pin(async move {
            let event = AuditEvent {
                id: uuid::Uuid::new_v4(),
                timestamp: chrono::Utc::now(),
                event_type: "remote_control.command".into(),
                actor: format!("{}:{}", entry.channel, entry.sender),
                resource: entry.command,
                details: format!("chat {}", entry.chat),
            };
            self.repo.log_audit_event(event).await.map_err(|e| e.user_message())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn accepted_commands_land_in_the_audit_chain_with_who_what_and_where() {
        let repo = Repository::open_in_memory().await.unwrap();
        let audit = RepoAudit::new(repo.clone());
        for (sender, command) in [("42", "run Nightly"), ("43", "stop Nightly")] {
            audit.record(AuditEntry { channel: "telegram", sender: sender.into(), chat: "-100".into(), command: command.into() }).await.unwrap();
        }
        let events = repo.query_audit_events(50, 0).await.unwrap();
        let ours: Vec<&AuditEvent> = events.iter().filter(|e| e.event_type == "remote_control.command").collect();
        assert_eq!(ours.len(), 2);
        let mut who: Vec<(String, String, String)> = ours.iter().map(|e| (e.actor.clone(), e.resource.clone(), e.details.clone())).collect();
        who.sort();
        assert_eq!(who, vec![("telegram:42".into(), "run Nightly".into(), "chat -100".into()), ("telegram:43".into(), "stop Nightly".into(), "chat -100".into())]);
    }
}
