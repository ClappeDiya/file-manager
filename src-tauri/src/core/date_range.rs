//! Date bounds the history views pass to the backend: RFC 3339 instants
//! (the UI turns "Today" or a picked day into local-midnight instants, so
//! the ranges follow the user's clock, not UTC).

use crate::core::error::AppError;
use chrono::{DateTime, Utc};

/// One bound; `None` or blank means no bound.
pub fn parse_bound(label: &str, value: Option<&str>) -> Result<Option<DateTime<Utc>>, AppError> {
    let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) else { return Ok(None) };
    DateTime::parse_from_rfc3339(v)
        .map(|d| Some(d.with_timezone(&Utc)))
        .map_err(|_| AppError::validation(format!("\"{v}\" is not a valid {label} date; use a date and time like 2026-01-31T00:00:00Z")))
}

/// Both bounds of a range; a range that ends before it starts is refused.
pub fn parse_range(from: Option<&str>, to: Option<&str>) -> Result<(Option<DateTime<Utc>>, Option<DateTime<Utc>>), AppError> {
    let (f, t) = (parse_bound("start", from)?, parse_bound("end", to)?);
    if let (Some(f), Some(t)) = (f, t) {
        if t < f {
            return Err(AppError::validation("The end of the date range is before its start"));
        }
    }
    Ok((f, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_means_open_and_offsets_normalise_to_utc() {
        assert_eq!(parse_range(None, Some("  ")).unwrap(), (None, None));
        let (from, _) = parse_range(Some("2026-03-01T00:00:00+02:00"), None).unwrap();
        assert_eq!(from.unwrap().to_rfc3339(), "2026-02-28T22:00:00+00:00");
    }

    #[test]
    fn bad_and_inverted_ranges_are_refused_with_a_reason() {
        assert!(parse_range(Some("yesterday"), None).is_err());
        assert!(parse_range(Some("2026-03-02T00:00:00Z"), Some("2026-03-01T00:00:00Z")).is_err());
    }
}
