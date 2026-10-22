//! Plain text from office documents, for previews and the search index:
//! Office Open XML (`docx`, `xlsx`, `pptx` and their macro/template
//! variants), OpenDocument (`odt`, `ods`, `odp`) and RTF. Legacy binary
//! formats (`doc`, `xls`, `ppt`) and iWork are recognized but have no text
//! here (`Ok(None)`); callers render them through the OS instead.
//!
//! Reads are bounded: each archive entry is decompressed up to a limit and
//! the output stops at `max_chars`, so a hostile file cannot exhaust memory.

use std::io::Read;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficeKind {
    Docx,
    Xlsx,
    Pptx,
    Odt,
    Ods,
    Odp,
    Rtf,
    /// `doc`, `xls`, `ppt`, `pages`, `numbers`, `key`, `vsdx`…: no text extraction.
    Opaque,
}

/// The office format of a file extension (case-insensitive), if any.
pub fn office_kind(ext: &str) -> Option<OfficeKind> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "docx" | "docm" | "dotx" | "dotm" => OfficeKind::Docx,
        "xlsx" | "xlsm" | "xltx" | "xltm" => OfficeKind::Xlsx,
        "pptx" | "pptm" | "potx" | "ppsx" => OfficeKind::Pptx,
        "odt" | "ott" => OfficeKind::Odt,
        "ods" | "ots" => OfficeKind::Ods,
        "odp" | "otp" => OfficeKind::Odp,
        "rtf" => OfficeKind::Rtf,
        "doc" | "dot" | "xls" | "xlt" | "ppt" | "pps" | "pages" | "numbers" | "key" | "vsd" | "vsdx" | "pub" | "xlsb" => OfficeKind::Opaque,
        _ => return None,
    })
}

/// Largest decompressed size read from one archive entry.
const MAX_ENTRY_BYTES: u64 = 32 * 1024 * 1024;

/// Text of `path`, at most `max_chars` characters; `Ok(None)` for formats without extraction.
pub fn extract_text(path: &Path, max_chars: usize) -> Result<Option<String>, String> {
    let ext = path.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    let Some(kind) = office_kind(&ext) else { return Ok(None) };
    let mut out = Out::new(max_chars);
    match kind {
        OfficeKind::Opaque => return Ok(None),
        OfficeKind::Rtf => {
            let mut raw = Vec::new();
            std::fs::File::open(path).map_err(|e| e.to_string())?.take(MAX_ENTRY_BYTES).read_to_end(&mut raw).map_err(|e| e.to_string())?;
            rtf_text(&String::from_utf8_lossy(&raw), &mut out);
        }
        _ => {
            let f = std::fs::File::open(path).map_err(|e| e.to_string())?;
            let mut zip = zip::ZipArchive::new(f).map_err(|e| format!("not a valid {ext} file: {e}"))?;
            let names: Vec<String> = zip.file_names().map(String::from).collect();
            let mut entry = |name: &str| -> Result<Option<String>, String> {
                let Ok(mut e) = zip.by_name(name) else { return Ok(None) };
                let mut s = String::new();
                (&mut e).take(MAX_ENTRY_BYTES).read_to_string(&mut s).map_err(|e| format!("{name}: {e}"))?;
                Ok(Some(s))
            };
            match kind {
                OfficeKind::Docx => {
                    for part in ["word/document.xml", "word/footnotes.xml", "word/endnotes.xml"] {
                        if let Some(x) = entry(part)? {
                            xml_text(&x, &WORD, &mut out);
                        }
                    }
                }
                OfficeKind::Xlsx => {
                    let names_in_order = entry("xl/workbook.xml")?.map(|wb| attr_values(&wb, "sheet", "name")).unwrap_or_default();
                    let shared = entry("xl/sharedStrings.xml")?.map(|x| shared_strings(&x)).unwrap_or_default();
                    for (i, name) in numbered(&names, "xl/worksheets/sheet", ".xml").iter().enumerate() {
                        if let Some(x) = entry(name)? {
                            let title = names_in_order.get(i).cloned().unwrap_or_else(|| format!("Sheet {}", i + 1));
                            out.push_str(&format!("{}{title}\n", if i == 0 { "" } else { "\n" }));
                            sheet_text(&x, &shared, &mut out);
                        }
                    }
                }
                OfficeKind::Pptx => {
                    for (i, name) in numbered(&names, "ppt/slides/slide", ".xml").iter().enumerate() {
                        if let Some(x) = entry(name)? {
                            out.push_str(&format!("{}Slide {}\n", if i == 0 { "" } else { "\n" }, i + 1));
                            xml_text(&x, &SLIDE, &mut out);
                        }
                    }
                }
                _ => {
                    if let Some(x) = entry("content.xml")? {
                        xml_text(&x, &ODF, &mut out);
                    }
                }
            }
        }
    }
    Ok(Some(out.finish()))
}

/// Archive entries `prefix<N>suffix`, in numeric order.
fn numbered(names: &[String], prefix: &str, suffix: &str) -> Vec<String> {
    let mut v: Vec<(u32, String)> = names.iter().filter_map(|n| n.strip_prefix(prefix)?.strip_suffix(suffix)?.parse().ok().map(|i| (i, n.clone()))).collect();
    v.sort();
    v.into_iter().map(|(_, n)| n).collect()
}

/// Bounded output that collapses runs of blank lines.
struct Out {
    s: String,
    max: usize,
    chars: usize,
}

impl Out {
    fn new(max: usize) -> Self {
        Self { s: String::new(), max, chars: 0 }
    }
    fn full(&self) -> bool {
        self.chars >= self.max
    }
    fn push(&mut self, c: char) {
        if c == '\n' {
            // A line never ends in cell separators or spaces.
            while self.s.ends_with(['\t', ' ']) {
                self.s.pop();
                self.chars -= 1;
            }
            if self.s.is_empty() || self.s.ends_with("\n\n") {
                return;
            }
        }
        if self.full() {
            return;
        }
        self.s.push(c);
        self.chars += 1;
    }
    /// End a table cell: its trailing space becomes the separator.
    fn end_cell(&mut self) {
        while self.s.ends_with(' ') {
            self.s.pop();
            self.chars -= 1;
        }
        self.push('\t');
    }
    fn push_str(&mut self, s: &str) {
        for c in s.chars() {
            self.push(c);
        }
    }
    fn finish(self) -> String {
        self.s.trim_end().to_string()
    }
}

/// How one XML vocabulary marks text structure.
struct Vocab {
    /// Closing these elements ends a line.
    line_end: &'static [&'static str],
    /// These empty elements are a tab / a line break.
    tab: &'static [&'static str],
    br: &'static [&'static str],
    /// Text inside these is not document text (field codes, deleted text…).
    skip: &'static [&'static str],
    /// Only text inside these elements counts (whitespace between others is layout).
    text_in: &'static [&'static str],
    /// Table cells: their paragraphs join with spaces and cells end with a tab.
    cell: &'static [&'static str],
}

const WORD: Vocab = Vocab { line_end: &["w:p", "w:tr"], tab: &["w:tab"], br: &["w:br", "w:cr"], skip: &["w:instrText", "w:delText"], text_in: &["w:t"], cell: &["w:tc"] };
const SHEET: Vocab = Vocab { line_end: &["si", "is"], tab: &[], br: &[], skip: &["rPh"], text_in: &["t"], cell: &[] };
const SLIDE: Vocab = Vocab { line_end: &["a:p"], tab: &["a:tab"], br: &["a:br"], skip: &[], text_in: &["a:t"], cell: &["a:tc"] };
const ODF: Vocab = Vocab { line_end: &["text:p", "text:h", "table:table-row"], tab: &["text:tab"], br: &["text:line-break"], skip: &["office:annotation"], text_in: &["text:p", "text:h"], cell: &["table:table-cell"] };

fn xml_text(xml: &str, v: &Vocab, out: &mut Out) {
    let mut rest = xml;
    let mut skipping = 0usize;
    let mut inside = 0usize;
    let mut cells = 0usize;
    while !rest.is_empty() && !out.full() {
        match rest.find('<') {
            Some(i) => {
                if skipping == 0 && inside > 0 {
                    decode_entities(&rest[..i], out);
                }
                let Some(end) = rest[i..].find('>') else { break };
                let tag = &rest[i + 1..i + end];
                rest = &rest[i + end + 1..];
                if tag.starts_with('?') || tag.starts_with('!') {
                    continue;
                }
                let closing = tag.starts_with('/');
                let self_closing = tag.ends_with('/');
                let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("");
                if v.skip.contains(&name) && !self_closing {
                    if closing {
                        skipping = skipping.saturating_sub(1);
                    } else {
                        skipping += 1;
                    }
                    continue;
                }
                if skipping > 0 {
                    continue;
                }
                if v.text_in.contains(&name) && !self_closing {
                    if closing {
                        inside = inside.saturating_sub(1);
                    } else {
                        inside += 1;
                    }
                }
                if v.cell.contains(&name) {
                    if self_closing {
                        out.push('\t');
                    } else if closing {
                        cells = cells.saturating_sub(1);
                        out.end_cell();
                    } else {
                        cells += 1;
                    }
                } else if closing && v.line_end.contains(&name) {
                    if cells > 0 {
                        out.push(' ');
                    } else {
                        out.push('\n');
                    }
                } else if v.tab.contains(&name) {
                    out.push('\t');
                } else if v.br.contains(&name) {
                    out.push(if cells > 0 { ' ' } else { '\n' });
                } else if name == "text:s" {
                    out.push(' ');
                }
            }
            None => {
                if skipping == 0 && inside > 0 {
                    decode_entities(rest, out);
                }
                break;
            }
        }
    }
}

fn decode_entities(s: &str, out: &mut Out) {
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        match after.find(';').filter(|j| *j <= 10) {
            Some(j) => {
                let ent = &after[..j];
                let c = match ent {
                    "amp" => Some('&'),
                    "lt" => Some('<'),
                    "gt" => Some('>'),
                    "quot" => Some('"'),
                    "apos" => Some('\''),
                    _ => ent
                        .strip_prefix("#x")
                        .or_else(|| ent.strip_prefix("#X"))
                        .and_then(|h| u32::from_str_radix(h, 16).ok())
                        .or_else(|| ent.strip_prefix('#').and_then(|d| d.parse().ok()))
                        .and_then(char::from_u32),
                };
                match c {
                    Some(c) => out.push(c),
                    None => out.push_str(&rest[i..i + j + 2]),
                }
                rest = &after[j + 1..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
}

/// The shared string table of a workbook.
fn shared_strings(xml: &str) -> Vec<String> {
    xml.split("<si>")
        .skip(1)
        .map(|si| {
            let mut o = Out::new(32 * 1024);
            xml_text(&format!("<si>{}", &si[..si.find("</si>").unwrap_or(si.len())]), &SHEET, &mut o);
            o.finish()
        })
        .collect()
}

/// A worksheet's cells, one row per line, cells separated by tabs (blank cells kept in position).
fn sheet_text(xml: &str, shared: &[String], out: &mut Out) {
    for row in xml.split("<row").skip(1) {
        let row = &row[..row.find("</row>").unwrap_or(row.len())];
        let mut col = 0usize;
        for cell in row.split("<c ").skip(1) {
            let head = &cell[..cell.find('>').unwrap_or(cell.len())];
            let attr = |k: &str| head.split(&format!("{k}=\"")).nth(1).and_then(|v| v.split('"').next());
            // Column from the reference (e.g. "C7" -> 3) so blank cells keep their place.
            if let Some(r) = attr("r") {
                let letters: String = r.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
                let n = letters.chars().fold(0usize, |n, c| n * 26 + (c.to_ascii_uppercase() as usize - 'A' as usize + 1));
                while col + 1 < n {
                    out.push('\t');
                    col += 1;
                }
            }
            let body = if head.ends_with('/') { "" } else { &cell[head.len() + 1..cell.find("</c>").unwrap_or(cell.len())] };
            let v = body.split("<v>").nth(1).and_then(|v| v.split("</v>").next()).unwrap_or("");
            let mut val = Out::new(32 * 1024);
            match attr("t") {
                Some("s") => val.push_str(v.parse::<usize>().ok().and_then(|i| shared.get(i)).map(String::as_str).unwrap_or("")),
                Some("inlineStr") => xml_text(body, &SHEET, &mut val),
                Some("b") => val.push_str(if v == "1" { "TRUE" } else { "FALSE" }),
                _ => decode_entities(v, &mut val),
            }
            if col > 0 {
                out.push('\t');
            }
            out.push_str(&val.finish().replace(['\n', '\t'], " "));
            col += 1;
        }
        out.push('\n');
    }
}

/// Values of `attr` on every `<elem …>` (e.g. sheet names).
fn attr_values(xml: &str, elem: &str, attr: &str) -> Vec<String> {
    let open = format!("<{elem} ");
    let key = format!("{attr}=\"");
    xml.split(&open)
        .skip(1)
        .filter_map(|t| {
            let t = &t[..t.find('>').unwrap_or(t.len())];
            let v = t.split(&key).nth(1)?;
            let mut o = Out::new(256);
            decode_entities(&v[..v.find('"')?], &mut o);
            Some(o.finish())
        })
        .collect()
}

/// RTF to text: control words dropped, `\par`/`\line` as newlines, `\tab`,
/// `\'hh` (Windows-1252, ASCII range kept) and `\uN` escapes decoded;
/// destination groups (`{\*…}`, fonts, colors, styles, pictures) skipped.
fn rtf_text(rtf: &str, out: &mut Out) {
    let b = rtf.as_bytes();
    let mut i = 0;
    let mut depth = 0usize;
    let mut skip_from: Option<usize> = None;
    let mut uc_skip = 0usize;
    while i < b.len() && !out.full() {
        let c = b[i];
        match c {
            b'{' => {
                depth += 1;
                i += 1;
                // `{\*\dest …}` and known non-text destinations.
                if skip_from.is_none() {
                    let rest = &rtf[i..];
                    if rest.starts_with("\\*") || ["\\fonttbl", "\\colortbl", "\\stylesheet", "\\info", "\\pict", "\\header", "\\footer", "\\object", "\\themedata", "\\datastore", "\\latentstyles"].iter().any(|d| rest.starts_with(d)) {
                        skip_from = Some(depth);
                    }
                }
            }
            b'}' => {
                if skip_from == Some(depth) {
                    skip_from = None;
                }
                depth = depth.saturating_sub(1);
                i += 1;
            }
            b'\\' => {
                i += 1;
                let Some(&n) = b.get(i) else { break };
                if n == b'\'' {
                    let hex = rtf.get(i + 1..i + 3).and_then(|h| u8::from_str_radix(h, 16).ok());
                    if skip_from.is_none() && uc_skip == 0 {
                        if let Some(h) = hex {
                            out.push(if h.is_ascii() { h as char } else { cp1252(h) });
                        }
                    }
                    uc_skip = uc_skip.saturating_sub(1);
                    i += 3;
                } else if n.is_ascii_alphabetic() {
                    let start = i;
                    while i < b.len() && b[i].is_ascii_alphabetic() {
                        i += 1;
                    }
                    let word = &rtf[start..i];
                    let num_start = i;
                    if i < b.len() && (b[i] == b'-' || b[i].is_ascii_digit()) {
                        i += 1;
                        while i < b.len() && b[i].is_ascii_digit() {
                            i += 1;
                        }
                    }
                    let num: Option<i32> = rtf[num_start..i].parse().ok();
                    if i < b.len() && b[i] == b' ' {
                        i += 1;
                    }
                    if skip_from.is_some() {
                        continue;
                    }
                    match word {
                        "par" | "line" | "row" | "sect" | "page" => out.push('\n'),
                        "tab" | "cell" => out.push('\t'),
                        "u" => {
                            if let Some(n) = num {
                                let cp = if n < 0 { (n + 65536) as u32 } else { n as u32 };
                                if let Some(ch) = char::from_u32(cp) {
                                    out.push(ch);
                                }
                                uc_skip = 1;
                            }
                        }
                        _ => {}
                    }
                } else {
                    // Escaped symbol: \{ \} \\ \~ (nbsp) \- (soft hyphen) …
                    if skip_from.is_none() {
                        match n {
                            b'{' | b'}' | b'\\' => out.push(n as char),
                            b'~' => out.push(' '),
                            b'\n' | b'\r' => out.push('\n'),
                            _ => {}
                        }
                    }
                    i += 1;
                }
            }
            b'\r' | b'\n' => i += 1,
            _ => {
                let start = i;
                while i < b.len() && !matches!(b[i], b'{' | b'}' | b'\\' | b'\r' | b'\n') {
                    i += 1;
                }
                if skip_from.is_none() {
                    let mut text = &rtf[start..i];
                    if uc_skip > 0 && !text.is_empty() {
                        let mut cs = text.chars();
                        cs.next();
                        text = cs.as_str();
                        uc_skip = 0;
                    }
                    out.push_str(text);
                }
            }
        }
    }
}

fn cp1252(b: u8) -> char {
    const HIGH: [char; 32] = ['€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ'];
    match b {
        0x80..=0x9f => HIGH[(b - 0x80) as usize],
        _ => char::from(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_file(dir: &Path, name: &str, entries: &[(&str, &str)]) -> std::path::PathBuf {
        let p = dir.join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        for (n, body) in entries {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(body.as_bytes()).unwrap();
        }
        z.finish().unwrap();
        p
    }

    #[test]
    fn docx_paragraphs_tabs_entities_and_skipped_fields() {
        let d = tempfile::tempdir().unwrap();
        let doc = r#"<?xml version="1.0"?><w:document><w:body>
            <w:p><w:r><w:t>Invoice &amp; terms</w:t></w:r><w:r><w:tab/><w:t xml:space="preserve">No. 42</w:t></w:r></w:p>
            <w:p><w:r><w:instrText> PAGE \* MERGEFORMAT </w:instrText><w:t>Caf&#233; &#x2014; total</w:t></w:r></w:p>
            <w:p/><w:p/><w:p><w:r><w:t>Line</w:t><w:br/><w:t>two</w:t><w:delText>gone</w:delText></w:r></w:p>
        </w:body></w:document>"#;
        let p = zip_file(d.path(), "a.docx", &[("word/document.xml", doc), ("word/styles.xml", "<x>style text</x>")]);
        assert_eq!(extract_text(&p, 10_000).unwrap().unwrap(), "Invoice & terms\tNo. 42\nCafé — total\nLine\ntwo");
    }

    #[test]
    fn xlsx_sheets_and_strings_pptx_slides_odt() {
        let d = tempfile::tempdir().unwrap();
        let x = zip_file(
            d.path(),
            "b.xlsx",
            &[
                ("xl/workbook.xml", r#"<workbook><sheets><sheet name="Q1 &amp; Q2" sheetId="1"/><sheet name="Notes" sheetId="2"/></sheets></workbook>"#),
                ("xl/sharedStrings.xml", r#"<sst><si><t>Revenue</t></si><si><r><t>Cost</t></r><r><t>s</t></r><rPh><t>ignored</t></rPh></si></sst>"#),
                ("xl/worksheets/sheet1.xml", r#"<worksheet><sheetData><row r="1"><c r="A1" t="s"><v>0</v></c><c r="C1" t="s"><v>1</v></c></row><row r="2"><c r="A2"><v>12.5</v></c><c r="B2" t="b"><v>1</v></c><c r="C2" t="inlineStr"><is><t>Inline &amp; more</t></is></c></row></sheetData></worksheet>"#),
                ("xl/worksheets/sheet2.xml", r#"<worksheet><sheetData><row r="1"><c r="A1" t="str"><v>formula text</v></c></row></sheetData></worksheet>"#),
            ],
        );
        assert_eq!(extract_text(&x, 10_000).unwrap().unwrap(), "Q1 & Q2\nRevenue\t\tCosts\n12.5\tTRUE\tInline & more\n\nNotes\nformula text");
        let s = zip_file(
            d.path(),
            "c.pptx",
            &[("ppt/slides/slide10.xml", "<p:sld><a:p><a:r><a:t>Ten</a:t></a:r></a:p></p:sld>"), ("ppt/slides/slide2.xml", "<p:sld><a:p><a:r><a:t>Two</a:t></a:r></a:p></p:sld>")],
        );
        assert_eq!(extract_text(&s, 10_000).unwrap().unwrap(), "Slide 1\nTwo\n\nSlide 2\nTen");
        let o = zip_file(d.path(), "d.odt", &[("content.xml", "<office:text><text:h>Title</text:h><text:p>a<text:s/>b<text:tab/>c<office:annotation><text:p>note</text:p></office:annotation></text:p></office:text>")]);
        assert_eq!(extract_text(&o, 10_000).unwrap().unwrap(), "Title\na b\tc");
    }

    #[test]
    fn rtf_text_and_limits() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("e.rtf");
        std::fs::write(&p, r"{\rtf1\ansi{\fonttbl{\f0 Arial;}}{\*\generator Word;}\f0\fs24 Hello \b world\b0\par Caf\'e9 \u8364?5\tab x \{y\}\par}").unwrap();
        assert_eq!(extract_text(&p, 10_000).unwrap().unwrap(), "Hello world\nCafé €5\tx {y}");
        assert_eq!(extract_text(&p, 5).unwrap().unwrap(), "Hello");
        assert_eq!(extract_text(&d.path().join("x.doc"), 10).unwrap(), None);
        assert_eq!(extract_text(&d.path().join("x.txt"), 10).unwrap(), None);
        std::fs::write(d.path().join("bad.docx"), "not a zip").unwrap();
        assert!(extract_text(&d.path().join("bad.docx"), 10).unwrap_err().contains("not a valid docx"));
        assert_eq!(office_kind("XLSM"), Some(OfficeKind::Xlsx));
    }

    /// Real files from Word/Excel/PowerPoint/LibreOffice: set UFOP_DOCTEXT_SAMPLES
    /// to a folder of them and run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_documents() {
        let Ok(dir) = std::env::var("UFOP_DOCTEXT_SAMPLES") else { return };
        let mut files: Vec<_> = std::fs::read_dir(dir).unwrap().filter_map(Result::ok).map(|e| e.path()).collect();
        files.sort();
        for p in files {
            let t = extract_text(&p, 2_000);
            println!("=== {} ===\n{t:?}\n", p.display());
            assert!(matches!(t, Ok(Some(ref s)) if !s.is_empty()) || office_kind(&p.extension().unwrap_or_default().to_string_lossy()).is_none(), "{}", p.display());
        }
    }
}
