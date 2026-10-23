//! Source selection and run-control tests: ignore files, one filesystem,
//! error policy, rehash, streams, upload cap, parallel large files.

use super::*;
use crate::backup::backend::LocalBackend;
use crate::backup::crypto::KdfParams;
use crate::backup::db::BackupDb;
use crate::backup::errors::{ErrorAction, ErrorPolicy, UnknownEntryAction};
use crate::backup::retention::RetentionPolicy;
use crate::backup::test_util::write;
use crate::backup::{FilterAction, FilterKind, FilterRule};

fn opts() -> BackupOptions {
    BackupOptions { block_size: 1024, volume_size: 4096, verify_samples: 0, ..Default::default() }
}

fn filter(edit: impl FnOnce(&mut SourceFilter)) -> SourceFilter {
    let mut f = SourceFilter::default();
    edit(&mut f);
    f.compile().unwrap()
}

fn rels(root: &Path, entries: &[(PathBuf, FileEntry)]) -> Vec<String> {
    let mut v: Vec<String> = entries.iter().filter_map(|(p, _)| p.strip_prefix(root).ok()).map(|p| p.to_string_lossy().replace('\\', "/")).filter(|s| !s.is_empty()).collect();
    v.sort();
    v
}

#[test]
fn ignore_files_nested_with_negations_and_rule_precedence() {
    let d = tempfile::tempdir().unwrap();
    let root = d.path();
    write(root, ".ufopignore", b"*.log\nbuild/\n");
    write(root, "a.log", b"x");
    write(root, "a.txt", b"x");
    write(root, "build/out.bin", b"x");
    write(root, "app/.ufopignore", b"!keep.log\n/local/\n");
    write(root, "app/keep.log", b"x");
    write(root, "app/drop.log", b"x");
    write(root, "app/local/x", b"x");
    write(root, "app/sub/local/y", b"x");
    write(root, "repo/.gitignore", b"target/\n");
    write(root, "repo/.git/info/exclude", b"*.o\n");
    write(root, "repo/target/t", b"x");
    write(root, "repo/m.o", b"x");
    write(root, "repo/src.rs", b"x");
    let o = opts();
    let (e, _) = scan_sources(&[root.into()], &SourceFilter::default().compile().unwrap(), &o).unwrap();
    let got = rels(root, &e);
    for want in ["a.txt", "app/keep.log", "app/sub/local/y", "repo/target/t", "repo/m.o", ".ufopignore"] {
        assert!(got.contains(&want.to_string()), "{want} missing from {got:?}");
    }
    for gone in ["a.log", "build", "build/out.bin", "app/drop.log", "app/local", "app/local/x"] {
        assert!(!got.contains(&gone.to_string()), "{gone} should be ignored: {got:?}");
    }
    // .gitignore on: the repository's rules apply too.
    let f = filter(|f| f.respect_gitignore = true);
    let got = rels(root, &scan_sources(&[root.into()], &f, &o).unwrap().0);
    assert!(!got.contains(&"repo/target/t".to_string()) && !got.contains(&"repo/m.o".to_string()), "{got:?}");
    assert!(got.contains(&"repo/src.rs".to_string()));
    // An explicit include rule wins over the ignore file.
    let f = filter(|f| f.rules = vec![FilterRule { action: FilterAction::Include, kind: FilterKind::Wildcard, pattern: "*/a.log".into() }]);
    let got = rels(root, &scan_sources(&[root.into()], &f, &o).unwrap().0);
    assert!(got.contains(&"a.log".to_string()), "{got:?}");
    // Custom names only; the default file is no longer read.
    let f = filter(|f| f.ignore_files = vec![".backupignore".into()]);
    let got = rels(root, &scan_sources(&[root.into()], &f, &o).unwrap().0);
    assert!(got.contains(&"a.log".to_string()));
}

fn backup(be: &LocalBackend, db: &BackupDb, src: &Path, f: &SourceFilter, o: &BackupOptions) -> BackupReport {
    run_backup(be, db, &[src.into()], f, o, &RetentionPolicy::KeepAll, None, KdfParams::insecure_fast(), &OpContext::new()).unwrap()
}

#[cfg(unix)]
#[test]
fn one_file_system_keeps_mount_points_empty() {
    // A folder on another device: /dev/shm is a tmpfs in most containers.
    let Ok(other) = tempfile::tempdir_in("/dev/shm") else {
        eprintln!("skipped: /dev/shm unavailable");
        return;
    };
    let src = tempfile::tempdir().unwrap();
    let dev = |p: &Path| meta::device_id(p, &std::fs::metadata(p).unwrap());
    if dev(other.path()) == dev(src.path()) {
        eprintln!("skipped: no second filesystem");
        return;
    }
    write(src.path(), "local.txt", b"here");
    write(other.path(), "remote.txt", b"there");
    std::os::unix::fs::symlink(other.path(), src.path().join("mnt")).unwrap();
    let mut o = opts();
    o.symlink_policy = crate::backup::SymlinkPolicy::Follow;
    let f = SourceFilter::default().compile().unwrap();
    let (e, _) = scan_sources(&[src.path().into()], &f, &o).unwrap();
    assert!(rels(src.path(), &e).contains(&"mnt/remote.txt".to_string()));
    o.one_file_system = true;
    let dst = tempfile::tempdir().unwrap();
    let (be, db) = (LocalBackend::new(dst.path()), BackupDb::open_in_memory().unwrap());
    let r = backup(&be, &db, src.path(), &f, &o);
    assert!(r.errors.is_empty(), "{:?}", r.errors);
    assert!(r.info.iter().any(|i| i.contains("another filesystem") && i.contains("mnt")), "{:?}", r.info);
    let files = db.files(db.filesets().unwrap()[0].id).unwrap();
    let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
    assert!(paths.iter().any(|p| p.ends_with("/mnt")), "the mount point itself is kept: {paths:?}");
    assert!(!paths.iter().any(|p| p.ends_with("remote.txt")), "{paths:?}");
    assert!(paths.iter().any(|p| p.ends_with("local.txt")));
}

#[cfg(unix)]
fn locked(path: &Path) -> std::fs::File {
    use std::os::fd::AsRawFd;
    let f = std::fs::File::open(path).unwrap();
    assert_eq!(unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX) }, 0);
    f
}

#[cfg(unix)]
#[test]
fn file_error_policy_continue_fail_and_fail_fast() {
    let src = tempfile::tempdir().unwrap();
    for i in 0..30 {
        write(src.path(), &format!("f{i:02}.txt"), format!("content {i}").as_bytes());
    }
    let _hold = locked(&src.path().join("f05.txt"));
    let f = SourceFilter::default().compile().unwrap();
    let run = |policy: ErrorPolicy| {
        let dst = tempfile::tempdir().unwrap();
        let (be, db) = (LocalBackend::new(dst.path()), BackupDb::open_in_memory().unwrap());
        let mut o = opts();
        o.respect_advisory_locks = true;
        o.hash_threads = 1;
        o.error_policy = policy;
        let r = backup(&be, &db, src.path(), &f, &o);
        let n = db.filesets().unwrap().first().map(|fs| db.files(fs.id).unwrap().len());
        (r, n)
    };
    // Default: a warning, the run succeeds.
    let (r, _) = run(ErrorPolicy::default());
    assert!(r.errors.is_empty() && r.warnings.iter().any(|w| w.contains("f05.txt")), "{r:?}");
    assert!(!r.partial);
    // Fail: an error, but the version is complete.
    let (r, n) = run(ErrorPolicy { on_file_error: ErrorAction::Fail, ..Default::default() });
    assert!(r.errors.iter().any(|e| e.contains("f05.txt")), "{:?}", r.errors);
    assert!(!r.partial && r.version.is_some());
    assert_eq!(n, Some(30), "29 files + the root folder");
    // Fail-fast: stops at the first error with a partial version (or none).
    let (r, _) = run(ErrorPolicy { on_file_error: ErrorAction::Fail, fail_fast: true, ..Default::default() });
    assert!(r.errors.iter().any(|e| e.contains("fail-fast")), "{:?}", r.errors);
    assert!(r.version.is_none() || r.partial);
    // An error budget of zero stops too, even for warnings.
    let (r, _) = run(ErrorPolicy { max_errors: Some(0), ..Default::default() });
    assert!(r.errors.iter().any(|e| e.contains("more than 0")), "{:?}", r.errors);
}

#[cfg(unix)]
#[test]
fn unknown_entries_and_scan_errors() {
    let src = tempfile::tempdir().unwrap();
    write(src.path(), "a.txt", b"a");
    let fifo = std::ffi::CString::new(src.path().join("pipe").to_string_lossy().as_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(fifo.as_ptr(), 0o644) }, 0);
    std::os::unix::fs::symlink(src.path().join("missing"), src.path().join("dangling")).unwrap();
    let f = SourceFilter::default().compile().unwrap();
    let mut o = opts();
    o.symlink_policy = crate::backup::SymlinkPolicy::Follow;
    let (_, w) = scan_sources(&[src.path().into()], &f, &o).unwrap();
    assert!(w.iter().any(|w| w.contains("Broken link")) && !w.iter().any(|w| w.contains("pipe")), "{w:?}");
    o.error_policy.on_unknown_entry = UnknownEntryAction::Warn;
    let (_, w) = scan_sources(&[src.path().into()], &f, &o).unwrap();
    assert!(w.iter().any(|w| w.contains("pipe") && w.contains("not a regular file")), "{w:?}");
    // Fail-fast while scanning: no version at all.
    o.error_policy = ErrorPolicy { on_unknown_entry: UnknownEntryAction::Fail, fail_fast: true, ..Default::default() };
    let dst = tempfile::tempdir().unwrap();
    let (be, db) = (LocalBackend::new(dst.path()), BackupDb::open_in_memory().unwrap());
    let r = backup(&be, &db, src.path(), &f, &o);
    assert!(r.version.is_none() && r.errors.iter().any(|e| e.contains("fail-fast")), "{r:?}");
    assert!(db.filesets().unwrap().is_empty());
    // Folder errors can only be provoked without root rights.
    if unsafe { libc::geteuid() } != 0 {
        use std::os::unix::fs::PermissionsExt;
        write(src.path(), "closed/x", b"x");
        std::fs::set_permissions(src.path().join("closed"), std::fs::Permissions::from_mode(0o000)).unwrap();
        let mut o = opts();
        o.error_policy.on_dir_error = ErrorAction::Fail;
        let r = backup(&be, &db, src.path(), &f, &o);
        std::fs::set_permissions(src.path().join("closed"), std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(r.errors.iter().any(|e| e.contains("Cannot list")), "{r:?}");
    }
}
