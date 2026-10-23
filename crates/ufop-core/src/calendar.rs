//! Holiday (business) calendars: days on which scheduled work is skipped.
//!
//! A calendar holds one-off dates, dates that recur every year, and
//! optionally weekends. Calendars can be imported from iCalendar (`.ics`)
//! files: every all-day `VEVENT` start date (and its date range) becomes a
//! date; a yearly `RRULE` makes it recur.

use chrono::{Datelike, NaiveDate, Weekday};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct HolidayCalendar {
    pub id: String,
    pub name: String,
    /// One-off dates (`YYYY-MM-DD`).
    pub dates: Vec<NaiveDate>,
    /// Dates that recur every year, as `MM-DD`.
    pub annual: Vec<String>,
    /// Also skip Saturdays and Sundays.
    pub skip_weekends: bool,
}

impl HolidayCalendar {
    /// Whether `d` is a day off in this calendar.
    pub fn blocks(&self, d: NaiveDate) -> bool {
        if self.skip_weekends && matches!(d.weekday(), Weekday::Sat | Weekday::Sun) {
            return true;
        }
        if self.dates.contains(&d) {
            return true;
        }
        let md = format!("{:02}-{:02}", d.month(), d.day());
        self.annual.contains(&md)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.name.trim().is_empty() || self.name.chars().count() > 100 {
            return Err("Give the calendar a name (up to 100 characters)".into());
        }
        for a in &self.annual {
            let ok = a.len() == 5 && NaiveDate::parse_from_str(&format!("2024-{a}"), "%Y-%m-%d").is_ok();
            if !ok {
                return Err(format!("'{a}' is not a yearly date (use MM-DD, e.g. 12-25)"));
            }
        }
        if self.dates.len() + self.annual.len() > 5000 {
            return Err("A calendar can hold at most 5000 dates".into());
        }
        Ok(())
    }

    /// Merge dates from an iCalendar file (all-day events; yearly rules recur).
    /// Returns how many dates were added.
    pub fn import_ics(&mut self, ics: &str) -> Result<usize, String> {
        let before = self.dates.len() + self.annual.len();
        let events = parse_ics(ics)?;
        for e in events {
            if e.yearly {
                let md = format!("{:02}-{:02}", e.start.month(), e.start.day());
                if !self.annual.contains(&md) {
                    self.annual.push(md);
                }
                continue;
            }
            let mut d = e.start;
            while d < e.end_exclusive {
                if !self.dates.contains(&d) {
                    self.dates.push(d);
                }
                d = d.succ_opt().ok_or("date overflow")?;
            }
        }
        self.dates.sort();
        self.annual.sort();
        Ok(self.dates.len() + self.annual.len() - before)
    }
}

/// The first calendar (by name) that blocks `d`, if any.
pub fn blocking<'a>(calendars: impl IntoIterator<Item = &'a HolidayCalendar>, d: NaiveDate) -> Option<&'a HolidayCalendar> {
    calendars.into_iter().find(|c| c.blocks(d))
}

struct IcsEvent {
    start: NaiveDate,
    end_exclusive: NaiveDate,
    yearly: bool,
}

fn ics_date(v: &str) -> Option<NaiveDate> {
    let v = v.trim();
    NaiveDate::parse_from_str(v.get(..8)?, "%Y%m%d").ok()
}

fn parse_ics(ics: &str) -> Result<Vec<IcsEvent>, String> {
    if !ics.contains("BEGIN:VCALENDAR") {
        return Err("Not an iCalendar file (no BEGIN:VCALENDAR)".into());
    }
    // Unfold continuation lines (RFC 5545 §3.1).
    let mut lines: Vec<String> = Vec::new();
    for raw in ics.lines() {
        let raw = raw.trim_end_matches('\r');
        if (raw.starts_with(' ') || raw.starts_with('\t')) && !lines.is_empty() {
            if let Some(last) = lines.last_mut() {
                last.push_str(&raw[1..]);
            }
        } else {
            lines.push(raw.to_string());
        }
    }
    let mut out = Vec::new();
    let (mut in_event, mut yearly) = (false, false);
    let (mut start, mut end): (Option<NaiveDate>, Option<NaiveDate>) = (None, None);
    for l in lines {
        let (key, value) = match l.split_once(':') {
            Some((k, v)) => (k.split(';').next().unwrap_or("").to_ascii_uppercase(), v.to_string()),
            None => continue,
        };
        match (key.as_str(), value.trim()) {
            ("BEGIN", "VEVENT") => {
                in_event = true;
                start = None;
                end = None;
                yearly = false;
            }
            ("END", "VEVENT") if in_event => {
                in_event = false;
                if let Some(s) = start {
                    let e = end.filter(|e| *e > s).unwrap_or_else(|| s.succ_opt().unwrap_or(s));
                    // Bound ranges: a stray multi-year event must not explode.
                    let e = if (e - s).num_days() > 366 { s.succ_opt().unwrap_or(s) } else { e };
                    out.push(IcsEvent { start: s, end_exclusive: e, yearly });
                }
            }
            ("DTSTART", v) if in_event => start = ics_date(v),
            ("DTEND", v) if in_event => end = ics_date(v),
            ("RRULE", v) if in_event => yearly = v.to_ascii_uppercase().split(';').any(|p| p == "FREQ=YEARLY"),
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn blocking_days() {
        let c = HolidayCalendar { id: "c".into(), name: "Office".into(), dates: vec![d("2026-04-03")], annual: vec!["12-25".into(), "01-01".into()], skip_weekends: true };
        assert!(c.blocks(d("2026-04-03")) && c.blocks(d("2027-12-25")) && c.blocks(d("2026-01-01")));
        assert!(c.blocks(d("2026-10-24")), "Saturday");
        assert!(!c.blocks(d("2026-10-22")), "Thursday");
        let plain = HolidayCalendar { skip_weekends: false, ..c.clone() };
        assert!(!plain.blocks(d("2026-10-24")));
        assert_eq!(blocking([&plain, &c], d("2026-10-25")).map(|c| c.name.as_str()), Some("Office"));
        assert!(blocking([&plain], d("2026-10-25")).is_none());
    }

    #[test]
    fn validation() {
        let ok = HolidayCalendar { name: "x".into(), annual: vec!["02-29".into()], ..Default::default() };
        assert!(ok.validate().is_ok());
        assert!(HolidayCalendar { name: " ".into(), ..Default::default() }.validate().is_err());
        assert!(HolidayCalendar { name: "x".into(), annual: vec!["13-01".into()], ..Default::default() }.validate().is_err());
        assert!(HolidayCalendar { name: "x".into(), annual: vec!["1-1".into()], ..Default::default() }.validate().is_err());
    }

    #[test]
    fn ics_import() {
        let ics = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VEVENT\r\nSUMMARY:Christmas\r\nDTSTART;VALUE=DATE:20261225\r\nRRULE:FREQ=YEARLY\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Office closed\r\nDTSTART;VALUE=DATE:20261228\r\nDTEND;VALUE=DATE:20261231\r\nEND:VEVENT\r\nBEGIN:VEVENT\r\nSUMMARY:Long\r\n description\r\nDTSTART:20260501T090000Z\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";
        let mut c = HolidayCalendar { name: "Imported".into(), ..Default::default() };
        assert_eq!(c.import_ics(ics).unwrap(), 5);
        assert_eq!(c.annual, vec!["12-25".to_string()]);
        assert_eq!(c.dates, vec![d("2026-05-01"), d("2026-12-28"), d("2026-12-29"), d("2026-12-30")]);
        // Importing again adds nothing.
        assert_eq!(c.import_ics(ics).unwrap(), 0);
        assert!(c.import_ics("hello").is_err());
    }
}
