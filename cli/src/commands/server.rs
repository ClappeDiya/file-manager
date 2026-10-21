//! `ufop server …` — control a running `ufop-server` (ServerUtil
//! equivalent): sign in and store credentials, list/run jobs, pause and
//! resume the scheduler, read logs, issue automation tokens and check
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
        /// Use a long-lived automation token instead of a password
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
}

struct Client {
    http: reqwest::Client,
    url: String,
    cfg: CliConfig,
}

fn refresh_from(headers: &reqwest::header::HeaderMap) -> Option<String> {
    headers.get_all("set-cookie").iter().filter_map(|v| v.to_str().ok()).find_map(|c| c.strip_prefix("ufop_refresh=").map(|r| r.split(';').next().unwrap_or("").to_string())).filter(|s| !s.is_empty())
}

impl Client {
    fn new() -> Result<Self, String> {
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

    async fn call(&mut self, method: reqwest::Method, path: &str, body: Option<Value>) -> Result<Value, String> {
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

    async fn job_id(&mut self, name_or_id: &str) -> Result<String, String> {
        let jobs = self.call(reqwest::Method::GET, "/jobs", None).await?;
        jobs.as_array()
            .into_iter()
            .flatten()
            .find(|j| j["id"] == name_or_id || j["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name_or_id)))
            .and_then(|j| j["id"].as_str().map(String::from))
            .ok_or_else(|| format!("No job named or with id '{name_or_id}'"))
    }
}

fn show(format: &OutputFormat, v: &Value) {
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
