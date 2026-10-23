//! `ufop completions <shell>` and `ufop man`, through the real binary; the
//! bash script is also loaded into bash and asked for completions.

use std::process::{Command, Stdio};

fn ufop(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ufop")).args(args).stdin(Stdio::null()).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn every_shell_script_contains_the_subcommands() {
    for sh in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let (code, out, err) = ufop(&["completions", sh]);
        assert_eq!(code, 0, "{sh}: {err}");
        for w in ["backup", "bench", "debug", "restore", "completions", "--dest", "--passphrase-env"] {
            assert!(out.contains(w), "{sh} script lacks {w}");
        }
    }
    let (code, _, err) = ufop(&["completions", "tcsh"]);
    assert_eq!(code, 2, "{err}");
}

/// Complete `words` (the last one is being typed) with the bash script.
fn bash_complete(script: &std::path::Path, words: &[&str]) -> Vec<String> {
    let quoted: Vec<String> = words.iter().map(|w| format!("'{w}'")).collect();
    let prog = format!(
        "source '{}'; COMP_WORDS=({}); COMP_CWORD={}; _ufop; printf '%s\\n' \"${{COMPREPLY[@]}}\"",
        script.display(),
        quoted.join(" "),
        words.len() - 1
    );
    let out = Command::new("bash").arg("--norc").arg("-c").arg(prog).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).lines().filter(|l| !l.is_empty()).map(String::from).collect()
}

#[test]
fn bash_completes_nested_subcommands_options_and_values() {
    if Command::new("bash").arg("--version").output().is_err() {
        return;
    }
    let t = tempfile::tempdir().unwrap();
    let script = t.path().join("ufop.bash");
    let (code, out, _) = ufop(&["completions", "bash"]);
    assert_eq!(code, 0);
    std::fs::write(&script, out).unwrap();
    let c = bash_complete(&script, &["ufop", "backup", "de"]);
    assert!(c.contains(&"debug".to_string()) && c.contains(&"delete".to_string()), "{c:?}");
    let c = bash_complete(&script, &["ufop", "--json", "backup", "debug", "s"]);
    assert_eq!(c, vec!["stats", "sessions"], "global options are skipped while walking");
    let c = bash_complete(&script, &["ufop", "backup", "bench", "--pri"]);
    assert_eq!(c, vec!["--print-options"]);
    // Global options are offered on subcommands too.
    assert!(bash_complete(&script, &["ufop", "backup", "bench", "--pro"]).contains(&"--profile".to_string()));
    let c = bash_complete(&script, &["ufop", "--format", ""]);
    assert_eq!(c, vec!["human", "json", "yaml", "csv"]);
    let c = bash_complete(&script, &["ufop", "completions", "p"]);
    assert_eq!(c, vec!["powershell"]);
    // An option value is not taken for a subcommand.
    let c = bash_complete(&script, &["ufop", "--profile", "backup", "r"]);
    assert_eq!(c, vec!["rename"]);
}

#[test]
fn man_pages_print_and_write() {
    let (code, out, err) = ufop(&["man", "backup", "bench"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.starts_with(".TH \"UFOP-BACKUP-BENCH\" \"1\""), "{out}");
    assert!(out.contains("\\-\\-print\\-options"));
    let t = tempfile::tempdir().unwrap();
    let (code, _, err) = ufop(&["man", "--out", t.path().to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    for p in ["ufop.1", "ufop-backup.1", "ufop-backup-debug-stats.1", "ufop-completions.1"] {
        assert!(t.path().join(p).exists(), "{p} missing");
    }
    let (code, _, _) = ufop(&["man", "nonsense"]);
    assert_eq!(code, 2);
}
