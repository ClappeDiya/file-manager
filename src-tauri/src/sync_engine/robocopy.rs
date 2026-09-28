//! Robocopy command-line compatibility.
//!
//! `parse_command` turns a pasted `robocopy <src> <dst> [files] [options]`
//! line into a sync pair configuration, so Windows users can bring existing
//! scripts and scheduled jobs across unchanged. `to_command` goes the other
//! way and shows the equivalent Robocopy command for any pair.

use crate::core::error::AppError;
use crate::core::types::*;
use serde::{Deserialize, Serialize};

/// A Robocopy command translated into sync-pair settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RobocopyJob {
    pub source_path: String,
    pub dest_path: String,
    pub mode: SyncMode,
    /// "manual", "watch" (/MON) or "scheduled" (/MOT)
    pub trigger: String,
    pub cron_expr: Option<String>,
    pub filter: SyncFilter,
    pub copy_options: CopyOptions,
    /// /L — the user asked for a list-only run; suggest a preview.
    pub list_only: bool,
    /// Switches that only affect Robocopy's console output (ignored).
    pub ignored: Vec<String>,
    /// Switches with no cross-platform equivalent, with an explanation each.
    pub unsupported: Vec<String>,
}

/// Split a command line into arguments, honouring double quotes.
fn tokenize(cmd: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_quotes = false;
    let mut has_token = false;
    for c in cmd.chars() {
        match c {
            '"' => {
                in_quotes = !in_quotes;
                has_token = true;
            }
            c if c.is_whitespace() && !in_quotes => {
                if has_token {
                    out.push(std::mem::take(&mut cur));
                    has_token = false;
                }
            }
            c => {
                cur.push(c);
                has_token = true;
            }
        }
    }
    if has_token {
        out.push(cur);
    }
    out
}

fn is_switch(arg: &str) -> bool {
    arg.starts_with('/') && arg.len() > 1 && !arg[1..].contains('/')
}

fn num(value: Option<&str>, flag: &str) -> Result<u64, AppError> {
    value
        .and_then(|v| v.trim().parse::<u64>().ok())
        .ok_or_else(|| AppError::Configuration {
            message: format!("{flag} needs a number, e.g. {flag}:5"),
            advice: "Check the Robocopy switch syntax.".to_string(),
        })
}

/// /MAXAGE and /MINAGE take either days (n < 1900) or a YYYYMMDD date.
fn age_days(value: Option<&str>, flag: &str) -> Result<u32, AppError> {
    let n = num(value, flag)?;
    if n < 1900 {
        return Ok(n as u32);
    }
    let s = n.to_string();
    let date = chrono::NaiveDate::parse_from_str(&s, "%Y%m%d").map_err(|_| AppError::Configuration {
        message: format!("{flag}:{s} is not a valid day count or YYYYMMDD date"),
        advice: "Use a number of days (e.g. 30) or a date like 20240131.".to_string(),
    })?;
    let days = (chrono::Local::now().date_naive() - date).num_days().max(0);
    Ok(days as u32)
}

/// Parse a Robocopy command line.
pub fn parse_command(cmd: &str) -> Result<RobocopyJob, AppError> {
    let mut args = tokenize(cmd.trim());
    if args
        .first()
        .map(|a| {
            let a = a.to_ascii_lowercase();
            a == "robocopy" || a.ends_with("robocopy.exe") || a.ends_with("\\robocopy")
        })
        .unwrap_or(false)
    {
        args.remove(0);
    }

    let mut positional = Vec::new();
    let mut job = RobocopyJob {
        source_path: String::new(),
        dest_path: String::new(),
        mode: SyncMode::OneWay,
        trigger: "manual".to_string(),
        cron_expr: None,
        filter: SyncFilter::default(),
        // Robocopy copies only the top-level folder unless /S, /E or /MIR.
        copy_options: CopyOptions {
            copy_subdirs: false,
            ..Default::default()
        },
        list_only: false,
        ignored: Vec::new(),
        unsupported: Vec::new(),
    };
    let o = &mut job.copy_options;

    // Which list the following bare arguments belong to (/XF, /XD, /IF).
    #[derive(PartialEq)]
    enum ListCtx {
        None,
        ExcludeFiles,
        ExcludeDirs,
        IncludeFiles,
    }
    let mut ctx = ListCtx::None;

    for arg in &args {
        if !is_switch(arg) {
            match ctx {
                ListCtx::ExcludeFiles => job.filter.exclude_patterns.push(arg.clone()),
                ListCtx::ExcludeDirs => o.exclude_dirs.push(arg.clone()),
                ListCtx::IncludeFiles => job.filter.include_patterns.push(arg.clone()),
                ListCtx::None => positional.push(arg.clone()),
            }
            continue;
        }
        ctx = ListCtx::None;

        let body = &arg[1..];
        let (name, value) = match body.split_once(':') {
            Some((n, v)) => (n.to_ascii_uppercase(), Some(v)),
            None => (body.to_ascii_uppercase(), None),
        };

        match name.as_str() {
            "S" => o.copy_subdirs = true,
            "E" => {
                o.copy_subdirs = true;
                o.include_empty_dirs = true;
            }
            "MIR" => {
                job.mode = SyncMode::Mirror;
                o.copy_subdirs = true;
                o.include_empty_dirs = true;
            }
            "PURGE" => o.purge = true,
            "LEV" => {
                o.copy_subdirs = true;
                o.max_depth = num(value, "/LEV")? as u32;
            }
            "MOV" => o.move_files = true,
            "MOVE" => {
                o.move_files = true;
                o.move_dirs = true;
            }
            "CREATE" => o.create_only = true,
            "Z" | "ZB" => o.restartable = true,
            "B" => {
                o.restartable = true;
                job.unsupported.push(
                    "/B: Windows backup-privilege mode isn't available; using restartable mode instead."
                        .to_string(),
                );
            }
            "XO" => o.exclude_older = true,
            "XN" => o.exclude_newer = true,
            "XC" => o.exclude_changed = true,
            "XX" => o.exclude_extra = true,
            "XL" => o.exclude_lonely = true,
            "IS" => o.include_same = true,
            "IT" | "IM" => job
                .ignored
                .push(format!("{arg} (tweaked/modified detection is covered by size+time checks)")),
            "FFT" => o.fat_time_tolerance = true,
            "DST" => o.dst_tolerance = true,
            "MAX" => job.filter.max_size = num(value, "/MAX")?,
            "MIN" => job.filter.min_size = num(value, "/MIN")?,
            "MAXAGE" => o.max_age_days = age_days(value, "/MAXAGE")?,
            "MINAGE" => o.min_age_days = age_days(value, "/MINAGE")?,
            "MAXLAD" | "MINLAD" => job.unsupported.push(format!(
                "{arg}: last-access-date filters aren't reliable across platforms (often disabled by the OS)."
            )),
            "XF" => ctx = ListCtx::ExcludeFiles,
            "XD" => ctx = ListCtx::ExcludeDirs,
            "IF" => ctx = ListCtx::IncludeFiles,
            "XA" => {
                for c in value.unwrap_or("").chars() {
                    match c.to_ascii_uppercase() {
                        'H' => o.exclude_hidden = true,
                        'R' => o.exclude_readonly = true,
                        other => job.unsupported.push(format!(
                            "/XA:{other}: only H (hidden) and R (read-only) attributes are portable."
                        )),
                    }
                }
            }
            "XJ" | "XJD" | "XJF" => o.symlinks = SymlinkMode::Skip,
            "SL" | "SJ" => o.symlinks = SymlinkMode::CopyLink,
            "R" => o.retries = num(value, "/R")? as u32,
            "W" => o.retry_wait_secs = num(value, "/W")? as u32,
            "MT" => {
                o.threads = match value {
                    Some(_) => (num(value, "/MT")? as u32).clamp(1, 128),
                    None => 8,
                }
            }
            "IPG" => o.inter_packet_gap_ms = num(value, "/IPG")? as u32,
            "RH" => {
                let v = value.unwrap_or("");
                if super::copier::parse_run_hours(v).is_none() {
                    return Err(AppError::Configuration {
                        message: format!("/RH:{v} is not a valid run-hours window"),
                        advice: "Use /RH:hhmm-hhmm, e.g. /RH:2200-0600.".to_string(),
                    });
                }
                o.run_hours = v.to_string();
            }
            "LOG" | "UNILOG" => {
                o.log_file = value.unwrap_or("").to_string();
                o.log_append = false;
            }
            "LOG+" | "UNILOG+" => {
                o.log_file = value.unwrap_or("").to_string();
                o.log_append = true;
            }
            "L" => job.list_only = true,
            "MON" => job.trigger = "watch".to_string(),
            "MOT" => {
                let m = num(value, "/MOT")?.clamp(1, 59);
                job.trigger = "scheduled".to_string();
                job.cron_expr = Some(format!("0 */{m} * * * * *"));
            }
            "COPY" => {
                let flags = value.unwrap_or("DAT").to_ascii_uppercase();
                o.copy_timestamps = flags.contains('T');
                for c in flags.chars().filter(|c| "SOUX".contains(*c)) {
                    job.unsupported.push(format!(
                        "/COPY:{c}: Windows security/owner/auditing info isn't copied; file permissions are."
                    ));
                }
            }
            "DCOPY" => o.copy_dir_timestamps = value.unwrap_or("").to_ascii_uppercase().contains('T'),
            "NODCOPY" => o.copy_dir_timestamps = false,
            "COPYALL" | "SEC" | "SECFIX" => {
                o.copy_timestamps = true;
                job.unsupported.push(format!(
                    "{arg}: Windows ACLs/owner/auditing aren't copied; data, timestamps and permissions are."
                ));
            }
            "TIMFIX" => o.copy_timestamps = true,
            "A" | "M" | "IA" | "A+" | "A-" => job.unsupported.push(format!(
                "{arg}: the Windows archive attribute isn't portable; use age or pattern filters instead."
            )),
            "EFSRAW" | "COMPRESS" | "J" | "NOOFFLOAD" | "256" | "SPARSE" | "NOCLONE" => job
                .ignored
                .push(format!("{arg} (Windows copy-engine tuning; not needed)")),
            "NP" | "NFL" | "NDL" | "NJH" | "NJS" | "NC" | "NS" | "TEE" | "V" | "ETA" | "BYTES"
            | "X" | "TS" | "FP" | "UNICODE" | "QUIT" | "PF" => job
                .ignored
                .push(format!("{arg} (console output only; the report covers this)")),
            "JOB" | "SAVE" | "NOSD" | "NODD" => job.ignored.push(format!(
                "{arg} (saved sync pairs replace Robocopy job files)"
            )),
            _ => job
                .unsupported
                .push(format!("{arg}: unrecognised switch, ignored.")),
        }
    }

    let mut positional = positional.into_iter();
    job.source_path = positional.next().unwrap_or_default();
    job.dest_path = positional.next().unwrap_or_default();
    for pattern in positional {
        if pattern != "*.*" && pattern != "*" {
            job.filter.include_patterns.push(pattern);
        }
    }

    if job.source_path.is_empty() || job.dest_path.is_empty() {
        return Err(AppError::Configuration {
            message: "A Robocopy command needs a source and a destination folder.".to_string(),
            advice: "Example: robocopy C:\\Data D:\\Backup /MIR /R:3 /W:5".to_string(),
        });
    }
    Ok(job)
}

fn quote(s: &str) -> String {
    if s.is_empty() || s.contains(char::is_whitespace) {
        format!("\"{s}\"")
    } else {
        s.to_string()
    }
}

/// Build the equivalent Robocopy command for a pair's settings.
pub fn to_command(
    source: &str,
    dest: &str,
    mode: SyncMode,
    filter: &SyncFilter,
    o: &CopyOptions,
) -> String {
    let mut parts = vec!["robocopy".to_string(), quote(source), quote(dest)];
    parts.extend(filter.include_patterns.iter().map(|p| quote(p)));

    let mirror = mode == SyncMode::Mirror;
    if mirror {
        parts.push("/MIR".into());
    } else if o.copy_subdirs && o.include_empty_dirs {
        parts.push("/E".into());
    } else if o.copy_subdirs {
        parts.push("/S".into());
    }
    if o.copy_subdirs && o.max_depth > 0 {
        parts.push(format!("/LEV:{}", o.max_depth));
    }
    if o.purge && !mirror {
        parts.push("/PURGE".into());
    }
    if o.move_dirs {
        parts.push("/MOVE".into());
    } else if o.move_files {
        parts.push("/MOV".into());
    }
    if o.create_only {
        parts.push("/CREATE".into());
    }
    if o.restartable {
        parts.push("/Z".into());
    }
    if !o.copy_timestamps {
        parts.push("/COPY:DA".into());
    }
    if o.copy_dir_timestamps {
        parts.push("/DCOPY:T".into());
    }
    for (on, flag) in [
        (o.exclude_older, "/XO"),
        (o.exclude_newer, "/XN"),
        (o.exclude_changed, "/XC"),
        (o.exclude_extra, "/XX"),
        (o.exclude_lonely, "/XL"),
        (o.include_same, "/IS"),
        (o.fat_time_tolerance, "/FFT"),
        (o.dst_tolerance, "/DST"),
    ] {
        if on {
            parts.push(flag.into());
        }
    }
    let mut xa = String::new();
    if o.exclude_readonly {
        xa.push('R');
    }
    if o.exclude_hidden {
        xa.push('H');
    }
    if !xa.is_empty() {
        parts.push(format!("/XA:{xa}"));
    }
    match o.symlinks {
        SymlinkMode::Skip => parts.push("/XJ".into()),
        SymlinkMode::CopyLink => parts.push("/SL".into()),
        SymlinkMode::Follow => {}
    }
    if filter.max_size > 0 {
        parts.push(format!("/MAX:{}", filter.max_size));
    }
    if filter.min_size > 0 {
        parts.push(format!("/MIN:{}", filter.min_size));
    }
    if o.max_age_days > 0 {
        parts.push(format!("/MAXAGE:{}", o.max_age_days));
    }
    if o.min_age_days > 0 {
        parts.push(format!("/MINAGE:{}", o.min_age_days));
    }
    if !filter.exclude_patterns.is_empty() {
        parts.push("/XF".into());
        parts.extend(filter.exclude_patterns.iter().map(|p| quote(p)));
    }
    if !o.exclude_dirs.is_empty() {
        parts.push("/XD".into());
        parts.extend(o.exclude_dirs.iter().map(|p| quote(p)));
    }
    parts.push(format!("/R:{}", o.retries));
    parts.push(format!("/W:{}", o.retry_wait_secs));
    if o.threads > 1 {
        parts.push(format!("/MT:{}", o.threads));
    }
    if o.inter_packet_gap_ms > 0 {
        parts.push(format!("/IPG:{}", o.inter_packet_gap_ms));
    }
    if !o.run_hours.trim().is_empty() {
        parts.push(format!("/RH:{}", o.run_hours.trim()));
    }
    if !o.log_file.trim().is_empty() {
        let flag = if o.log_append { "/LOG+:" } else { "/LOG:" };
        parts.push(format!("{flag}{}", quote(o.log_file.trim())));
    }
    parts.join(" ")
}

/// Short switch summary used in log headers.
pub fn options_summary(pair: &SyncPair) -> String {
    let cmd = to_command("", "", pair.mode, &pair.filter, &pair.copy_options);
    // Drop "robocopy" and the two quoted empty paths.
    cmd.splitn(4, ' ').nth(3).unwrap_or("").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_typical_mirror_command() {
        let job = parse_command(
            r#"robocopy "C:\My Data" D:\Backup *.docx /MIR /XD node_modules .git /XF *.tmp /R:3 /W:5 /MT:16 /Z /FFT /LOG+:C:\logs\backup.log"#,
        )
        .unwrap();
        assert_eq!(job.source_path, r"C:\My Data");
        assert_eq!(job.dest_path, r"D:\Backup");
        assert_eq!(job.mode, SyncMode::Mirror);
        assert_eq!(job.filter.include_patterns, vec!["*.docx"]);
        assert_eq!(job.filter.exclude_patterns, vec!["*.tmp"]);
        let o = &job.copy_options;
        assert!(o.copy_subdirs && o.include_empty_dirs);
        assert_eq!(o.exclude_dirs, vec!["node_modules", ".git"]);
        assert_eq!((o.retries, o.retry_wait_secs, o.threads), (3, 5, 16));
        assert!(o.restartable && o.fat_time_tolerance && o.log_append);
        assert_eq!(o.log_file, r"C:\logs\backup.log");
    }

    #[test]
    fn default_is_top_level_only() {
        let job = parse_command("robocopy a b").unwrap();
        assert!(!job.copy_options.copy_subdirs);
    }

    #[test]
    fn parses_move_age_and_attrs() {
        let job = parse_command("robocopy src dst /MOVE /MAXAGE:30 /MINAGE:2 /XA:HRS /XO /XL /MT").unwrap();
        let o = &job.copy_options;
        assert!(o.move_files && o.move_dirs);
        assert_eq!((o.max_age_days, o.min_age_days), (30, 2));
        assert!(o.exclude_hidden && o.exclude_readonly && o.exclude_older && o.exclude_lonely);
        assert_eq!(o.threads, 8);
        assert_eq!(job.unsupported.len(), 1); // /XA:S
    }

    #[test]
    fn monitor_switches_set_trigger() {
        assert_eq!(parse_command("robocopy a b /MON:1").unwrap().trigger, "watch");
        let j = parse_command("robocopy a b /MOT:15").unwrap();
        assert_eq!(j.trigger, "scheduled");
        assert_eq!(j.cron_expr.as_deref(), Some("0 */15 * * * * *"));
    }

    #[test]
    fn rejects_missing_paths_and_bad_values() {
        assert!(parse_command("robocopy /MIR").is_err());
        assert!(parse_command("robocopy a b /R:x").is_err());
        assert!(parse_command("robocopy a b /RH:9999-0000").is_err());
    }

    #[test]
    fn round_trips_through_to_command() {
        let original = "robocopy src dst /E /XO /XA:H /MAX:1000 /XF *.bak /XD tmp /R:2 /W:1 /MT:4 /RH:2200-0600";
        let job = parse_command(original).unwrap();
        let cmd = to_command(&job.source_path, &job.dest_path, job.mode, &job.filter, &job.copy_options);
        let again = parse_command(&cmd).unwrap();
        assert_eq!(again.copy_options, job.copy_options);
        assert_eq!(again.filter.exclude_patterns, job.filter.exclude_patterns);
        assert_eq!(again.filter.max_size, 1000);
    }
}
