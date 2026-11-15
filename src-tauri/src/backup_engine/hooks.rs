//! Backup run hooks: pre/post scripts and HTTP report callbacks.
//!
//! Scripts receive the run context through `UFOP_*` environment variables
//! (and, after a run, a JSON/text result file), may print `LOG:INFO`,
//! `LOG:WARN` or `LOG:ERROR` lines that become run-log entries, and are
//! killed after their timeout. A non-zero exit from a *required* pre-script
//! aborts the operation.
//!
//! Each script runs in one of three [`HookMode`]s: `required` (waits; a
//! failure aborts), `optional` (waits; a failure is a warning) or `async`
//! (started and not waited for: its outcome goes into the run log if the
//! operation is still running when it ends, otherwise into the app log; the
//! timeout still kills it in the background).

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptConfig {
    /// Command line run before the operation (arguments are shell-split).
    pub run_before: Option<String>,
    /// Abort the operation when the pre-script fails or times out (older
    /// setting; `before_mode` replaces it when set).
    pub run_before_required: bool,
    /// How the pre-script runs (`None`: required or optional per `run_before_required`).
    #[serde(default)]
    pub before_mode: Option<HookMode>,
    pub run_after: Option<String>,
    /// How the post-script runs (`None` = optional: waited for, failures logged).
    #[serde(default)]
    pub after_mode: Option<HookMode>,
    /// Run as soon as the backup has finished reading the sources (before
    /// uploads complete), e.g. to release a snapshot or restart a service.
    #[serde(default)]
    pub run_on_source_release: Option<String>,
    /// 0 = 60 s default.
    pub timeout_secs: u64,
    /// `json` (default) or `text` for the result file given to post-scripts.
    pub result_format: String,
}

/// How a hook script runs relative to the operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookMode {
    /// Wait for it; a failure or timeout aborts the operation (pre-scripts).
    Required,
    /// Wait for it; a failure is logged as a warning.
    Optional,
    /// Start it and carry on (fire-and-forget).
    Async,
}

impl ScriptConfig {
    /// The pre-script's mode, mapping the older `run_before_required`.
    pub fn before_mode(&self) -> HookMode {
        self.before_mode.unwrap_or(if self.run_before_required { HookMode::Required } else { HookMode::Optional })
    }

    /// The post-script's mode (`required` behaves as optional: the operation is over).
    pub fn after_mode(&self) -> HookMode {
        match self.after_mode {
            Some(HookMode::Async) => HookMode::Async,
            _ => HookMode::Optional,
        }
    }
}

/// Where an asynchronous hook's outcome goes: the run log while the
/// operation is running ([`AsyncHookLog::close`] ends that), the app log after.
#[derive(Clone, Default)]
pub struct AsyncHookLog(std::sync::Arc<std::sync::Mutex<Option<Vec<LogLine>>>>);

impl AsyncHookLog {
    pub fn open() -> Self {
        Self(std::sync::Arc::new(std::sync::Mutex::new(Some(Vec::new()))))
    }

    /// Deliver lines; `false` when the run log is closed (they went to the app log).
    pub fn push(&self, lines: Vec<LogLine>) -> bool {
        let mut g = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match g.as_mut() {
            Some(v) => {
                v.extend(lines);
                true
            }
            None => {
                for l in lines {
                    match l.level {
                        LogLevel::Error => tracing::error!(target: "ufop::backup", "async hook: {}", l.message),
                        LogLevel::Warning => tracing::warn!(target: "ufop::backup", "async hook: {}", l.message),
                        LogLevel::Info => tracing::info!(target: "ufop::backup", "async hook: {}", l.message),
                    }
                }
                false
            }
        }
    }

    /// Lines delivered so far; later ones go to the app log.
    pub fn close(&self) -> Vec<LogLine> {
        self.0.lock().unwrap_or_else(|e| e.into_inner()).take().unwrap_or_default()
    }
}

/// Start a script without waiting for it. Its log lines and exit status
/// go to `sink`; the timeout kills it in the background. The handle
/// resolves to whether the outcome reached the run log.
pub fn spawn_script(command_line: &str, env: &BTreeMap<String, String>, timeout_secs: u64, label: &str, sink: AsyncHookLog) -> tokio::task::JoinHandle<bool> {
    let (cmd, env, label) = (command_line.to_string(), env.clone(), label.to_string());
    tokio::spawn(async move {
        let mut lines = match run_script(&cmd, &env, timeout_secs).await {
            Ok(out) => {
                let mut l = out.log.clone();
                if out.succeeded() {
                    l.push(LogLine::info(format!("{label} (async) finished")));
                } else if !out.timed_out {
                    l.push(LogLine::warn(format!("{label} (async) failed with exit code {:?}", out.exit_code)));
                }
                l
            }
            Err(e) => vec![LogLine::warn(format!("{label} (async): {}", e.user_message()))],
        };
        for l in &mut lines {
            l.tag.get_or_insert_with(|| "script".into());
        }
        sink.push(lines)
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    #[default]
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub level: LogLevel,
    pub message: String,
    /// Source area (`engine`, `script`, `replica`, `report`, …) for filters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

impl LogLine {
    pub fn info(m: impl Into<String>) -> Self {
        Self { level: LogLevel::Info, message: m.into(), tag: None }
    }
    pub fn warn(m: impl Into<String>) -> Self {
        Self { level: LogLevel::Warning, message: m.into(), tag: None }
    }
    pub fn error(m: impl Into<String>) -> Self {
        Self { level: LogLevel::Error, message: m.into(), tag: None }
    }
    pub fn tagged(mut self, tag: &str) -> Self {
        self.tag = Some(tag.to_string());
        self
    }
}

/// Parse `LOG:INFO|WARN|ERROR message` lines from script output.
pub fn parse_script_log(output: &str) -> Vec<LogLine> {
    output
        .lines()
        .filter_map(|l| {
            let l = l.trim();
            let (lvl, msg) = l.strip_prefix("LOG:")?.split_once(' ')?;
            let level = match lvl {
                "INFO" => LogLevel::Info,
                "WARN" | "WARNING" => LogLevel::Warning,
                "ERROR" => LogLevel::Error,
                _ => return None,
            };
            Some(LogLine { level, message: format!("script: {}", msg.trim()), tag: Some("script".into()) })
        })
        .collect()
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScriptOutcome {
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub log: Vec<LogLine>,
}

impl ScriptOutcome {
    pub fn succeeded(&self) -> bool {
        !self.timed_out && self.exit_code == Some(0)
    }
}

/// Run one hook script with the given environment.
pub async fn run_script(command_line: &str, env: &BTreeMap<String, String>, timeout_secs: u64) -> Result<ScriptOutcome, AppError> {
    let parts = shlex::split(command_line)
        .filter(|p| !p.is_empty())
        .ok_or_else(|| AppError::validation(format!("Cannot parse script command line '{command_line}'")))?;
    let mut cmd = tokio::process::Command::new(&parts[0]);
    cmd.args(&parts[1..]).envs(env).kill_on_drop(true).stdin(std::process::Stdio::null());
    let child = cmd.output();
    let timeout = Duration::from_secs(if timeout_secs == 0 { 60 } else { timeout_secs });
    match tokio::time::timeout(timeout, child).await {
        Err(_) => Ok(ScriptOutcome {
            exit_code: None,
            timed_out: true,
            log: vec![LogLine::error(format!("script '{}' timed out after {}s", parts[0], timeout.as_secs()))],
        }),
        Ok(Err(e)) => Ok(ScriptOutcome {
            exit_code: None,
            timed_out: false,
            log: vec![LogLine::error(format!("script '{}' could not start: {e}", parts[0]))],
        }),
        Ok(Ok(out)) => {
            let mut log = parse_script_log(&String::from_utf8_lossy(&out.stdout));
            log.extend(parse_script_log(&String::from_utf8_lossy(&out.stderr)));
            let code = out.status.code();
            if code != Some(0) {
                log.push(LogLine::warn(format!("script '{}' exited with {:?}", parts[0], code)));
            }
            Ok(ScriptOutcome { exit_code: code, timed_out: false, log })
        }
    }
}

/// A command whose standard output is backed up as one virtual file of
/// each version (for example `pg_dump mydb`). It gets the same `UFOP_*`
/// environment as hook scripts; a non-zero exit fails the run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct StdinCommand {
    /// Command line, split like hook scripts' (no shell unless you run one).
    pub command: String,
    /// Name of the file in the version: `name` → `/stdin/name`, or an
    /// absolute path such as `/dumps/mydb.sql`.
    pub path: String,
    /// Kill the command after this many seconds (0 = no limit).
    pub timeout_secs: u64,
}

impl StdinCommand {
    pub fn is_set(&self) -> bool {
        !self.command.trim().is_empty()
    }

    pub fn validate(&self) -> Result<(), AppError> {
        if !self.is_set() {
            return Ok(());
        }
        if shlex::split(&self.command).is_none_or(|p| p.is_empty()) {
            return Err(AppError::validation(format!("Cannot parse the backup command line '{}'", self.command)));
        }
        if self.virtual_path().split('/').any(|c| c == "..") {
            return Err(AppError::validation("The command output's file name cannot contain '..'"));
        }
        Ok(())
    }

    /// Path of the virtual file in the version.
    pub fn virtual_path(&self) -> String {
        let p = self.path.trim();
        ufop_core::backup::reader::stream_path(if p.is_empty() { "command-output" } else { p })
    }
}

/// Kills the command if the backup stops before reading to the end.
struct ChildGuard {
    child: std::sync::Mutex<Option<std::process::Child>>,
}

impl ChildGuard {
    fn kill(&self) {
        if let Some(c) = self.child.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

struct ChildOutput {
    out: std::process::ChildStdout,
    _guard: std::sync::Arc<ChildGuard>,
}

impl std::io::Read for ChildOutput {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.out.read(buf)
    }
}

/// Start `cmd` and return its standard output as a stream source for the
/// backup engine. Called on the engine's (blocking) thread.
pub fn command_stream(cmd: &StdinCommand, env: &BTreeMap<String, String>) -> Result<ufop_core::backup::StreamSource, AppError> {
    use std::io::Read;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    cmd.validate()?;
    let parts = shlex::split(&cmd.command).filter(|p| !p.is_empty()).ok_or_else(|| AppError::validation("The backup command is empty"))?;
    let mut child = std::process::Command::new(&parts[0])
        .args(&parts[1..])
        .envs(env)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| AppError::file_op(format!("Could not start the backup command '{}': {e}", parts[0]), "Check the command and that the program is installed."))?;
    let out = child.stdout.take().ok_or_else(|| AppError::internal("command output not captured"))?;
    let stderr = child.stderr.take();
    let guard = Arc::new(ChildGuard { child: Mutex::new(Some(child)) });
    // Keep the end of stderr for the error message.
    let err_tail: Arc<Mutex<String>> = Arc::default();
    let err_thread = stderr.map(|mut e| {
        let tail = err_tail.clone();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = e.read_to_end(&mut buf);
            let text = String::from_utf8_lossy(&buf);
            let t = text.trim();
            let start = t.char_indices().rev().nth(500).map(|(i, _)| i).unwrap_or(0);
            *tail.lock().unwrap_or_else(|e| e.into_inner()) = t[start..].to_string();
        })
    });
    let timed_out = Arc::new(AtomicBool::new(false));
    let finished = Arc::new(AtomicBool::new(false));
    if cmd.timeout_secs > 0 {
        let (weak, timed_out, finished, limit) = (Arc::downgrade(&guard), timed_out.clone(), finished.clone(), std::time::Duration::from_secs(cmd.timeout_secs));
        std::thread::spawn(move || {
            let start = std::time::Instant::now();
            while !finished.load(Ordering::SeqCst) {
                if start.elapsed() >= limit {
                    if let Some(g) = weak.upgrade() {
                        timed_out.store(true, Ordering::SeqCst);
                        g.kill();
                    }
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
    }
    let (program, timeout) = (parts[0].clone(), cmd.timeout_secs);
    let finish_guard = guard.clone();
    let reader = ChildOutput { out, _guard: guard };
    Ok(ufop_core::backup::StreamSource::new(cmd.virtual_path(), reader).with_finish(move || {
        let status = finish_guard.child.lock().unwrap_or_else(|e| e.into_inner()).take().map(|mut c| c.wait());
        finished.store(true, Ordering::SeqCst);
        if let Some(t) = err_thread {
            let _ = t.join();
        }
        let tail = err_tail.lock().map(|t| t.clone()).unwrap_or_default();
        let detail = if tail.is_empty() { String::new() } else { format!(": {tail}") };
        if timed_out.load(Ordering::SeqCst) {
            return Err(format!("'{program}' timed out after {timeout}s{detail}"));
        }
        match status {
            Some(Ok(s)) if s.success() => Ok(()),
            Some(Ok(s)) => Err(format!("'{program}' exited with {}{detail}", s.code().map(|c| format!("status {c}")).unwrap_or_else(|| "a signal".into()))),
            Some(Err(e)) => Err(format!("'{program}': {e}")),
            None => Err(format!("'{program}' was stopped")),
        }
    }))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReportFormat {
    Json,
    Form,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ReportConfig {
    /// Callback URLs; each receives every report.
    pub http_urls: Vec<String>,
    /// POST, PUT, PATCH or GET.
    pub http_method: String,
    pub format: ReportFormat,
    /// Results that trigger a report: `success`, `warning`, `error`.
    pub send_on: Vec<String>,
    pub retries: u32,
    /// Accept self-signed / invalid TLS certificates on callback URLs.
    pub accept_invalid_certs: bool,
    /// Message template; `{{name}}`, `{{operation}}`, `{{result}}`,
    /// `{{machine}}`, `{{duration}}`, `{{files}}`, `{{bytes}}`, `{{errors}}`.
    pub message_template: String,
    /// Extra JSON fields merged into every payload.
    pub extra_fields: BTreeMap<String, String>,
    /// Override the machine name sent in reports.
    pub machine_name: Option<String>,
    /// Override the backup identifier sent in reports.
    pub backup_id: Option<String>,
    /// Max log lines included.
    pub max_log_lines: usize,
    /// Replace file paths in reports and notifications with placeholders.
    #[serde(default)]
    pub redact_paths: bool,
    /// Trusted CA or pinned self-signed certificate for callback servers (PEM).
    #[serde(default)]
    pub ca_pem: Option<String>,
    /// First retry delay (ms); doubles each attempt.
    #[serde(default = "default_retry_delay")]
    pub retry_delay_ms: u64,
    /// Lowest log level included in reports.
    #[serde(default)]
    pub min_log_level: LogLevel,
    /// SHA-256 certificate fingerprints; when set, only these are accepted.
    #[serde(default)]
    pub pinned_sha256: Vec<String>,
    /// Extra request headers; values may use the template placeholders.
    /// Secret values live in the credential store (see `WebhookHeader`).
    #[serde(default)]
    pub headers: Vec<super::notify::webhook::WebhookHeader>,
}

fn default_retry_delay() -> u64 {
    500
}

impl Default for ReportConfig {
    fn default() -> Self {
        Self {
            http_urls: Vec::new(),
            http_method: "POST".into(),
            format: ReportFormat::Json,
            send_on: vec!["warning".into(), "error".into()],
            retries: 3,
            accept_invalid_certs: false,
            message_template: "{{operation}} of {{name}} on {{machine}}: {{result}}".into(),
            extra_fields: BTreeMap::new(),
            machine_name: None,
            backup_id: None,
            max_log_lines: 100,
            redact_paths: false,
            ca_pem: None,
            retry_delay_ms: 500,
            min_log_level: LogLevel::Info,
            pinned_sha256: Vec::new(),
            headers: Vec::new(),
        }
    }
}

/// Substitute `{{var}}` placeholders.
pub fn render_template(template: &str, vars: &BTreeMap<String, String>) -> String {
    let mut out = template.to_string();
    for (k, v) in vars {
        out = out.replace(&format!("{{{{{k}}}}}"), v);
    }
    out
}

pub fn machine_name() -> String {
    hostname::get().map(|h| h.to_string_lossy().into_owned()).unwrap_or_else(|_| "unknown".into())
}

/// Send the report to every configured URL; returns per-URL failures.
pub async fn send_reports(cfg: &ReportConfig, result: &str, vars: &BTreeMap<String, String>, payload: serde_json::Value) -> Vec<String> {
    if cfg.http_urls.is_empty() || !cfg.send_on.iter().any(|s| s == result) {
        return Vec::new();
    }
    let mut builder = crate::net::builder().timeout(Duration::from_secs(30)).danger_accept_invalid_certs(cfg.accept_invalid_certs);
    if !cfg.pinned_sha256.is_empty() {
        match crate::security::tls::client_config(false, None, &cfg.pinned_sha256) {
            Ok(tls) => builder = crate::net::builder().timeout(Duration::from_secs(30)).use_preconfigured_tls(tls),
            Err(e) => return vec![e.user_message()],
        }
    } else if let Some(pem) = cfg.ca_pem.as_deref().filter(|p| !p.trim().is_empty()) {
        match reqwest::Certificate::from_pem(pem.as_bytes()) {
            Ok(c) => builder = builder.add_root_certificate(c),
            Err(e) => return vec![format!("Invalid callback CA certificate: {e}")],
        }
    }
    let client = match builder.build() {
        Ok(c) => c,
        Err(e) => return vec![format!("HTTP client: {e}")],
    };
    let mut body = serde_json::Map::new();
    body.insert("message".into(), render_template(&cfg.message_template, vars).into());
    for (k, v) in vars {
        body.insert(k.clone(), v.clone().into());
    }
    for (k, v) in &cfg.extra_fields {
        body.insert(k.clone(), v.clone().into());
    }
    body.insert("data".into(), payload);
    let mut failures = Vec::new();
    // Same schedule as before: first delay `retry_delay_ms`, doubling, at
    // most 64× (shared policy, see ufop_core::retry).
    let first = cfg.retry_delay_ms.max(1);
    let policy = ufop_core::retry::RetryPolicy { retries: cfg.retries, delay_ms: first, exponential: true, max_delay_ms: first.saturating_mul(64) };
    let method = reqwest::Method::from_bytes(cfg.http_method.to_uppercase().as_bytes()).unwrap_or(reqwest::Method::POST);
    for url in &cfg.http_urls {
        let sent = ufop_core::retry::retry_async(
            &policy,
            |_| {
                let mut req = client.request(method.clone(), url);
                for h in &cfg.headers {
                    let v = super::notify::webhook::render(&h.value, vars, super::notify::webhook::Escape::Header);
                    req = req.header(h.name.trim(), v);
                }
                req = match (cfg.format, method == reqwest::Method::GET) {
                    (_, true) => req.query(&body.iter().filter(|(_, v)| v.is_string()).map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect::<Vec<_>>()),
                    (ReportFormat::Json, false) => req.json(&body),
                    (ReportFormat::Form, false) => req.form(&[("message", body["message"].as_str().unwrap_or_default().to_string()), ("data", serde_json::Value::Object(body.clone()).to_string())]),
                };
                async move {
                    match req.send().await {
                        Ok(r) if r.status().is_success() => Ok(()),
                        Ok(r) => Err(format!("HTTP {}", r.status())),
                        Err(e) => Err(e.to_string()),
                    }
                }
            },
            |_| true,
            |_, _, _| {},
        )
        .await;
        if let Err(last) = sent.result {
            failures.push(format!("report to {url} failed: {last}"));
        }
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_log_lines() {
        let l = parse_script_log("hello\nLOG:INFO started\nLOG:WARN low disk\nLOG:ERROR boom\nLOG:DEBUG x");
        assert_eq!(l.len(), 3);
        assert_eq!(l[1].level, LogLevel::Warning);
        assert_eq!(l[2].message, "script: boom");
    }

    #[test]
    fn templates() {
        let mut v = BTreeMap::new();
        v.insert("name".to_string(), "Docs".to_string());
        v.insert("result".to_string(), "success".to_string());
        assert_eq!(render_template("{{name}}: {{result}} {{x}}", &v), "Docs: success {{x}}");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn script_exit_timeout_and_env() {
        let mut env = BTreeMap::new();
        env.insert("UFOP_BACKUP_NAME".into(), "job".into());
        let ok = run_script("sh -c 'echo LOG:INFO $UFOP_BACKUP_NAME'", &env, 5).await.unwrap();
        assert!(ok.succeeded());
        assert_eq!(ok.log[0].message, "script: job");
        let fail = run_script("sh -c 'exit 3'", &env, 5).await.unwrap();
        assert_eq!(fail.exit_code, Some(3));
        let slow = run_script("sleep 5", &env, 1).await.unwrap();
        assert!(slow.timed_out && !slow.succeeded());
        let missing = run_script("/nonexistent/script", &env, 1).await.unwrap();
        assert!(!missing.succeeded());
    }

    #[test]
    fn hook_modes_map_the_older_flag() {
        let mut c = ScriptConfig::default();
        assert_eq!(c.before_mode(), HookMode::Optional);
        c.run_before_required = true;
        assert_eq!(c.before_mode(), HookMode::Required);
        c.before_mode = Some(HookMode::Async);
        assert_eq!(c.before_mode(), HookMode::Async, "the explicit mode wins");
        assert_eq!(c.after_mode(), HookMode::Optional);
        c.after_mode = Some(HookMode::Required);
        assert_eq!(c.after_mode(), HookMode::Optional);
        // Old JSON without modes still parses.
        let old: ScriptConfig = serde_json::from_str(r#"{"run_before":"x","run_before_required":true}"#).unwrap();
        assert_eq!(old.before_mode(), HookMode::Required);
        let new: ScriptConfig = serde_json::from_str(r#"{"run_before":"x","before_mode":"async"}"#).unwrap();
        assert_eq!(new.before_mode(), HookMode::Async);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn async_hooks_do_not_block_and_log_late() {
        let env = BTreeMap::new();
        // Delivered to the run log while it is open.
        let sink = AsyncHookLog::open();
        let started = std::time::Instant::now();
        let h = spawn_script("sh -c 'sleep 0.3; echo LOG:INFO done'", &env, 5, "Pre-backup script", sink.clone());
        assert!(started.elapsed() < std::time::Duration::from_millis(200), "spawning does not wait");
        assert!(h.await.unwrap());
        let lines = sink.close();
        assert!(lines.iter().any(|l| l.message == "script: done"), "{lines:?}");
        assert!(lines.iter().any(|l| l.message.contains("(async) finished")));

        // Run already over: the outcome goes to the app log instead.
        let sink = AsyncHookLog::open();
        let h = spawn_script("sh -c 'sleep 0.2; exit 4'", &env, 5, "Post-backup script", sink.clone());
        assert!(sink.close().is_empty());
        assert!(!h.await.unwrap());

        // The timeout still kills it in the background.
        let sink = AsyncHookLog::open();
        let started = std::time::Instant::now();
        let h = spawn_script("sleep 30", &env, 1, "Pre-backup script", sink.clone());
        assert!(h.await.unwrap());
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
        assert!(sink.close().iter().any(|l| l.message.contains("timed out")));
    }

    #[tokio::test]
    async fn report_filtering_skips_unselected_results() {
        let cfg = ReportConfig { http_urls: vec!["http://127.0.0.1:9/x".into()], send_on: vec!["error".into()], ..Default::default() };
        assert!(send_reports(&cfg, "success", &BTreeMap::new(), serde_json::json!({})).await.is_empty());
        let cfg = ReportConfig { retries: 0, ..cfg };
        assert_eq!(send_reports(&cfg, "error", &BTreeMap::new(), serde_json::json!({})).await.len(), 1);
    }
}
