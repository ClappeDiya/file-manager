//! Choosing a version to restore, read or mount: by number, newest, oldest,
//! "as it was at" a point in time, or by the version's timestamp / file-list
//! name. [`VersionSelector::resolve`] maps a selector to a version number
//! (0 = newest) in a list of versions ordered newest first, the order
//! [`super::db::BackupDb::filesets`] returns them in.
//!
//! Partial versions (interrupted runs and checkpoints) are skipped by
//! `latest`, `oldest` and `as_of` unless `allow_partial` is set, so "restore
//! as of 9:00" never silently picks a half-finished run.

use super::Fileset;
use crate::error::AppError;
use chrono::{DateTime, Local, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum VersionSelector {
    /// Version number, 0 = newest (partial versions count).
    Index { index: usize },
    /// The newest complete version.
    Latest {
        #[serde(default)]
        allow_partial: bool,
    },
    /// The oldest complete version.
    Oldest {
        #[serde(default)]
        allow_partial: bool,
    },
    /// The newest complete version made at or before `time`.
    AsOf {
        time: DateTime<Utc>,
        #[serde(default)]
        allow_partial: bool,
    },
    /// A version by its timestamp (RFC 3339, any offset) or file-list name.
    Id { id: String },
}

impl VersionSelector {
    /// Version number (0 = newest) in `filesets` (newest first).
    pub fn resolve(&self, filesets: &[Fileset]) -> Result<usize, AppError> {
        let usable = |allow: bool| filesets.iter().enumerate().filter(move |(_, f)| allow || !f.partial);
        let none = |what: String| AppError::validation(what);
        match self {
            Self::Index { index } => {
                if *index < filesets.len() {
                    Ok(*index)
                } else {
                    Err(none(format!("Backup version {index} does not exist ({} version(s))", filesets.len())))
                }
            }
            Self::Latest { allow_partial } => usable(*allow_partial).map(|(i, _)| i).next().ok_or_else(|| none(no_complete(filesets))),
            Self::Oldest { allow_partial } => usable(*allow_partial).map(|(i, _)| i).next_back().ok_or_else(|| none(no_complete(filesets))),
            Self::AsOf { time, allow_partial } => {
                // Newest first: the first version not after `time` wins; on a
                // tie (same second) the newer entry in the list is taken.
                let hit = usable(*allow_partial).find(|(_, f)| parse_ts(&f.timestamp).is_some_and(|t| t <= *time)).map(|(i, _)| i);
                hit.ok_or_else(|| {
                    let oldest = usable(*allow_partial).next_back().map(|(_, f)| f.timestamp.clone());
                    match oldest {
                        Some(o) => AppError::file_op(
                            format!("No backup version was made at or before {}", time.with_timezone(&Local).format("%Y-%m-%d %H:%M:%S")),
                            format!("The oldest version is from {o}. Pick a later time."),
                        ),
                        None => none(no_complete(filesets)),
                    }
                })
            }
            Self::Id { id } => {
                let want = parse_ts(id);
                filesets
                    .iter()
                    .position(|f| f.volume_name == *id || f.timestamp == *id || (want.is_some() && parse_ts(&f.timestamp) == want))
                    .ok_or_else(|| none(format!("No backup version '{id}'")))
            }
        }
    }

    /// Parse a CLI `--version` value: `latest`, `oldest`, a number, or a
    /// version's timestamp / file-list name.
    pub fn parse(s: &str) -> Result<Self, AppError> {
        let s = s.trim();
        match s.to_ascii_lowercase().as_str() {
            "" => Err(AppError::validation("Empty version")),
            "latest" | "newest" => Ok(Self::Latest { allow_partial: false }),
            "oldest" => Ok(Self::Oldest { allow_partial: false }),
            _ => match s.parse::<usize>() {
                Ok(index) => Ok(Self::Index { index }),
                Err(_) => Ok(Self::Id { id: s.to_string() }),
            },
        }
    }

    /// From query parameters or CLI flags: `time` ("as of") wins over
    /// `version` (`latest`, `oldest`, a number or a version id); neither =
    /// the newest version. `allow_partial` applies to latest/oldest/time.
    pub fn from_parts(version: Option<&str>, time: Option<&str>, allow_partial: bool) -> Result<Self, AppError> {
        if let Some(t) = time.filter(|t| !t.trim().is_empty()) {
            return Self::as_of(t, allow_partial);
        }
        match version.filter(|v| !v.trim().is_empty()) {
            None => Ok(Self::Index { index: 0 }),
            Some(v) => Ok(match Self::parse(v)? {
                Self::Latest { .. } => Self::Latest { allow_partial },
                Self::Oldest { .. } => Self::Oldest { allow_partial },
                other => other,
            }),
        }
    }

    /// `as_of` from a user-typed time (see [`parse_time`]).
    pub fn as_of(s: &str, allow_partial: bool) -> Result<Self, AppError> {
        Ok(Self::AsOf { time: parse_time(s)?, allow_partial })
    }
}

fn no_complete(filesets: &[Fileset]) -> String {
    if filesets.is_empty() {
        "This backup has no versions yet".into()
    } else {
        "This backup has only partial versions; allow partial versions to use one".into()
    }
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(s).ok().map(|t| t.with_timezone(&Utc))
}

/// Parse a point in time: RFC 3339 (`2026-10-01T09:00:00Z`, any offset), or
/// a local date and time in the forms `2026-10-01 09:00[:30]`,
/// `2026-10-01T09:00[:30]` and `2026-10-01` (start of that day is not what
/// people mean by "as of a date", so a bare date means its end, 23:59:59).
pub fn parse_time(s: &str) -> Result<DateTime<Utc>, AppError> {
    parse_time_in(s, &Local)
}

/// [`parse_time`] in a given time zone (tests use UTC and fixed offsets).
pub fn parse_time_in<Tz: TimeZone>(s: &str, tz: &Tz) -> Result<DateTime<Utc>, AppError> {
    let s = s.trim();
    if let Some(t) = parse_ts(s) {
        return Ok(t);
    }
    let local = |n: NaiveDateTime| {
        // A time skipped by a DST change has no local reading; an ambiguous
        // one (repeated hour) takes the later instant.
        tz.from_local_datetime(&n).latest().map(|t| t.with_timezone(&Utc)).ok_or_else(|| AppError::validation(format!("{s} does not exist in the local time zone")))
    };
    for f in ["%Y-%m-%d %H:%M:%S", "%Y-%m-%d %H:%M", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%dT%H:%M", "%Y/%m/%d %H:%M:%S", "%Y/%m/%d %H:%M"] {
        if let Ok(n) = NaiveDateTime::parse_from_str(s, f) {
            return local(n);
        }
    }
    for f in ["%Y-%m-%d", "%Y/%m/%d"] {
        if let Ok(d) = NaiveDate::parse_from_str(s, f) {
            return local(d.and_time(NaiveTime::from_hms_opt(23, 59, 59).unwrap_or_default()));
        }
    }
    Err(AppError::file_op(
        format!("Cannot read the time '{s}'"),
        "Use 2026-10-01 09:00 (local time), 2026-10-01, or an RFC 3339 time such as 2026-10-01T09:00:00Z.",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    fn fs(ts: &str, partial: bool) -> Fileset {
        Fileset { timestamp: ts.into(), volume_name: format!("p-{ts}.dlist"), partial, ..Default::default() }
    }

    fn sample() -> Vec<Fileset> {
        // Newest first, like BackupDb::filesets.
        vec![
            fs("2026-10-03T12:00:00+00:00", true),
            fs("2026-10-02T12:00:00+00:00", false),
            fs("2026-10-01T12:00:00+00:00", true),
            fs("2026-10-01T09:00:00+00:00", false),
            fs("2026-09-30T09:00:00+00:00", true),
        ]
    }

    fn t(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    #[test]
    fn latest_oldest_and_index_skip_partial_versions() {
        let v = sample();
        assert_eq!(VersionSelector::Latest { allow_partial: false }.resolve(&v).unwrap(), 1);
        assert_eq!(VersionSelector::Latest { allow_partial: true }.resolve(&v).unwrap(), 0);
        assert_eq!(VersionSelector::Oldest { allow_partial: false }.resolve(&v).unwrap(), 3);
        assert_eq!(VersionSelector::Oldest { allow_partial: true }.resolve(&v).unwrap(), 4);
        assert_eq!(VersionSelector::Index { index: 2 }.resolve(&v).unwrap(), 2, "a number picks exactly that version");
        assert!(VersionSelector::Index { index: 5 }.resolve(&v).is_err());
        assert!(VersionSelector::Latest { allow_partial: false }.resolve(&[]).is_err());
        let only_partial = vec![fs("2026-10-03T12:00:00+00:00", true)];
        let e = VersionSelector::Latest { allow_partial: false }.resolve(&only_partial).unwrap_err();
        assert!(e.user_message().contains("only partial"));
    }

    #[test]
    fn as_of_picks_newest_complete_not_after_the_time() {
        let v = sample();
        let at = |s: &str, p: bool| VersionSelector::AsOf { time: t(s), allow_partial: p }.resolve(&v);
        // Between versions: the complete one before.
        assert_eq!(at("2026-10-02T00:00:00Z", false).unwrap(), 3, "the partial run at 12:00 on 10-01 is skipped");
        assert_eq!(at("2026-10-02T00:00:00Z", true).unwrap(), 2);
        // Exactly on a version's time: that version (<=).
        assert_eq!(at("2026-10-02T12:00:00Z", false).unwrap(), 1);
        assert_eq!(at("2026-10-01T09:00:00Z", false).unwrap(), 3);
        // Offsets are compared as instants.
        assert_eq!(at("2026-10-02T14:00:00+02:00", false).unwrap(), 1);
        // Far future: the newest complete.
        assert_eq!(at("2030-01-01T00:00:00Z", false).unwrap(), 1);
        // Before everything: an error naming the oldest version.
        let e = at("2026-09-01T00:00:00Z", false).unwrap_err();
        assert!(e.user_message().contains("No backup version"), "{}", e.user_message());
        // Only a partial version is old enough.
        assert!(at("2026-09-30T10:00:00Z", false).is_err());
        assert_eq!(at("2026-09-30T10:00:00Z", true).unwrap(), 4);
    }

    #[test]
    fn as_of_ties_take_the_first_listed() {
        // Two sources wrote at the same second ("*" lists them together).
        let v = vec![fs("2026-10-02T12:00:00+00:00", false), fs("2026-10-02T12:00:00Z", false), fs("2026-10-01T12:00:00+00:00", false)];
        assert_eq!(VersionSelector::AsOf { time: t("2026-10-02T12:00:00Z"), allow_partial: false }.resolve(&v).unwrap(), 0);
    }

    #[test]
    fn id_matches_timestamp_in_any_offset_or_volume_name() {
        let v = sample();
        assert_eq!(VersionSelector::Id { id: "2026-10-01T09:00:00+00:00".into() }.resolve(&v).unwrap(), 3);
        assert_eq!(VersionSelector::Id { id: "2026-10-01T11:00:00+02:00".into() }.resolve(&v).unwrap(), 3);
        assert_eq!(VersionSelector::Id { id: "p-2026-10-02T12:00:00+00:00.dlist".into() }.resolve(&v).unwrap(), 1);
        assert!(VersionSelector::Id { id: "nope".into() }.resolve(&v).is_err());
    }

    #[test]
    fn parse_cli_values() {
        assert_eq!(VersionSelector::parse("latest").unwrap(), VersionSelector::Latest { allow_partial: false });
        assert_eq!(VersionSelector::parse("OLDEST").unwrap(), VersionSelector::Oldest { allow_partial: false });
        assert_eq!(VersionSelector::parse("3").unwrap(), VersionSelector::Index { index: 3 });
        assert_eq!(VersionSelector::parse("2026-10-01T09:00:00Z").unwrap(), VersionSelector::Id { id: "2026-10-01T09:00:00Z".into() });
        assert!(VersionSelector::parse(" ").is_err());
        assert_eq!(VersionSelector::from_parts(None, None, false).unwrap(), VersionSelector::Index { index: 0 });
        assert_eq!(VersionSelector::from_parts(Some("oldest"), None, true).unwrap(), VersionSelector::Oldest { allow_partial: true });
        assert_eq!(VersionSelector::from_parts(Some("2"), Some(""), false).unwrap(), VersionSelector::Index { index: 2 });
        assert!(matches!(VersionSelector::from_parts(Some("2"), Some("2026-10-01T09:00:00Z"), false).unwrap(), VersionSelector::AsOf { .. }), "time wins");
        assert!(VersionSelector::from_parts(None, Some("garbage"), false).is_err());
    }

    #[test]
    fn friendly_times_are_local() {
        let utc = Utc;
        assert_eq!(parse_time_in("2026-10-01 09:00", &utc).unwrap(), t("2026-10-01T09:00:00Z"));
        assert_eq!(parse_time_in("2026-10-01T09:00:30", &utc).unwrap(), t("2026-10-01T09:00:30Z"));
        assert_eq!(parse_time_in("2026/10/01 09:00", &utc).unwrap(), t("2026-10-01T09:00:00Z"));
        assert_eq!(parse_time_in("2026-10-01", &utc).unwrap(), t("2026-10-01T23:59:59Z"), "a date means its end");
        let plus2 = FixedOffset::east_opt(2 * 3600).unwrap();
        assert_eq!(parse_time_in("2026-10-01 09:00", &plus2).unwrap(), t("2026-10-01T07:00:00Z"));
        // RFC 3339 keeps its own offset whatever the zone.
        assert_eq!(parse_time_in("2026-10-01T09:00:00-05:00", &plus2).unwrap(), t("2026-10-01T14:00:00Z"));
        assert!(parse_time_in("yesterday-ish", &utc).is_err());
        assert!(parse_time("2026-10-01 09:00").is_ok());
    }

    #[test]
    fn serde_shape() {
        let s: VersionSelector = serde_json::from_str(r#"{"type":"as_of","time":"2026-10-01T09:00:00Z"}"#).unwrap();
        assert_eq!(s, VersionSelector::AsOf { time: t("2026-10-01T09:00:00Z"), allow_partial: false });
        let s: VersionSelector = serde_json::from_str(r#"{"type":"latest"}"#).unwrap();
        assert_eq!(s, VersionSelector::Latest { allow_partial: false });
        assert_eq!(serde_json::to_value(VersionSelector::Index { index: 2 }).unwrap(), serde_json::json!({"type": "index", "index": 2}));
    }
}
