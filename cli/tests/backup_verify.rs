//! `ufop backup verify` scope and sampling flags, driven through the real binary.

use std::path::Path;
use std::process::{Command, Stdio};

fn ufop(args: &[&str], home: &Path) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_ufop"))
        .args(args)
        .env("HOME", home)
        .env("XDG_CONFIG_HOME", home.join("cfg"))
        .env("XDG_DATA_HOME", home.join("data"))
        .env_remove("UFOP_PASSPHRASE")
        .stdin(Stdio::null())
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn data(v: &serde_json::Value) -> &serde_json::Value {
    if v.get("data").is_some() { &v["data"] } else { v }
}

#[test]
fn verify_scope_percent_files_and_max_errors() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let src = t.path().join("src");
    std::fs::create_dir_all(src.join("docs")).unwrap();
    std::fs::write(src.join("docs/a.txt"), (0..5000u32).map(|i| (i % 251) as u8).collect::<Vec<_>>()).unwrap();
    std::fs::write(src.join("b.bin"), (0..9000u32).map(|i| (i * 13 % 239) as u8).collect::<Vec<_>>()).unwrap();
    let (dest, db) = (t.path().join("dest"), t.path().join("db.sqlite"));
    let (dest_s, db_s, src_s) = (dest.to_string_lossy().into_owned(), db.to_string_lossy().into_owned(), src.to_string_lossy().into_owned());
    let repo = ["--dest", dest_s.as_str(), "--dbpath", db_s.as_str(), "--passphrase", "pw"];
    let mut run = vec!["backup", "run", "--source", src_s.as_str(), "--block-size", "1024", "--volume-size", "4096", "--verify-samples", "0", "--no-quota-warning"];
    run.extend(repo);
    let (code, out, err) = ufop(&run, home);
    assert_eq!(code, 0, "{out}\n{err}");

    let mut v = vec!["--json", "backup", "verify", "--version", "latest", "--path", "*/docs/*", "--percent", "100", "--files-percent", "100", "--parallel", "2"];
    v.extend(repo);
    let (code, out, err) = ufop(&v, home);
    assert_eq!(code, 0, "{out}\n{err}");
    let r: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    let r = data(&r);
    assert_eq!((r["files_in_scope"].as_u64(), r["files_checked"].as_u64(), r["versions_checked"].as_u64()), (Some(1), Some(1), Some(1)), "{r}");
    assert!(r["problems"].as_array().unwrap().is_empty());

    // Remove every block volume: structure finds them, --max-errors 1 stops at the first.
    for e in std::fs::read_dir(&dest).unwrap().flatten() {
        if e.file_name().to_string_lossy().contains(".dblock") {
            std::fs::remove_file(e.path()).unwrap();
        }
    }
    let mut v = vec!["--json", "backup", "verify", "--structure", "--max-errors", "1"];
    v.extend(repo);
    let (code, out, err) = ufop(&v, home);
    assert_eq!(code, 2, "{out}\n{err}");
    let r: serde_json::Value = serde_json::from_str(out.trim()).unwrap();
    let r = data(&r);
    assert_eq!(r["problems"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["problems"][0]["kind"], "missing_volume");
    assert!(r["stop_reason"].as_str().unwrap().contains("max errors"));
    // Human output names the problem.
    let mut v = vec!["backup", "verify", "--structure"];
    v.extend(repo);
    let (code, out, _) = ufop(&v, home);
    assert_eq!(code, 2);
    assert!(out.contains("[missing_volume]"), "{out}");
}
