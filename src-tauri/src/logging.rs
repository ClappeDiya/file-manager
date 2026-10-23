//! Application logging: stdout plus daily rolling files in the app's log
//! folder, a level that can be changed at runtime, and retention by age
//! and total size.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, SystemTime};
use tracing_subscriber::{fmt, layer::SubscriberExt, reload, util::SubscriberInitExt, EnvFilter, Registry};

pub const FILE_PREFIX: &str = "ufop";
pub const FILE_SUFFIX: &str = "log";

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

/// `<local data dir>/logs`.
pub fn log_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "ufop", "unified-file-ops")
        .map(|d| d.data_local_dir().join("logs"))
        .unwrap_or_else(|| PathBuf::from("./logs"))
}

/// Install the global subscriber (stdout + rolling file). Call once.
pub fn init(dir: &Path) {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let (filter, handle) = reload::Layer::new(filter);
    let file_layer = std::fs::create_dir_all(dir).ok().and_then(|_| {
        tracing_appender::rolling::Builder::new()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix(FILE_PREFIX)
            .filename_suffix(FILE_SUFFIX)
            .build(dir)
            .ok()
    });
    let registry = tracing_subscriber::registry().with(filter).with(fmt::layer());
    let result = match file_layer {
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

/// Log files in `dir`, newest first: (path, size, modified).
pub fn log_files(dir: &Path) -> Vec<(PathBuf, u64, SystemTime)> {
    let mut v: Vec<_> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_string();
            n.starts_with(FILE_PREFIX) && n.contains(&format!(".{FILE_SUFFIX}"))
        })
        .filter_map(|e| {
            let m = e.metadata().ok()?;
            m.is_file().then(|| (e.path(), m.len(), m.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect();
    v.sort_by_key(|e| std::cmp::Reverse(e.2));
    v
}

/// Remove log files older than `max_age` and, oldest first, beyond
/// `max_total` bytes. The newest file is always kept. Returns removed count.
pub fn prune(dir: &Path, max_age: Duration, max_total: u64) -> usize {
    let now = SystemTime::now();
    let mut total = 0u64;
    let mut removed = 0;
    for (i, (p, len, modified)) in log_files(dir).into_iter().enumerate() {
        let too_old = now.duration_since(modified).is_ok_and(|age| age > max_age);
        if i > 0 && (too_old || total + len > max_total) {
            if std::fs::remove_file(&p).is_ok() {
                removed += 1;
            }
            continue;
        }
        total += len;
    }
    removed
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
}
