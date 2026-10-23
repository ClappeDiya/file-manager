//! Source selection and run-control tests: ignore files, one filesystem,
//! error policy, rehash, streams, upload cap, parallel large files.

use super::*;
use crate::backup::backend::LocalBackend;
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
