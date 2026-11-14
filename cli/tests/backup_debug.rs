//! `ufop backup bench` and `ufop backup debug …`, driven through the real binary.

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

fn json(out: &str) -> serde_json::Value {
    let v: serde_json::Value = serde_json::from_str(out.trim()).unwrap_or_else(|e| panic!("{e}: {out}"));
    if v.get("data").is_some() { v["data"].clone() } else { v }
}

#[test]
fn bench_prints_usable_run_flags() {
    let t = tempfile::tempdir().unwrap();
    let base = ["backup", "bench", "--size", "128K", "--block-size", "16K", "--repeat", "1", "--kdf-target-ms", "0", "--only", "compression,crypto,chunkers"];
    let mut a = base.to_vec();
    a.push("--print-options");
    let (code, out, err) = ufop(&a, t.path());
    assert_eq!(code, 0, "{out}\n{err}");
    assert!(out.contains("--block-codec") && out.contains("--cipher") && out.contains("--chunker fastcdc"), "{out}");
    let mut j = vec!["--json"];
    j.extend(base);
    let (code, out, err) = ufop(&j, t.path());
    assert_eq!(code, 0, "{out}\n{err}");
    let r = json(&out);
    assert!(r["compression"].as_array().unwrap().len() >= 8, "{r}");
    assert_eq!(r["ciphers"].as_array().unwrap().len(), 2);
    assert!(r["hashes"].as_array().unwrap().is_empty());
}

#[test]
fn debug_views_of_a_backup() {
    let t = tempfile::tempdir().unwrap();
    let home = t.path();
    let src = t.path().join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(src.join("a.txt"), b"some text to back up ".repeat(300)).unwrap();
    let (dest, db) = (t.path().join("dest"), t.path().join("db.sqlite"));
    let (dest_s, db_s, src_s) = (dest.to_string_lossy().into_owned(), db.to_string_lossy().into_owned(), src.to_string_lossy().into_owned());
    let repo = ["--dest", dest_s.as_str(), "--dbpath", db_s.as_str(), "--passphrase", "pw"];
    let mut run = vec!["backup", "run", "--source", src_s.as_str(), "--block-size", "1024", "--volume-size", "4096", "--verify-samples", "0", "--no-quota-warning"];
    run.extend(repo);
    let (code, out, err) = ufop(&run, home);
    assert_eq!(code, 0, "{out}\n{err}");

    let dbg = |sub: &[&str]| {
        let mut v = vec!["--json", "backup", "debug"];
        v.extend_from_slice(sub);
        v.extend(repo);
        let (code, out, err) = ufop(&v, home);
        assert_eq!(code, 0, "{sub:?}: {out}\n{err}");
        json(&out)
    };
    let h = dbg(&["header"]);
    assert_eq!(h["header"]["encrypted"], true);
    assert!(h["format"]["format_version"].as_u64().unwrap() >= 1);
    let vols = dbg(&["volumes", "--kind", "dindex"]);
    let idx = vols.as_array().unwrap()[0]["name"].as_str().unwrap().to_string();
    let s = dbg(&["stats"]);
    assert_eq!(s["versions"].as_array().unwrap().len(), 1);
    let blocks = dbg(&["blocks", "--details"]);
    let first = blocks.as_array().unwrap()[0]["hash"].as_str().unwrap().to_string();
    assert!(blocks.as_array().unwrap()[0]["codec"].is_string());
    let out_file = t.path().join("block.bin");
    let b = dbg(&["block", &first, "--out", out_file.to_str().unwrap()]);
    assert_eq!(b["hash_ok"], true);
    assert_eq!(std::fs::metadata(&out_file).unwrap().len(), b["size"].as_u64().unwrap());
    let i = dbg(&["index", &idx]);
    assert!(!i["blocks"].as_array().unwrap().is_empty());
    let f = dbg(&["fileset", "0"]);
    assert!(f["files"].as_array().unwrap().iter().any(|x| x["path"].as_str().unwrap().ends_with("a.txt")));
    let sess = dbg(&["sessions"]);
    assert!(sess["writer"].is_null());

    let (code, out, err) = ufop(&["--json", "backup", "debug", "db", db_s.as_str()], home);
    assert_eq!(code, 0, "{out}\n{err}");
}
