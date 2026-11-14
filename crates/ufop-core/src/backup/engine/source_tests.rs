//! Source selection and run-control tests: ignore files, one filesystem,
//! error policy, rehash, streams, upload cap, parallel large files.

use super::*;
use crate::backup::backend::LocalBackend;
use crate::backup::crypto::KdfParams;
use crate::backup::db::BackupDb;
#[cfg(unix)]
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

/// Incompressible, deterministic test data.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut x = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    (0..len)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 24) as u8
        })
        .collect()
}

fn latest_files(db: &BackupDb) -> Vec<FileEntry> {
    db.files(db.filesets().unwrap()[0].id).unwrap()
}

#[test]
fn rehash_selection_is_deterministic_and_spread() {
    let paths: Vec<String> = (0..2000).map(|i| format!("/data/file-{i}.bin")).collect();
    let pick = |run| paths.iter().filter(|p| rehash_selected(p, run, 10)).cloned().collect::<Vec<_>>();
    let (a, b) = (pick(1), pick(2));
    assert!((120..=280).contains(&a.len()), "about 10% of 2000: {}", a.len());
    assert_eq!(a, pick(1), "same run, same files");
    assert_ne!(a, b, "the next run checks other files");
    assert!(paths.iter().all(|p| rehash_selected(p, 7, 100)));
    assert!(!paths.iter().any(|p| rehash_selected(p, 7, 0)));
}

#[test]
fn rehash_finds_content_changed_without_timestamp_change() {
    let src = tempfile::tempdir().unwrap();
    for i in 0..5 {
        write(src.path(), &format!("f{i}"), &noise(i, 3000));
    }
    let dst = tempfile::tempdir().unwrap();
    let (be, db) = (LocalBackend::new(dst.path()), BackupDb::open_in_memory().unwrap());
    let f = SourceFilter::default().compile().unwrap();
    let mut o = opts();
    backup(&be, &db, src.path(), &f, &o);
    // Silent corruption: same size, same timestamp, other bytes.
    let p = src.path().join("f2");
    let mtime = std::fs::metadata(&p).unwrap().modified().unwrap();
    std::fs::write(&p, noise(99, 3000)).unwrap();
    std::fs::File::options().write(true).open(&p).unwrap().set_modified(mtime).unwrap();
    let r = backup(&be, &db, src.path(), &f, &o);
    assert!(r.skipped_unchanged && r.files_rehashed == 0, "without rehash the change goes unnoticed");
    o.rehash_percent = 100;
    let r = backup(&be, &db, src.path(), &f, &o);
    assert_eq!((r.files_rehashed, r.rehash_mismatches), (5, 1), "{r:?}");
    assert!(r.warnings.iter().any(|w| w.contains("f2") && w.contains("possible silent corruption")), "{:?}", r.warnings);
    assert!(r.version.is_some());
    let e = latest_files(&db).into_iter().find(|e| e.path.ends_with("f2")).unwrap();
    assert_eq!(e.hash.as_deref(), Some(hash_bytes(HashAlgorithm::Sha256, &noise(99, 3000)).as_str()), "the new content is backed up");
    // Nothing differs now: rehashing reads but changes nothing.
    let r = backup(&be, &db, src.path(), &f, &o);
    assert!(r.skipped_unchanged && r.rehash_mismatches == 0 && r.files_rehashed == 5, "{r:?}");
    o.rehash_percent = 101;
    assert!(run_backup(&be, &db, &[src.path().into()], &f, &o, &RetentionPolicy::KeepAll, None, KdfParams::insecure_fast(), &OpContext::new()).is_err());
}

fn stream_backup(be: &LocalBackend, db: &BackupDb, sources: &[PathBuf], o: &BackupOptions, streams: Vec<StreamSource>) -> BackupReport {
    let f = SourceFilter::default().compile().unwrap();
    run_backup_with_streams(be, db, sources, &f, o, &RetentionPolicy::KeepAll, Some("pw"), None, KdfParams::insecure_fast(), &OpContext::new(), streams).unwrap()
}

#[test]
fn stream_is_backed_up_as_a_virtual_file_and_restores() {
    let dst = tempfile::tempdir().unwrap();
    let (be, db) = (LocalBackend::new(dst.path()), BackupDb::open_in_memory().unwrap());
    let data = noise(5, 50_000);
    let o = opts();
    let r = stream_backup(&be, &db, &[], &o, vec![StreamSource::new("db.sql", std::io::Cursor::new(data.clone()))]);
    assert!(r.errors.is_empty() && r.version.is_some(), "{r:?}");
    let files = latest_files(&db);
    let e = files.iter().find(|e| e.path == "/stdin/db.sql").unwrap();
    assert_eq!((e.size, e.mode), (50_000, Some(0o100644)));
    assert!(files.iter().any(|e| e.path == "/stdin" && e.kind == EntryKind::Dir));
    // Restore it like any file.
    let out = tempfile::tempdir().unwrap();
    let repo = Repo::open(&be, &db, Some("pw"), &o, false, KdfParams::insecure_fast()).unwrap();
    let rr = crate::backup::run_restore(&repo, &crate::backup::RestoreOptions { target: Some(out.path().into()), ..Default::default() }, &OpContext::new()).unwrap();
    assert!(rr.errors.is_empty(), "{:?}", rr.errors);
    let restored = walk(out.path()).into_iter().find(|p| p.ends_with("db.sql")).unwrap();
    assert_eq!(std::fs::read(restored).unwrap(), data);

    // Together with folders; a failing producer fails the run.
    let src = tempfile::tempdir().unwrap();
    write(src.path(), "a.txt", b"folder file");
    let bad = StreamSource::new("/dumps/broken.sql", std::io::Cursor::new(b"partial".to_vec())).with_finish(|| Err("pg_dump exited with status 1".into()));
    let good = StreamSource::new("ok.txt", std::io::Cursor::new(b"fine".to_vec()));
    let r = stream_backup(&be, &db, &[src.path().into()], &o, vec![bad, good]);
    assert!(r.errors.iter().any(|e| e.contains("/dumps/broken.sql") && e.contains("status 1")), "{:?}", r.errors);
    let paths: Vec<String> = latest_files(&db).into_iter().map(|e| e.path).collect();
    assert!(paths.iter().any(|p| p.ends_with("a.txt")) && paths.contains(&"/stdin/ok.txt".to_string()), "{paths:?}");
    assert!(!paths.contains(&"/dumps/broken.sql".to_string()));
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[test]
fn upload_cap_stops_with_a_partial_version_and_the_next_run_continues() {
    let src = tempfile::tempdir().unwrap();
    for i in 0..24 {
        write(src.path(), &format!("f{i:02}"), &noise(100 + i, 6000));
    }
    let dst = tempfile::tempdir().unwrap();
    let (be, db) = (LocalBackend::new(dst.path()), BackupDb::open_in_memory().unwrap());
    let f = SourceFilter::default().compile().unwrap();
    let mut o = opts();
    o.hash_threads = 1;
    o.max_upload_bytes_per_run = Some(30_000);
    let r = backup(&be, &db, src.path(), &f, &o);
    assert!(r.upload_cap_reached && r.partial && r.errors.is_empty(), "{r:?}");
    assert!(r.warnings.iter().any(|w| w.contains("Upload limit")), "{:?}", r.warnings);
    assert!(r.bytes_uploaded < 24 * 6000, "stopped early: {}", r.bytes_uploaded);
    let first = db.filesets().unwrap()[0].clone();
    assert!(first.partial && first.stats.is_some_and(|s| s.upload_cap_reached));
    let first_files = db.files(first.id).unwrap().iter().filter(|e| e.kind == EntryKind::File).count();
    assert!(first_files > 0 && first_files < 24, "{first_files}");
    // The next run without a cap uploads only the rest.
    o.max_upload_bytes_per_run = None;
    let r2 = backup(&be, &db, src.path(), &f, &o);
    assert!(!r2.partial && !r2.upload_cap_reached && r2.errors.is_empty(), "{r2:?}");
    assert_eq!(latest_files(&db).iter().filter(|e| e.kind == EntryKind::File).count(), 24);
    assert!(r2.new_blocks < 24 * 6, "already uploaded blocks are not sent again: {}", r2.new_blocks);
}

#[test]
fn parallel_large_files_match_sequential_reading() {
    let src = tempfile::tempdir().unwrap();
    let big = noise(42, 3 * 1024 * 1024 + 123);
    write(src.path(), "big.bin", &big);
    write(src.path(), "small.txt", b"small");
    let f = SourceFilter::default().compile().unwrap();
    let run = |threshold: u64| {
        let dst = tempfile::tempdir().unwrap();
        let (be, db) = (LocalBackend::new(dst.path()), BackupDb::open_in_memory().unwrap());
        let o = BackupOptions { block_size: 16 * 1024, volume_size: 1024 * 1024, verify_samples: 0, hash_threads: 4, parallel_file_threshold: threshold, ..Default::default() };
        let t = Instant::now();
        let r = run_backup_with_streams(&be, &db, &[src.path().into()], &f, &o, &RetentionPolicy::KeepAll, None, None, KdfParams::insecure_fast(), &OpContext::new(), vec![StreamSource::new("s.bin", std::io::Cursor::new(big.clone()))]).unwrap();
        assert!(r.errors.is_empty(), "{:?}", r.errors);
        let elapsed = t.elapsed();
        let mut files = latest_files(&db);
        files.sort_by(|a, b| a.path.cmp(&b.path));
        (files.into_iter().map(|e| (e.path, e.hash, e.blocks, e.size)).collect::<Vec<_>>(), r.new_blocks, elapsed)
    };
    let (seq, seq_blocks, seq_t) = run(0);
    let (par, par_blocks, par_t) = run(1024 * 1024);
    assert_eq!(seq, par, "identical block lists and hashes");
    assert_eq!(seq_blocks, par_blocks);
    assert!(seq.iter().any(|(p, _, b, _)| p.ends_with("big.bin") && b.len() > 50));
    eprintln!("large file: sequential {seq_t:?}, parallel {par_t:?} (informational)");
}
