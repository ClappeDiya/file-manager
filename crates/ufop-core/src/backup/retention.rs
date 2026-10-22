//! Version retention: keep-all, age, count, smart and custom thinning rules.
//!
//! Custom rules use Duplicati's `timeframe:interval` syntax, comma separated:
//! `1W:1D,4W:1W,12M:1M` keeps one version per day for a week, one per week
//! for four weeks and one per month for a year; anything older than every
//! timeframe is removed. Units: `s`, `m` (minutes), `h`, `D`, `W`, `M`
//! (30 days), `Y` (365 days). Interval `U` keeps every version in its frame.
//! The newest version is never selected for deletion.

use crate::error::AppError;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

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
}

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

fn parse_rules(spec: &str) -> Result<Vec<Rule>, AppError> {
    let mut rules = Vec::new();
    for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
        let (tf, iv) = part
            .split_once(':')
            .ok_or_else(|| AppError::validation(format!("Retention rule '{part}' must be timeframe:interval")))?;
        let interval = if iv.trim().eq_ignore_ascii_case("U") { None } else { Some(parse_duration(iv)?) };
        rules.push(Rule { timeframe: parse_duration(tf)?, interval });
    }
    if rules.is_empty() {
        return Err(AppError::validation("Custom retention needs at least one rule"));
    }
    rules.sort_by_key(|r| r.timeframe);
    Ok(rules)
}

impl RetentionPolicy {
    /// Validate the policy's parameters.
    pub fn validate(&self) -> Result<(), AppError> {
        match self {
            Self::KeepDays { days: 0 } | Self::KeepCount { count: 0 } => {
                Err(AppError::validation("Retention value must be at least 1"))
            }
            Self::Custom { rules } => parse_rules(rules).map(|_| ()),
            _ => Ok(()),
        }
    }

    /// Return the ids of versions to delete. `versions` is (id, time) in any
    /// order.
    pub fn select_deletions(&self, versions: &[(i64, DateTime<Utc>)], now: DateTime<Utc>) -> Result<Vec<i64>, AppError> {
        let mut v = versions.to_vec();
        v.sort_by_key(|(_, t)| std::cmp::Reverse(*t)); // newest first
        if v.len() <= 1 {
            return Ok(Vec::new());
        }
        let newest = v[0].0;
        let mut del: Vec<i64> = match self {
            Self::KeepAll => Vec::new(),
            Self::KeepDays { days } => {
                let cutoff = now - Duration::days(*days as i64);
                v.iter().filter(|(_, t)| *t < cutoff).map(|(id, _)| *id).collect()
            }
            Self::KeepCount { count } => v.iter().skip(*count as usize).map(|(id, _)| *id).collect(),
            Self::Smart => thin(&v, &parse_rules(SMART_RULES)?, now),
            Self::Custom { rules } => thin(&v, &parse_rules(rules)?, now),
        };
        del.retain(|id| *id != newest);
        Ok(del)
    }
}

fn thin(newest_first: &[(i64, DateTime<Utc>)], rules: &[Rule], now: DateTime<Utc>) -> Vec<i64> {
    let mut del = Vec::new();
    // Bucket each version by the smallest timeframe that contains it.
    let mut buckets: Vec<Vec<(i64, DateTime<Utc>)>> = vec![Vec::new(); rules.len()];
    for &(id, t) in newest_first {
        let age = now - t;
        match rules.iter().position(|r| age <= r.timeframe) {
            Some(i) => buckets[i].push((id, t)),
            None => del.push(id),
        }
    }
    for (rule, mut items) in rules.iter().zip(buckets) {
        let Some(interval) = rule.interval else { continue };
        items.sort_by_key(|(_, t)| *t); // oldest first
        let mut last_kept: Option<DateTime<Utc>> = None;
        for (id, t) in items {
            match last_kept {
                Some(k) if t - k < interval => del.push(id),
                _ => last_kept = Some(t),
            }
        }
    }
    del
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
}
