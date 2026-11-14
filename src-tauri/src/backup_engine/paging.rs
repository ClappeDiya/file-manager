//! Optional paging of long backup lists (K122): a version's files, the run
//! history and a destination's raw file listing can hold hundreds of
//! thousands of rows, so their IPC/REST calls take an optional
//! [`PageRequest`] (offset, limit, sort column, direction, text filter) and
//! then answer a [`Page`] with the total. Callers that pass no page keep
//! getting the plain array ([`Listing::All`]).

use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

/// Largest page served at once (also the cap for "all rows" requests).
pub const MAX_PAGE: usize = 100_000;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PageRequest {
    /// Rows to skip (after filtering and sorting).
    pub offset: usize,
    /// Rows to return; `None` = all remaining (capped at [`MAX_PAGE`]).
    pub limit: Option<usize>,
    /// Column to sort by (see each row type's `SORT_FIELDS`); `None` keeps
    /// the natural order.
    pub sort: Option<String>,
    pub descending: bool,
    /// Case-insensitive text the row's main text (path, name, summary) must contain.
    pub filter: Option<String>,
}

impl PageRequest {
    /// The effective page size.
    pub fn take(&self) -> usize {
        self.limit.unwrap_or(MAX_PAGE).clamp(1, MAX_PAGE)
    }

    /// The trimmed, lower-cased filter, when one is set.
    pub fn needle(&self) -> Option<String> {
        self.filter.as_deref().map(str::trim).filter(|f| !f.is_empty()).map(str::to_lowercase)
    }

    /// Checks the sort column against the row type's columns.
    pub fn check_sort(&self, fields: &[&str]) -> Result<(), AppError> {
        match self.sort.as_deref() {
            Some(f) if !fields.contains(&f) => Err(AppError::validation(format!("Cannot sort by \"{f}\"; use one of: {}", fields.join(", ")))),
            _ => Ok(()),
        }
    }
}

/// One page of rows and how many rows match in total.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Page<T> {
    pub items: Vec<T>,
    /// Rows matching the filter, across all pages.
    pub total: usize,
    pub offset: usize,
    /// The page size used.
    pub limit: usize,
}

/// A list answer: the plain array (no page asked) or a [`Page`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Listing<T> {
    All(Vec<T>),
    Page(Page<T>),
}

/// A row that can be sorted by named columns and filtered by text.
pub trait Pageable {
    const SORT_FIELDS: &'static [&'static str];
    /// Compares by a column from `SORT_FIELDS`.
    fn compare_by(&self, other: &Self, field: &str) -> Ordering;
    /// The text the filter matches.
    fn text(&self) -> &str;
}

/// Filters, sorts (stable, so equal rows keep their order) and slices.
pub fn paginate<T: Pageable>(mut items: Vec<T>, req: &PageRequest) -> Result<Page<T>, AppError> {
    req.check_sort(T::SORT_FIELDS)?;
    if let Some(n) = req.needle() {
        items.retain(|r| r.text().to_lowercase().contains(&n));
    }
    if let Some(f) = req.sort.as_deref() {
        items.sort_by(|a, b| {
            let o = a.compare_by(b, f);
            if req.descending { o.reverse() } else { o }
        });
    }
    let total = items.len();
    let limit = req.take();
    let items = items.into_iter().skip(req.offset).take(limit).collect();
    Ok(Page { items, total, offset: req.offset, limit })
}

/// The plain list, or a page of it when `page` is given.
pub fn listing<T: Pageable>(items: Vec<T>, page: Option<&PageRequest>) -> Result<Listing<T>, AppError> {
    match page {
        None => Ok(Listing::All(items)),
        Some(p) => Ok(Listing::Page(paginate(items, p)?)),
    }
}

impl Pageable for ufop_core::backup::FileEntry {
    const SORT_FIELDS: &'static [&'static str] = &["path", "size", "kind", "modified"];
    fn compare_by(&self, other: &Self, field: &str) -> Ordering {
        match field {
            "size" => self.size.cmp(&other.size),
            "kind" => format!("{:?}", self.kind).cmp(&format!("{:?}", other.kind)),
            "modified" => self.mtime.cmp(&other.mtime),
            _ => self.path.cmp(&other.path),
        }
    }
    fn text(&self) -> &str {
        &self.path
    }
}

impl Pageable for ufop_core::backup::RemoteFile {
    const SORT_FIELDS: &'static [&'static str] = &["name", "size"];
    fn compare_by(&self, other: &Self, field: &str) -> Ordering {
        match field {
            "size" => self.size.cmp(&other.size),
            _ => self.name.cmp(&other.name),
        }
    }
    fn text(&self) -> &str {
        &self.name
    }
}

impl Pageable for super::BackupRun {
    const SORT_FIELDS: &'static [&'static str] = RUN_SORT_FIELDS;
    fn compare_by(&self, other: &Self, field: &str) -> Ordering {
        match field {
            "finished_at" => self.finished_at.cmp(&other.finished_at),
            "status" => self.status.cmp(&other.status),
            "operation" => self.operation.cmp(&other.operation),
            "job_id" => self.job_id.cmp(&other.job_id),
            _ => self.started_at.cmp(&other.started_at),
        }
    }
    fn text(&self) -> &str {
        &self.summary
    }
}

/// Run history columns, also used by the SQL pager (each maps to a column).
pub const RUN_SORT_FIELDS: &[&str] = &["started_at", "finished_at", "status", "operation", "job_id"];

#[cfg(test)]
mod tests {
    use super::*;
    use ufop_core::backup::RemoteFile;

    fn files() -> Vec<RemoteFile> {
        [("b.dat", 30), ("A.dat", 10), ("c.idx", 20), ("a2.dat", 20)].into_iter().map(|(n, s)| RemoteFile { name: n.into(), size: s }).collect()
    }
    fn names(p: &Page<RemoteFile>) -> Vec<&str> {
        p.items.iter().map(|f| f.name.as_str()).collect()
    }

    #[test]
    fn sorts_pages_and_counts() {
        let req = PageRequest { sort: Some("size".into()), limit: Some(2), ..Default::default() };
        let p = paginate(files(), &req).unwrap();
        assert_eq!((p.total, p.offset, p.limit), (4, 0, 2));
        assert_eq!(names(&p), ["A.dat", "c.idx"], "stable: equal sizes keep their order");
        let p = paginate(files(), &PageRequest { offset: 2, ..req.clone() }).unwrap();
        assert_eq!(names(&p), ["a2.dat", "b.dat"]);
        let p = paginate(files(), &PageRequest { descending: true, ..req }).unwrap();
        assert_eq!(names(&p), ["b.dat", "c.idx"]);
        let p = paginate(files(), &PageRequest { offset: 10, ..Default::default() }).unwrap();
        assert!(p.items.is_empty());
        assert_eq!(p.total, 4);
    }

    #[test]
    fn filters_case_insensitively_before_counting() {
        let p = paginate(files(), &PageRequest { filter: Some(" A ".into()), sort: Some("name".into()), ..Default::default() }).unwrap();
        assert_eq!(p.total, 3, "c.idx has no a");
        let p = paginate(files(), &PageRequest { filter: Some(".DAT".into()), sort: Some("name".into()), ..Default::default() }).unwrap();
        assert_eq!(names(&p), ["A.dat", "a2.dat", "b.dat"]);
        assert_eq!(p.total, 3);
    }

    #[test]
    fn unknown_sort_column_is_refused_and_limits_are_capped() {
        let e = paginate(files(), &PageRequest { sort: Some("owner".into()), ..Default::default() }).unwrap_err();
        assert!(e.user_message().contains("name, size"), "{}", e.user_message());
        assert_eq!(PageRequest { limit: Some(0), ..Default::default() }.take(), 1);
        assert_eq!(PageRequest { limit: Some(usize::MAX), ..Default::default() }.take(), MAX_PAGE);
    }

    #[test]
    fn listing_keeps_the_plain_array_without_a_page() {
        let all = serde_json::to_value(listing(files(), None).unwrap()).unwrap();
        assert!(all.is_array());
        let page = serde_json::to_value(listing(files(), Some(&PageRequest { limit: Some(1), ..Default::default() })).unwrap()).unwrap();
        assert_eq!(page["total"], 4);
        assert_eq!(page["items"].as_array().unwrap().len(), 1);
        let req: PageRequest = serde_json::from_value(serde_json::json!({ "limit": 25, "sort": "size", "descending": true })).unwrap();
        assert_eq!((req.offset, req.take(), req.descending), (0, 25, true));
    }
}
