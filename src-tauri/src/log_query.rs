//! Reads the app's own log files for the in-app log viewer (Advanced).
//!
//! The viewer prefers the JSON-lines twin of each day's log
//! (`ufop.<date>.jsonl`, written by [`crate::logging::json_lines`]), whose
//! lines parse exactly. A day that only has the human-readable `ufop.<date>.log`
//! (written before the twin existed) is parsed as well as that format allows:
//! timestamp, level, optional span scope, target and message, with indented or
//! otherwise timestamp-less lines joined to the record above them.
//!
//! Records are read newest first, from the end of each file, so a page near
//! the end of a very large log costs a few blocks, not the whole file; the
//! `cursor` of a page is the id (`<date>@<byte offset>`) of its oldest record
//! and the next call continues just before it.

use crate::core::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub const DEFAULT_LIMIT: usize = 500;
pub const MAX_LIMIT: usize = 5_000;
/// Most bytes one call reads, so a filter that matches nothing cannot read gigabytes.
pub const SCAN_BUDGET: u64 = 64 * 1024 * 1024;
const BLOCK: usize = 64 * 1024;
/// Distinct modules reported for the viewer's source picker.
const MAX_SOURCES: usize = 200;

/// What the viewer asks for. Every filter is optional; they combine with AND.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct LogQuery {
    /// The lowest level to show: `error`, `warn`, `info`, `debug` or `trace`.
    pub level: Option<String>,
    /// Text the record's target (module path) contains, ignoring case.
    pub source: Option<String>,
    /// RFC 3339 instant; records from then on (inclusive).
    pub since: Option<String>,
    /// RFC 3339 instant; records up to then (inclusive).
    pub until: Option<String>,
    /// Text the message or its fields contain, ignoring case.
    pub text: Option<String>,
    /// Records per page (default 500, at most 5000).
    pub limit: Option<usize>,
    /// The `next_cursor` of the previous page: continue with older records.
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LogRecord {
    /// `<date>@<byte offset>`: unique and stable while the file is kept.
    pub id: String,
    pub ts: String,
    /// `ERROR`, `WARN`, `INFO`, `DEBUG` or `TRACE`.
    pub level: String,
    pub target: String,
    pub message: String,
    pub fields: Option<serde_json::Value>,
    /// Names of the spans the event happened in, outermost first.
    pub spans: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LogPage {
    /// Matching records, newest first.
    pub records: Vec<LogRecord>,
    /// Pass as `cursor` for older records; `None` when there are none.
    pub next_cursor: Option<String>,
    /// Modules seen while reading (`crate::module`), for a source picker.
    pub sources: Vec<String>,
    /// Reading stopped at the per-call byte budget before the page was full.
    pub scan_limited: bool,
    /// The folder the logs live in.
    pub directory: String,
}

fn rank(level: &str) -> Option<u8> {
    match level.trim().to_ascii_uppercase().as_str() {
        "TRACE" => Some(0),
        "DEBUG" => Some(1),
        "INFO" => Some(2),
        "WARN" | "WARNING" => Some(3),
        "ERROR" => Some(4),
        _ => None,
    }
}

fn parse_instant(label: &str, v: &Option<String>) -> Result<Option<DateTime<Utc>>, AppError> {
    crate::core::date_range::parse_bound(label, v.as_deref())
}

/// One day's log: the JSON twin when there is one, else the human log.
struct DayFile {
    date: String,
    path: PathBuf,
    json: bool,
}

fn day_of(path: &Path) -> String {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = name.strip_prefix(&format!("{}.", crate::logging::FILE_PREFIX)).unwrap_or(&name);
    stem.split('.').next().unwrap_or(stem).to_string()
}

/// The days that have a log, newest first.
fn day_files(dir: &Path) -> Vec<DayFile> {
    let mut by_day: std::collections::BTreeMap<String, DayFile> = std::collections::BTreeMap::new();
    for (path, ..) in crate::logging::log_files(dir) {
        let date = day_of(&path);
        by_day.insert(date.clone(), DayFile { date, path, json: false });
    }
    for (path, ..) in crate::logging::json_log_files(dir) {
        let date = day_of(&path);
        by_day.insert(date.clone(), DayFile { date, path, json: true });
    }
    let mut v: Vec<DayFile> = by_day.into_values().collect();
    v.sort_by(|a, b| b.date.cmp(&a.date));
    v
}

/// The lines of a file from the last to the first, each with its byte offset.
struct RevLines {
    file: std::fs::File,
    chunk: Vec<u8>,
    chunk_start: u64,
    block: usize,
    bytes_read: u64,
}

impl RevLines {
    fn open(path: &Path, end: Option<u64>, block: usize) -> std::io::Result<Self> {
        let file = std::fs::File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, chunk: Vec::new(), chunk_start: end.map_or(len, |e| e.min(len)), block: block.max(16), bytes_read: 0 })
    }

    fn next_line(&mut self) -> std::io::Result<Option<(u64, Vec<u8>)>> {
        loop {
            if let Some(i) = self.chunk.iter().rposition(|b| *b == b'\n') {
                let line = self.chunk.split_off(i + 1);
                let offset = self.chunk_start + i as u64 + 1;
                self.chunk.pop();
                if line.is_empty() {
                    continue;
                }
                return Ok(Some((offset, line)));
            }
            if self.chunk_start == 0 {
                return Ok(if self.chunk.is_empty() { None } else { Some((0, std::mem::take(&mut self.chunk))) });
            }
            let take = (self.block as u64).min(self.chunk_start);
            let start = self.chunk_start - take;
            let mut buf = vec![0u8; take as usize];
            self.file.seek(SeekFrom::Start(start))?;
            self.file.read_exact(&mut buf)?;
            self.bytes_read += take;
            buf.extend_from_slice(&self.chunk);
            self.chunk = buf;
            self.chunk_start = start;
        }
    }
}

fn parse_json_line(date: &str, offset: u64, line: &[u8]) -> Option<LogRecord> {
    let v: serde_json::Value = serde_json::from_slice(line).ok()?;
    let text = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or_default().to_string();
    let level = text("level").to_ascii_uppercase();
    rank(&level)?;
    Some(LogRecord {
        id: format!("{date}@{offset}"),
        ts: text("ts"),
        level,
        target: text("target"),
        message: text("message"),
        fields: v.get("fields").filter(|f| f.as_object().is_some_and(|o| !o.is_empty())).cloned(),
        spans: v.get("spans").and_then(|s| s.as_array()).map(|a| a.iter().filter_map(|n| n.as_str().map(String::from)).collect()).unwrap_or_default(),
    })
}

/// The header of a human-format record, or `None` for a continuation line.
fn parse_human_header(date: &str, offset: u64, line: &str) -> Option<LogRecord> {
    let (ts, rest) = line.split_once(char::is_whitespace)?;
    DateTime::parse_from_rfc3339(ts).ok()?;
    // The level is right-aligned with spaces (`  INFO`).
    let rest = rest.trim_start();
    let (level, tail) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let tail = tail.trim_start();
    rank(level)?;
    // `span{fields}:span{fields}: target: message`, or `target: message`.
    let (scope, body) = match tail.split_once(": ") {
        Some((first, more)) if first.contains('{') => (Some(first), more),
        _ => (None, tail),
    };
    let (target, message) = body.split_once(": ").map(|(t, m)| (t.trim(), m)).unwrap_or(("", body));
    let spans = scope
        .map(|s| s.split(':').filter_map(|part| part.split('{').next()).filter(|n| !n.is_empty()).map(String::from).collect())
        .unwrap_or_default();
    Some(LogRecord { id: format!("{date}@{offset}"), ts: ts.to_string(), level: level.to_ascii_uppercase(), target: target.to_string(), message: message.to_string(), fields: None, spans })
}

/// Records of one file, newest first, joining timestamp-less lines of the
/// human format to the record above them.
struct RecordsRev<'a> {
    lines: RevLines,
    day: &'a DayFile,
    pending: Vec<String>,
}

impl RecordsRev<'_> {
    fn next_record(&mut self) -> std::io::Result<Option<LogRecord>> {
        while let Some((offset, raw)) = self.lines.next_line()? {
            let line = String::from_utf8_lossy(&raw);
            let line = line.trim_end_matches('\r');
            if self.day.json {
                if let Some(r) = parse_json_line(&self.day.date, offset, line.as_bytes()) {
                    return Ok(Some(r));
                }
                continue;
            }
            match parse_human_header(&self.day.date, offset, line) {
                Some(mut r) => {
                    for extra in self.pending.drain(..).rev() {
                        r.message.push('\n');
                        r.message.push_str(&extra);
                    }
                    return Ok(Some(r));
                }
                None => self.pending.push(line.to_string()),
            }
        }
        // Lines at the top of the file with no header above them are not a record.
        self.pending.clear();
        Ok(None)
    }
}

fn module_of(target: &str) -> String {
    target.split("::").take(2).collect::<Vec<_>>().join("::")
}

struct Filters {
    min: Option<u8>,
    source: Option<String>,
    since: Option<DateTime<Utc>>,
    until: Option<DateTime<Utc>>,
    text: Option<String>,
}

impl Filters {
    fn from(q: &LogQuery) -> Result<Self, AppError> {
        let min = match q.level.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            Some(l) => Some(rank(l).ok_or_else(|| AppError::validation(format!("\"{l}\" is not a log level; use error, warn, info, debug or trace")))?),
            None => None,
        };
        let lower = |s: &Option<String>| s.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_lowercase);
        Ok(Self { min, source: lower(&q.source), since: parse_instant("start", &q.since)?, until: parse_instant("end", &q.until)?, text: lower(&q.text) })
    }

    fn matches(&self, r: &LogRecord) -> bool {
        if self.min.is_some_and(|m| rank(&r.level).unwrap_or(0) < m) {
            return false;
        }
        if self.source.as_ref().is_some_and(|s| !r.target.to_lowercase().contains(s)) {
            return false;
        }
        if let Some(t) = &self.text {
            let in_message = r.message.to_lowercase().contains(t);
            let in_fields = r.fields.as_ref().is_some_and(|f| f.to_string().to_lowercase().contains(t));
            if !in_message && !in_fields {
                return false;
            }
        }
        true
    }
}

fn split_cursor(cursor: &str) -> Result<(String, u64), AppError> {
    cursor
        .split_once('@')
        .and_then(|(d, o)| Some((d.to_string(), o.parse().ok()?)))
        .ok_or_else(|| AppError::validation("That log position is not valid; reload the log"))
}

/// One page of matching records, newest first.
pub fn query(dir: &Path, q: &LogQuery) -> Result<LogPage, AppError> {
    query_with(dir, q, BLOCK, SCAN_BUDGET)
}

pub(crate) fn query_with(dir: &Path, q: &LogQuery, block: usize, budget: u64) -> Result<LogPage, AppError> {
    let filters = Filters::from(q)?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    let resume = q.cursor.as_deref().filter(|c| !c.is_empty()).map(split_cursor).transpose()?;
    let mut page = LogPage { directory: dir.to_string_lossy().into_owned(), ..Default::default() };
    let mut sources: BTreeSet<String> = BTreeSet::new();
    let mut read: u64 = 0;

    'days: for day in day_files(dir) {
        // Days newer than the cursor's were already shown.
        let end = match &resume {
            Some((d, _)) if day.date.as_str() > d.as_str() => continue,
            Some((d, off)) if day.date == *d => Some(*off),
            _ => None,
        };
        let Ok(lines) = RevLines::open(&day.path, end, block) else { continue };
        let mut records = RecordsRev { lines, day: &day, pending: Vec::new() };
        loop {
            let before = records.lines.bytes_read;
            let next = records.next_record().map_err(|e| AppError::internal(format!("Could not read the log: {e}")))?;
            read += records.lines.bytes_read - before;
            let Some(r) = next else { continue 'days };
            if sources.len() < MAX_SOURCES {
                sources.insert(module_of(&r.target));
            }
            let scanned_id = r.id.clone();
            let ts = DateTime::parse_from_rfc3339(&r.ts).ok().map(|t| t.with_timezone(&Utc));
            // Newest first: once a record is older than `since`, so is everything after it.
            if let (Some(since), Some(ts)) = (filters.since, ts) {
                if ts < since {
                    break 'days;
                }
            }
            let after_until = matches!((filters.until, ts), (Some(u), Some(t)) if t > u);
            if !after_until && filters.matches(&r) {
                let id = r.id.clone();
                page.records.push(r);
                if page.records.len() >= limit {
                    page.next_cursor = Some(id);
                    break 'days;
                }
            }
            if read >= budget {
                page.scan_limited = true;
                page.next_cursor = Some(scanned_id);
                break 'days;
            }
        }
    }
    page.sources = sources.into_iter().collect();
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn json_line(ts: &str, level: &str, target: &str, msg: &str) -> String {
        serde_json::json!({ "ts": ts, "level": level, "target": target, "message": msg }).to_string()
    }

    fn write(dir: &Path, name: &str, lines: &[String]) {
        std::fs::write(dir.join(name), lines.join("\n") + "\n").unwrap();
    }

    fn q() -> LogQuery {
        LogQuery::default()
    }

    /// Two days of JSON logs: day 1 has 5 records, day 2 has 5.
    fn two_days() -> tempfile::TempDir {
        let d = tempfile::tempdir().unwrap();
        let day = |date: &str, base: usize| -> Vec<String> {
            (0..5)
                .map(|i| {
                    let (lvl, target) = [("INFO", "app::sync_engine"), ("WARN", "app::backup_engine"), ("ERROR", "app::sync_engine"), ("DEBUG", "app::storage"), ("INFO", "app::backup_engine")][i];
                    json_line(&format!("{date}T10:00:0{i}.000000Z"), lvl, target, &format!("event {}", base + i))
                })
                .collect()
        };
        write(d.path(), "ufop.2026-10-23.jsonl", &day("2026-10-23", 0));
        write(d.path(), "ufop.2026-10-24.jsonl", &day("2026-10-24", 5));
        d
    }

    fn msgs(p: &LogPage) -> Vec<&str> {
        p.records.iter().map(|r| r.message.as_str()).collect()
    }

    #[test]
    fn returns_newest_first_across_days() {
        let d = two_days();
        let page = query(d.path(), &q()).unwrap();
        assert_eq!(msgs(&page), ["event 9", "event 8", "event 7", "event 6", "event 5", "event 4", "event 3", "event 2", "event 1", "event 0"]);
        assert!(page.next_cursor.is_none(), "everything fit");
        assert_eq!(page.directory, d.path().to_string_lossy());
    }

    #[test]
    fn pages_with_a_cursor_without_gaps_or_repeats() {
        let d = two_days();
        let mut seen = Vec::new();
        let mut cursor = None;
        for _ in 0..10 {
            let page = query(d.path(), &LogQuery { limit: Some(3), cursor: cursor.clone(), ..q() }).unwrap();
            seen.extend(page.records.iter().map(|r| r.message.clone()));
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        assert_eq!(seen, (0..10).rev().map(|i| format!("event {i}")).collect::<Vec<_>>());
    }

    #[test]
    fn reads_across_many_blocks_and_lines_longer_than_a_block() {
        let d = tempfile::tempdir().unwrap();
        let mut lines: Vec<String> = (0..300).map(|i| json_line(&format!("2026-10-24T10:{:02}:{:02}.000000Z", i / 60, i % 60), "INFO", "app::x", &format!("m{i}"))).collect();
        lines[150] = json_line("2026-10-24T10:02:30.000000Z", "INFO", "app::x", &"long ".repeat(400));
        write(d.path(), "ufop.2026-10-24.jsonl", &lines);
        // A 64-byte block forces every line across several reads.
        let page = query_with(d.path(), &LogQuery { limit: Some(1000), ..q() }, 64, SCAN_BUDGET).unwrap();
        assert_eq!(page.records.len(), 300);
        assert_eq!(page.records[0].message, "m299");
        assert_eq!(page.records[299].message, "m0");
        assert!(page.records[149].message.starts_with("long long"));
    }

    #[test]
    fn filters_by_level_source_text_and_time() {
        let d = two_days();
        let by = |f: LogQuery| msgs(&query(d.path(), &f).unwrap()).into_iter().map(String::from).collect::<Vec<_>>();
        // At least WARN.
        assert_eq!(by(LogQuery { level: Some("warn".into()), limit: Some(4), ..q() }), ["event 7", "event 6", "event 2", "event 1"]);
        // A module, any case.
        assert_eq!(by(LogQuery { source: Some("STORAGE".into()), ..q() }), ["event 8", "event 3"]);
        // Text in the message.
        assert_eq!(by(LogQuery { text: Some("event 5".into()), ..q() }), ["event 5"]);
        // A window, both ends included.
        assert_eq!(by(LogQuery { since: Some("2026-10-23T10:00:03Z".into()), until: Some("2026-10-24T10:00:01Z".into()), ..q() }), ["event 6", "event 5", "event 4", "event 3"]);
        // Everything together.
        assert_eq!(by(LogQuery { level: Some("error".into()), source: Some("sync".into()), ..q() }), ["event 7", "event 2"]);
    }

    #[test]
    fn text_also_searches_the_fields() {
        let d = tempfile::tempdir().unwrap();
        let with_fields = serde_json::json!({ "ts": "2026-10-24T10:00:00.000000Z", "level": "INFO", "target": "app::x", "message": "done", "fields": { "job": "Nightly Docs" }, "spans": ["backup.operation"] }).to_string();
        write(d.path(), "ufop.2026-10-24.jsonl", &[with_fields, json_line("2026-10-24T10:00:01.000000Z", "INFO", "app::x", "other")]);
        let page = query(d.path(), &LogQuery { text: Some("nightly".into()), ..q() }).unwrap();
        assert_eq!(msgs(&page), ["done"]);
        assert_eq!(page.records[0].spans, ["backup.operation"]);
        assert_eq!(page.records[0].fields.as_ref().unwrap()["job"], "Nightly Docs");
    }

    #[test]
    fn since_stops_early_and_is_inclusive_for_live_tailing() {
        let d = two_days();
        let first = query(d.path(), &LogQuery { limit: Some(2), ..q() }).unwrap();
        let newest_ts = first.records[0].ts.clone();
        // A tail asks again from the newest record it has: it gets it back (the viewer
        // drops repeats by id) and anything newer.
        let tail = query(d.path(), &LogQuery { since: Some(newest_ts), ..q() }).unwrap();
        assert_eq!(msgs(&tail), ["event 9"]);
        assert_eq!(tail.records[0].id, first.records[0].id, "ids are stable between calls");
        std::fs::OpenOptions::new().append(true).open(d.path().join("ufop.2026-10-24.jsonl")).unwrap().write_all((json_line("2026-10-24T10:00:09.000000Z", "INFO", "app::x", "event 10") + "\n").as_bytes()).unwrap();
        let tail = query(d.path(), &LogQuery { since: Some(first.records[0].ts.clone()), ..q() }).unwrap();
        assert_eq!(msgs(&tail), ["event 10", "event 9"]);
    }

    #[test]
    fn a_torn_last_line_is_skipped_not_fatal() {
        let d = two_days();
        std::fs::OpenOptions::new().append(true).open(d.path().join("ufop.2026-10-24.jsonl")).unwrap().write_all(br#"{"ts":"2026-10-24T10:00:09"#).unwrap();
        let page = query(d.path(), &LogQuery { limit: Some(1), ..q() }).unwrap();
        assert_eq!(msgs(&page), ["event 9"]);
    }

    #[test]
    fn human_logs_parse_with_spans_and_continuation_lines() {
        let d = tempfile::tempdir().unwrap();
        let text = [
            "2026-10-24T10:00:00.000001Z  INFO unified_file_ops_lib::sync_engine: Scheduled sync Photos finished",
            "2026-10-24T10:00:01.000002Z  WARN backup.operation{job.id=abc}:inner{n=1}: unified_file_ops_lib::backup_engine: disk almost full",
            "  second line of the warning",
            "\tthird line",
            "2026-10-24T10:00:02.000003Z ERROR unified_file_ops_lib::storage: pool closed",
            "",
        ]
        .join("\n");
        std::fs::write(d.path().join("ufop.2026-10-24.log"), text).unwrap();
        let page = query(d.path(), &q()).unwrap();
        assert_eq!(page.records.len(), 3);
        assert_eq!((page.records[0].level.as_str(), page.records[0].target.as_str(), page.records[0].message.as_str()), ("ERROR", "unified_file_ops_lib::storage", "pool closed"));
        let warn = &page.records[1];
        assert_eq!((warn.level.as_str(), warn.target.as_str()), ("WARN", "unified_file_ops_lib::backup_engine"));
        assert_eq!(warn.message, "disk almost full\n  second line of the warning\n\tthird line");
        assert_eq!(warn.spans, ["backup.operation", "inner"]);
        assert_eq!(page.records[2].message, "Scheduled sync Photos finished");
        assert!(page.sources.contains(&"unified_file_ops_lib::sync_engine".to_string()));
    }

    #[test]
    fn a_days_json_twin_wins_over_its_human_log_and_older_days_fall_back() {
        let d = tempfile::tempdir().unwrap();
        write(d.path(), "ufop.2026-10-24.jsonl", &[json_line("2026-10-24T10:00:00.000000Z", "INFO", "app::x", "from json")]);
        std::fs::write(d.path().join("ufop.2026-10-24.log"), "2026-10-24T10:00:00.000000Z  INFO app::x: from text\n").unwrap();
        std::fs::write(d.path().join("ufop.2026-10-23.log"), "2026-10-23T10:00:00.000000Z  INFO app::x: yesterday\n").unwrap();
        assert_eq!(msgs(&query(d.path(), &q()).unwrap()), ["from json", "yesterday"]);
    }

    #[test]
    fn bad_input_is_refused_with_a_reason_and_an_empty_folder_is_empty() {
        let d = two_days();
        assert!(query(d.path(), &LogQuery { level: Some("loud".into()), ..q() }).unwrap_err().user_message().contains("not a log level"));
        assert!(query(d.path(), &LogQuery { since: Some("yesterday".into()), ..q() }).is_err());
        assert!(query(d.path(), &LogQuery { cursor: Some("garbage".into()), ..q() }).is_err());
        let empty = tempfile::tempdir().unwrap();
        let page = query(empty.path(), &q()).unwrap();
        assert!(page.records.is_empty() && page.next_cursor.is_none());
        assert!(query(&empty.path().join("missing"), &q()).unwrap().records.is_empty());
    }

    #[test]
    fn a_filter_that_matches_nothing_stops_at_the_byte_budget_and_can_continue() {
        let d = tempfile::tempdir().unwrap();
        let lines: Vec<String> = (0..2000).map(|i| json_line(&format!("2026-10-24T10:00:00.{i:06}Z"), "INFO", "app::x", &format!("filler {i}"))).collect();
        write(d.path(), "ufop.2026-10-24.jsonl", &lines);
        let f = LogQuery { text: Some("filler 3".into()), limit: Some(1), ..q() };
        // 4 KiB of budget with 1 KiB blocks cannot reach the first match from the end.
        let page = query_with(d.path(), &f, 1024, 4096).unwrap();
        assert!(page.scan_limited && page.records.is_empty());
        let mut cursor = page.next_cursor.clone().expect("a place to continue from");
        let mut found = None;
        for _ in 0..200 {
            let next = query_with(d.path(), &LogQuery { cursor: Some(cursor.clone()), ..f.clone() }, 1024, 4096).unwrap();
            if let Some(r) = next.records.first() {
                found = Some(r.message.clone());
                break;
            }
            match next.next_cursor {
                Some(c) => cursor = c,
                None => break,
            }
        }
        assert!(found.is_some_and(|m| m.contains("filler 3")), "continuing from the cursor reaches the match");
    }

}
