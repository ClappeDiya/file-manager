//! Version retention: keep-all, age, count, smart, custom thinning and
//! count-based GFS (grandfather-father-son) rules.
//!
//! Custom rules use Duplicati's `timeframe:interval` syntax, comma separated:
//! `1W:1D,4W:1W,12M:1M` keeps one version per day for a week, one per week
//! for four weeks and one per month for a year; anything older than every
//! timeframe is removed. Units: `s`, `m` (minutes), `h`, `D`, `W`, `M`
//! (30 days), `Y` (365 days). Interval `U` keeps every version in its frame.
//!
//! [`RetentionPolicy::Counts`] follows Kopia: for each period (hour, day,
//! ISO week, month, year, all in UTC) the newest version of each of the N
//! most recent periods that have versions is kept, plus the `keep_latest`
//! newest versions. A version can satisfy several rules. All zero keeps
//! everything.
//!
//! [`RetentionPolicy::plan`] is the single decision function: it gives
//! every version keep/delete with the rules that keep it, honouring the
//! newest version (always kept), pins ("Keep forever"), object locks and a
//! legal hold. Previews and [`apply`](super::maintenance::apply_retention)
//! both use it.

use crate::error::AppError;
use chrono::{DateTime, Datelike, Duration, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

/// Smart retention preset.
pub const SMART_RULES: &str = "1W:1D,4W:1W,12M:1M";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RetentionPolicy {
    #[default]
    KeepAll,
    KeepDays { days: u32 },
    KeepCount { count: u32 },
    Smart,
    Custom { rules: String },
    /// Count-based GFS: newest of the N most recent hours/days/weeks/months/years.
    Counts {
        #[serde(default)]
        keep_latest: u32,
        #[serde(default)]
        keep_hourly: u32,
        #[serde(default)]
        keep_daily: u32,
        #[serde(default)]
        keep_weekly: u32,
        #[serde(default)]
        keep_monthly: u32,
        #[serde(default)]
        keep_yearly: u32,
    },
}

/// Largest count accepted by [`RetentionPolicy::Counts`].
pub const MAX_COUNT: u32 = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Rule {
    timeframe: Duration,
    /// `None` = unlimited (keep all).
    interval: Option<Duration>,
}

fn parse_duration(s: &str) -> Result<Duration, AppError> {
    let s = s.trim();
    let bad = || AppError::validation(format!("Invalid retention duration '{s}' (use e.g. 7D, 4W, 12M, 1Y)"));
    let unit = s.chars().last().ok_or_else(bad)?;
    let n: i64 = s[..s.len() - unit.len_utf8()].parse().map_err(|_| bad())?;
    if n <= 0 {
        return Err(bad());
    }
    Ok(match unit {
        's' => Duration::seconds(n),
        'm' => Duration::minutes(n),
        'h' => Duration::hours(n),
        'D' | 'd' => Duration::days(n),
        'W' | 'w' => Duration::weeks(n),
        'M' => Duration::days(30 * n),
        'Y' | 'y' => Duration::days(365 * n),
        _ => return Err(bad()),
    })
}

fn parse_rules(spec: &str) -> Result<Vec<(Rule, String)>, AppError> {
    let mut rules = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (tf, iv) = part
            .split_once(':')
            .ok_or_else(|| AppError::validation(format!("Retention rule '{part}' must be timeframe:interval")))?;
        let interval = if iv.trim().eq_ignore_ascii_case("U") { None } else { Some(parse_duration(iv)?) };
        let label = match interval {
            None => format!("all within {}", tf.trim()),
            Some(_) => format!("one per {} within {}", iv.trim(), tf.trim()),
        };
        rules.push((Rule { timeframe: parse_duration(tf)?, interval }, label));
    }
    if rules.is_empty() {
        return Err(AppError::validation("Custom retention needs at least one rule"));
    }
    rules.sort_by_key(|(r, _)| r.timeframe);
    Ok(rules)
}

/// One version as retention sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetentionCandidate {
    pub id: i64,
    pub time: DateTime<Utc>,
    pub pinned: bool,
    /// Object-lock expiry when the version's file list is still locked.
    pub locked_until: Option<String>,
}

impl RetentionCandidate {
    pub fn new(id: i64, time: DateTime<Utc>) -> Self {
        Self { id, time, pinned: false, locked_until: None }
    }
}

/// The decision for one version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetentionVerdict {
    pub id: i64,
    /// UTC timestamp (RFC 3339).
    pub timestamp: String,
    pub keep: bool,
    /// Why it is kept ("latest 3", "daily 7", "pinned", …) or why it goes.
    pub reasons: Vec<String>,
    /// One line: "kept: daily 7, weekly 4" or "deleted: …".
    pub summary: String,
}

impl RetentionPolicy {
    /// Validate the policy's parameters.
    pub fn validate(&self) -> Result<(), AppError> {
        match self {
            Self::KeepDays { days: 0 } | Self::KeepCount { count: 0 } => {
                Err(AppError::validation("Retention value must be at least 1"))
            }
            Self::Custom { rules } => parse_rules(rules).map(|_| ()),
            Self::Counts { keep_latest, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly } => {
                if [keep_latest, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly].iter().any(|n| **n > MAX_COUNT) {
                    return Err(AppError::validation(format!("Retention counts are limited to {MAX_COUNT}")));
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Return the ids of versions to delete. `versions` is (id, time) in any
    /// order. Same decisions as [`plan`](Self::plan) without pins or holds.
    pub fn select_deletions(&self, versions: &[(i64, DateTime<Utc>)], now: DateTime<Utc>) -> Result<Vec<i64>, AppError> {
        let c: Vec<RetentionCandidate> = versions.iter().map(|(id, t)| RetentionCandidate::new(*id, *t)).collect();
        Ok(self.plan(&c, now, false)?.into_iter().filter(|v| !v.keep).map(|v| v.id).collect())
    }

    /// Decide every version (newest first in the result): kept by the
    /// policy's rules, as the newest version, pinned, locked or under a
    /// legal hold, or deleted.
    pub fn plan(&self, versions: &[RetentionCandidate], now: DateTime<Utc>, legal_hold: bool) -> Result<Vec<RetentionVerdict>, AppError> {
        self.validate()?;
        let mut v = versions.to_vec();
        v.sort_by_key(|c| std::cmp::Reverse(c.time)); // newest first
        let times: Vec<(i64, DateTime<Utc>)> = v.iter().map(|c| (c.id, c.time)).collect();
        let (mut keep, del) = self.policy_reasons(&times, now)?;
        let mut out = Vec::with_capacity(v.len());
        for (i, c) in v.iter().enumerate() {
            let mut reasons = keep.remove(&c.id).unwrap_or_default();
            if i == 0 {
                reasons.insert(0, "newest version".into());
            }
            if c.pinned {
                reasons.push("pinned".into());
            }
            if let Some(u) = &c.locked_until {
                reasons.push(format!("locked until {u}"));
            }
            if legal_hold {
                reasons.push("legal hold".into());
            }
            let keep = !reasons.is_empty();
            if !keep {
                reasons.push(del.get(&c.id).cloned().unwrap_or_else(|| "not selected by any rule".into()));
            }
            let summary = format!("{}: {}", if keep { "kept" } else { "deleted" }, reasons.join(", "));
            out.push(RetentionVerdict { id: c.id, timestamp: c.time.to_rfc3339(), keep, reasons, summary });
        }
        Ok(out)
    }

    /// Keep reasons per id, and why the others are deleted. `v` is newest first.
    #[allow(clippy::type_complexity)]
    fn policy_reasons(&self, v: &[(i64, DateTime<Utc>)], now: DateTime<Utc>) -> Result<(HashMap<i64, Vec<String>>, HashMap<i64, String>), AppError> {
        let mut keep: HashMap<i64, Vec<String>> = HashMap::new();
        let mut del: HashMap<i64, String> = HashMap::new();
        let mut add = |id: i64, why: String| keep.entry(id).or_default().push(why);
        match self {
            Self::KeepAll => v.iter().for_each(|(id, _)| add(*id, "keep all versions".into())),
            Self::KeepDays { days } => {
                let cutoff = now - Duration::days(*days as i64);
                for (id, t) in v {
                    if *t >= cutoff {
                        add(*id, format!("newer than {days} days"));
                    } else {
                        del.insert(*id, format!("older than {days} days"));
                    }
                }
            }
            Self::KeepCount { count } => {
                for (i, (id, _)) in v.iter().enumerate() {
                    if i < *count as usize {
                        add(*id, format!("latest {count}"));
                    } else {
                        del.insert(*id, format!("beyond the latest {count}"));
                    }
                }
            }
            Self::Smart => thin(v, &parse_rules(SMART_RULES)?, now, &mut keep, &mut del),
            Self::Custom { rules } => thin(v, &parse_rules(rules)?, now, &mut keep, &mut del),
            Self::Counts { keep_latest, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly } => {
                let all = [keep_latest, keep_hourly, keep_daily, keep_weekly, keep_monthly, keep_yearly];
                if all.iter().all(|n| **n == 0) {
                    v.iter().for_each(|(id, _)| add(*id, "no limits set (keep all)".into()));
                    return Ok((keep, del));
                }
                for (id, _) in v.iter().take(*keep_latest as usize) {
                    add(*id, format!("latest {keep_latest}"));
                }
                let periods: [(&str, u32, fn(&DateTime<Utc>) -> (i32, u32, u32, u32)); 5] = [
                    ("hourly", *keep_hourly, |t| (t.year(), t.ordinal(), t.hour(), 0)),
                    ("daily", *keep_daily, |t| (t.year(), t.ordinal(), 0, 0)),
                    ("weekly", *keep_weekly, |t| {
                        let w = t.iso_week();
                        (w.year(), w.week(), 0, 0)
                    }),
                    ("monthly", *keep_monthly, |t| (t.year(), t.month(), 0, 0)),
                    ("yearly", *keep_yearly, |t| (t.year(), 0, 0, 0)),
                ];
                for (name, n, key) in periods {
                    if n == 0 {
                        continue;
                    }
                    let mut seen = HashSet::new();
                    for (id, t) in v {
                        // Newest first: the first version seen in a period is its newest.
                        if seen.len() >= n as usize && !seen.contains(&key(t)) {
                            break;
                        }
                        if seen.insert(key(t)) {
                            add(*id, format!("{name} {n}"));
                        }
                    }
                }
            }
        }
        Ok((keep, del))
    }
}

fn thin(newest_first: &[(i64, DateTime<Utc>)], rules: &[(Rule, String)], now: DateTime<Utc>, keep: &mut HashMap<i64, Vec<String>>, del: &mut HashMap<i64, String>) {
    // Bucket each version by the smallest timeframe that contains it.
    let mut buckets: Vec<Vec<(i64, DateTime<Utc>)>> = vec![Vec::new(); rules.len()];
    for &(id, t) in newest_first {
        let age = now - t;
        match rules.iter().position(|(r, _)| age <= r.timeframe) {
            Some(i) => buckets[i].push((id, t)),
            None => {
                del.insert(id, "older than every rule".into());
            }
        }
    }
    for ((rule, label), mut items) in rules.iter().zip(buckets) {
        let Some(interval) = rule.interval else {
            for (id, _) in items {
                keep.entry(id).or_default().push(label.clone());
            }
            continue;
        };
        items.sort_by_key(|(_, t)| *t); // oldest first
        let mut last_kept: Option<DateTime<Utc>> = None;
        for (id, t) in items {
            match last_kept {
                Some(k) if t - k < interval => {
                    del.insert(id, format!("another version within {label}"));
                }
                _ => {
                    last_kept = Some(t);
                    keep.entry(id).or_default().push(label.clone());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(now: DateTime<Utc>, hours_ago: i64) -> DateTime<Utc> {
        now - Duration::hours(hours_ago)
    }

    #[test]
    fn count_and_days() {
        let now = Utc::now();
        let v: Vec<_> = (0..5).map(|i| (i, at(now, i * 24))).collect();
        let mut d = RetentionPolicy::KeepCount { count: 2 }.select_deletions(&v, now).unwrap();
        d.sort();
        assert_eq!(d, vec![2, 3, 4]);
        let mut d = RetentionPolicy::KeepDays { days: 2 }.select_deletions(&v, now).unwrap();
        d.sort();
        assert_eq!(d, vec![3, 4]);
        assert!(RetentionPolicy::KeepAll.select_deletions(&v, now).unwrap().is_empty());
    }

    #[test]
    fn never_deletes_newest_even_if_old() {
        let now = Utc::now();
        let v = vec![(1, at(now, 24 * 100)), (2, at(now, 24 * 200))];
        assert_eq!(RetentionPolicy::KeepDays { days: 1 }.select_deletions(&v, now).unwrap(), vec![2]);
    }

    #[test]
    fn custom_thinning() {
        let now = Utc::now();
        // Hourly versions over 3 days with rule "1D:U,7D:1D".
        let v: Vec<_> = (0..72).map(|h| (h, at(now, h))).collect();
        let d = RetentionPolicy::Custom { rules: "1D:U,7D:1D".into() }.select_deletions(&v, now).unwrap();
        let kept: Vec<_> = v.iter().filter(|(id, _)| !d.contains(id)).collect();
        // 25 in the last day (0..=24h) + one per day for the remaining 2 days.
        assert_eq!(kept.len(), 25 + 2);
        // Older than every timeframe → deleted.
        let v = vec![(1, now), (2, at(now, 24 * 30))];
        assert_eq!(RetentionPolicy::Custom { rules: "7D:1D".into() }.select_deletions(&v, now).unwrap(), vec![2]);
    }

    #[test]
    fn validation() {
        assert!(RetentionPolicy::Custom { rules: "1X:1D".into() }.validate().is_err());
        assert!(RetentionPolicy::Custom { rules: "".into() }.validate().is_err());
        assert!(RetentionPolicy::KeepCount { count: 0 }.validate().is_err());
        assert!(RetentionPolicy::Smart.validate().is_ok());
    }

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn kept(p: &RetentionPolicy, v: &[(i64, DateTime<Utc>)], now: DateTime<Utc>) -> Vec<i64> {
        let d = p.select_deletions(v, now).unwrap();
        let mut k: Vec<i64> = v.iter().map(|x| x.0).filter(|id| !d.contains(id)).collect();
        k.sort();
        k
    }

    fn counts(latest: u32, hourly: u32, daily: u32, weekly: u32, monthly: u32, yearly: u32) -> RetentionPolicy {
        RetentionPolicy::Counts { keep_latest: latest, keep_hourly: hourly, keep_daily: daily, keep_weekly: weekly, keep_monthly: monthly, keep_yearly: yearly }
    }

    #[test]
    fn counts_daily_keeps_newest_of_each_recent_day() {
        // Three versions a day for five days.
        let now = ts("2026-03-10T23:00:00Z");
        let mut v = Vec::new();
        for d in 0..5i64 {
            for h in [2i64, 10, 18] {
                v.push((d * 10 + h, ts("2026-03-10T00:00:00Z") - Duration::days(d) + Duration::hours(h)));
            }
        }
        // Newest of the 3 most recent days: day 0, 1, 2 at 18:00.
        assert_eq!(kept(&counts(0, 0, 3, 0, 0, 0), &v, now), vec![18, 28, 38]);
        // keep_latest adds the two newest; daily 18 overlaps with latest.
        assert_eq!(kept(&counts(2, 0, 3, 0, 0, 0), &v, now), vec![10, 18, 28, 38]);
        // All zero keeps everything.
        assert_eq!(kept(&counts(0, 0, 0, 0, 0, 0), &v, now).len(), v.len());
        // Gaps: the N most recent days that have versions, not calendar days.
        let sparse = vec![(1, ts("2026-03-10T05:00:00Z")), (2, ts("2026-03-01T05:00:00Z")), (3, ts("2026-02-01T05:00:00Z")), (4, ts("2026-01-01T05:00:00Z"))];
        assert_eq!(kept(&counts(0, 0, 3, 0, 0, 0), &sparse, now), vec![1, 2, 3]);
    }

    #[test]
    fn counts_weekly_uses_iso_weeks() {
        // 2026-01-04 is a Sunday (ISO week 2026-W01), 2026-01-05 a Monday (W02).
        // 2025-12-29 (Monday) already belongs to ISO week 2026-W01.
        let v = vec![
            (1, ts("2026-01-05T08:00:00Z")),
            (2, ts("2026-01-04T23:59:59Z")),
            (3, ts("2025-12-29T00:00:00Z")),
            (4, ts("2025-12-28T12:00:00Z")),
        ];
        let now = ts("2026-01-06T00:00:00Z");
        // Week 02: 1; week 01 (Dec 29 – Jan 4): newest is 2; week 2025-52: 4.
        assert_eq!(kept(&counts(0, 0, 0, 2, 0, 0), &v, now), vec![1, 2]);
        assert_eq!(kept(&counts(0, 0, 0, 3, 0, 0), &v, now), vec![1, 2, 4]);
        // Year boundary: yearly keeps the newest of 2026 and of 2025.
        assert_eq!(kept(&counts(0, 0, 0, 0, 0, 2), &v, now), vec![1, 3]);
    }

    #[test]
    fn counts_monthly_month_ends_and_hourly() {
        let v = vec![
            (1, ts("2026-03-01T00:00:00Z")),
            (2, ts("2026-02-28T23:59:59Z")),
            (3, ts("2026-02-01T00:00:00Z")),
            (4, ts("2026-01-31T23:00:00Z")),
            (5, ts("2026-01-31T22:30:00Z")),
        ];
        let now = ts("2026-03-02T00:00:00Z");
        assert_eq!(kept(&counts(0, 0, 0, 0, 2, 0), &v, now), vec![1, 2]);
        assert_eq!(kept(&counts(0, 0, 0, 0, 3, 0), &v, now), vec![1, 2, 4]);
        // Hourly: 23:00 and 22:30 are different hours; all five are in distinct hours.
        assert_eq!(kept(&counts(0, 5, 0, 0, 0, 0), &v, now), vec![1, 2, 3, 4, 5]);
        assert_eq!(kept(&counts(0, 4, 0, 0, 0, 0), &v, now), vec![1, 2, 3, 4]);
        assert!(counts(0, 0, MAX_COUNT + 1, 0, 0, 0).validate().is_err());
    }

    #[test]
    fn plan_reports_reasons_pins_locks_and_holds() {
        let now = ts("2026-03-10T12:00:00Z");
        let mk = |id: i64, days: i64| RetentionCandidate::new(id, now - Duration::days(days));
        let mut v = vec![mk(1, 0), mk(2, 1), mk(3, 2), mk(4, 30), mk(5, 40)];
        v[3].pinned = true;
        v[4].locked_until = Some("2027-01-01T00:00:00Z".into());
        let p = counts(1, 0, 2, 0, 0, 0);
        let plan = p.plan(&v, now, false).unwrap();
        let by_id: HashMap<i64, &RetentionVerdict> = plan.iter().map(|x| (x.id, x)).collect();
        assert_eq!(plan[0].id, 1, "newest first");
        assert_eq!(by_id[&1].summary, "kept: newest version, latest 1, daily 2");
        assert_eq!(by_id[&2].summary, "kept: daily 2");
        assert!(!by_id[&3].keep && by_id[&3].summary == "deleted: not selected by any rule");
        assert_eq!(by_id[&4].summary, "kept: pinned");
        assert!(by_id[&5].keep && by_id[&5].reasons[0].starts_with("locked until"));
        // Legal hold keeps everything.
        assert!(p.plan(&v, now, true).unwrap().iter().all(|x| x.keep));
        // The older policies give reasons too.
        let d = RetentionPolicy::KeepDays { days: 7 }.plan(&v, now, false).unwrap();
        assert_eq!(d.iter().find(|x| x.id == 3).unwrap().summary, "kept: newer than 7 days");
        let c = RetentionPolicy::Custom { rules: "7D:U".into() }.plan(&[mk(1, 0), mk(2, 30)], now, false).unwrap();
        assert_eq!(c[1].summary, "deleted: older than every rule");
        assert_eq!(c[0].reasons, vec!["newest version", "all within 7D"]);
    }

    #[test]
    fn counts_serde_shape() {
        let p: RetentionPolicy = serde_json::from_str(r#"{"type":"counts","keep_daily":7,"keep_weekly":4}"#).unwrap();
        assert_eq!(p, counts(0, 0, 7, 4, 0, 0));
        assert_eq!(serde_json::to_value(&p).unwrap()["type"], "counts");
    }
}
