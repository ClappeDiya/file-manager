//! Name and path templates: `{token}` substitution shared by batch rename,
//! automation destinations, sync version names and archive names.
//!
//! Tokens (all values come from the file the template is applied to):
//!
//! | Token | Value |
//! |---|---|
//! | `{name}` | file stem (`report` for `report.pdf`) |
//! | `{ext}` | extension without the dot |
//! | `{filename}` | full file name |
//! | `{parent}` | name of the containing folder |
//! | `{size}` | size in bytes |
//! | `{num}` / `{counter}` | sequence number; `{num:3}` pads to 3 digits |
//! | `{date}` | modification date as `YYYYMMDD` |
//! | `{yyyy}` `{mm}` `{dd}` `{HH}` `{MM}` `{SS}` | parts of the modification time |
//! | `{modified:FMT}` `{created:FMT}` `{now:FMT}` | a time formatted with strftime `FMT` |
//! | `{1}` … `{9}`, `{match}` | capture groups / whole match of the route regex |
//! | `{key}` | any extra value supplied by the caller (e.g. webhook fields) |
//!
//! Filters follow a pipe: `{name|lower}`, `{parent|slug}`; available filters
//! are `lower`, `upper`, `title`, `snake`, `slug`, `trim`. `{{` and `}}`
//! produce literal braces. Times use the local time zone unless the caller
//! sets one.

use chrono::{DateTime, FixedOffset, Local, Utc};
use std::collections::HashMap;
use std::path::Path;

/// Why a template could not be rendered.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    #[error("Unknown token {{{0}}}")]
    UnknownToken(String),
    #[error("Unknown filter '{0}'")]
    UnknownFilter(String),
    #[error("Unclosed '{{' at position {0}")]
    Unclosed(usize),
    #[error("Token {{{0}}} has no value for this file")]
    Missing(String),
    #[error("Invalid time format '{0}'")]
    BadFormat(String),
}

/// What happens to tokens the template doesn't know.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UnknownTokens {
    /// Leave `{token}` in the output (batch rename's historic behaviour).
    #[default]
    Keep,
    /// Fail, so a misspelt automation destination is caught before it runs.
    Error,
}

/// Everything a template can draw from.
#[derive(Debug, Clone, Default)]
pub struct TokenContext {
    pub file_name: String,
    pub stem: String,
    pub ext: String,
    pub parent: String,
    pub size: Option<u64>,
    pub modified: Option<DateTime<Utc>>,
    pub created: Option<DateTime<Utc>>,
    pub now: Option<DateTime<Utc>>,
    pub counter: Option<u64>,
    /// Default zero-padding for `{num}` without an explicit width.
    pub counter_pad: usize,
    /// Regex captures: index 0 is the whole match.
    pub captures: Vec<String>,
    /// Caller-supplied values (`{key}`).
    pub extra: HashMap<String, String>,
    /// Offset for time tokens; `None` means local time.
    pub tz: Option<FixedOffset>,
}

impl TokenContext {
    /// Context for a path, reading size and times from the filesystem when
    /// the file exists. Never fails: missing metadata just leaves tokens empty.
    pub fn for_path(path: &Path) -> Self {
        let file_name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let stem = path.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_default();
        let ext = path.extension().map(|e| e.to_string_lossy().to_string()).unwrap_or_default();
        let parent = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|f| f.to_string_lossy().to_string())
            .unwrap_or_default();
        let meta = std::fs::metadata(path).ok();
        Self {
            file_name,
            stem,
            ext,
            parent,
            size: meta.as_ref().map(|m| m.len()),
            modified: meta.as_ref().and_then(|m| m.modified().ok()).map(Into::into),
            created: meta.as_ref().and_then(|m| m.created().ok()).map(Into::into),
            ..Default::default()
        }
    }

    pub fn with_counter(mut self, counter: u64, pad: usize) -> Self {
        self.counter = Some(counter);
        self.counter_pad = pad;
        self
    }

    pub fn with_captures(mut self, captures: Vec<String>) -> Self {
        self.captures = captures;
        self
    }

    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.insert(key.into(), value.into());
        self
    }

    /// Fill `{1}`…`{9}` and `{match}` from `regex` matched against the file
    /// name. Returns false (and leaves captures empty) when it doesn't match.
    pub fn capture_from(&mut self, regex: &regex::Regex) -> bool {
        match regex.captures(&self.file_name) {
            Some(c) => {
                self.captures = c.iter().map(|m| m.map(|m| m.as_str().to_string()).unwrap_or_default()).collect();
                true
            }
            None => false,
        }
    }

    fn format_time(&self, t: DateTime<Utc>, fmt: &str) -> Result<String, TemplateError> {
        use std::fmt::Write;
        let mut out = String::new();
        let res = match self.tz {
            Some(tz) => write!(out, "{}", t.with_timezone(&tz).format(fmt)),
            None => write!(out, "{}", t.with_timezone(&Local).format(fmt)),
        };
        res.map_err(|_| TemplateError::BadFormat(fmt.to_string()))?;
        Ok(out)
    }

    fn time(&self, which: &str) -> Option<DateTime<Utc>> {
        match which {
            "modified" => self.modified,
            "created" => self.created.or(self.modified),
            "now" => Some(self.now.unwrap_or_else(Utc::now)),
            _ => None,
        }
    }

    /// Value of one token (without filters), `Ok(None)` if unknown.
    fn value(&self, name: &str, arg: Option<&str>) -> Result<Option<String>, TemplateError> {
        let modified_part = |fmt: &str| -> Result<Option<String>, TemplateError> {
            let t = self.modified.ok_or_else(|| TemplateError::Missing(name.to_string()))?;
            self.format_time(t, fmt).map(Some)
        };
        Ok(match name {
            "name" => Some(self.stem.clone()),
            "ext" => Some(self.ext.clone()),
            "filename" => Some(self.file_name.clone()),
            "parent" => Some(self.parent.clone()),
            "size" => Some(self.size.ok_or_else(|| TemplateError::Missing(name.into()))?.to_string()),
            "num" | "counter" => {
                let n = self.counter.ok_or_else(|| TemplateError::Missing(name.into()))?;
                let width = match arg {
                    Some(a) => a.parse::<usize>().map_err(|_| TemplateError::BadFormat(a.to_string()))?,
                    None => self.counter_pad,
                };
                Some(format!("{n:0>width$}"))
            }
            "date" => match arg {
                Some(fmt) => modified_part(fmt)?,
                None => Some(
                    self.modified
                        .map(|t| self.format_time(t, "%Y%m%d"))
                        .transpose()?
                        .unwrap_or_else(|| "unknown".into()),
                ),
            },
            "yyyy" => modified_part("%Y")?,
            "yy" => modified_part("%y")?,
            "mm" => modified_part("%m")?,
            "dd" => modified_part("%d")?,
            "HH" => modified_part("%H")?,
            "MM" => modified_part("%M")?,
            "SS" => modified_part("%S")?,
            "modified" | "created" | "now" => {
                let t = self.time(name).ok_or_else(|| TemplateError::Missing(name.into()))?;
                Some(self.format_time(t, arg.unwrap_or("%Y-%m-%d"))?)
            }
            "match" => Some(self.captures.first().cloned().ok_or_else(|| TemplateError::Missing(name.into()))?),
            n if n.len() == 1 && n.as_bytes()[0].is_ascii_digit() && n != "0" => {
                let i: usize = n.parse().unwrap_or(0);
                Some(self.captures.get(i).cloned().ok_or_else(|| TemplateError::Missing(name.into()))?)
            }
            other => self.extra.get(other).cloned(),
        })
    }
}

fn apply_filter(value: String, filter: &str) -> Result<String, TemplateError> {
    Ok(match filter {
        "lower" => value.to_lowercase(),
        "upper" => value.to_uppercase(),
        "title" => title_case(&value),
        "trim" => value.trim().to_string(),
        "snake" => value
            .chars()
            .map(|c| if c == ' ' || c == '-' { '_' } else { c })
            .collect::<String>()
            .to_lowercase(),
        "slug" => {
            let mut out = String::new();
            for c in value.to_lowercase().chars() {
                if c.is_alphanumeric() {
                    out.push(c);
                } else if !out.ends_with('-') {
                    out.push('-');
                }
            }
            out.trim_matches('-').to_string()
        }
        other => return Err(TemplateError::UnknownFilter(other.to_string())),
    })
}

/// Capitalise the first letter of every whitespace-separated word.
pub fn title_case(s: &str) -> String {
    s.split(' ')
        .map(|word| {
            let mut c = word.chars();
            match c.next() {
                Some(first) => first.to_uppercase().collect::<String>() + &c.as_str().to_lowercase(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Capitalise only the first letter.
pub fn sentence_case(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut c = lower.chars();
    match c.next() {
        Some(first) => first.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

/// Render `template` against `ctx`.
pub fn render(template: &str, ctx: &TokenContext, unknown: UnknownTokens) -> Result<String, TemplateError> {
    render_with(template, ctx, unknown, |v| v)
}

/// Render a *path* template: token values can never introduce a separator
/// or a `..` component, so `{name}` from a hostile file name stays one
/// path segment. Literal separators in the template are kept.
pub fn render_path(template: &str, ctx: &TokenContext, unknown: UnknownTokens) -> Result<String, TemplateError> {
    render_with(template, ctx, unknown, sanitize_segment)
}

/// Make a token value safe as (part of) one path segment.
pub fn sanitize_segment(v: String) -> String {
    let v: String = v.chars().map(|c| if c == '/' || c == '\\' || c == '\0' { '_' } else { c }).collect();
    if v == ".." || v == "." {
        "_".to_string()
    } else {
        v
    }
}

fn render_with(
    template: &str,
    ctx: &TokenContext,
    unknown: UnknownTokens,
    post: impl Fn(String) -> String,
) -> Result<String, TemplateError> {
    let mut out = String::with_capacity(template.len());
    let bytes = template.as_bytes();
    let mut i = 0;
    while i < template.len() {
        match bytes[i] {
            b'{' if bytes.get(i + 1) == Some(&b'{') => {
                out.push('{');
                i += 2;
            }
            b'}' if bytes.get(i + 1) == Some(&b'}') => {
                out.push('}');
                i += 2;
            }
            b'{' => {
                let end = template[i..].find('}').map(|e| i + e).ok_or(TemplateError::Unclosed(i))?;
                let inner = &template[i + 1..end];
                let (spec, filters) = match inner.split_once('|') {
                    Some((s, f)) => (s, f.split('|').map(str::trim).collect::<Vec<_>>()),
                    None => (inner, vec![]),
                };
                let (name, arg) = match spec.split_once(':') {
                    Some((n, a)) => (n.trim(), Some(a)),
                    None => (spec.trim(), None),
                };
                match ctx.value(name, arg)? {
                    Some(mut v) => {
                        for f in &filters {
                            v = apply_filter(v, f)?;
                        }
                        out.push_str(&post(v));
                    }
                    None if unknown == UnknownTokens::Keep => out.push_str(&template[i..=end]),
                    None => return Err(TemplateError::UnknownToken(name.to_string())),
                }
                i = end + 1;
            }
            _ => {
                let ch = template[i..].chars().next().unwrap_or_default();
                out.push(ch);
                i += ch.len_utf8();
            }
        }
    }
    Ok(out)
}

/// Token names a template uses (for validation and UI hints).
pub fn tokens_in(template: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = template.replace("{{", "").replace("}}", "");
    while let Some(start) = rest.find('{') {
        let Some(end) = rest[start..].find('}') else { break };
        let inner = &rest[start + 1..start + end];
        let name = inner.split(['|', ':']).next().unwrap_or("").trim().to_string();
        if !name.is_empty() && !found.contains(&name) {
            found.push(name);
        }
        rest = rest[start + end + 1..].to_string();
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ctx() -> TokenContext {
        TokenContext {
            file_name: "Q3 Report-final.pdf".into(),
            stem: "Q3 Report-final".into(),
            ext: "pdf".into(),
            parent: "Inbox".into(),
            size: Some(2048),
            modified: Some(Utc.with_ymd_and_hms(2026, 3, 7, 14, 5, 9).unwrap()),
            created: None,
            now: Some(Utc.with_ymd_and_hms(2026, 10, 22, 8, 0, 0).unwrap()),
            counter: Some(7),
            counter_pad: 0,
            tz: Some(FixedOffset::east_opt(0).unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn historic_batch_rename_tokens_are_unchanged() {
        let c = ctx();
        assert_eq!(render("{name}_{num}.{ext}", &c, UnknownTokens::Keep).unwrap(), "Q3 Report-final_7.pdf");
        assert_eq!(render("{date}-{parent}-{counter}", &c, UnknownTokens::Keep).unwrap(), "20260307-Inbox-7");
        let padded = TokenContext { counter_pad: 3, ..ctx() };
        assert_eq!(render("{num}", &padded, UnknownTokens::Keep).unwrap(), "007");
        assert_eq!(render("{nope}", &c, UnknownTokens::Keep).unwrap(), "{nope}");
    }

    #[test]
    fn date_parts_formats_and_filters() {
        let c = ctx();
        assert_eq!(render("{yyyy}/{mm}/{dd} {HH}{MM}{SS}", &c, UnknownTokens::Error).unwrap(), "2026/03/07 140509");
        assert_eq!(render("{modified:%Y-W%V}", &c, UnknownTokens::Error).unwrap(), "2026-W10");
        assert_eq!(render("{now:%Y}", &c, UnknownTokens::Error).unwrap(), "2026");
        assert_eq!(render("{created:%m}", &c, UnknownTokens::Error).unwrap(), "03", "created falls back to modified");
        assert_eq!(render("{name|slug}", &c, UnknownTokens::Error).unwrap(), "q3-report-final");
        assert_eq!(render("{name|snake}", &c, UnknownTokens::Error).unwrap(), "q3_report_final");
        assert_eq!(render("{ext|upper}{num:4}", &c, UnknownTokens::Error).unwrap(), "PDF0007");
        assert_eq!(render("{{literal}}", &c, UnknownTokens::Error).unwrap(), "{literal}");
        assert_eq!(render("{size}", &c, UnknownTokens::Error).unwrap(), "2048");
    }

    #[test]
    fn regex_captures_and_extras() {
        let mut c = ctx();
        let re = regex::Regex::new(r"^(Q\d) (\w+)").unwrap();
        assert!(c.capture_from(&re));
        assert_eq!(render("{2}/{1}/{match}", &c, UnknownTokens::Error).unwrap(), "Report/Q3/Q3 Report");
        let c = c.with_extra("customer", "acme");
        assert_eq!(render("{customer}", &c, UnknownTokens::Error).unwrap(), "acme");
        let mut miss = ctx();
        assert!(!miss.capture_from(&regex::Regex::new("^zzz").unwrap()));
        assert_eq!(render("{1}", &miss, UnknownTokens::Error), Err(TemplateError::Missing("1".into())));
    }

    #[test]
    fn errors_are_specific() {
        let c = ctx();
        assert_eq!(render("{nmae}", &c, UnknownTokens::Error), Err(TemplateError::UnknownToken("nmae".into())));
        assert_eq!(render("{name|shout}", &c, UnknownTokens::Error), Err(TemplateError::UnknownFilter("shout".into())));
        assert_eq!(render("abc{name", &c, UnknownTokens::Error), Err(TemplateError::Unclosed(3)));
        let none = TokenContext { modified: None, ..ctx() };
        assert_eq!(render("{yyyy}", &none, UnknownTokens::Error), Err(TemplateError::Missing("yyyy".into())));
        assert_eq!(render("{date}", &none, UnknownTokens::Error).unwrap(), "unknown");
    }

    #[test]
    fn path_rendering_keeps_values_inside_one_segment() {
        let mut c = ctx();
        c.extra.insert("evil".into(), "../../etc".into());
        c.extra.insert("dots".into(), "..".into());
        assert_eq!(render_path("/out/{yyyy}/{evil}/{ext}", &c, UnknownTokens::Error).unwrap(), "/out/2026/.._.._etc/pdf");
        assert_eq!(render_path("/out/{dots}", &c, UnknownTokens::Error).unwrap(), "/out/_");
    }

    #[test]
    fn unicode_literals_survive() {
        let c = ctx();
        assert_eq!(render("Rapport–{name|lower}–é", &c, UnknownTokens::Error).unwrap(), "Rapport–q3 report-final–é");
    }

    #[test]
    fn lists_tokens() {
        assert_eq!(tokens_in("{yyyy}/{name|lower}-{num:3}{{x}}"), vec!["yyyy", "name", "num"]);
    }

    #[test]
    fn context_from_a_real_file() {
        let dir = std::env::temp_dir().join(format!("ufop-tokens-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, b"hello").unwrap();
        let c = TokenContext::for_path(&f);
        assert_eq!((c.stem.as_str(), c.ext.as_str(), c.size), ("a", "txt", Some(5)));
        assert!(c.modified.is_some());
        std::fs::remove_dir_all(dir).ok();
    }
}
