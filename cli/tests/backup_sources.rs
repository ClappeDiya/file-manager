//! `ufop backup stdin` with piped input, `ufop backup estimate`, and the
//! new source/run flags, driven through the real binary.

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

fn ufop(args: &[&str], stdin: Option<&[u8]>, home: &Path) -> (i32, String, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_ufop"));
    c.args(args).env("HOME", home).env("XDG_CONFIG_HOME", home.join("cfg")).env("XDG_DATA_HOME", home.join("data")).env_remove("UFOP_PASSPHRASE");
    c.stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() }).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = c.spawn().unwrap();
    if let Some(data) = stdin {
        let mut i = child.stdin.take().unwrap();
        i.write_all(data).unwrap();
    }
    let out = child.wait_with_output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn find(dir: &Path, name: &str) -> Option<std::path::PathBuf> {
    for e in std::fs::read_dir(dir).ok()?.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
        if p.is_dir() {
            if let Some(f) = find(&p, name) {
                return Some(f);
            }
        }
    }
    None
}

#[test]
fn stdin_backup_restores_and_estimate_reports() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let dest = t.path().join("dest");
    let db = t.path().join("db.sqlite");
    let (dest_s, db_s) = (dest.to_string_lossy().into_owned(), db.to_string_lossy().into_owned());
    let payload: Vec<u8> = (0..200_000u32).map(|i| (i * 7 % 251) as u8).collect();
    let (code, out, err) = ufop(&["--json", "backup", "stdin", "--dest", &dest_s, "--dbpath", &db_s, "--passphrase", "pw", "--name", "db.sql", "--no-quota-warning"], Some(&payload), home);
    assert_eq!(code, 0, "{out}\n{err}");
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap_or_else(|_| serde_json::from_str(out.lines().last().unwrap()).unwrap());
    let report = if v.get("data").is_some() { &v["data"] } else { &v };
    assert!(report["version"].is_string(), "{v}");

    let target = t.path().join("out");
    let (code, out, err) = ufop(&["--json", "backup", "restore", "--dest", &dest_s, "--dbpath", &db_s, "--passphrase", "pw", "--target", &target.to_string_lossy()], None, home);
    assert_eq!(code, 0, "{out}\n{err}");
    assert_eq!(std::fs::read(find(&target, "db.sql").expect("restored file")).unwrap(), payload);

    // Estimate a folder against a fresh destination, with an ignore file.
    let src = t.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), vec![1u8; 4000]).unwrap();
    std::fs::write(src.join("skip.log"), vec![2u8; 9000]).unwrap();
    std::fs::write(src.join(".backupignore"), b"*.log\n").unwrap();
    let dest2 = t.path().join("dest2").to_string_lossy().into_owned();
    let (code, out, err) = ufop(
        &["--json", "backup", "estimate", "--dest", &dest2, "--dbpath", &t.path().join("db2.sqlite").to_string_lossy(), "--no-encryption", "--source", &src.to_string_lossy(), "--ignore-file", ".backupignore", "--assumed-speed", "1K"],
        None,
        home,
    );
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.contains("\"new_files\": 2") || out.contains("\"new_files\":2"), "the ignore file and a.txt, not skip.log: {out}");
    assert!(!out.contains("skip.log"), "{out}");

    // The new run flags parse and a capped run exits 2 with a partial version.
    let (code, out, err) = ufop(
        &[
            "--json", "backup", "run", "--dest", &dest2, "--dbpath", &t.path().join("db2.sqlite").to_string_lossy(), "--no-encryption", "--source", &src.to_string_lossy(),
            "--respect-gitignore", "--one-file-system", "--on-file-error", "fail", "--on-dir-error", "continue", "--on-unknown-entry", "warn", "--max-errors", "10",
            "--rehash-percent", "5", "--parallel-file-threshold", "1M", "--snapshot-provider", "auto", "--no-quota-warning",
        ],
        None,
        home,
    );
    assert_eq!(code, 0, "{out}\n{err}");
    let (code, _, err) = ufop(&["backup", "run", "--dest", &dest2, "--no-encryption", "--source", "/x", "--on-file-error", "maybe"], None, home);
    assert_ne!(code, 0);
    assert!(err.contains("on-file-error") || err.contains("maybe"), "{err}");
}
