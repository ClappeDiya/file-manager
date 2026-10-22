use super::*;
use std::fs;

fn tree() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("docs/2026")).unwrap();
    fs::create_dir_all(d.path().join("node_modules/pkg")).unwrap();
    fs::write(d.path().join("docs/Quarterly Report.txt"), "Revenue grew in the third quarter; café openings drove it.").unwrap();
    fs::write(d.path().join("docs/2026/invoice_100%.csv"), "id,amount\n1,250\n").unwrap();
    fs::write(d.path().join("docs/photo.bin"), [0u8, 159, 146, 150, 0, 1]).unwrap();
    fs::write(d.path().join("node_modules/pkg/index.js"), "revenue").unwrap();
    d
}

fn idx(root: &Path, content: bool) -> (SearchIndex, IndexStats) {
    let i = SearchIndex::open_in_memory().unwrap();
    i.add_root(root.to_str().unwrap(), content).unwrap();
    let s = i.index_root(root.to_str().unwrap(), &AtomicBool::new(false), &AtomicU64::new(0)).unwrap();
    (i, s)
}

fn q(text: &str, root: Option<&Path>, content: bool) -> SearchQuery {
    SearchQuery { text: text.into(), root: root.map(|r| r.to_string_lossy().to_string()), content, limit: None }
}

#[test]
fn finds_names_by_substring_and_skips_junk_dirs() {
    let d = tree();
    let (i, stats) = idx(d.path(), false);
    // docs, docs/2026, 3 files; node_modules is skipped.
    assert_eq!(stats.added, 5);
    let names = |text: &str| i.search(&q(text, None, false)).unwrap().into_iter().map(|h| h.name).collect::<Vec<_>>();
    assert_eq!(names("report"), vec!["Quarterly Report.txt"]);
    assert_eq!(names("100%"), vec!["invoice_100%.csv"], "LIKE wildcards are literal");
    assert_eq!(names("_1"), vec!["invoice_100%.csv"]);
    assert!(names("index").is_empty());
    let dirs = i.search(&q("2026", None, false)).unwrap();
    assert!(dirs[0].is_dir);
    assert_eq!(i.roots().unwrap()[0].files, 5);
}

#[test]
fn content_search_with_snippets_prefixes_and_diacritics() {
    let d = tree();
    let (i, _) = idx(d.path(), true);
    let hits = i.search(&q("quart revenue", None, true)).unwrap();
    assert_eq!(hits.len(), 1);
    assert!(hits[0].snippet.as_deref().unwrap().contains("[Revenue]"), "{:?}", hits[0].snippet);
    assert_eq!(i.search(&q("cafe", None, true)).unwrap().len(), 1, "diacritics folded");
    assert!(i.search(&q("\"OR\" AND (", None, true)).unwrap().is_empty(), "operators are literal");
    // Binary files have no content, but their names still match.
    assert_eq!(i.search(&q("photo", None, true)).unwrap().len(), 1);
    // Names-only roots don't index content.
    let (names_only, _) = idx(d.path(), false);
    assert!(names_only.search(&q("revenue", None, true)).unwrap().is_empty());
}

#[test]
fn root_filter_and_covering_root() {
    let d = tree();
    let (i, _) = idx(d.path(), false);
    let sub = d.path().join("docs/2026");
    let hits = i.search(&q("i", Some(&sub), false)).unwrap();
    assert_eq!(hits.iter().map(|h| h.name.as_str()).collect::<Vec<_>>(), vec!["invoice_100%.csv"]);
    // A sibling folder sharing the prefix is not "under" the root.
    assert!(i.search(&q("report", Some(&d.path().join("doc")), false)).unwrap().is_empty());
    assert_eq!(i.root_covering(sub.to_str().unwrap()).unwrap().unwrap().path, d.path().to_str().unwrap());
    assert!(i.root_covering("/elsewhere").unwrap().is_none());
}

#[test]
fn incremental_reindex_updates_and_removes() {
    let d = tree();
    let (i, _) = idx(d.path(), true);
    let root = d.path().to_str().unwrap();
    let again = i.index_root(root, &AtomicBool::new(false), &AtomicU64::new(0)).unwrap();
    assert_eq!((again.added, again.updated, again.removed), (0, 0, 0), "unchanged files are skipped");

    let report = d.path().join("docs/Quarterly Report.txt");
    fs::write(&report, "now about margins, a longer text than before").unwrap();
    fs::remove_file(d.path().join("docs/photo.bin")).unwrap();
    fs::write(d.path().join("docs/new.md"), "fresh").unwrap();
    let s = i.index_root(root, &AtomicBool::new(false), &AtomicU64::new(0)).unwrap();
    // Updated: the edited file, and `docs` itself when its mtime ticked.
    assert_eq!((s.added, s.removed), (1, 1));
    assert!((1..=2).contains(&s.updated), "{s:?}");
    assert!(i.search(&q("revenue", None, true)).unwrap().is_empty(), "old content gone");
    assert_eq!(i.search(&q("margins", None, true)).unwrap().len(), 1);
    assert!(i.search(&q("photo", None, false)).unwrap().is_empty());

    i.remove_root(root).unwrap();
    assert_eq!(i.file_count().unwrap(), 0);
    assert!(i.search(&q("margins", None, true)).unwrap().is_empty());
}

#[test]
fn cancel_stops_without_marking_indexed() {
    let d = tree();
    let i = SearchIndex::open_in_memory().unwrap();
    let root = d.path().to_str().unwrap();
    i.add_root(root, false).unwrap();
    let s = i.index_root(root, &AtomicBool::new(true), &AtomicU64::new(0)).unwrap();
    assert!(s.cancelled);
    assert!(i.roots().unwrap()[0].indexed_at.is_none());
    assert!(i.index_root("/not/a/root", &AtomicBool::new(false), &AtomicU64::new(0)).is_err());
}

#[test]
fn manager_runs_jobs_in_background() {
    let d = tree();
    let m = SearchManager::new(SearchIndex::open_in_memory().unwrap());
    let root = d.path().to_str().unwrap();
    m.index().add_root(root, false).unwrap();
    m.start(root);
    for _ in 0..200 {
        if !m.is_running(root) {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let job = m.jobs().into_iter().find(|j| j.root == root).unwrap();
    assert!(!job.running && job.error.is_none());
    assert_eq!(job.last.unwrap().added, 5);
}

#[test]
fn fts_query_quotes_terms() {
    assert_eq!(fts_query("hello  wor-ld").as_deref(), Some("\"hello\"* \"wor-ld\"*"));
    assert_eq!(fts_query(" !! "), None);
}

#[test]
fn remote_listings_are_searchable_and_replaced_per_folder() {
    let i = SearchIndex::open_in_memory().unwrap();
    let e = |name: &str, dir: bool| ListingEntry { name: name.into(), path: format!("/srv/{name}"), is_dir: dir, size: 7, modified_ms: 1_700_000_000_000 };
    i.record_listing("sftp", "/srv", &[e("backup-2026.tar", false), e("logs", true)]).unwrap();
    let hits = i.search(&q("backup", None, false)).unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!((hits[0].path.as_str(), hits[0].source.as_deref(), hits[0].modified), ("/srv/backup-2026.tar", Some("sftp"), 1_700_000_000));
    // Names-and-contents search finds remote names too.
    assert_eq!(i.search(&q("backup", None, true)).unwrap().len(), 1);
    // A new listing of the same folder replaces the old one.
    i.record_listing("sftp", "/srv/", &[e("logs", true)]).unwrap();
    assert!(i.search(&q("backup", None, false)).unwrap().is_empty());
    let root = i.roots().unwrap().into_iter().find(|r| r.path == "remote:sftp").unwrap();
    assert_eq!(root.files, 1);
    // Local-folder searches never include server entries.
    assert!(i.search(&q("logs", Some(Path::new("/srv")), false)).unwrap().is_empty());
    assert!(i.index_root("remote:sftp", &AtomicBool::new(false), &AtomicU64::new(0)).is_err());
    i.remove_root("remote:sftp").unwrap();
    assert_eq!(i.file_count().unwrap(), 0);
}
