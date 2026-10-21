//! Per-job log outputs: a rotating log file, the Windows Event Log (or the
//! system log on Linux/macOS), each with a minimum severity and tag filters.

use super::hooks::{LogLevel, LogLine};
use serde::{Deserialize, Serialize};
use std::io::Write;

/// Which lines an output receives.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogFilter {
    pub min_level: LogLevel,
    /// Only lines with one of these tags (empty = all).
    pub include_tags: Vec<String>,
    /// Never lines with these tags.
    pub exclude_tags: Vec<String>,
    /// Only lines matching this regular expression (empty = all).
    pub pattern: String,
}

impl LogFilter {
    pub fn accepts(&self, l: &LogLine) -> bool {
        let tag = l.tag.as_deref().unwrap_or("engine");
        l.level >= self.min_level
            && (self.include_tags.is_empty() || self.include_tags.iter().any(|t| t == tag))
            && !self.exclude_tags.iter().any(|t| t == tag)
            && (self.pattern.is_empty() || regex::Regex::new(&self.pattern).map(|r| r.is_match(&l.message)).unwrap_or(true))
    }

    pub fn validate(&self) -> Result<(), crate::core::error::AppError> {
        if !self.pattern.is_empty() {
            regex::Regex::new(&self.pattern).map_err(|e| crate::core::error::AppError::validation(format!("Invalid log filter pattern: {e}")))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct LogOutputs {
    /// Append run logs to this file.
    pub file: Option<String>,
    pub file_filter: LogFilter,
    /// Rotate the file (keeping one `.1` copy) above this size (MiB, 0 = never).
    pub file_max_mb: u64,
    /// Windows Event Log (source "UFOP"); syslog on Linux/macOS.
    pub system_log: bool,
    pub system_filter: LogFilter,
}

fn level_name(l: LogLevel) -> &'static str {
    match l {
        LogLevel::Info => "INFO",
        LogLevel::Warning => "WARN",
        LogLevel::Error => "ERROR",
    }
}

/// Write a finished run to the configured outputs (fail-open; returns problems).
pub fn write_run(out: &LogOutputs, job: &str, op: &str, status: &str, summary: &str, lines: &[LogLine]) -> Vec<String> {
    let mut problems = Vec::new();
    let head = LogLine { level: if status == "error" { LogLevel::Error } else if status == "warning" { LogLevel::Warning } else { LogLevel::Info }, message: format!("{op} {status}: {summary}"), tag: Some("result".into()) };
    if let Some(path) = out.file.as_deref().filter(|p| !p.trim().is_empty()) {
        if let Err(e) = write_file(path, out.file_max_mb, job, std::iter::once(&head).chain(lines.iter()).filter(|l| l.tag.as_deref() == Some("result") || out.file_filter.accepts(l))) {
            problems.push(format!("log file {path}: {e}"));
        }
    }
    if out.system_log {
        for l in std::iter::once(&head).chain(lines.iter()).filter(|l| l.tag.as_deref() == Some("result") || out.system_filter.accepts(l)) {
            if let Err(e) = system::write(l.level, &format!("[{job}] {}", l.message)) {
                problems.push(format!("system log: {e}"));
                break;
            }
        }
    }
    problems
}

fn write_file<'a>(path: &str, max_mb: u64, job: &str, lines: impl Iterator<Item = &'a LogLine>) -> std::io::Result<()> {
    let p = std::path::Path::new(path);
    if let Some(d) = p.parent() {
        std::fs::create_dir_all(d)?;
    }
    if max_mb > 0 && std::fs::metadata(p).map(|m| m.len() > max_mb << 20).unwrap_or(false) {
        std::fs::rename(p, format!("{path}.1"))?;
    }
    let mut f = std::fs::OpenOptions::new().create(true).append(true).open(p)?;
    let ts = chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f");
    for l in lines {
        writeln!(f, "{ts} {:5} [{job}] {}: {}", level_name(l.level), l.tag.as_deref().unwrap_or("engine"), l.message.replace('\n', " | "))?;
    }
    f.flush()
}

#[cfg(windows)]
mod system {
    use super::LogLevel;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::EventLog::{DeregisterEventSource, RegisterEventSourceW, ReportEventW, EVENTLOG_ERROR_TYPE, EVENTLOG_INFORMATION_TYPE, EVENTLOG_WARNING_TYPE};

    fn wide(s: &str) -> Vec<u16> {
        std::ffi::OsStr::new(s).encode_wide().chain(Some(0)).collect()
    }

    pub fn write(level: LogLevel, msg: &str) -> std::io::Result<()> {
        let source = wide("UFOP");
        let text = wide(msg);
        unsafe {
            let h = RegisterEventSourceW(std::ptr::null(), source.as_ptr());
            if h.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let kind = match level {
                LogLevel::Info => EVENTLOG_INFORMATION_TYPE,
                LogLevel::Warning => EVENTLOG_WARNING_TYPE,
                LogLevel::Error => EVENTLOG_ERROR_TYPE,
            };
            let strings = [text.as_ptr()];
            let ok = ReportEventW(h, kind, 0, 1000, std::ptr::null_mut(), 1, 0, strings.as_ptr(), std::ptr::null());
            DeregisterEventSource(h);
            if ok == 0 {
                return Err(std::io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

#[cfg(unix)]
mod system {
    use super::LogLevel;

    pub fn write(level: LogLevel, msg: &str) -> std::io::Result<()> {
        let prio = match level {
            LogLevel::Info => libc::LOG_INFO,
            LogLevel::Warning => libc::LOG_WARNING,
            LogLevel::Error => libc::LOG_ERR,
        };
        let ident = c"ufop";
        let fmt = c"%s";
        let m = std::ffi::CString::new(msg.replace('\0', " "))?;
        unsafe {
            libc::openlog(ident.as_ptr(), libc::LOG_PID, libc::LOG_USER);
            libc::syslog(prio, fmt.as_ptr(), m.as_ptr());
            libc::closelog();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_and_file_output_with_rotation() {
        let d = tempfile::tempdir().unwrap();
        let path = d.path().join("logs/job.log").to_string_lossy().into_owned();
        let out = LogOutputs {
            file: Some(path.clone()),
            file_filter: LogFilter { min_level: LogLevel::Warning, exclude_tags: vec!["script".into()], ..Default::default() },
            file_max_mb: 1,
            ..Default::default()
        };
        let lines = vec![LogLine::info("quiet"), LogLine::warn("disk nearly full"), LogLine::error("script boom").tagged("script"), LogLine::error("upload failed").tagged("network")];
        assert!(write_run(&out, "Docs", "backup", "warning", "3 files", &lines).is_empty());
        let txt = std::fs::read_to_string(&path).unwrap();
        assert!(txt.contains("WARN  [Docs] result: backup warning: 3 files"));
        assert!(txt.contains("disk nearly full") && txt.contains("network: upload failed"));
        assert!(!txt.contains("quiet") && !txt.contains("script boom"));
        // Rotation above the size limit.
        std::fs::write(&path, vec![b'x'; 2 << 20]).unwrap();
        write_run(&out, "Docs", "backup", "success", "ok", &[]);
        assert!(std::path::Path::new(&format!("{path}.1")).exists());
        assert!(std::fs::metadata(&path).unwrap().len() < 1000);
        let f = LogFilter { include_tags: vec!["network".into()], pattern: "fail".into(), ..Default::default() };
        assert!(f.accepts(&LogLine::error("upload failed").tagged("network")));
        assert!(!f.accepts(&LogLine::error("ok").tagged("network")));
        assert!(LogFilter { pattern: "(".into(), ..Default::default() }.validate().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn system_log_write_succeeds() {
        assert!(system::write(LogLevel::Info, "ufop test message").is_ok());
    }
}
