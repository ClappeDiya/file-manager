//! Application logging: stdout plus daily rolling files in the app's log
//! folder, a level that can be changed at runtime, and retention by age
//! and total size.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use tracing_subscriber::{fmt, layer::SubscriberExt, reload, util::SubscriberInitExt, EnvFilter, Registry};

pub const FILE_PREFIX: &str = "ufop";
pub const FILE_SUFFIX: &str = "log";
/// Suffix of the machine-readable twin of each day's log: one JSON object per
/// line (`ufop.2026-10-24.jsonl`). The log viewer reads these; the
/// human-readable `.log` files stay exactly as they were.
pub const JSON_SUFFIX: &str = "jsonl";

/// Default retention: two weeks, at most 200 MB of logs.
pub const DEFAULT_MAX_AGE_DAYS: u64 = 14;
pub const DEFAULT_MAX_TOTAL_BYTES: u64 = 200 * 1024 * 1024;

/// User-configurable retention, saved as `retention.json` in the log folder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Retention {
    pub max_age_days: u64,
    pub max_total_mb: u64,
}

impl Default for Retention {
    fn default() -> Self {
        Self { max_age_days: DEFAULT_MAX_AGE_DAYS, max_total_mb: DEFAULT_MAX_TOTAL_BYTES / (1024 * 1024) }
    }
}

impl Retention {
    const FILE: &'static str = "retention.json";

    pub fn validate(&self) -> Result<(), String> {
        if !(1..=3650).contains(&self.max_age_days) {
            return Err("Keep logs for 1 to 3650 days".into());
        }
        if !(1..=100_000).contains(&self.max_total_mb) {
            return Err("Allow 1 MB to 100 GB of logs".into());
        }
        Ok(())
    }

    /// The saved retention, else the defaults.
    pub fn load(dir: &Path) -> Self {
        std::fs::read(dir.join(Self::FILE))
            .ok()
            .and_then(|b| serde_json::from_slice::<Self>(&b).ok())
            .filter(|r| r.validate().is_ok())
            .unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) -> Result<(), String> {
        self.validate()?;
        let bytes = serde_json::to_vec_pretty(self).map_err(|e| e.to_string())?;
        ufop_core::fsutil::write_private_atomic(&dir.join(Self::FILE), &bytes).map_err(|e| format!("Cannot save the log retention: {e}"))
    }

    /// Delete logs outside this retention; returns how many were removed.
    pub fn apply(&self, dir: &Path) -> usize {
        prune(dir, Duration::from_secs(self.max_age_days * 86_400), self.max_total_mb * 1024 * 1024)
    }
}

static RELOAD: OnceLock<reload::Handle<EnvFilter, Registry>> = OnceLock::new();
static GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();
static JSON_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

/// `<local data dir>/logs`.
pub fn log_dir() -> PathBuf {
    crate::paths::data_local_dir().join("logs")
}

/// Install the global subscriber (stdout + rolling file + a JSON-lines
/// twin of the file for the log viewer). Call once.
pub fn init(dir: &Path) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter, handle) = reload::Layer::new(filter);
    let rolling = |suffix: &str| {
        std::fs::create_dir_all(dir).ok().and_then(|_| {
            tracing_appender::rolling::Builder::new()
                .rotation(tracing_appender::rolling::Rotation::DAILY)
                .filename_prefix(FILE_PREFIX)
                .filename_suffix(suffix)
                .build(dir)
                .ok()
        })
    };
    let registry = tracing_subscriber::registry().with(filter).with(fmt::layer());
    let json_layer = rolling(JSON_SUFFIX).map(|appender| {
        let (writer, guard) = tracing_appender::non_blocking(appender);
        let _ = JSON_GUARD.set(guard);
        json_lines::JsonLinesLayer::new(writer)
    });
    let registry = registry.with(json_layer);
    let result = match rolling(FILE_SUFFIX) {
        Some(appender) => {
            let (writer, guard) = tracing_appender::non_blocking(appender);
            let _ = GUARD.set(guard);
            registry.with(fmt::layer().with_ansi(false).with_writer(writer)).try_init()
        }
        None => registry.try_init(),
    };
    if result.is_ok() {
        let _ = RELOAD.set(handle);
    }
}

/// Change the level (`error`…`trace`, or any EnvFilter directive).
pub fn set_level(directive: &str) -> Result<(), String> {
    let filter = EnvFilter::try_new(directive).map_err(|e| format!("Invalid log level: {e}"))?;
    // Without an initialised subscriber (unit tests, embedders) there is
    // nothing to reload; the directive is still validated.
    match RELOAD.get() {
        Some(handle) => handle.reload(filter).map_err(|e| e.to_string()),
        None => Ok(()),
    }
}

/// The human-readable log files in `dir`, newest first: (path, size, modified).
pub fn log_files(dir: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    files_with_suffix(dir, FILE_SUFFIX)
}

/// The JSON-lines twins of the daily logs in `dir`, newest first.
pub fn json_log_files(dir: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    files_with_suffix(dir, JSON_SUFFIX)
}

fn files_with_suffix(dir: &Path, suffix: &str) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            // `ufop.<date>.log` (as before: anything containing `.log`) or `ufop.<date>.jsonl`.
            n.starts_with(FILE_PREFIX) && if suffix == JSON_SUFFIX { n.ends_with(&format!(".{JSON_SUFFIX}")) } else { n.contains(&format!(".{suffix}")) }
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (e.path(), m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.2));
    v
}

/// Remove log files (both kinds) older than `max_age` and, oldest first,
/// beyond `max_total` bytes. The newest file of each kind is always kept.
/// Returns the removed count.
pub fn prune(dir: &Path, max_age: Duration, max_total: u64) -> usize {
    let now = SystemTime::now();
    let mut all: Vec<(PathBuf, u64, SystemTime, bool)> = log_files(dir).into_iter().map(|(p, l, m)| (p, l, m, true)).collect();
    all.extend(json_log_files(dir).into_iter().map(|(p, l, m)| (p, l, m, false)));
    all.sort_by_key(|e| std::cmp::Reverse(e.2));
    let (mut kept_human, mut kept_json) = (false, false);
    let mut total = 0u64;
    let mut removed = 0;
    for (p, len, modified, human) in all {
        let newest_of_kind = if human { !std::mem::replace(&mut kept_human, true) } else { !std::mem::replace(&mut kept_json, true) };
        let too_old = now.duration_since(modified).is_ok_and(|age| age > max_age);
        if !newest_of_kind && (too_old || total + len > max_total) {
            if std::fs::remove_file(&p).is_ok() {
                removed += 1;
            }
            continue;
        }
        total += len;
    }
    removed
}

/// One JSON object per event, written to its own rolling file.
pub mod json_lines {
    use serde_json::{json, Map, Value};
    use std::io::Write;
    use tracing::field::{Field, Visit};
    use tracing::{Event, Subscriber};
    use tracing_subscriber::fmt::MakeWriter;
    use tracing_subscriber::layer::Context;
    use tracing_subscriber::registry::LookupSpan;
    use tracing_subscriber::Layer;

    /// Writes every event as a line:
    /// `{"ts":"2026-10-24T19:11:47.284071Z","level":"INFO","target":"…","message":"…","fields":{…},"spans":["a","b"]}`.
    /// A line is one `write_all`, so lines from different threads never mix.
    pub struct JsonLinesLayer<W> {
        writer: W,
    }

    impl<W> JsonLinesLayer<W> {
        pub fn new(writer: W) -> Self {
            Self { writer }
        }
    }

    #[derive(Default)]
    struct Collect {
        message: Option<String>,
        fields: Map<String, Value>,
    }

    impl Collect {
        fn put(&mut self, field: &Field, value: Value) {
            if field.name() == "message" {
                self.message = Some(match value {
                    Value::String(s) => s,
                    other => other.to_string(),
                });
            } else {
                self.fields.insert(field.name().to_string(), value);
            }
        }
    }

    impl Visit for Collect {
        fn record_str(&mut self, field: &Field, value: &str) {
            self.put(field, Value::String(value.to_string()));
        }
        fn record_i64(&mut self, field: &Field, value: i64) {
            self.put(field, json!(value));
        }
        fn record_u64(&mut self, field: &Field, value: u64) {
            self.put(field, json!(value));
        }
        fn record_bool(&mut self, field: &Field, value: bool) {
            self.put(field, json!(value));
        }
        fn record_f64(&mut self, field: &Field, value: f64) {
            self.put(field, json!(value));
        }
        fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
            self.put(field, Value::String(format!("{value:?}")));
        }
    }

    impl<S, W> Layer<S> for JsonLinesLayer<W>
    where
        S: Subscriber + for<'a> LookupSpan<'a>,
        W: for<'a> MakeWriter<'a> + 'static,
    {
        fn on_event(&self, event: &Event<'_>, ctx: Context<'_, S>) {
            let meta = event.metadata();
            let mut seen = Collect::default();
            event.record(&mut seen);
            let spans: Vec<&str> = ctx.event_scope(event).map(|scope| scope.from_root().map(|s| s.name()).collect()).unwrap_or_default();
            let mut line = json!({
                "ts": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
                "level": meta.level().as_str(),
                "target": meta.target(),
                "message": seen.message.unwrap_or_default(),
            });
            if let Some(o) = line.as_object_mut() {
                if !seen.fields.is_empty() {
                    o.insert("fields".into(), Value::Object(seen.fields));
                }
                if !spans.is_empty() {
                    o.insert("spans".into(), json!(spans));
                }
            }
            let mut bytes = line.to_string().into_bytes();
            bytes.push(b'\n');
            let mut out = self.writer.make_writer();
            let _ = out.write_all(&bytes);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(dir: &Path, name: &str, bytes: usize, age_days: u64) {
        let p = dir.join(name);
        std::fs::write(&p, vec![b'x'; bytes]).unwrap();
        let t = SystemTime::now() - Duration::from_secs(age_days * 86_400 + 60);
        std::fs::File::options().write(true).open(&p).unwrap().set_modified(t).unwrap();
    }

    #[test]
    fn prunes_by_age_and_size_keeping_the_newest() {
        let d = tempfile::tempdir().unwrap();
        file(d.path(), "ufop.2026-10-22.log", 100, 0);
        file(d.path(), "ufop.2026-10-21.log", 100, 1);
        file(d.path(), "ufop.2026-10-20.log", 100, 2);
        file(d.path(), "ufop.2026-09-01.log", 10, 50);
        file(d.path(), "other.txt", 10, 50);
        // Age: the 50-day-old file goes; size: only 250 bytes may stay.
        assert_eq!(prune(d.path(), Duration::from_secs(14 * 86_400), 250), 2);
        let left: Vec<String> = log_files(d.path()).iter().map(|(p, ..)| p.file_name().unwrap().to_string_lossy().into()).collect();
        assert_eq!(left, vec!["ufop.2026-10-22.log", "ufop.2026-10-21.log"]);
        assert!(d.path().join("other.txt").exists(), "only our logs are touched");
        // The newest file survives even past the size cap.
        assert_eq!(prune(d.path(), Duration::from_secs(0), 0), 1);
        assert_eq!(log_files(d.path()).len(), 1);
    }

    #[test]
    fn retention_round_trips_and_rejects_bad_values() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(Retention::load(d.path()), Retention::default());
        let r = Retention { max_age_days: 3, max_total_mb: 5 };
        r.save(d.path()).unwrap();
        assert_eq!(Retention::load(d.path()), r);
        assert!(Retention { max_age_days: 0, max_total_mb: 5 }.save(d.path()).is_err());
        assert!(Retention { max_age_days: 3, max_total_mb: 0 }.validate().is_err());
        // A hand-edited invalid file falls back to the defaults.
        std::fs::write(d.path().join("retention.json"), br#"{"max_age_days":0,"max_total_mb":1}"#).unwrap();
        assert_eq!(Retention::load(d.path()), Retention::default());
        file(d.path(), "ufop.2026-10-22.log", 10, 0);
        file(d.path(), "ufop.2026-10-01.log", 10, 21);
        assert_eq!(Retention { max_age_days: 14, max_total_mb: 1 }.apply(d.path()), 1);
    }

    #[test]
    fn level_directives_are_validated() {
        assert!(set_level("not a [valid directive").is_err());
    }

    /// A writer tests can read back.
    #[derive(Clone, Default)]
    struct Sink(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
    impl std::io::Write for Sink {
        fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Sink {
        type Writer = Sink;
        fn make_writer(&'a self) -> Sink {
            self.clone()
        }
    }

    #[test]
    fn the_json_sink_writes_one_parsable_line_per_event() {
        let sink = Sink::default();
        let subscriber = tracing_subscriber::registry().with(json_lines::JsonLinesLayer::new(sink.clone()));
        tracing::subscriber::with_default(subscriber, || {
            {
                let span = tracing::info_span!("backup.operation", job = "Docs");
                let _in = span.enter();
                tracing::warn!(files = 3u64, ok = true, path = "C:\\Users\\me", "disk \"almost\" full\nsecond line");
            }
            tracing::error!("plain");
        });
        let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
        let lines: Vec<serde_json::Value> = text.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2, "a multi-line message is still one line: {text}");
        assert_eq!(lines[0]["level"], "WARN");
        assert_eq!(lines[0]["message"], "disk \"almost\" full\nsecond line");
        assert_eq!(lines[0]["fields"]["files"], 3);
        assert_eq!(lines[0]["fields"]["ok"], true);
        assert_eq!(lines[0]["fields"]["path"], "C:\\Users\\me");
        assert_eq!(lines[0]["spans"], serde_json::json!(["backup.operation"]));
        assert!(lines[0]["target"].as_str().unwrap().contains("logging"));
        assert!(chrono::DateTime::parse_from_rfc3339(lines[0]["ts"].as_str().unwrap()).is_ok());
        assert_eq!((lines[1]["level"].as_str(), lines[1]["message"].as_str()), (Some("ERROR"), Some("plain")));
        assert!(lines[1].get("fields").is_none() && lines[1].get("spans").is_none());
    }

    #[test]
    fn both_kinds_of_log_are_pruned_and_the_newest_of_each_kept() {
        let d = tempfile::tempdir().unwrap();
        file(d.path(), "ufop.2026-10-22.log", 100, 0);
        file(d.path(), "ufop.2026-10-22.jsonl", 100, 0);
        file(d.path(), "ufop.2026-10-01.log", 100, 21);
        file(d.path(), "ufop.2026-10-01.jsonl", 100, 21);
        file(d.path(), "ufop.2026-09-01.jsonl", 100, 50);
        assert_eq!(log_files(d.path()).len(), 2, "the human list does not include the JSON files");
        assert_eq!(json_log_files(d.path()).len(), 3);
        // Two weeks: the three old files go; each kind keeps its newest.
        assert_eq!(prune(d.path(), Duration::from_secs(14 * 86_400), u64::MAX), 3);
        assert_eq!(log_files(d.path()).len(), 1);
        assert_eq!(json_log_files(d.path()).len(), 1);
        // Nothing allowed: still the newest of each kind.
        assert_eq!(prune(d.path(), Duration::from_secs(0), 0), 0);
    }
}
