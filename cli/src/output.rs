//! Output formatting for human, JSON, and YAML modes.

use crate::OutputFormat;
use serde::Serialize;

/// A status message: colored on the terminal, structured for json/yaml, plain on
/// stderr for CSV (stdout stays pure data).
fn message(format: &OutputFormat, status: &str, icon: &str, message: &str, stderr: bool) {
    let structured = || serde_json::json!({ "status": status, "message": message });
    let line = match format {
        OutputFormat::Human => format!("{icon} {message}"),
        OutputFormat::Json => serde_json::to_string_pretty(&structured()).unwrap_or_default(),
        OutputFormat::Yaml => serde_yaml::to_string(&structured()).unwrap_or_default(),
        OutputFormat::Csv => return eprintln!("{status}: {message}"),
    };
    if stderr {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}

/// Print a success message in the appropriate format.
pub fn print_success(format: &OutputFormat, message: &str) {
    self::message(format, "success", "\x1b[32m✓\x1b[0m", message, false);
}

/// Print an error message in the appropriate format.
pub fn print_error(format: &OutputFormat, message: &str) {
    self::message(format, "error", "\x1b[31m✗\x1b[0m", message, true);
}

/// Print a warning message.
pub fn print_warning(format: &OutputFormat, message: &str) {
    self::message(format, "warning", "\x1b[33m!\x1b[0m", message, true);
}

/// Print structured data in the appropriate format.
pub fn print_data<T: Serialize>(format: &OutputFormat, data: &T) {
    match format {
        // For human mode, callers with a nicer layout handle it themselves.
        OutputFormat::Human | OutputFormat::Json => println!("{}", serde_json::to_string_pretty(data).unwrap_or_default()),
        OutputFormat::Yaml => println!("{}", serde_yaml::to_string(data).unwrap_or_default()),
        OutputFormat::Csv => print!("{}", csv_from_value(&serde_json::to_value(data).unwrap_or_default())),
    }
}

/// Print a table in human mode (CSV: the same headers and rows), structured data otherwise.
pub fn print_table<T: Serialize>(format: &OutputFormat, headers: &[&str], rows: &[Vec<String>], data: &T) {
    match format {
        OutputFormat::Human => {
            if rows.is_empty() {
                println!("  (no results)");
                return;
            }
            let mut widths: Vec<usize> = headers.iter().map(|h| h.chars().count()).collect();
            for row in rows {
                for (w, cell) in widths.iter_mut().zip(row) {
                    *w = (*w).max(cell.chars().count());
                }
            }
            let line = |cells: Vec<String>| println!("  {}", cells.iter().zip(&widths).map(|(c, w)| format!("{c:<w$}")).collect::<Vec<_>>().join("  "));
            line(headers.iter().map(|h| h.to_string()).collect());
            line(widths.iter().map(|w| "-".repeat(*w)).collect());
            for row in rows {
                line(row.clone());
            }
        }
        OutputFormat::Csv => {
            let mut out = csv_record(headers.iter().copied());
            for row in rows {
                out.push_str(&csv_record(row.iter().map(String::as_str)));
            }
            print!("{out}");
        }
        _ => print_data(format, data),
    }
}

/// One RFC 4180 record (CRLF-terminated); fields with `,` `"` CR or LF are quoted.
/// Cells that a spreadsheet would run as a formula (`=`, `+`, `-`, `@`, tab, CR)
/// are prefixed with `'`, except plain numbers.
pub fn csv_record<'a>(fields: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = fields
        .into_iter()
        .map(|f| {
            let formula = f.starts_with(['=', '+', '-', '@', '\t', '\r']) && f.parse::<f64>().is_err();
            let f = if formula { format!("'{f}") } else { f.to_string() };
            if f.contains([',', '"', '\n', '\r']) {
                format!("\"{}\"", f.replace('"', "\"\""))
            } else {
                f
            }
        })
        .collect::<Vec<_>>()
        .join(",");
    out.push_str("\r\n");
    out
}

fn csv_cell(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Bool(_) | serde_json::Value::Number(_) => v.to_string(),
        other => other.to_string(),
    }
}

/// CSV for structured data: a list of objects is one row each (columns in
/// first-seen order), a list of scalars one `value` column, an object one row.
/// Nested values are compact JSON.
pub fn csv_from_value(v: &serde_json::Value) -> String {
    use serde_json::Value;
    let items: Vec<&Value> = match v {
        Value::Array(a) => a.iter().collect(),
        Value::Null => vec![],
        other => vec![other],
    };
    if items.is_empty() {
        return String::new();
    }
    if !items.iter().all(|i| i.is_object()) {
        let mut out = csv_record(["value"]);
        for i in items {
            out.push_str(&csv_record([csv_cell(i).as_str()]));
        }
        return out;
    }
    let mut cols: Vec<&str> = Vec::new();
    for i in &items {
        for k in i.as_object().into_iter().flat_map(|o| o.keys()) {
            if !cols.contains(&k.as_str()) {
                cols.push(k);
            }
        }
    }
    let mut out = csv_record(cols.iter().copied());
    for i in &items {
        let cells: Vec<String> = cols.iter().map(|c| i.get(*c).map(csv_cell).unwrap_or_default()).collect();
        out.push_str(&csv_record(cells.iter().map(String::as_str)));
    }
    out
}

/// JSON event emitter for --json mode progress.
#[derive(Debug, Serialize)]
pub struct ProgressEvent {
    pub event: String,
    pub progress: f64,
    pub bytes_transferred: u64,
    pub bytes_total: u64,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
    pub current_file: Option<String>,
}

impl ProgressEvent {
    pub fn emit(&self) {
        println!("{}", serde_json::to_string(self).unwrap());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn csv_quotes_and_neutralizes_formulas() {
        assert_eq!(csv_record(["a", "b,c", "say \"hi\"", "x\ny"]), "a,\"b,c\",\"say \"\"hi\"\"\",\"x\ny\"\r\n");
        assert_eq!(csv_record(["=SUM(A1)", "-5", "-1.5e3", "@x", "+cmd"]), "'=SUM(A1),-5,-1.5e3,'@x,'+cmd\r\n");
    }

    #[test]
    fn csv_from_structured_data() {
        let v = json!([{ "name": "a", "size": 1 }, { "name": "b", "tags": ["x"], "size": null }]);
        assert_eq!(csv_from_value(&v), "name,size,tags\r\na,1,\r\nb,,\"[\"\"x\"\"]\"\r\n");
        assert_eq!(csv_from_value(&json!({ "ok": true, "n": 2 })), "n,ok\r\n2,true\r\n");
        assert_eq!(csv_from_value(&json!(["p", 3])), "value\r\np\r\n3\r\n");
        assert_eq!(csv_from_value(&json!([])), "");
    }
}
