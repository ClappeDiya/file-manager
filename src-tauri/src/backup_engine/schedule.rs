//! Backup schedule math (interval and cron modes, time zones, maintenance
//! schedules). The implementation lives in `ufop_core::backup::schedule` so
//! the standalone `ufop agent` host runs managed jobs on the same slots;
//! holiday calendars are the schedule library's, kept in
//! `ufop_core::calendar::registry`.

pub use ufop_core::backup::schedule::*;
