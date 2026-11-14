//! `ufop help-json` and the snapshot of it the desktop terminal's help
//! drawer ships with (`crates/ufop-core/data/ufop-cli-help.json`).
//!
//! The snapshot must match the real command tree. When it does not, this test
//! fails: run `scripts/gen-cli-help.sh` and commit the result.

use std::process::{Command, Stdio};

fn ufop(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ufop")).args(args).stdin(Stdio::null()).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn help_json_lists_commands_with_descriptions_and_usage() {
    let (code, out, err) = ufop(&["help-json"]);
    assert_eq!(code, 0, "{err}");
    let cmds: Vec<ufop_core::cli_gen::CommandHelp> = serde_json::from_str(&out).expect("valid JSON");
    let find = |p: &[&str]| cmds.iter().find(|c| c.path == p).unwrap_or_else(|| panic!("no {p:?} in {:?}", cmds.iter().map(|c| c.path.join(" ")).collect::<Vec<_>>()));
    let backup = find(&["backup"]);
    assert!(backup.has_subcommands && !backup.about.is_empty());
    let run = find(&["backup", "run"]);
    assert!(run.usage.starts_with("backup run"), "{}", run.usage);
    assert!(cmds.iter().all(|c| !c.about.is_empty()), "every command has a description: {:?}", cmds.iter().filter(|c| c.about.is_empty()).map(|c| c.path.join(" ")).collect::<Vec<_>>());
    // The hidden command lists itself nowhere.
    assert!(!cmds.iter().any(|c| c.path == ["help-json"]));
    let transfer = find(&["transfer"]);
    assert!(transfer.usage.contains("<SRC>") || transfer.usage.contains("<src>"), "{}", transfer.usage);
    assert!(transfer.options.iter().any(|o| o.flags.contains(&"--overwrite".to_string()) && !o.takes_value));
    assert!(transfer.options.iter().any(|o| o.flags.contains(&"--parallel".to_string()) && o.takes_value));
}

#[test]
fn the_bundled_snapshot_matches_the_command_tree() {
    let (code, live, err) = ufop(&["help-json"]);
    assert_eq!(code, 0, "{err}");
    let live: Vec<ufop_core::cli_gen::CommandHelp> = serde_json::from_str(&live).unwrap();
    let bundled = ufop_core::cli_gen::bundled_ufop_help();
    assert!(!bundled.is_empty(), "the snapshot is empty: run scripts/gen-cli-help.sh");
    assert_eq!(bundled, live, "the CLI's commands changed: run scripts/gen-cli-help.sh and commit crates/ufop-core/data/ufop-cli-help.json");
}
