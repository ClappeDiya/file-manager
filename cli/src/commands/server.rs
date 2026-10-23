//! `ufop server …` — control a running `ufop-server` (ServerUtil
//! equivalent): sign in and store credentials, list/run jobs, pause and
//! resume the scheduler, read logs, manage automation and scoped API tokens and check
//! health with script-friendly exit codes (0 healthy, 1 degraded, 2 down).

use crate::config::CliConfig;
use crate::output;
use crate::OutputFormat;
use clap::Subcommand;
use serde_json::{json, Value};

#[derive(Subcommand, Debug)]
pub enum ServerCommands {
    /// Sign in with the server password (or store an automation token) and save it
    Login {
        #[arg(long, env = "UFOP_SERVER_URL", default_value = "http://127.0.0.1:8200")]
        url: String,
        #[arg(long, env = "UFOP_SERVER_PASSWORD")]
        password: Option<String>,
        /// Use an automation or API token (ufop_pat_…) instead of a password
        #[arg(long, env = "UFOP_SERVER_TOKEN")]
        token: Option<String>,
    },
    /// Forget stored server credentials
    Logout,
    /// Health check (exit 0 healthy, 1 degraded, 2 unreachable)
    Health,
    /// Scheduler state, running and queued tasks
    Status,
    /// List backup jobs
    List,
    /// Start a backup by job id or name
    Run {
        job: String,
        #[arg(long)]
        dry_run: bool,
        /// Wait until it finishes and exit with its result
        #[arg(long)]
        wait: bool,
    },
    /// Back up every job now (read-only and busy jobs are skipped)
    RunAll {
        /// Only these jobs (id or name, repeatable)
        #[arg(long = "job")]
        jobs: Vec<String>,
        /// Only jobs with a schedule
        #[arg(long)]
        scheduled_only: bool,
        #[arg(long)]
        dry_run: bool,
        /// Wait until every started backup finished; exit with the worst result
        #[arg(long)]
        wait: bool,
    },
    /// Cancel a task or all tasks of a job
    Cancel { id: String },
    /// Pause scheduled backups
    Pause {
        /// Minutes (default: until resumed)
        #[arg(long)]
        minutes: Option<u32>,
    },
    /// Resume scheduled backups
    Resume,
    /// Recent run history
    Logs {
        #[arg(long)]
        job: Option<String>,
        /// Number of runs to show
        #[arg(long, default_value_t = 20)]
        count: u32,
    },
    /// Issue a long-lived automation token (must be enabled on the server)
    IssueToken,
    /// Stop the server gracefully: running backups stop at a safe point
    /// (partial version) within the grace period, then it exits; run it again to stop at once
    Shutdown {
        /// Seconds running backups get (default: the server's --shutdown-grace)
        #[arg(long)]
        grace: Option<u64>,
    },
    /// Named, scoped, expiring API tokens (read, run, write, admin)
    #[command(subcommand)]
    Tokens(TokenCommands),
    /// Share links to files and folders under the server's --share-root
    #[command(subcommand)]
    Shares(ShareCommands),
    /// Policies of shared folders: lock, accepted types, limits, link rules, intake naming
    #[command(subcommand)]
    Folders(FolderCommands),
}

#[derive(Subcommand, Debug)]
pub enum FolderCommands {
    /// Folders that have a policy of their own
    List,
    /// A folder's own policy and the one in force (with what it inherits)
    Show { path: String },
    /// Change a folder's own policy (only the options given change)
    Set {
        path: String,
        /// Take no uploads and no upload links
        #[arg(long, conflicts_with = "unlock")]
        lock: bool,
        #[arg(long)]
        unlock: bool,
        /// Accept only these extensions (comma-separated; "" = inherit)
        #[arg(long, value_delimiter = ',')]
        allow: Option<Vec<String>>,
        /// Refuse these extensions (comma-separated; "" = inherit)
        #[arg(long, value_delimiter = ',')]
        block: Option<Vec<String>>,
        /// Largest upload per file, in MB (0 = inherit)
        #[arg(long)]
        max_upload_mb: Option<i64>,
        /// Links must have a password (true/false)
        #[arg(long)]
        require_password: Option<bool>,
        /// Links must expire within N days (0 = inherit)
        #[arg(long)]
        max_link_days: Option<i64>,
        /// How uploads are named: keep, timestamp or sequence
        #[arg(long, value_parser = ["keep", "timestamp", "sequence"])]
        intake: Option<String>,
    },
    /// Remove a folder's own policy (it inherits again)
    Clear { path: String },
    /// Copy one folder's own policy to another
    Copy { from: String, to: String },
}

#[derive(Subcommand, Debug)]
pub enum ShareCommands {
    /// List links (their tokens are never shown again)
    List,
    /// Create a link; the URL is shown once
    Create {
        /// File or folder on the server (under a --share-root)
        path: String,
        #[arg(long)]
        name: Option<String>,
        /// download (browse and download), upload (drop box) or both
        #[arg(long, default_value = "download", value_parser = ["download", "upload", "both"])]
        kind: String,
        /// Require this password (8+ characters)
        #[arg(long, env = "UFOP_SHARE_PASSWORD")]
        password: Option<String>,
        /// Expire after this many days
        #[arg(long)]
        expires_in_days: Option<i64>,
        /// Stop after this many downloads
        #[arg(long)]
        max_downloads: Option<i64>,
        /// Largest upload per file, in MB (default 1024)
        #[arg(long)]
        max_upload_mb: Option<i64>,
    },
    /// Turn a link off
    Revoke { id: String },
    /// A link's access log
    Log {
        id: String,
        /// Newest N events
        #[arg(long = "last", short = 'n', default_value_t = 50)]
        last: i64,
    },
}

#[derive(Subcommand, Debug)]
pub enum TokenCommands {
    /// List API tokens (never their values)
    List,
    /// Create a token; its value is shown once
    Create {
        name: String,
        /// read (GETs), run (+ start/cancel backups, pause/resume), write (+ change jobs), admin (everything)
        #[arg(long, default_value = "read", value_parser = ["read", "run", "write", "admin"])]
        scope: String,
        /// Expires after this many days (1–365)
        #[arg(long, default_value_t = 90)]
        days: i64,
    },
    /// Revoke a token by id
    Revoke { id: String },
}

pub(crate) fn urlencode(s: &str) -> String {
    s.bytes().map(|b| if b.is_ascii_alphanumeric() || b"-_.~/".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") }).collect()
}

pub(crate) struct Client {
    http: reqwest::Client,
    pub(crate) url: String,
    cfg: CliConfig,
}

fn refresh_from(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers.get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find_map(|c| c.strip_prefix("ufop_refresh=").map(|r| r.split(';').next().unwrap_or("").to_string())).filter(|s| !s.is_empty())
}

impl Client {
    pub(crate) fn new() -> Result<Self, String> {
        let cfg = CliConfig::load();
        let url = cfg.server_url.clone().ok_or("Not signed in: run `ufop server login --url …` first")?;
        Ok(Self { http: reqwest::Client::new(), url, cfg })
    }

    async fn renew(&mut self) -> Result<(), String> {
        let rt = self.cfg.server_refresh.clone().ok_or("Session expired: run `ufop server login` again")?;
        let r = self.http.post(format!("{}/api/v1/auth/refresh", self.url)).header("Cookie", format!("ufop_refresh={rt}")).send().await.map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err("Session expired: run `ufop server login` again".into());
        }
        self.cfg.server_refresh = refresh_from(r.headers());
        let v: Value = r.json().await.map_err(|e| e.to_string())?;
        self.cfg.server_token = v["access_token"].as_str().map(String::from);
        self.cfg.save().map_err(|e| e.to_string())
    }

    /// A request with the stored credentials (for streamed bodies).
    pub(crate) fn raw(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let req = self.http.request(method, format!("{}/api/v1{path}", self.url));
        match &self.cfg.server_token {
            Some(t) => req.bearer_auth(t),
            None => req,
        }
    }

    pub(crate) async fn call(&mut self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, String> {
        for attempt in 0..2 {
            let mut req = self.http.request(method.clone(), format!("{}/api/v1{path}", self.url));
            if let Some(t) = &self.cfg.server_token {
                req = req.bearer_auth(t);
            }
            if let Some(b) = &body {
                req = req.json(b);
            }
            let r = req.send().await.map_err(|e| format!("Server unreachable: {e}"))?;
            if r.status() == 401 && attempt == 0 && self.cfg.server_refresh.is_some() {
                self.renew().await?;
                continue;
            }
            let ok = r.status().is_success();
            let v: Value = r.json().await.unwrap_or(Value::Null);
            if !ok {
                return Err(format!("{} {}", v["message"].as_str().unwrap_or("Request failed"), v["advice"].as_str().unwrap_or("")).trim().to_string());
            }
            return Ok(v);
        }
        Err("Not authorized".into())
    }

    pub(crate) async fn job_id(&mut self, name_or_id: &str) -> Result<String, String> {
        let jobs = self.call(reqwest::Method::GET, "/jobs", None).await?;
        jobs.as_array()
            .into_iter()
            .flatten()
            .find(|j| j["id"] == name_or_id || j["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name_or_id)))
            .and_then(|j| j["id"].as_str().map(String::from))
            .ok_or_else(|| format!("No job named or with id '{name_or_id}'"))
    }
}

pub(crate) fn show(format: &OutputFormat, v: &Value) {
    match format {
        OutputFormat::Human => println!("{}", serde_json::to_string_pretty(v).unwrap_or_default()),
        _ => output::print_data(format, v),
    }
}

pub async fn execute(cmd: ServerCommands, format: &OutputFormat) -> Result<u8, Box<dyn std::error::Error>> {
    let res: Result<u8, String> = async {
        match cmd {
            ServerCommands::Login { url, password, token } => {
                let mut cfg = CliConfig::load();
                let url = url.trim_end_matches('/').to_string();
                let http = reqwest::Client::new();
                if let Some(t) = token {
                    let r = http.get(format!("{url}/api/v1/status")).bearer_auth(&t).send().await.map_err(|e| e.to_string())?;
                    if !r.status().is_success() {
                        return Err("The token was rejected".into());
                    }
                    cfg.server_token = Some(t);
                    cfg.server_refresh = None;
                } else {
                    let pw = password.ok_or("Give --password (or UFOP_SERVER_PASSWORD) or --token")?;
                    let r = http.post(format!("{url}/api/v1/auth/login")).json(&json!({"password": pw})).send().await.map_err(|e| format!("Server unreachable: {e}"))?;
                    if !r.status().is_success() {
                        return Err(if r.status() == 429 { "Too many sign-in attempts; wait and retry".into() } else { "Wrong password".into() });
                    }
                    cfg.server_refresh = refresh_from(r.headers());
                    let v: Value = r.json().await.map_err(|e| e.to_string())?;
                    cfg.server_token = v["access_token"].as_str().map(String::from);
                }
                cfg.server_url = Some(url.clone());
                cfg.save().map_err(|e| e.to_string())?;
                output::print_success(format, &format!("Signed in to {url}"));
                Ok(0)
            }
            ServerCommands::Logout => {
                let mut cfg = CliConfig::load();
                cfg.server_token = None;
                cfg.server_refresh = None;
                cfg.save().map_err(|e| e.to_string())?;
                output::print_success(format, "Signed out");
                Ok(0)
            }
            ServerCommands::Health => {
                let url = CliConfig::load().server_url.unwrap_or_else(|| "http://127.0.0.1:8200".into());
                let r = reqwest::Client::new().get(format!("{url}/api/v1/health")).timeout(std::time::Duration::from_secs(10)).send().await;
                match r {
                    Ok(r) if r.status().is_success() => {
                        let v: Value = r.json().await.unwrap_or(Value::Null);
                        show(format, &v);
                        Ok(if v["scheduler_running"] == true { 0 } else { 1 })
                    }
                    Ok(r) => {
                        show(format, &json!({"status": "error", "http_status": r.status().as_u16()}));
                        Ok(2)
                    }
                    Err(e) => {
                        show(format, &json!({"status": "unreachable", "error": e.to_string()}));
                        Ok(2)
                    }
                }
            }
            ServerCommands::Status => {
                let mut c = Client::new()?;
                show(format, &c.call(reqwest::Method::GET, "/status", None).await?);
                Ok(0)
            }
            ServerCommands::List => {
                let mut c = Client::new()?;
                let jobs = c.call(reqwest::Method::GET, "/jobs", None).await?;
                let rows: Vec<Value> = jobs
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|j| json!({"id": j["id"], "name": j["name"], "versions": j["stats"]["version_count"], "last_result": j["stats"]["last_result"], "last_run_at": j["stats"]["last_run_at"]}))
                    .collect();
                show(format, &Value::Array(rows));
                Ok(0)
            }
            ServerCommands::Run { job, dry_run, wait } => {
                let mut c = Client::new()?;
                let id = c.job_id(&job).await?;
                let t = c.call(reqwest::Method::POST, &format!("/jobs/{id}/run"), Some(json!({"dry_run": dry_run}))).await?;
                if !wait {
                    show(format, &t);
                    return Ok(0);
                }
                loop {
                    let tasks = c.call(reqwest::Method::GET, "/tasks", None).await?;
                    if !tasks.as_array().is_some_and(|a| a.iter().any(|x| x["job_id"] == id.as_str())) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                let runs = c.call(reqwest::Method::GET, &format!("/runs?job_id={id}&limit=1"), None).await?;
                let last = runs.get(0).cloned().unwrap_or(Value::Null);
                show(format, &last);
                Ok(match last["status"].as_str() {
                    Some("success") => 0,
                    Some("warning") => 1,
                    _ => 2,
                })
            }
            ServerCommands::RunAll { jobs, scheduled_only, dry_run, wait } => {
                let mut c = Client::new()?;
                let mut ids = Vec::new();
                for j in &jobs {
                    ids.push(c.job_id(j).await?);
                }
                let r = c.call(reqwest::Method::POST, "/jobs/run-all", Some(json!({"job_ids": ids, "scheduled_only": scheduled_only, "dry_run": dry_run}))).await?;
                if !wait {
                    show(format, &r);
                    return Ok(if r["skipped"].as_array().is_some_and(|a| !a.is_empty()) { 1 } else { 0 });
                }
                let started: Vec<String> = r["started"].as_array().into_iter().flatten().filter_map(|s| s["job_id"].as_str().map(String::from)).collect();
                loop {
                    let tasks = c.call(reqwest::Method::GET, "/tasks", None).await?;
                    if !tasks.as_array().is_some_and(|a| a.iter().any(|x| x["job_id"].as_str().is_some_and(|j| started.iter().any(|s| s == j)))) {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
                let mut results = Vec::new();
                let mut worst = 0u8;
                for id in &started {
                    let runs = c.call(reqwest::Method::GET, &format!("/runs?job_id={id}&limit=1"), None).await?;
                    let last = runs.get(0).cloned().unwrap_or(Value::Null);
                    worst = worst.max(match last["status"].as_str() {
                        Some("success") => 0,
                        Some("warning") => 1,
                        _ => 2,
                    });
                    results.push(last);
                }
                show(format, &json!({"started": r["started"], "skipped": r["skipped"], "results": results}));
                Ok(worst)
            }
            ServerCommands::Cancel { id } => {
                let mut c = Client::new()?;
                show(format, &c.call(reqwest::Method::POST, &format!("/tasks/{id}/cancel"), None).await?);
                Ok(0)
            }
            ServerCommands::Pause { minutes } => {
                let mut c = Client::new()?;
                show(format, &c.call(reqwest::Method::POST, "/scheduler/pause", Some(json!({"minutes": minutes}))).await?);
                Ok(0)
            }
            ServerCommands::Resume => {
                let mut c = Client::new()?;
                show(format, &c.call(reqwest::Method::POST, "/scheduler/resume", None).await?);
                Ok(0)
            }
            ServerCommands::Logs { job, count: limit } => {
                let mut c = Client::new()?;
                let q = match job {
                    Some(j) => format!("/runs?job_id={}&limit={limit}", c.job_id(&j).await?),
                    None => format!("/runs?limit={limit}"),
                };
                show(format, &c.call(reqwest::Method::GET, &q, None).await?);
                Ok(0)
            }
            ServerCommands::IssueToken => {
                let mut c = Client::new()?;
                show(format, &c.call(reqwest::Method::POST, "/auth/automation-token", None).await?);
                Ok(0)
            }
            ServerCommands::Shutdown { grace } => {
                let mut c = Client::new()?;
                show(format, &c.call(reqwest::Method::POST, "/server/shutdown", Some(json!({"grace_secs": grace}))).await?);
                Ok(0)
            }
            ServerCommands::Folders(fc) => {
                let mut c = Client::new()?;
                let v = match fc {
                    FolderCommands::List => c.call(reqwest::Method::GET, "/folder-policies", None).await?["policies"].clone(),
                    FolderCommands::Show { path } => c.call(reqwest::Method::GET, &format!("/folder-policies/effective?path={}", urlencode(&path)), None).await?,
                    FolderCommands::Clear { path } => c.call(reqwest::Method::PUT, "/folder-policies", Some(json!({"path": path, "policy": {}}))).await?,
                    FolderCommands::Copy { from, to } => c.call(reqwest::Method::POST, "/folder-policies/copy", Some(json!({"from": from, "to": to}))).await?,
                    FolderCommands::Set { path, lock, unlock, allow, block, max_upload_mb, require_password, max_link_days, intake } => {
                        let cur = c.call(reqwest::Method::GET, &format!("/folder-policies/effective?path={}", urlencode(&path)), None).await?;
                        let mut p = cur["own"].as_object().cloned().unwrap_or_default();
                        let list = |v: Vec<String>| {
                            let v: Vec<String> = v.into_iter().map(|e| e.trim().to_string()).filter(|e| !e.is_empty()).collect();
                            if v.is_empty() { Value::Null } else { json!(v) }
                        };
                        if lock || unlock {
                            p.insert("locked".into(), json!(lock));
                        }
                        if let Some(a) = allow {
                            p.insert("allowed_extensions".into(), list(a));
                        }
                        if let Some(b) = block {
                            p.insert("blocked_extensions".into(), list(b));
                        }
                        if let Some(m) = max_upload_mb {
                            p.insert("max_upload_mb".into(), if m == 0 { Value::Null } else { json!(m) });
                        }
                        if let Some(r) = require_password {
                            p.insert("require_link_password".into(), json!(r));
                        }
                        if let Some(d) = max_link_days {
                            p.insert("max_link_days".into(), if d == 0 { Value::Null } else { json!(d) });
                        }
                        if let Some(i) = intake {
                            p.insert("intake_naming".into(), json!(i));
                        }
                        c.call(reqwest::Method::PUT, "/folder-policies", Some(json!({"path": path, "policy": p}))).await?
                    }
                };
                show(format, &v);
                Ok(0)
            }
            ServerCommands::Shares(sc) => {
                let mut c = Client::new()?;
                let v = match sc {
                    ShareCommands::List => {
                        let v = c.call(reqwest::Method::GET, "/shares", None).await?;
                        // Rows for people and spreadsheets; JSON/YAML keep the share roots too.
                        if matches!(format, OutputFormat::Human | OutputFormat::Csv) {
                            v["shares"].clone()
                        } else {
                            v
                        }
                    }
                    ShareCommands::Create { path, name, kind, password, expires_in_days, max_downloads, max_upload_mb } => {
                        let name = name.unwrap_or_else(|| path.trim_end_matches(['/', '\\']).rsplit(['/', '\\']).next().unwrap_or("Shared").to_string());
                        let mut v = c
                            .call(reqwest::Method::POST, "/shares", Some(json!({"name": name, "path": path, "kind": kind, "password": password, "expires_in_days": expires_in_days, "max_downloads": max_downloads, "max_upload_mb": max_upload_mb})))
                            .await?;
                        // The full link, ready to send.
                        if let Some(u) = v["url"].as_str() {
                            v["url"] = json!(format!("{}{u}", c.url.trim_end_matches('/')));
                        }
                        v
                    }
                    ShareCommands::Revoke { id } => c.call(reqwest::Method::DELETE, &format!("/shares/{id}"), None).await?,
                    ShareCommands::Log { id, last } => c.call(reqwest::Method::GET, &format!("/shares/{id}/events?limit={last}"), None).await?["events"].clone(),
                };
                show(format, &v);
                Ok(0)
            }
            ServerCommands::Tokens(t) => {
                let mut c = Client::new()?;
                let v = match t {
                    TokenCommands::List => c.call(reqwest::Method::GET, "/auth/api-tokens", None).await?,
                    TokenCommands::Create { name, scope, days } => c.call(reqwest::Method::POST, "/auth/api-tokens", Some(json!({"name": name, "scope": scope, "expires_in_days": days}))).await?,
                    TokenCommands::Revoke { id } => c.call(reqwest::Method::DELETE, &format!("/auth/api-tokens/{id}"), None).await?,
                };
                show(format, &v);
                Ok(0)
            }
        }
    }
    .await;
    match res {
        Ok(c) => Ok(c),
        Err(e) => {
            output::print_error(format, &e);
            Ok(2)
        }
    }
}
