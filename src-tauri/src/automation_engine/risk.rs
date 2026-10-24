//! How risky each automation step is, with the safety interlock's levels.
//!
//! The classification looks only at what the step is configured to do, so
//! the editor can show a badge before anything runs and ask for an explicit
//! confirmation when a rule contains a High step. (At run time the safety
//! interlock separately judges *how much* a destructive step is about to
//! touch against the user's history; see `Purge`.)

use super::workflow::{NodeKind, Workflow};
use super::{AutomationRule, RuleAction};
use crate::safety::RiskLevel;
use serde::Serialize;

/// A step's risk and the reason to show next to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct StepRisk {
    pub level: RiskLevel,
    pub reason: &'static str,
}

const fn risk(level: RiskLevel, reason: &'static str) -> StepRisk {
    StepRisk { level, reason }
}

/// The risk of one action. Exhaustive on purpose: a new action cannot be
/// added without deciding how risky it is.
pub fn action_risk(action: &RuleAction) -> StepRisk {
    use RiskLevel::{High, Low, Medium};
    match action {
        RuleAction::Notify { .. } => risk(Low, "Shows a notification"),
        RuleAction::SendMessage { .. } => risk(Low, "Sends a message"),
        RuleAction::CreateFolder { .. } => risk(Low, "Creates a folder"),
        RuleAction::CopyFile { .. } => risk(Low, "Copies files; nothing is removed"),
        RuleAction::RenameFile { .. } => risk(Low, "Renames files"),
        RuleAction::CompressFolder { .. } => risk(Low, "Creates an archive"),
        RuleAction::ConvertImage { .. } | RuleAction::ConvertDocument { .. } => risk(Low, "Writes a converted copy"),
        RuleAction::ValidateBatch { .. } => risk(Low, "Reads files and writes a report"),
        RuleAction::AiTask { .. } => risk(Low, "Reads a file and writes the AI's answer beside it"),
        RuleAction::ScanForMalware { .. } => risk(Low, "Scans files; infected ones are moved to quarantine"),
        RuleAction::ListFiles { .. } => risk(Low, "Writes a file listing"),
        RuleAction::VerifyPair { .. } => risk(Low, "Reads both sides of a sync pair; changes nothing"),
        RuleAction::MoveFile { .. } => risk(Medium, "Moves files out of their folder"),
        RuleAction::ReplayOp { kind, .. } if kind == "move" => risk(Medium, "Moves files out of their folder"),
        RuleAction::ReplayOp { .. } => risk(Low, "Copies files; nothing is removed"),
        RuleAction::ExtractArchive { delete_archive: true, .. } => risk(Medium, "Extracts an archive and deletes it"),
        RuleAction::ExtractArchive { .. } => risk(Low, "Extracts an archive"),
        RuleAction::DownloadUrl { destination_connection_id: Some(c), .. } if !c.is_empty() => risk(Medium, "Downloads from the internet and uploads to a remote"),
        RuleAction::DownloadUrl { overwrite: true, .. } => risk(Medium, "Downloads from the internet and replaces an existing file"),
        RuleAction::DownloadUrl { .. } => risk(Low, "Downloads a file from the internet"),
        RuleAction::TransferToConnector { .. } => risk(Medium, "Sends files to a remote connection"),
        RuleAction::RunSync { .. } => risk(Medium, "Runs a sync pair, which can change or delete files"),
        RuleAction::RemoveEmptyDirs { .. } => risk(Medium, "Removes empty folders"),
        RuleAction::DeleteFile { to_trash: true } => risk(Medium, "Moves files to a trash folder"),
        RuleAction::DeleteFile { to_trash: false } => risk(High, "Deletes files permanently"),
        RuleAction::ExecuteCustomCommand { .. } => risk(Medium, "Runs a saved custom command"),
        RuleAction::RunCommand { .. } => risk(High, "Runs a program on this computer"),
        RuleAction::Purge { .. } => risk(High, "Removes a folder and everything in it"),
    }
}

/// The highest of two levels.
pub fn worse(a: RiskLevel, b: RiskLevel) -> RiskLevel {
    let rank = |l: RiskLevel| match l {
        RiskLevel::Low => 0,
        RiskLevel::Medium => 1,
        RiskLevel::High => 2,
    };
    if rank(b) > rank(a) {
        b
    } else {
        a
    }
}

/// The step ids (or `Action n`) of a rule that need confirming, with why.
pub fn needs_confirmation(rule: &AutomationRule) -> Vec<(String, StepRisk)> {
    let mut out = Vec::new();
    match &rule.workflow {
        Some(wf) => workflow_risks(wf, &mut out),
        None => {
            for (i, (action, _)) in rule.all_steps().into_iter().enumerate() {
                let r = action_risk(action);
                if r.level == RiskLevel::High {
                    out.push((format!("Action {}", i + 1), r));
                }
            }
        }
    }
    out
}

fn workflow_risks(wf: &Workflow, out: &mut Vec<(String, StepRisk)>) {
    for n in &wf.nodes {
        if let NodeKind::Action { action } = &n.kind {
            let r = action_risk(action);
            if r.level == RiskLevel::High {
                out.push((format!("Step {}", if n.label.is_empty() { &n.id } else { &n.label }), r));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::automation_engine::{ListFormat, RuleTrigger};
    use RiskLevel::{High, Low, Medium};

    fn notification() -> crate::notify_hub::Notification {
        crate::notify_hub::Notification { channel: crate::notify_hub::Channel::Slack {}, send_on: vec![], credential_id: String::new(), template: String::new(), cert: Default::default() }
    }

    /// One sample of every action (and every risk-relevant setting), with
    /// its expected level. A new `RuleAction` variant fails to compile in
    /// `action_risk`, and must be added here too.
    fn samples() -> Vec<(RuleAction, RiskLevel)> {
        let s = |x: &str| x.to_string();
        vec![
            (RuleAction::MoveFile { dest_path: s("/d") }, Medium),
            (RuleAction::CopyFile { dest_path: s("/d") }, Low),
            (RuleAction::RenameFile { pattern: s("x") }, Low),
            (RuleAction::DeleteFile { to_trash: true }, Medium),
            (RuleAction::DeleteFile { to_trash: false }, High),
            (RuleAction::CompressFolder { dest_path: s("/d"), format: s("zip") }, Low),
            (RuleAction::RunSync { sync_pair_id: s("p") }, Medium),
            (RuleAction::TransferToConnector { connector_protocol: s("sftp"), connection_id: s("c"), remote_path: s("/") }, Medium),
            (RuleAction::ExecuteCustomCommand { command_id: s("c") }, Medium),
            (RuleAction::Notify { title: s("t"), body: s("b") }, Low),
            (RuleAction::CreateFolder { path: s("/x") }, Low),
            (RuleAction::ExtractArchive { dest_path: s("/d"), max_entries: 10, delete_archive: false }, Low),
            (RuleAction::ExtractArchive { dest_path: s("/d"), max_entries: 10, delete_archive: true }, Medium),
            (RuleAction::RunCommand { program: s("p"), args: vec![], working_dir: None, timeout_secs: 5 }, High),
            (RuleAction::SendMessage { notification: notification(), title: s("t"), body: s("b") }, Low),
            (RuleAction::ReplayOp { kind: s("copy"), source_paths: vec![], dest_path: s("/d") }, Low),
            (RuleAction::ReplayOp { kind: s("move"), source_paths: vec![], dest_path: s("/d") }, Medium),
            (RuleAction::ConvertImage { format: None, max_width: None, max_height: None, quality: None, dest_path: s("") }, Low),
            (RuleAction::ConvertDocument { to: s("pdf"), dest_path: s("") }, Low),
            (RuleAction::ValidateBatch { folder: s(""), rules: Default::default(), report_folder: None, fail_on_invalid: true }, Low),
            (RuleAction::AiTask { instruction: s("i"), dest_path: s(""), suffix: s("ai.md"), max_chars: 100 }, Low),
            (RuleAction::ScanForMalware { quarantine_path: s("/q"), notify: true }, Low),
            (download(false, None), Low),
            (download(true, None), Medium),
            (download(false, Some("conn")), Medium),
            (RuleAction::VerifyPair { sync_pair_id: s("p"), full: true }, Low),
            (RuleAction::RemoveEmptyDirs { path: s("/x"), remove_root: false }, Medium),
            (RuleAction::ListFiles { path: s("/x"), output: s("/l.csv"), format: ListFormat::Csv, recursive: true, include_dirs: false, overwrite: true }, Low),
            (RuleAction::Purge { path: s("/x/y"), max_files: None }, High),
        ]
    }

    fn download(overwrite: bool, conn: Option<&str>) -> RuleAction {
        RuleAction::DownloadUrl {
            url: "https://h/f".into(),
            dest_path: "/d".into(),
            file_name: None,
            overwrite,
            max_mb: 1,
            sha256: None,
            allow_private: false,
            destination_connection_id: conn.map(Into::into),
            destination_protocol: None,
            remote_path: None,
        }
    }

    #[test]
    fn every_action_has_the_expected_level_and_a_reason() {
        for (a, expected) in samples() {
            let r = action_risk(&a);
            assert_eq!(r.level, expected, "{a:?}");
            assert!(!r.reason.is_empty());
        }
    }

    #[test]
    fn the_samples_cover_every_action_kind() {
        // Every `type` the engine can run appears (so a new variant is not forgotten here).
        let kinds: std::collections::BTreeSet<String> = samples().iter().map(|(a, _)| super::super::executor::action_kind(a).to_string()).collect();
        for k in [
            "move_file", "copy_file", "rename_file", "delete_file", "compress_folder", "run_sync", "transfer_to_connector", "execute_custom_command", "notify", "create_folder", "extract_archive", "run_command",
            "send_message", "replay_op", "download_url", "convert_image", "convert_document", "validate_batch", "ai_task", "scan_for_malware", "verify_pair", "remove_empty_dirs", "list_files", "purge",
        ] {
            assert!(kinds.contains(k), "no sample for {k}");
        }
        assert_eq!(kinds.len(), 24);
    }

    #[test]
    fn worse_picks_the_higher_level() {
        assert_eq!(worse(Low, Medium), Medium);
        assert_eq!(worse(High, Medium), High);
        assert_eq!(worse(Low, Low), Low);
    }

    #[test]
    fn rules_list_their_high_risk_steps() {
        let mut r = AutomationRule::new("r".into(), RuleTrigger::Manual, RuleAction::DeleteFile { to_trash: true });
        assert!(needs_confirmation(&r).is_empty());
        r.steps.push(crate::automation_engine::ActionStep { action: RuleAction::Purge { path: "/a/b".into(), max_files: None }, continue_on_error: false, output_as: None });
        let n = needs_confirmation(&r);
        assert_eq!(n.len(), 1);
        assert_eq!(n[0].0, "Action 2");
        assert_eq!(n[0].1.level, High);
        // The same in a graph.
        let mut wf = Workflow::from_rule(&r);
        wf.nodes[0].label = "Trash".into();
        wf.nodes[1].label = "Wipe".into();
        r.workflow = Some(wf);
        assert_eq!(needs_confirmation(&r).iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>(), vec!["Step Wipe"]);
    }
}
