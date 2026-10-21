//! Backup run hooks: pre/post scripts and HTTP report callbacks.
//!
//! Scripts receive the run context through `UFOP_*` environment variables
//! (and, after a run, a JSON/text result file), may print `LOG:INFO`,
//! `LOG:WARN` or `LOG:ERROR` lines that become run-log entries, and are
//! killed after their timeout. A non-zero exit from a *required* pre-script
//! aborts the operation.

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::time::Duration;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ScriptConfig {
    /// Command line run before the operation (arguments are shell-split).
    pub run_before: Option<String>,
    /// Abort the operation when the pre-script fails or times out.
    pub run_before_required: bool,
    pub run_after: Option<String>,
    /// 0 = 60 s default.
    pub timeout_secs: u64,
    /// `json` (default) or `text` for the result file given to post-scripts.
    pub result_format: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLine {
    pub level: LogLevel,
    pub message: String,
}

impl LogLine {
    pub fn info(m: impl Into<String>) -> Self {
        Self { level: LogLevel::Info, message: m.into() }
    }
    pub fn warn(m: impl Into<String>) -> Self {
        Self { level: LogLevel::Warning, message: m.into() }
    }
    pub fn error(m: impl Into<String>) -> Self {
        Self { level: LogLevel::Error, message: m.into() }
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
            Some(LogLine { level, message: format!("script: {}", msg.trim()) })
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
    /// POST, PUT or GET.
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
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .danger_accept_invalid_certs(cfg.accept_invalid_certs)
        .build()
    {
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
    for url in &cfg.http_urls {
        let mut last = String::new();
        let mut ok = false;
        for attempt in 0..=cfg.retries {
            let method = reqwest::Method::from_bytes(cfg.http_method.to_uppercase().as_bytes()).unwrap_or(reqwest::Method::POST);
            let mut req = client.request(method.clone(), url);
            req = match (cfg.format, method == reqwest::Method::GET) {
                (_, true) => req.query(&body.iter().filter(|(_, v)| v.is_string()).map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string())).collect::<Vec<_>>()),
                (ReportFormat::Json, false) => req.json(&body),
                (ReportFormat::Form, false) => req.form(&[("message", body["message"].as_str().unwrap_or_default().to_string()), ("data", serde_json::Value::Object(body.clone()).to_string())]),
            };
            match req.send().await {
                Ok(r) if r.status().is_success() => {
                    ok = true;
                    break;
                }
                Ok(r) => last = format!("HTTP {}", r.status()),
                Err(e) => last = e.to_string(),
            }
            if attempt < cfg.retries {
                tokio::time::sleep(Duration::from_millis(500 * (1 << attempt.min(6)))).await;
            }
        }
        if !ok {
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

    #[tokio::test]
    async fn report_filtering_skips_unselected_results() {
        let cfg = ReportConfig { http_urls: vec!["http://127.0.0.1:9/x".into()], send_on: vec!["error".into()], ..Default::default() };
        assert!(send_reports(&cfg, "success", &BTreeMap::new(), serde_json::json!({})).await.is_empty());
        let cfg = ReportConfig { retries: 0, ..cfg };
        assert_eq!(send_reports(&cfg, "error", &BTreeMap::new(), serde_json::json!({})).await.len(), 1);
    }
}
