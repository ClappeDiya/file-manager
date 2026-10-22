//! `ufop agent …` — connect this machine to a UFOP management console.
//!
//! The protocol client and loop are `ufop_core::agent` (shared with the
//! desktop app and `ufop-server`). The CLI has no backup manager, so its
//! host keeps a small store of its own:
//!
//! - `~/.config/ufop/agent.json` (0600): agent settings and the managed jobs
//!   created from fleet policies (no secrets);
//! - `~/.config/ufop/agent-token` (0600): the agent token, like the CLI's
//!   other tokens;
//! - central credentials are held in memory only, re-delivered by the
//!   console each time `ufop agent run` starts.
//!
//! Managed jobs must back up to a local folder (disk, removable drive or
//! mounted share), the destinations `ufop backup` supports; policies with
//! other destinations are reported as not applicable — use `ufop-server`
//! on such machines.

use crate::output;
use crate::OutputFormat;
use clap::Subcommand;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use ufop_core::agent::policy::{self, managed_job_id, policy_of_job};
use ufop_core::agent::{parse_tags, Agent, AgentConfig, AgentHost, AgentStatus, BoxFuture, ClientInfo, FleetPolicy, JobSummary, ReportStats, RunReport};
use ufop_core::backup::crypto::KdfParams;
use ufop_core::backup::db::BackupDb;
use ufop_core::backup::{run_backup, BackupOptions, LocalBackend, OpContext, RetentionPolicy, SourceFilter};
use ufop_core::error::AppError;

#[derive(Subcommand, Debug)]
pub enum AgentCommands {
    /// Register with a console using a registration link token
    Register {
        /// Console address, e.g. https://console.example.com
        #[arg(long, env = "UFOP_CONSOLE_URL")]
        url: String,
        /// Registration token (ufop_reg_…)
        #[arg(long, env = "UFOP_CONSOLE_REGISTRATION_TOKEN")]
        token: String,
        /// Tags (comma separated)
        #[arg(long, env = "UFOP_CONSOLE_TAGS")]
        tags: Option<String>,
    },
    /// Show a code to enter in the console, and wait until an administrator claims this machine
    Claim {
        #[arg(long, env = "UFOP_CONSOLE_URL")]
        url: String,
        /// Seconds between polls
        #[arg(long, default_value_t = 5)]
        interval: u64,
    },
    /// Registration, last heartbeat, managed policies and queued reports
    Status,
    /// Leave the console (removes managed jobs; backup data stays)
    Unregister {
        /// Forget the registration even when the console cannot be reached
        #[arg(long)]
        force: bool,
    },
    /// One heartbeat / command / policy round now
    Sync,
    /// Run the agent in the foreground: heartbeats, commands, policies and scheduled managed backups
    Run {
        /// Heartbeat interval in seconds (15–3600; default: the console's, 60)
        #[arg(long, env = "UFOP_CONSOLE_HEARTBEAT_SECONDS")]
        heartbeat: Option<u64>,
    },
}

/// A backup job created from a fleet policy.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct CliJob {
    pub id: String,
    pub policy_id: String,
    pub name: String,
    pub sources: Vec<String>,
    pub destination: String,
    pub encrypted: bool,
    /// `$central:<id>`; the value lives only in memory.
    pub passphrase: Option<String>,
    pub filter: SourceFilter,
    pub options: BackupOptions,
    pub retention: RetentionPolicy,
    pub schedule_interval_minutes: Option<u32>,
    pub last_run_at: Option<String>,
    pub last_result: Option<String>,
    pub last_summary: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
struct StateFile {
    agent: AgentConfig,
    jobs: BTreeMap<String, CliJob>,
}

pub struct CliHost {
    dir: PathBuf,
    data_dir: PathBuf,
    state: Mutex<StateFile>,
    creds: Mutex<HashMap<String, String>>,
    busy: Mutex<HashSet<String>>,
    /// Runs spawned and not yet finished (including ones not started yet).
    spawned: std::sync::atomic::AtomicUsize,
    agent: OnceLock<Weak<Agent<CliHost>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

fn write_private(path: &Path, data: &str) -> Result<(), AppError> {
    if let Some(p) = path.parent() {
        std::fs::create_dir_all(p)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

impl CliHost {
    /// `dir`: where agent.json / agent-token live; `data_dir`: job databases.
    pub fn new(dir: PathBuf, data_dir: PathBuf) -> Self {
        let state = std::fs::read_to_string(dir.join("agent.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        Self { dir, data_dir, state: Mutex::new(state), creds: Mutex::new(HashMap::new()), busy: Mutex::new(HashSet::new()), spawned: Default::default(), agent: OnceLock::new() }
    }

    pub fn default_paths() -> (PathBuf, PathBuf) {
        let dir = crate::config::CliConfig::path().parent().map(Path::to_path_buf).unwrap_or_else(|| PathBuf::from("."));
        let data = dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("ufop").join("agent-backups");
        (dir, data)
    }

    fn save_state(&self) -> Result<(), AppError> {
        let s = serde_json::to_string_pretty(&*lock(&self.state))?;
        write_private(&self.dir.join("agent.json"), &s)
    }

    pub fn jobs_snapshot(&self) -> Vec<CliJob> {
        lock(&self.state).jobs.values().cloned().collect()
    }

    fn job(&self, id: &str) -> Result<CliJob, AppError> {
        lock(&self.state).jobs.get(id).cloned().ok_or_else(|| AppError::not_found(format!("Backup job {id} not found")))
    }

    fn passphrase(&self, job: &CliJob) -> Result<Option<String>, AppError> {
        if !job.encrypted {
            return Ok(None);
        }
        let Some(r) = &job.passphrase else {
            return Err(AppError::validation("The encrypted job has no passphrase credential"));
        };
        let id = r.trim_start_matches('$');
        lock(&self.creds).get(id).cloned().map(Some).ok_or_else(|| AppError::Configuration {
            message: format!("The central credential '{id}' has not been delivered"),
            advice: "Keep `ufop agent run` running; the console delivers credentials when policies are synced.".into(),
        })
    }

    fn job_from_policy(&self, p: &FleetPolicy) -> Result<CliJob, AppError> {
        let id = managed_job_id(&p.id)?;
        let t = policy::prepare_template(&p.job)?;
        let f = &t.fields;
        let get = |k: &str| f.get(k).cloned().unwrap_or(Value::Null);
        let dest = get("destination");
        let path = match dest.get("type").and_then(|t| t.as_str()) {
            Some("local") => dest.get("path").and_then(|p| p.as_str()).unwrap_or_default().to_string(),
            other => {
                return Err(AppError::Configuration {
                    message: format!("The ufop CLI agent backs up only to local folders, not to '{}' destinations", other.unwrap_or("unknown")),
                    advice: "Use a local-folder destination for CLI machines, or install ufop-server there.".into(),
                })
            }
        };
        let sources: Vec<String> = serde_json::from_value(get("sources")).unwrap_or_default();
        if sources.iter().all(|s| s.trim().is_empty()) || path.trim().is_empty() {
            return Err(AppError::validation("The policy needs sources and a destination folder"));
        }
        fn field<T: Default + serde::de::DeserializeOwned>(f: &serde_json::Map<String, Value>, k: &str) -> Result<T, AppError> {
            match f.get(k) {
                None | Some(Value::Null) => Ok(T::default()),
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| AppError::validation(format!("Policy field '{k}': {e}"))),
            }
        }
        let filter: SourceFilter = field(f, "filter")?;
        filter.clone().compile()?;
        let options: BackupOptions = field(f, "options")?;
        let retention: RetentionPolicy = field(f, "retention")?;
        let schedule = get("schedule");
        let interval = (schedule.get("enabled").and_then(|e| e.as_bool()) == Some(true))
            .then(|| schedule.get("interval_minutes").and_then(|i| i.as_u64()).unwrap_or(1440).max(5) as u32);
        let encrypted = f.get("encrypted").and_then(|e| e.as_bool()).unwrap_or(true);
        if encrypted && t.passphrase.is_none() {
            return Err(AppError::validation("An encrypted policy needs a passphrase credential"));
        }
        let old = lock(&self.state).jobs.get(&id).cloned().unwrap_or_default();
        Ok(CliJob {
            id,
            policy_id: p.id.clone(),
            name: f.get("name").and_then(|n| n.as_str()).unwrap_or(&p.name).to_string(),
            sources,
            destination: path,
            encrypted,
            passphrase: t.passphrase,
            filter,
            options,
            retention,
            schedule_interval_minutes: interval,
            last_run_at: old.last_run_at,
            last_result: old.last_result,
            last_summary: old.last_summary,
        })
    }

    fn db_path(&self, id: &str) -> PathBuf {
        self.data_dir.join(format!("{id}.sqlite"))
    }

    /// Run a managed job to completion (blocking engine call on a worker thread).
    pub async fn run_job(&self, id: &str) -> Result<RunReport, AppError> {
        let job = self.job(id)?;
        if !lock(&self.busy).insert(id.to_string()) {
            return Err(AppError::state("This job is already running", "Wait for it to finish."));
        }
        let pass = self.passphrase(&job);
        let db_path = self.db_path(id);
        let started = chrono::Utc::now();
        let j = job.clone();
        let res = match pass {
            Err(e) => Err(e),
            Ok(pass) => tokio::task::spawn_blocking(move || -> Result<(Value, ReportStats), AppError> {
                if let Some(p) = db_path.parent() {
                    std::fs::create_dir_all(p)?;
                }
                let filter = j.filter.clone().compile()?;
                let be = LocalBackend::new(&j.destination);
                let db = BackupDb::open(&db_path)?;
                let srcs: Vec<PathBuf> = j.sources.iter().map(PathBuf::from).collect();
                let r = run_backup(&be, &db, &srcs, &filter, &j.options, &j.retention, pass.as_deref(), KdfParams::default(), &OpContext::new())?;
                let fs = db.filesets()?;
                let stats = ReportStats {
                    version_count: fs.len() as u64,
                    source_size: fs.first().map(|f| f.total_size).unwrap_or(0),
                    backend_size: db.volumes(None)?.iter().map(|v| v.size).sum(),
                    schedule_interval_minutes: j.schedule_interval_minutes,
                };
                Ok((serde_json::to_value(&r)?, stats))
            })
            .await
            .map_err(|e| AppError::internal(e.to_string()))
            .and_then(|r| r),
        };
        lock(&self.busy).remove(id);
        let finished = chrono::Utc::now();
        let (result, message, data, stats) = match res {
            Ok((data, stats)) => {
                let n = |k: &str| data.get(k).and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
                let result = if n("errors") > 0 || data["partial"] == true { "error" } else if n("warnings") > 0 { "warning" } else { "success" };
                let msg = format!("{} files examined, {} bytes uploaded", data["files_examined"], data["bytes_uploaded"]);
                (result.to_string(), msg, data, stats)
            }
            Err(e) => ("error".to_string(), e.user_message(), json!({}), ReportStats { schedule_interval_minutes: job.schedule_interval_minutes, ..Default::default() }),
        };
        {
            let mut s = lock(&self.state);
            if let Some(j) = s.jobs.get_mut(id) {
                j.last_run_at = Some(finished.to_rfc3339());
                j.last_result = Some(result.clone());
                j.last_summary = Some(message.clone());
            }
        }
        let _ = self.save_state();
        let report = RunReport {
            backup_id: job.id.clone(),
            name: job.name.clone(),
            operation: "backup".into(),
            result,
            started_at: Some(started.to_rfc3339()),
            finished_at: Some(finished.to_rfc3339()),
            duration_ms: (finished - started).num_milliseconds().max(0) as u64,
            message,
            data,
            stats,
        };
        if let Some(a) = self.agent.get().and_then(Weak::upgrade) {
            a.enqueue_report(&report);
        }
        Ok(report)
    }

    /// Wait until no managed backup runs; `true` if one was running.
    pub async fn wait_idle(&self) -> bool {
        let mut waited = false;
        while self.spawned.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            waited = true;
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        waited
    }

    /// Jobs whose interval has passed since their last run.
    pub fn due_jobs(&self, now: chrono::DateTime<chrono::Utc>) -> Vec<String> {
        let busy = lock(&self.busy).clone();
        lock(&self.state)
            .jobs
            .values()
            .filter(|j| !busy.contains(&j.id))
            .filter_map(|j| {
                let every = chrono::Duration::minutes(j.schedule_interval_minutes? as i64);
                let last = j.last_run_at.as_deref().and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok()).map(|t| t.with_timezone(&chrono::Utc));
                (last.is_none_or(|l| l + every <= now)).then(|| j.id.clone())
            })
            .collect()
    }
}

/// Spawn a run in the background (the console gets the report when it ends).
fn spawn_run(host_agent: &Arc<Agent<CliHost>>, id: String) {
    let a = host_agent.clone();
    a.host().spawned.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    tokio::spawn(async move {
        if let Err(e) = a.host().run_job(&id).await {
            tracing::warn!("Managed backup {id} failed: {}", e.user_message());
        }
        a.host().spawned.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
    });
}

impl AgentHost for CliHost {
    fn client_info(&self) -> ClientInfo {
        ClientInfo::detect("cli", env!("CARGO_PKG_VERSION"))
    }

    fn load(&self) -> BoxFuture<'_, Result<(AgentConfig, Option<String>), AppError>> {
        Box::pin(async move {
            let token = std::fs::read_to_string(self.dir.join("agent-token")).ok().map(|t| t.trim().to_string()).filter(|t| !t.is_empty());
            Ok((lock(&self.state).agent.clone(), token))
        })
    }

    fn save_config(&self, cfg: AgentConfig) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            lock(&self.state).agent = cfg;
            self.save_state()
        })
    }

    fn save_token(&self, token: Option<String>) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            let p = self.dir.join("agent-token");
            match token {
                Some(t) => write_private(&p, &t),
                None => match std::fs::remove_file(&p) {
                    Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
                    _ => Ok(()),
                },
            }
        })
    }

    fn jobs(&self) -> BoxFuture<'_, Vec<JobSummary>> {
        Box::pin(async move {
            self.jobs_snapshot()
                .into_iter()
                .map(|j| JobSummary {
                    managed_by_policy: Some(j.policy_id),
                    schedule_interval_minutes: j.schedule_interval_minutes,
                    last_result: j.last_result,
                    last_run_at: j.last_run_at,
                    id: j.id,
                    name: j.name,
                })
                .collect()
        })
    }

    fn rpc(&self, command: String, args: Value) -> BoxFuture<'_, Result<Value, AppError>> {
        Box::pin(async move {
            let id = || args.get("id").and_then(|v| v.as_str()).map(String::from).ok_or_else(|| AppError::validation("Missing argument 'id'"));
            match command.as_str() {
                "backup_list_jobs" => Ok(serde_json::to_value(self.jobs_snapshot())?),
                "backup_get_job" | "backup_effective_job" => Ok(serde_json::to_value(self.job(&id()?)?)?),
                "backup_run" => self.run_backup(id()?).await.map(|v| v["taskId"].clone()),
                "backup_status" => Ok(json!({ "running": !lock(&self.busy).is_empty(), "paused": false, "tasks": lock(&self.busy).iter().collect::<Vec<_>>() })),
                "backup_list_versions" => {
                    let path = self.db_path(&self.job(&id()?)?.id);
                    let v = tokio::task::spawn_blocking(move || BackupDb::open(&path)?.filesets()).await.map_err(|e| AppError::internal(e.to_string()))??;
                    Ok(serde_json::to_value(v)?)
                }
                "backup_run_history" => Ok(serde_json::to_value(
                    self.jobs_snapshot().iter().filter(|j| j.last_run_at.is_some()).map(|j| json!({ "job_id": j.id, "status": j.last_result, "finished_at": j.last_run_at, "summary": j.last_summary })).collect::<Vec<_>>(),
                )?),
                "backup_capabilities" => Ok(json!({ "agent": "cli", "destinations": ["local"] })),
                other => Err(AppError::Configuration {
                    message: format!("'{other}' is not available on the ufop CLI agent"),
                    advice: "The CLI agent supports listing jobs, runs, status and versions; install ufop-server for the full client API.".into(),
                }),
            }
        })
    }

    fn run_backup(&self, job_id: String) -> BoxFuture<'_, Result<Value, AppError>> {
        Box::pin(async move {
            self.job(&job_id)?;
            if lock(&self.busy).contains(&job_id) {
                return Err(AppError::state("This job is already running", "Wait for it to finish."));
            }
            let a = self.agent.get().and_then(Weak::upgrade).ok_or_else(|| AppError::state("The agent is not running", "Run `ufop agent run`."))?;
            spawn_run(&a, job_id.clone());
            Ok(json!({ "taskId": job_id }))
        })
    }

    fn store_credential(&self, id: String, secret: Option<String>) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            match secret {
                Some(s) => lock(&self.creds).insert(id, s),
                None => lock(&self.creds).remove(&id),
            };
            Ok(())
        })
    }

    fn apply_policy(&self, p: FleetPolicy) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            let job = self.job_from_policy(&p)?;
            lock(&self.state).jobs.insert(job.id.clone(), job);
            self.save_state()
        })
    }

    fn remove_policy(&self, policy_id: String) -> BoxFuture<'_, Result<(), AppError>> {
        Box::pin(async move {
            let id = managed_job_id(&policy_id)?;
            lock(&self.state).jobs.remove(&id);
            self.save_state()
        })
    }

    fn managed_policies(&self) -> BoxFuture<'_, Vec<String>> {
        Box::pin(async move { lock(&self.state).jobs.keys().filter_map(|k| policy_of_job(k).map(String::from)).collect() })
    }
}

fn http() -> Result<reqwest::Client, AppError> {
    let mut b = reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(30));
    if let Some(p) = std::env::var("UFOP_CONSOLE_CA_FILE").ok().filter(|p| !p.trim().is_empty()) {
        let pem = std::fs::read(&p).map_err(|e| AppError::Configuration { message: format!("Cannot read UFOP_CONSOLE_CA_FILE ({p}): {e}"), advice: "Point it at a PEM certificate file.".into() })?;
        let cert = reqwest::Certificate::from_pem(&pem).map_err(|e| AppError::Configuration { message: format!("Invalid CA certificate: {e}"), advice: "Use a PEM file.".into() })?;
        b = b.add_root_certificate(cert);
    }
    b.build().map_err(|e| AppError::internal(e.to_string()))
}

pub async fn open_agent(dir: PathBuf, data_dir: PathBuf) -> Result<Arc<Agent<CliHost>>, AppError> {
    let a = Agent::load(CliHost::new(dir, data_dir), http()?).await?;
    let _ = a.host().agent.set(Arc::downgrade(&a));
    Ok(a)
}

fn print_status(format: &OutputFormat, st: &AgentStatus, jobs: &[CliJob]) {
    match format {
        OutputFormat::Human => {
            if !st.registered {
                println!("Not registered with a management console.");
                println!("  ufop agent register --url <console> --token <ufop_reg_…>   or   ufop agent claim --url <console>");
                return;
            }
            println!("Console:        {}", st.console_url);
            println!("Machine:        {} ({})", st.machine_id, if st.organization.is_empty() { "-" } else { &st.organization });
            println!("State:          {}{}", if st.machine_status.is_empty() { "active" } else { &st.machine_status }, if st.enabled { "" } else { " (agent turned off)" });
            println!("Heartbeat:      every {}s, last {}", st.heartbeat_seconds, st.last_heartbeat_at.as_deref().unwrap_or("never"));
            if let Some(e) = &st.last_error {
                println!("Last error:     {e}");
            }
            println!("Queued reports: {}{}", st.pending_reports, if st.dropped_reports > 0 { format!(" ({} dropped)", st.dropped_reports) } else { String::new() });
            for p in &st.managed_policies {
                let j = jobs.iter().find(|j| j.policy_id == p.policy_id);
                println!(
                    "Policy {} v{} → {}{}",
                    p.policy_id,
                    p.version,
                    j.map(|j| format!("{} ({})", j.name, j.last_result.as_deref().unwrap_or("not run"))).unwrap_or_else(|| p.job_id.clone()),
                    p.error.as_ref().map(|e| format!("  ERROR: {e}")).unwrap_or_default()
                );
            }
        }
        _ => output::print_data(format, &json!({ "status": st, "jobs": jobs })),
    }
}

pub async fn execute(cmd: AgentCommands, format: &OutputFormat) -> Result<u8, Box<dyn std::error::Error>> {
    let (dir, data) = CliHost::default_paths();
    match run(cmd, format, dir, data).await {
        Ok(code) => Ok(code),
        Err(e) => {
            output::print_error(format, &e.user_message());
            Ok(2)
        }
    }
}

pub async fn run(cmd: AgentCommands, format: &OutputFormat, dir: PathBuf, data: PathBuf) -> Result<u8, AppError> {
    let agent = open_agent(dir, data).await?;
    match cmd {
        AgentCommands::Register { url, token, tags } => {
            let st = agent.register(&url, &token, tags.as_deref().map(parse_tags).unwrap_or_default()).await?;
            if let Err(e) = agent.cycle().await {
                output::print_warning(format, &format!("Registered, but the first heartbeat failed: {}", e.user_message()));
            }
            output::print_success(format, &format!("Registered as {} with {}. Run `ufop agent run` (or install it as a service) to stay connected.", st.machine_id, st.console_url));
            print_status(format, &agent.status(), &agent.host().jobs_snapshot());
            Ok(0)
        }
        AgentCommands::Claim { url, interval } => {
            let c = agent.start_claim(&url).await?;
            match format {
                OutputFormat::Human => {
                    println!("Claim code: {}", c.code);
                    println!("Enter it in the console under Machines → Claim (valid until {}). Waiting…", c.expires_at);
                }
                _ => output::print_data(format, &c),
            }
            loop {
                tokio::time::sleep(std::time::Duration::from_secs(interval.max(1))).await;
                let r = match agent.poll_claim().await {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("claim poll: {}", e.user_message());
                        continue;
                    }
                };
                match r.state.as_str() {
                    "pending" => continue,
                    "registered" => {
                        let _ = agent.cycle().await;
                        output::print_success(format, &format!("Claimed: registered as {}", r.status.machine_id));
                        return Ok(0);
                    }
                    _ => {
                        output::print_error(format, "The claim code expired; run `ufop agent claim` again.");
                        return Ok(1);
                    }
                }
            }
        }
        AgentCommands::Status => {
            print_status(format, &agent.status(), &agent.host().jobs_snapshot());
            Ok(0)
        }
        AgentCommands::Unregister { force } => {
            agent.unregister(force).await?;
            output::print_success(format, "Unregistered from the management console.");
            Ok(0)
        }
        AgentCommands::Sync => {
            agent.sync_now().await?;
            // Backups the console started must finish, and report, before the process exits.
            if agent.host().wait_idle().await {
                agent.cycle().await?;
            }
            print_status(format, &agent.status(), &agent.host().jobs_snapshot());
            Ok(0)
        }
        AgentCommands::Run { heartbeat } => {
            if heartbeat.is_some() {
                agent.set_heartbeat_seconds(heartbeat).await;
            }
            let env = |k: &str| std::env::var(k).ok();
            if let Err(e) = agent.auto_register(env).await {
                output::print_warning(format, &format!("Automatic registration failed: {}", e.user_message()));
            }
            if !agent.status().registered {
                return Err(AppError::state("This machine is not registered with a management console", "Run `ufop agent register` or `ufop agent claim` first."));
            }
            let st = agent.status();
            eprintln!("ufop agent: connected to {} as {}; press Ctrl-C to stop", st.console_url, st.machine_id);
            let loop_task = tokio::spawn(agent.clone().run_loop());
            let sched = agent.clone();
            let sched_task = tokio::spawn(async move {
                loop {
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                    for id in sched.host().due_jobs(chrono::Utc::now()) {
                        spawn_run(&sched, id);
                    }
                }
            });
            let _ = tokio::signal::ctrl_c().await;
            loop_task.abort();
            sched_task.abort();
            eprintln!("ufop agent: stopped");
            Ok(0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ufop_core::agent::mock::{MockConsole, MockRequest};

    #[tokio::test(flavor = "multi_thread")]
    async fn cli_agent_applies_local_policy_runs_and_reports() {
        let t = tempfile::tempdir().unwrap();
        let (src, dst) = (t.path().join("src"), t.path().join("dst"));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("a.txt"), b"alpha").unwrap();
        let reports = Arc::new(Mutex::new(Vec::<Value>::new()));
        let completed = Arc::new(Mutex::new(Vec::<Value>::new()));
        let commands = Arc::new(Mutex::new(Vec::<Value>::new()));
        let (r2, c2, q2) = (reports.clone(), completed.clone(), commands.clone());
        let (s, d) = (src.to_string_lossy().to_string(), dst.to_string_lossy().to_string());
        let m = MockConsole::start(move |r: &MockRequest| match r.path.as_str() {
            "/api/agent/register" => (201, json!({ "machineId": "mch_c", "agentToken": "ufop_agent_c", "organization": "Acme", "heartbeatSeconds": 30 })),
            "/api/agent/heartbeat" => (200, json!({ "status": "active", "commands": std::mem::take(&mut *q2.lock().unwrap()), "policySetVersion": "v1", "policiesChanged": r.body["policySetVersion"] != "v1", "heartbeatSeconds": 30 })),
            "/api/agent/policies" => (200, json!({ "policySetVersion": "v1",
                "policies": [
                    { "id": "fpl_l", "name": "Local", "version": 2, "job": { "name": "Local", "sources": [s], "encrypted": true, "passphraseCredentialId": "central:crd_p",
                        "destination": { "type": "local", "path": d }, "options": { "block_size": 4096, "volume_size": 65536, "quota_disable": true },
                        "schedule": { "enabled": true, "interval_minutes": 60 } } },
                    { "id": "fpl_s3", "name": "Cloud", "version": 1, "job": { "name": "Cloud", "sources": ["/x"], "encrypted": false, "destination": { "type": "cloud", "config": {} } } }
                ],
                "credentials": [{ "credentialId": "central:crd_p", "name": "p", "secret": "pw-from-console", "updatedAt": "" }] })),
            "/api/agent/report" => { r2.lock().unwrap().push(r.body.clone()); (201, json!({ "id": "r" })) }
            p if p.starts_with("/api/agent/commands/") => { c2.lock().unwrap().push(r.body.clone()); (200, json!({ "ok": true })) }
            _ => (200, json!({ "ok": true })),
        })
        .unwrap();
        let (dir, data) = (t.path().join("cfg"), t.path().join("data"));
        let fmt = OutputFormat::Json;
        assert_eq!(run(AgentCommands::Register { url: m.url.clone(), token: "ufop_reg_1".into(), tags: Some("a,b".into()) }, &fmt, dir.clone(), data.clone()).await.unwrap(), 0);
        assert_eq!(std::fs::read_to_string(dir.join("agent-token")).unwrap(), "ufop_agent_c");
        let state = std::fs::read_to_string(dir.join("agent.json")).unwrap();
        assert!(!state.contains("pw-from-console") && !state.contains("ufop_agent_c"), "no secrets in agent.json");
        assert!(state.contains("policy-fpl_l"), "{state}");

        // A new process: credentials are re-fetched on its first cycle.
        let a = open_agent(dir.clone(), data.clone()).await.unwrap();
        let st = a.status();
        assert!(st.registered);
        assert!(st.managed_policies.iter().any(|p| p.policy_id == "fpl_s3" && p.error.as_deref().unwrap().contains("local folders")));
        assert_eq!(a.host().due_jobs(chrono::Utc::now()), vec!["policy-fpl_l"]);
        *commands.lock().unwrap() = vec![
            json!({ "id": "c1", "kind": "run_backup", "payload": { "jobId": "policy-fpl_l" } }),
            json!({ "id": "c2", "kind": "rpc", "payload": { "command": "backup_list_jobs", "args": {} } }),
            json!({ "id": "c3", "kind": "rpc", "payload": { "command": "backup_diagnose", "args": {} } }),
        ];
        a.cycle().await.unwrap();
        for _ in 0..200 {
            if a.status().pending_reports > 0 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        a.cycle().await.unwrap();
        let rep = reports.lock().unwrap()[0].clone();
        assert_eq!((rep["backup_id"].as_str(), rep["result"].as_str()), (Some("policy-fpl_l"), Some("success")), "{rep}");
        assert_eq!(rep["stats"]["version_count"], 1);
        assert_eq!(rep["stats"]["schedule_interval_minutes"], 60);
        let done = completed.lock().unwrap().clone();
        assert_eq!(done[0]["ok"], true);
        assert_eq!(done[1]["result"][0]["id"], "policy-fpl_l");
        assert_eq!(done[2]["ok"], false);
        assert!(a.host().due_jobs(chrono::Utc::now()).is_empty(), "just ran");

        assert_eq!(run(AgentCommands::Unregister { force: false }, &fmt, dir.clone(), data.clone()).await.unwrap(), 0);
        assert!(!dir.join("agent-token").exists());
        let a = open_agent(dir, data).await.unwrap();
        assert!(!a.status().registered && a.host().jobs_snapshot().is_empty());
    }
}
