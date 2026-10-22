//! Power awareness for backups: detect running on battery (to skip
//! scheduled runs) and keep the machine awake while an operation runs.
//!
//! - Linux: `/sys/class/power_supply` for AC state; `systemd-inhibit` holds
//!   a sleep inhibitor for the lifetime of the guard.
//! - macOS: `pmset -g batt`; `caffeinate -i -w <pid>`.
//! - Windows: `GetSystemPowerStatus`; `SetThreadExecutionState`.

use std::path::Path;

/// `Some(true)` on battery, `Some(false)` on AC, `None` when unknown
/// (desktops without batteries report `Some(false)` where detectable).
pub fn on_battery() -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        return linux_on_battery(Path::new("/sys/class/power_supply"));
    }
    #[cfg(target_os = "macos")]
    {
        let out = std::process::Command::new("pmset").args(["-g", "batt"]).output().ok()?;
        let s = String::from_utf8_lossy(&out.stdout);
        return parse_pmset(&s);
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
        let mut st: SYSTEM_POWER_STATUS = unsafe { std::mem::zeroed() };
        if unsafe { GetSystemPowerStatus(&mut st) } == 0 {
            return None;
        }
        return match st.ACLineStatus {
            0 => Some(true),
            1 => Some(false),
            _ => None,
        };
    }
    #[allow(unreachable_code)]
    None
}

/// Linux: on battery when a battery exists, is discharging, and no AC
/// adapter reports online.
pub fn linux_on_battery(root: &Path) -> Option<bool> {
    let rd = std::fs::read_dir(root).ok()?;
    let mut ac_online = false;
    let mut discharging = false;
    let mut any = false;
    for e in rd.flatten() {
        let p = e.path();
        let read = |f: &str| std::fs::read_to_string(p.join(f)).map(|s| s.trim().to_string()).unwrap_or_default();
        match read("type").as_str() {
            "Mains" | "USB" => {
                any = true;
                ac_online |= read("online") == "1";
            }
            "Battery" => {
                any = true;
                discharging |= read("status") == "Discharging";
            }
            _ => {}
        }
    }
    if !any {
        return Some(false);
    }
    Some(discharging && !ac_online)
}

/// Parse `pmset -g batt` output ("Now drawing from 'Battery Power'").
pub fn parse_pmset(s: &str) -> Option<bool> {
    if s.contains("'Battery Power'") {
        Some(true)
    } else if s.contains("'AC Power'") {
        Some(false)
    } else {
        None
    }
}

/// Keeps the system from sleeping until dropped.
pub struct SleepInhibitor {
    #[allow(dead_code)]
    child: Option<std::process::Child>,
}

impl SleepInhibitor {
    pub fn acquire(reason: &str) -> Self {
        #[cfg(target_os = "linux")]
        {
            let child = std::process::Command::new("systemd-inhibit")
                .args(["--what=sleep:idle", "--who=UFOP", &format!("--why={reason}"), "--mode=block", "sleep", "infinity"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .ok();
            if child.is_none() {
                tracing::debug!("systemd-inhibit unavailable; sleep prevention inactive");
            }
            return Self { child };
        }
        #[cfg(target_os = "macos")]
        {
            let _ = reason;
            let child = std::process::Command::new("caffeinate").args(["-i", "-w", &std::process::id().to_string()]).spawn().ok();
            return Self { child };
        }
        #[cfg(windows)]
        {
            let _ = reason;
            use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS, ES_SYSTEM_REQUIRED};
            unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
            return Self { child: None };
        }
        #[allow(unreachable_code)]
        {
            let _ = reason;
            Self { child: None }
        }
    }

    pub fn is_active(&self) -> bool {
        #[cfg(windows)]
        return true;
        #[allow(unreachable_code)]
        self.child.is_some()
    }
}

impl Drop for SleepInhibitor {
    fn drop(&mut self) {
        if let Some(c) = self.child.as_mut() {
            let _ = c.kill();
            let _ = c.wait();
        }
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Power::{SetThreadExecutionState, ES_CONTINUOUS};
            unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supply(root: &Path, name: &str, kind: &str, kv: &[(&str, &str)]) {
        let d = root.join(name);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("type"), kind).unwrap();
        for (k, v) in kv {
            std::fs::write(d.join(k), v).unwrap();
        }
    }

    #[test]
    fn linux_detection() {
        let t = tempfile::tempdir().unwrap();
        assert_eq!(linux_on_battery(t.path()), Some(false), "no supplies = desktop");
        supply(t.path(), "BAT0", "Battery", &[("status", "Discharging")]);
        supply(t.path(), "AC", "Mains", &[("online", "0")]);
        assert_eq!(linux_on_battery(t.path()), Some(true));
        std::fs::write(t.path().join("AC/online"), "1").unwrap();
        assert_eq!(linux_on_battery(t.path()), Some(false));
    }

    #[test]
    fn pmset_parsing() {
        assert_eq!(parse_pmset("Now drawing from 'Battery Power'\n -InternalBattery-0"), Some(true));
        assert_eq!(parse_pmset("Now drawing from 'AC Power'"), Some(false));
        assert_eq!(parse_pmset("garbage"), None);
    }

    #[test]
    fn inhibitor_lifecycle() {
        let g = SleepInhibitor::acquire("test");
        let _ = g.is_active();
        drop(g);
    }
}
