use std::{fs, process::Command};
use tempfile::tempdir;

#[test]
fn every_output_format_copies_and_verifies_the_destination_tree() {
    for format in ["human", "json", "yaml"] {
        let temp = tempdir().unwrap();
        let source = temp.path().join("source");
        let destination = temp.path().join("destination");
        fs::create_dir_all(source.join("nested/empty")).unwrap();
        fs::write(source.join("a.txt"), b"first file\n").unwrap();
        fs::write(source.join("nested/b.bin"), [0, 255, 42]).unwrap();
        let result = Command::new(env!("CARGO_BIN_EXE_ufop"))
            .args(["--format", format, "transfer"])
            .arg(&source)
            .arg(&destination)
            .arg("--verify")
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        assert_eq!(
            fs::read(destination.join("a.txt")).unwrap(),
            b"first file\n"
        );
        assert_eq!(
            fs::read(destination.join("nested/b.bin")).unwrap(),
            [0, 255, 42]
        );
        assert!(destination.join("nested/empty").is_dir());
        if format != "human" {
            let text = String::from_utf8(result.stdout).unwrap();
            let data: serde_json::Value = if format == "json" {
                serde_json::from_str(&text).unwrap()
            } else {
                serde_yaml::from_str(&text).unwrap()
            };
            assert_eq!(data["verified"], true);
            assert_eq!(data["files_transferred"], 2);
            assert_eq!(data["bytes_transferred"], 14);
        }
    }
}

#[test]
fn unsupported_remote_transfer_does_not_report_a_queued_job() {
    let result = Command::new(env!("CARGO_BIN_EXE_ufop"))
        .args([
            "--json",
            "transfer",
            "sftp://audit.invalid/a",
            "s3://audit.invalid/b",
            "--verify",
        ])
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!String::from_utf8_lossy(&result.stdout).contains("queued"));
}

#[test]
fn same_file_overwrite_preserves_source_and_dry_run_creates_nothing() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("a.txt");
    fs::write(&source, b"irreplaceable").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_ufop"))
        .args(["transfer"])
        .arg(&source)
        .arg(&source)
        .arg("--overwrite")
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(fs::read(&source).unwrap(), b"irreplaceable");
    let destination = temp.path().join("absent");
    let result = Command::new(env!("CARGO_BIN_EXE_ufop"))
        .args(["--dry-run", "transfer"])
        .arg(&source)
        .arg(&destination)
        .output()
        .unwrap();
    assert!(result.status.success());
    assert!(!destination.exists());
}

#[test]
fn a_partial_backup_reports_skips_without_claiming_verification() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::create_dir_all(&source).unwrap();
    fs::create_dir_all(&destination).unwrap();
    fs::write(source.join("a"), b"new").unwrap();
    fs::write(source.join("b"), b"copied").unwrap();
    fs::write(destination.join("a"), b"keep").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_ufop"))
        .args(["--json", "transfer"])
        .arg(&source)
        .arg(&destination)
        .arg("--verify")
        .output()
        .unwrap();
    assert_eq!(result.status.code(), Some(1));
    let data: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(data["verified"], false);
    assert_eq!(data["files_skipped"], 1);
    assert_eq!(fs::read(destination.join("a")).unwrap(), b"keep");
    assert_eq!(fs::read(destination.join("b")).unwrap(), b"copied");
    assert_eq!(fs::read(source.join("a")).unwrap(), b"new");
}

#[test]
fn overwrite_verifies_actual_replacement_and_missing_source_is_a_failure() {
    let temp = tempdir().unwrap();
    let source = temp.path().join("source");
    let destination = temp.path().join("destination");
    fs::write(&source, b"new contents").unwrap();
    fs::write(&destination, b"old contents").unwrap();
    let result = Command::new(env!("CARGO_BIN_EXE_ufop"))
        .args(["--json", "transfer"])
        .arg(&source)
        .arg(&destination)
        .args(["--overwrite", "--verify"])
        .output()
        .unwrap();
    assert!(result.status.success());
    assert_eq!(fs::read(&source).unwrap(), fs::read(&destination).unwrap());
    let data: serde_json::Value = serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(data["verified"], true);
    let missing_dest = temp.path().join("no-output");
    let result = Command::new(env!("CARGO_BIN_EXE_ufop"))
        .arg("transfer")
        .arg(temp.path().join("absent"))
        .arg(&missing_dest)
        .output()
        .unwrap();
    assert!(!result.status.success());
    assert!(!missing_dest.exists());
}
