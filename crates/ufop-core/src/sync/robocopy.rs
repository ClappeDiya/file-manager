//! Robocopy command-line compatibility.
//!
//! `parse_command` turns a pasted `robocopy <src> <dst> [files] [options]`
//! line into a sync pair configuration, so Windows users can bring existing
//! scripts and scheduled jobs across unchanged. `to_command` goes the other
//! way and shows the equivalent Robocopy command for any pair.

use crate::error::AppError;
use crate::sync_types::*;
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
    /// /MON:n — re-run when at least this many changes are seen.
    #[serde(default)]
    pub monitor_changes: Option<u32>,
    /// /MOT:m — re-run every m minutes if anything changed.
    #[serde(default)]
    pub monitor_minutes: Option<u32>,
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
    let date =
        chrono::NaiveDate::parse_from_str(&s, "%Y%m%d").map_err(|_| AppError::Configuration {
            message: format!("{flag}:{s} is not a valid day count or YYYYMMDD date"),
            advice: "Use a number of days (e.g. 30) or a date like 20240131.".to_string(),
        })?;
    let days = (chrono::Local::now().date_naive() - date).num_days().max(0);
    Ok(days as u32)
}

/// Note a switch that is honoured on Windows and does nothing elsewhere.
fn windows_only(notes: &mut Vec<String>, arg: &str) {
    if !super::winattr::SUPPORTED {
        notes.push(format!(
            "{arg}: only takes effect on Windows (NTFS attributes and permissions); ignored on this computer."
        ));
    }
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
    parse_args(&args)
}

/// Parse already-split Robocopy arguments: `<src> <dst> [files…] [/switches…]`.
pub fn parse_args(args: &[String]) -> Result<RobocopyJob, AppError> {
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
        monitor_changes: None,
        monitor_minutes: None,
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

    for arg in args {
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
                let mut rest = String::new();
                for c in value.unwrap_or("").chars() {
                    match c.to_ascii_uppercase() {
                        'H' => o.exclude_hidden = true,
                        'R' => o.exclude_readonly = true,
                        other => rest.push(other),
                    }
                }
                if !rest.is_empty() {
                    super::winattr::parse_letters(&rest)?;
                    o.exclude_attributes = rest;
                    windows_only(&mut job.unsupported, arg);
                }
            }
            "IA" => {
                let v = value.unwrap_or("").to_ascii_uppercase();
                super::winattr::parse_letters(&v)?;
                o.include_attributes = v;
                windows_only(&mut job.unsupported, arg);
            }
            "A+" | "A-" => {
                let v = value.unwrap_or("").to_ascii_uppercase();
                super::winattr::parse_letters(&v)?;
                if name == "A+" {
                    o.add_attributes = v;
                } else {
                    o.remove_attributes = v;
                }
                windows_only(&mut job.unsupported, arg);
            }
            "A" => {
                o.archive_only = true;
                windows_only(&mut job.unsupported, arg);
            }
            "M" => {
                o.archive_reset = true;
                windows_only(&mut job.unsupported, arg);
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
            "MON" => {
                job.trigger = "watch".to_string();
                job.monitor_changes = Some(num(value, "/MON")?.max(1) as u32);
            }
            "MOT" => {
                let m = num(value, "/MOT")?.max(1);
                job.monitor_minutes = Some(m as u32);
                job.trigger = "scheduled".to_string();
                // Cron can only express "every m minutes" up to 59.
                job.cron_expr = Some(format!("0 */{} * * * * *", m.clamp(1, 59)));
            }
            "COPY" => {
                let flags = value.unwrap_or("DAT").to_ascii_uppercase();
                o.copy_timestamps = flags.contains('T');
                o.copy_security = flags.contains('S');
                o.copy_owner = flags.contains('O');
                o.copy_auditing = flags.contains('U');
                if flags.chars().any(|c| "SOU".contains(c)) {
                    windows_only(&mut job.unsupported, arg);
                }
                if flags.contains('X') {
                    job.ignored.push("/COPY:X (alternate data streams are skipped anyway)".to_string());
                }
            }
            "DCOPY" => o.copy_dir_timestamps = value.unwrap_or("").to_ascii_uppercase().contains('T'),
            "NODCOPY" => o.copy_dir_timestamps = false,
            "SEC" => {
                o.copy_timestamps = true;
                o.copy_security = true;
                windows_only(&mut job.unsupported, arg);
            }
            "COPYALL" => {
                o.copy_timestamps = true;
                o.copy_security = true;
                o.copy_owner = true;
                o.copy_auditing = true;
                windows_only(&mut job.unsupported, arg);
            }
            "SECFIX" => {
                o.fix_security = true;
                if !(o.copy_security || o.copy_owner || o.copy_auditing) {
                    o.copy_security = true;
                }
                windows_only(&mut job.unsupported, arg);
            }
            "TIMFIX" => {
                o.copy_timestamps = true;
                o.fix_timestamps = true;
            }
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
    if !(o.copy_timestamps || o.copy_security || o.copy_owner || o.copy_auditing) {
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
    xa.push_str(o.exclude_attributes.trim());
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
    let mut copy_flags = String::from("DA");
    if o.copy_timestamps {
        copy_flags.push('T');
    }
    for (on, c) in [
        (o.copy_security, 'S'),
        (o.copy_owner, 'O'),
        (o.copy_auditing, 'U'),
    ] {
        if on {
            copy_flags.push(c);
        }
    }
    if copy_flags.len() > 3 {
        parts.push(format!("/COPY:{copy_flags}"));
    }
    if o.fix_timestamps {
        parts.push("/TIMFIX".into());
    }
    if o.fix_security {
        parts.push("/SECFIX".into());
    }
    if o.archive_reset {
        parts.push("/M".into());
    } else if o.archive_only {
        parts.push("/A".into());
    }
    for (letters, flag) in [
        (&o.include_attributes, "/IA:"),
        (&o.add_attributes, "/A+:"),
        (&o.remove_attributes, "/A-:"),
    ] {
        if !letters.trim().is_empty() {
            parts.push(format!("{flag}{}", letters.trim()));
        }
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
        let job = parse_command("robocopy src dst /MOVE /MAXAGE:30 /MINAGE:2 /XA:HRS /XO /XL /MT")
            .unwrap();
        let o = &job.copy_options;
        assert!(o.move_files && o.move_dirs);
        assert_eq!((o.max_age_days, o.min_age_days), (30, 2));
        assert!(o.exclude_hidden && o.exclude_readonly && o.exclude_older && o.exclude_lonely);
        assert_eq!(o.threads, 8);
        // /XA:S is honoured on Windows and noted as Windows-only elsewhere.
        assert_eq!(o.exclude_attributes, "S");
        assert_eq!(
            job.unsupported.len(),
            usize::from(!super::super::winattr::SUPPORTED)
        );
    }

    #[test]
    fn monitor_switches_set_trigger() {
        assert_eq!(
            parse_command("robocopy a b /MON:1").unwrap().trigger,
            "watch"
        );
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
        let cmd = to_command(
            &job.source_path,
            &job.dest_path,
            job.mode,
            &job.filter,
            &job.copy_options,
        );
        let again = parse_command(&cmd).unwrap();
        assert_eq!(again.copy_options, job.copy_options);
        assert_eq!(again.filter.exclude_patterns, job.filter.exclude_patterns);
        assert_eq!(again.filter.max_size, 1000);
    }

    #[test]
    fn parses_windows_switches_and_round_trips() {
        let job = parse_command(
            "robocopy a b /E /COPY:DATSOU /M /IA:RS /A+:R /A-:H /TIMFIX /SECFIX /XA:T",
        )
        .unwrap();
        let o = &job.copy_options;
        assert!(o.copy_security && o.copy_owner && o.copy_auditing && o.copy_timestamps);
        assert!(o.archive_reset && o.fix_timestamps && o.fix_security);
        assert_eq!(o.include_attributes, "RS");
        assert_eq!(
            (o.add_attributes.as_str(), o.remove_attributes.as_str()),
            ("R", "H")
        );
        assert_eq!(o.exclude_attributes, "T");
        if !super::super::winattr::SUPPORTED {
            assert!(job
                .unsupported
                .iter()
                .all(|n| n.contains("only takes effect on Windows")));
        }
        let cmd = to_command("a", "b", job.mode, &job.filter, o);
        assert!(cmd.contains("/COPY:DATSOU"), "{cmd}");
        let again = parse_command(&cmd).unwrap();
        assert_eq!(&again.copy_options, o);
    }

    #[test]
    fn copyall_and_bad_attribute_letters() {
        let o = parse_command("robocopy a b /COPYALL").unwrap().copy_options;
        assert!(o.copy_security && o.copy_owner && o.copy_auditing);
        assert!(parse_command("robocopy a b /IA:Q").is_err());
    }

    #[test]
    fn parse_args_keeps_spaces_and_monitor_values() {
        let args: Vec<String> = ["C:\\My Data", "D:\\Back up", "/MIR", "/MON:3", "/MOT:90"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let job = parse_args(&args).unwrap();
        assert_eq!(job.source_path, "C:\\My Data");
        assert_eq!(job.dest_path, "D:\\Back up");
        assert_eq!(job.monitor_changes, Some(3));
        assert_eq!(job.monitor_minutes, Some(90));
        assert_eq!(job.cron_expr.as_deref(), Some("0 */59 * * * * *"));
    }
}
