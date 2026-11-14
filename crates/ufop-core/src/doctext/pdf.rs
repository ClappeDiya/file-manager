//! Text from PDF files, for the search index and previews.
//!
//! A deliberately small reader: it finds the objects in the file (including
//! those packed in object streams), walks the page tree in order, inflates
//! `FlateDecode` content streams and interprets the text operators (`Tj`,
//! `TJ`, `'`, `"`, line moves). Glyph codes are turned into text through the
//! font's `ToUnicode` CMap, or read as single-byte Latin text for simple
//! fonts without one. Composite fonts without a `ToUnicode` map, text drawn
//! inside form XObjects, and encrypted files give no text.
//!
//! Every read is bounded (file size, inflated size per stream, object count,
//! nesting) so a hostile file cannot exhaust memory or recurse forever.

use super::Out;
use std::collections::HashMap;
use std::io::Read;

/// Largest PDF read.
pub(super) const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Largest inflated size of one stream.
const MAX_STREAM_BYTES: u64 = 16 * 1024 * 1024;
const MAX_OBJECTS: usize = 200_000;
const MAX_DEPTH: usize = 32;
const MAX_PAGES: usize = 10_000;

/// Write the text of the PDF in `raw` to `out`. `Ok(false)` when the file is
/// encrypted (its text can't be read without the password).
pub(super) fn pdf_text(raw: &[u8], out: &mut Out) -> Result<bool, String> {
    if find(&raw[..raw.len().min(1024)], b"%PDF-", 0).is_none() {
        return Err("not a PDF file".into());
    }
    let mut doc = Doc::parse(raw);
    if doc.encrypted {
        return Ok(false);
    }
    doc.expand_object_streams();
    let pages = doc.pages();
    let mut cmaps: HashMap<u32, Option<CMap>> = HashMap::new();
    for (i, page) in pages.iter().enumerate() {
        if out.full() {
            break;
        }
        if i > 0 {
            out.push('\n');
            out.push('\n');
        }
        let fonts = doc.page_fonts(page, &mut cmaps);
        let mut content = Vec::new();
        for r in doc.content_refs(page) {
            if let Some(data) = doc.stream_data(r) {
                content.extend_from_slice(&data);
                content.push(b'\n');
            }
        }
        interpret(&content, &fonts, out);
    }
    Ok(true)
}

// ── Lexer ──

#[derive(Debug, Clone, PartialEq)]
enum Value {
    Num(f64),
    Str(Vec<u8>),
    Name(String),
    Arr(Vec<Value>),
    Dict(HashMap<String, Value>),
    Ref(u32),
    /// Keywords such as `true`, `null`, or an operator in a content stream.
    Op(String),
}

impl Value {
    fn as_dict(&self) -> Option<&HashMap<String, Value>> {
        match self {
            Value::Dict(d) => Some(d),
            _ => None,
        }
    }
    fn as_num(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }
    fn is_name(&self, n: &str) -> bool {
        matches!(self, Value::Name(x) if x == n)
    }
}

enum Tok {
    V(Value),
    ArrOpen,
    ArrClose,
    DictOpen,
    DictClose,
}

fn is_ws(b: u8) -> bool {
    matches!(b, 0 | b'\t' | b'\n' | 0x0c | b'\r' | b' ')
}
fn is_delim(b: u8) -> bool {
    matches!(b, b'(' | b')' | b'<' | b'>' | b'[' | b']' | b'{' | b'}' | b'/' | b'%')
}

struct Lexer<'a> {
    s: &'a [u8],
    pos: usize,
}

impl<'a> Lexer<'a> {
    fn new(s: &'a [u8]) -> Self {
        Self { s, pos: 0 }
    }

    fn skip_ws(&mut self) {
        while self.pos < self.s.len() {
            let b = self.s[self.pos];
            if is_ws(b) {
                self.pos += 1;
            } else if b == b'%' {
                while self.pos < self.s.len() && !matches!(self.s[self.pos], b'\n' | b'\r') {
                    self.pos += 1;
                }
            } else {
                break;
            }
        }
    }

    fn token(&mut self) -> Option<Tok> {
        self.skip_ws();
        let s = self.s;
        let b = *s.get(self.pos)?;
        match b {
            b'(' => Some(Tok::V(Value::Str(self.literal()))),
            b'<' if s.get(self.pos + 1) == Some(&b'<') => {
                self.pos += 2;
                Some(Tok::DictOpen)
            }
            b'<' => Some(Tok::V(Value::Str(self.hex()))),
            b'>' if s.get(self.pos + 1) == Some(&b'>') => {
                self.pos += 2;
                Some(Tok::DictClose)
            }
            b'[' => {
                self.pos += 1;
                Some(Tok::ArrOpen)
            }
            b']' => {
                self.pos += 1;
                Some(Tok::ArrClose)
            }
            b'/' => {
                self.pos += 1;
                let raw = self.regular();
                Some(Tok::V(Value::Name(decode_name(raw))))
            }
            b')' | b'>' | b'{' | b'}' => {
                self.pos += 1;
                Some(Tok::V(Value::Op((b as char).to_string())))
            }
            _ => {
                let raw = self.regular();
                if raw.is_empty() {
                    self.pos += 1;
                    return Some(Tok::V(Value::Op(String::new())));
                }
                let text = String::from_utf8_lossy(raw);
                if matches!(raw[0], b'0'..=b'9' | b'+' | b'-' | b'.') {
                    if let Ok(n) = text.parse::<f64>() {
                        return Some(Tok::V(Value::Num(n)));
                    }
                }
                Some(Tok::V(Value::Op(text.into_owned())))
            }
        }
    }

    fn regular(&mut self) -> &'a [u8] {
        let start = self.pos;
        while self.pos < self.s.len() && !is_ws(self.s[self.pos]) && !is_delim(self.s[self.pos]) {
            self.pos += 1;
        }
        &self.s[start..self.pos]
    }

    fn literal(&mut self) -> Vec<u8> {
        let s = self.s;
        self.pos += 1;
        let (mut out, mut depth) = (Vec::new(), 1usize);
        while self.pos < s.len() {
            let b = s[self.pos];
            self.pos += 1;
            match b {
                b'(' => {
                    depth += 1;
                    out.push(b);
                }
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    out.push(b);
                }
                b'\\' => {
                    let Some(&e) = s.get(self.pos) else { break };
                    self.pos += 1;
                    match e {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b't' => out.push(b'\t'),
                        b'b' => out.push(8),
                        b'f' => out.push(12),
                        b'0'..=b'7' => {
                            let mut v = (e - b'0') as u32;
                            for _ in 0..2 {
                                match s.get(self.pos) {
                                    Some(&d @ b'0'..=b'7') => {
                                        v = v * 8 + (d - b'0') as u32;
                                        self.pos += 1;
                                    }
                                    _ => break,
                                }
                            }
                            out.push(v as u8);
                        }
                        b'\r' => {
                            if s.get(self.pos) == Some(&b'\n') {
                                self.pos += 1;
                            }
                        }
                        b'\n' => {}
                        other => out.push(other),
                    }
                }
                _ => out.push(b),
            }
        }
        out
    }

    fn hex(&mut self) -> Vec<u8> {
        self.pos += 1;
        let mut digits = Vec::new();
        while self.pos < self.s.len() {
            let b = self.s[self.pos];
            self.pos += 1;
            if b == b'>' {
                break;
            }
            if let Some(d) = (b as char).to_digit(16) {
                digits.push(d as u8);
            }
        }
        if digits.len() % 2 == 1 {
            digits.push(0);
        }
        digits.chunks(2).map(|p| (p[0] << 4) | p[1]).collect()
    }

    /// The next complete object (arrays and dictionaries included; `a b R` is a reference).
    fn value(&mut self, depth: usize) -> Option<Value> {
        let tok = self.token()?;
        self.value_from(tok, depth)
    }

    fn value_from(&mut self, tok: Tok, depth: usize) -> Option<Value> {
        if depth > MAX_DEPTH {
            return None;
        }
        match tok {
            Tok::V(Value::Num(a)) => {
                let save = self.pos;
                if let Some(Tok::V(Value::Num(_))) = self.token() {
                    if let Some(Tok::V(Value::Op(op))) = self.token() {
                        if op == "R" && a >= 0.0 {
                            return Some(Value::Ref(a as u32));
                        }
                    }
                }
                self.pos = save;
                Some(Value::Num(a))
            }
            Tok::V(v) => Some(v),
            Tok::ArrOpen => {
                let mut items = Vec::new();
                loop {
                    match self.token()? {
                        Tok::ArrClose => return Some(Value::Arr(items)),
                        t => items.push(self.value_from(t, depth + 1)?),
                    }
                }
            }
            Tok::DictOpen => {
                let mut d = HashMap::new();
                loop {
                    match self.token()? {
                        Tok::DictClose => return Some(Value::Dict(d)),
                        Tok::V(Value::Name(k)) => {
                            let v = self.value(depth + 1)?;
                            d.insert(k, v);
                        }
                        _ => {}
                    }
                }
            }
            Tok::ArrClose | Tok::DictClose => Some(Value::Op(String::new())),
        }
    }
}

fn decode_name(raw: &[u8]) -> String {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'#' && i + 2 < raw.len() {
            if let Ok(b) = u8::from_str_radix(&String::from_utf8_lossy(&raw[i + 1..i + 3]), 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(raw[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if from >= hay.len() || needle.is_empty() {
        return None;
    }
    hay[from..].windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

// ── Objects ──

struct Obj {
    dict: Value,
    stream: Option<Vec<u8>>,
}

struct Doc {
    objects: HashMap<u32, Obj>,
    encrypted: bool,
    root: Option<u32>,
}

impl Doc {
    fn parse(raw: &[u8]) -> Self {
        let header = regex::bytes::Regex::new(r"(?-u)(\d{1,10})\s+\d{1,5}\s+obj\b").expect("static pattern");
        let mut objects = HashMap::new();
        let mut at = 0;
        while objects.len() < MAX_OBJECTS {
            let Some(m) = header.captures_at(raw, at) else { break };
            let whole = m.get(0).expect("match");
            let num: u32 = String::from_utf8_lossy(&m[1]).parse().unwrap_or(0);
            let body = whole.end();
            let end = find(raw, b"endobj", body).unwrap_or(raw.len());
            let stream_kw = find(&raw[..end.min(raw.len())], b"stream", body).filter(|&p| !raw[..p].ends_with(b"end"));
            let mut lexer = Lexer::new(&raw[body..stream_kw.unwrap_or(end)]);
            let dict = lexer.value(0).unwrap_or(Value::Op(String::new()));
            let (stream, next) = match stream_kw {
                Some(p) => {
                    let mut start = p + b"stream".len();
                    if raw.get(start) == Some(&b'\r') {
                        start += 1;
                    }
                    if raw.get(start) == Some(&b'\n') {
                        start += 1;
                    }
                    let by_length = dict.as_dict().and_then(|d| d.get("Length")).and_then(Value::as_num).map(|l| start + l as usize).filter(|&e| {
                        e <= raw.len() && {
                            let mut k = e;
                            while k < raw.len() && is_ws(raw[k]) {
                                k += 1;
                            }
                            raw[k..].starts_with(b"endstream")
                        }
                    });
                    let data_end = by_length.or_else(|| find(raw, b"endstream", start)).unwrap_or(raw.len());
                    let after = find(raw, b"endobj", data_end).map(|e| e + 6).unwrap_or(raw.len());
                    (Some(raw[start.min(data_end)..data_end].to_vec()), after)
                }
                None => (None, end + 6),
            };
            objects.insert(num, Obj { dict, stream });
            at = next.max(whole.end());
            if at >= raw.len() {
                break;
            }
        }
        // The trailer (classic or in the cross-reference stream) names the catalog and any encryption.
        let mut root = None;
        let mut encrypted = false;
        let mut pos = 0;
        while let Some(t) = find(raw, b"trailer", pos) {
            if let Some(Value::Dict(d)) = Lexer::new(&raw[t + 7..]).value(0) {
                if let Some(Value::Ref(r)) = d.get("Root") {
                    root = Some(*r);
                }
                encrypted |= d.contains_key("Encrypt");
            }
            pos = t + 7;
        }
        for o in objects.values() {
            if let Some(d) = o.dict.as_dict() {
                if d.get("Type").is_some_and(|t| t.is_name("XRef")) {
                    encrypted |= d.contains_key("Encrypt");
                    if let Some(Value::Ref(r)) = d.get("Root") {
                        root = root.or(Some(*r));
                    }
                }
            }
        }
        Doc { objects, encrypted, root }
    }

    /// Objects packed in `/Type /ObjStm` streams join the table (objects
    /// written directly in the file win, as they are the later revision).
    fn expand_object_streams(&mut self) {
        let streams: Vec<u32> = self.objects.iter().filter(|(_, o)| o.dict.as_dict().is_some_and(|d| d.get("Type").is_some_and(|t| t.is_name("ObjStm")))).map(|(n, _)| *n).collect();
        for n in streams {
            let Some(data) = self.stream_data(n) else { continue };
            let Some(d) = self.objects.get(&n).and_then(|o| o.dict.as_dict()) else { continue };
            let count = d.get("N").and_then(Value::as_num).unwrap_or(0.0) as usize;
            let first = d.get("First").and_then(Value::as_num).unwrap_or(0.0) as usize;
            if first > data.len() {
                continue;
            }
            let mut head = Lexer::new(&data[..first]);
            let mut entries = Vec::new();
            for _ in 0..count.min(MAX_OBJECTS) {
                match (head.token(), head.token()) {
                    (Some(Tok::V(Value::Num(num))), Some(Tok::V(Value::Num(off)))) => entries.push((num as u32, first + off as usize)),
                    _ => break,
                }
            }
            for (num, off) in entries {
                if off >= data.len() || self.objects.len() >= MAX_OBJECTS {
                    continue;
                }
                if let Some(v) = Lexer::new(&data[off..]).value(0) {
                    self.objects.entry(num).or_insert(Obj { dict: v, stream: None });
                }
            }
        }
    }

    /// Follow references (a bounded number of hops).
    fn resolve<'a>(&'a self, v: &'a Value) -> Option<&'a Value> {
        let mut cur = v;
        for _ in 0..MAX_DEPTH {
            match cur {
                Value::Ref(r) => cur = &self.objects.get(r)?.dict,
                other => return Some(other),
            }
        }
        None
    }

    fn dict_of<'a>(&'a self, v: &'a Value) -> Option<&'a HashMap<String, Value>> {
        self.resolve(v)?.as_dict()
    }

    /// A stream's decoded bytes: unfiltered or `FlateDecode` (others give nothing).
    fn stream_data(&self, num: u32) -> Option<Vec<u8>> {
        let o = self.objects.get(&num)?;
        let raw = o.stream.as_ref()?;
        let filters: Vec<String> = match o.dict.as_dict().and_then(|d| d.get("Filter")).and_then(|f| self.resolve(f)) {
            None => vec![],
            Some(Value::Name(n)) => vec![n.clone()],
            Some(Value::Arr(a)) => a.iter().filter_map(|v| if let Value::Name(n) = v { Some(n.clone()) } else { None }).collect(),
            Some(_) => return None,
        };
        let mut data = raw.clone();
        for f in filters {
            data = match f.as_str() {
                "FlateDecode" | "Fl" => {
                    let mut out = Vec::new();
                    // A truncated stream still yields what inflated before the error.
                    let _ = flate2::read::ZlibDecoder::new(&data[..]).take(MAX_STREAM_BYTES).read_to_end(&mut out);
                    if out.is_empty() {
                        return None;
                    }
                    out
                }
                "ASCII85Decode" | "A85" => ascii85(&data)?,
                "ASCIIHexDecode" | "AHx" => {
                    let mut hex = b"<".to_vec();
                    hex.extend_from_slice(&data);
                    Lexer::new(&hex).hex()
                }
                _ => return None,
            };
        }
        Some(data)
    }

    /// Pages in reading order, each with its inherited resources merged in.
    fn pages(&self) -> Vec<HashMap<String, Value>> {
        let mut pages = Vec::new();
        let tree = self.root.and_then(|r| self.objects.get(&r)).and_then(|o| o.dict.as_dict()).and_then(|c| c.get("Pages")).cloned();
        if let Some(t) = tree {
            self.walk(&t, None, 0, &mut pages);
        }
        if pages.is_empty() {
            // No usable page tree: every page object, in object order.
            let mut nums: Vec<&u32> = self.objects.keys().collect();
            nums.sort();
            for n in nums {
                if let Some(d) = self.objects[n].dict.as_dict() {
                    if d.get("Type").is_some_and(|t| t.is_name("Page")) && pages.len() < MAX_PAGES {
                        pages.push(d.clone());
                    }
                }
            }
        }
        pages
    }

    fn walk(&self, node: &Value, inherited: Option<&Value>, depth: usize, pages: &mut Vec<HashMap<String, Value>>) {
        if depth > MAX_DEPTH || pages.len() >= MAX_PAGES {
            return;
        }
        let Some(d) = self.dict_of(node) else { return };
        let resources = d.get("Resources").or(inherited);
        if let Some(Value::Arr(kids)) = d.get("Kids").and_then(|k| self.resolve(k)) {
            for k in kids {
                self.walk(k, resources, depth + 1, pages);
            }
        } else {
            let mut page = d.clone();
            if let Some(r) = resources {
                page.entry("Resources".into()).or_insert_with(|| r.clone());
            }
            pages.push(page);
        }
    }

    fn content_refs(&self, page: &HashMap<String, Value>) -> Vec<u32> {
        match page.get("Contents") {
            Some(Value::Ref(r)) => match self.objects.get(r).map(|o| &o.dict) {
                Some(Value::Arr(a)) if self.objects[r].stream.is_none() => a.iter().filter_map(|v| if let Value::Ref(x) = v { Some(*x) } else { None }).collect(),
                _ => vec![*r],
            },
            Some(Value::Arr(a)) => a.iter().filter_map(|v| if let Value::Ref(x) = v { Some(*x) } else { None }).collect(),
            _ => vec![],
        }
    }

    /// `FirstChar` + `Widths` of a simple font (the standard 14 fonts have none: half an em is assumed).
    fn simple_widths(&self, fd: &HashMap<String, Value>) -> (HashMap<u32, f64>, f64) {
        let first = fd.get("FirstChar").and_then(|v| self.resolve(v)).and_then(Value::as_num).unwrap_or(0.0) as u32;
        let mut widths = HashMap::new();
        if let Some(Value::Arr(w)) = fd.get("Widths").and_then(|v| self.resolve(v)) {
            for (i, v) in w.iter().enumerate() {
                if let Some(n) = self.resolve(v).and_then(Value::as_num) {
                    widths.insert(first + i as u32, n);
                }
            }
        }
        (widths, 500.0)
    }

    /// `W` / `DW` of a composite font's descendant CID font.
    fn cid_widths(&self, fd: &HashMap<String, Value>) -> (HashMap<u32, f64>, f64) {
        let mut widths = HashMap::new();
        let desc = match fd.get("DescendantFonts").and_then(|v| self.resolve(v)) {
            Some(Value::Arr(a)) => a.first().and_then(|d| self.dict_of(d)),
            _ => None,
        };
        let Some(desc) = desc else { return (widths, 1000.0) };
        let dw = desc.get("DW").and_then(|v| self.resolve(v)).and_then(Value::as_num).unwrap_or(1000.0);
        if let Some(Value::Arr(w)) = desc.get("W").and_then(|v| self.resolve(v)) {
            let mut i = 0;
            while i < w.len() && widths.len() < 65_536 {
                let Some(c) = w[i].as_num() else { break };
                match (w.get(i + 1), w.get(i + 2)) {
                    (Some(Value::Arr(list)), _) => {
                        for (k, v) in list.iter().enumerate() {
                            if let Some(n) = v.as_num() {
                                widths.insert(c as u32 + k as u32, n);
                            }
                        }
                        i += 2;
                    }
                    (Some(Value::Num(last)), Some(Value::Num(n))) => {
                        for code in (c as u32)..=(*last as u32).min(c as u32 + 65_535) {
                            widths.insert(code, *n);
                        }
                        i += 3;
                    }
                    _ => break,
                }
            }
        }
        (widths, dw)
    }

    /// The page's fonts by resource name, with their text decoders.
    fn page_fonts(&self, page: &HashMap<String, Value>, cmaps: &mut HashMap<u32, Option<CMap>>) -> HashMap<String, Font> {
        let mut fonts = HashMap::new();
        let Some(res) = page.get("Resources").and_then(|r| self.dict_of(r)) else { return fonts };
        let Some(font_dict) = res.get("Font").and_then(|f| self.dict_of(f)) else { return fonts };
        for (name, f) in font_dict {
            let Some(fd) = self.dict_of(f) else { continue };
            let cmap = match fd.get("ToUnicode") {
                Some(Value::Ref(r)) => cmaps.entry(*r).or_insert_with(|| self.stream_data(*r).map(|d| CMap::parse(&d))).clone(),
                _ => None,
            };
            let composite = fd.get("Subtype").is_some_and(|s| s.is_name("Type0"));
            let (widths, default_width) = if composite { self.cid_widths(fd) } else { self.simple_widths(fd) };
            fonts.insert(name.clone(), Font { cmap, composite, widths, default_width });
        }
        fonts
    }
}

// ── Fonts and CMaps ──

#[derive(Clone)]
struct CMap {
    /// Bytes per character code (from the codespace ranges).
    code_len: usize,
    map: HashMap<u32, String>,
}

impl CMap {
    fn parse(data: &[u8]) -> CMap {
        let mut lx = Lexer::new(data);
        let mut cm = CMap { code_len: 0, map: HashMap::new() };
        let mut stack: Vec<Value> = Vec::new();
        while let Some(tok) = lx.token() {
            let Some(v) = lx.value_from(tok, 0) else { break };
            let Value::Op(op) = &v else {
                stack.push(v);
                continue;
            };
            match op.as_str() {
                "endcodespacerange" => {
                    if let Some(Value::Str(s)) = stack.first() {
                        cm.code_len = s.len().clamp(1, 4);
                    }
                }
                "endbfchar" => {
                    for pair in stack.chunks(2) {
                        if let [Value::Str(src), Value::Str(dst)] = pair {
                            cm.map.insert(code(src), utf16be(dst));
                        }
                    }
                }
                "endbfrange" => {
                    for triple in stack.chunks(3) {
                        let [Value::Str(lo), Value::Str(hi), dst] = triple else { continue };
                        let (lo, hi) = (code(lo), code(hi));
                        if hi < lo || hi - lo > 0xFFFF {
                            continue;
                        }
                        match dst {
                            Value::Str(d) => {
                                let units: Vec<u16> = d.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
                                for (i, c) in (lo..=hi).enumerate() {
                                    let mut u = units.clone();
                                    if let Some(last) = u.last_mut() {
                                        *last = last.wrapping_add(i as u16);
                                    }
                                    cm.map.insert(c, String::from_utf16_lossy(&u));
                                }
                            }
                            Value::Arr(a) => {
                                for (c, d) in (lo..=hi).zip(a) {
                                    if let Value::Str(d) = d {
                                        cm.map.insert(c, utf16be(d));
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            stack.clear();
        }
        if cm.code_len == 0 {
            cm.code_len = if cm.map.keys().any(|&k| k > 0xFF) { 2 } else { 1 };
        }
        cm
    }
}

/// A PDF text string: UTF-16BE with a byte order mark, else PDFDocEncoding (read as Latin-1).
fn text_string(b: &[u8]) -> String {
    match b.strip_prefix(&[0xfe, 0xff]) {
        Some(rest) => utf16be(rest),
        None => b.iter().map(|&c| c as char).collect(),
    }
}

fn code(bytes: &[u8]) -> u32 {
    bytes.iter().take(4).fold(0, |acc, &b| (acc << 8) | b as u32)
}

fn utf16be(b: &[u8]) -> String {
    let units: Vec<u16> = b.chunks(2).map(|c| u16::from_be_bytes([c[0], *c.get(1).unwrap_or(&0)])).collect();
    String::from_utf16_lossy(&units)
}

struct Font {
    cmap: Option<CMap>,
    /// A Type0 font: codes are glyph ids, unreadable without a CMap.
    composite: bool,
    /// Glyph advance per code, in thousandths of the font size.
    widths: HashMap<u32, f64>,
    default_width: f64,
}

impl Font {
    fn code_len(&self) -> usize {
        match &self.cmap {
            Some(cm) => cm.code_len,
            None if self.composite => 2,
            None => 1,
        }
    }
    fn width(&self, code: u32) -> f64 {
        self.widths.get(&code).copied().unwrap_or(self.default_width)
    }
}

/// ASCII base-85 (`<~ … ~>` framing optional).
fn ascii85(data: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 4 / 5);
    let (mut group, mut n) = (0u64, 0usize);
    let body = data.strip_prefix(b"<~").unwrap_or(data);
    for &b in body {
        match b {
            b'~' => break,
            b'z' if n == 0 => out.extend_from_slice(&[0; 4]),
            b'!'..=b'u' => {
                group = group * 85 + (b - b'!') as u64;
                n += 1;
                if n == 5 {
                    out.extend_from_slice(&(group as u32).to_be_bytes());
                    (group, n) = (0, 0);
                }
            }
            _ if is_ws(b) => {}
            _ => return None,
        }
    }
    if n > 1 {
        for _ in n..5 {
            group = group * 85 + 84;
        }
        out.extend_from_slice(&(group as u32).to_be_bytes()[..n - 1]);
    }
    Some(out)
}

/// Windows-1252's printable 0x80–0x9F (what simple fonts usually encode).
const CP1252_HIGH: [char; 32] = [
    '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž', '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}', 'ž', 'Ÿ',
];

/// Write the text of string `s` shown in `font`; returns its advance in thousandths of the font size.
fn decode(s: &[u8], font: Option<&Font>, out: &mut Out) -> f64 {
    let Some(font) = font else {
        for &b in s {
            push_byte(b, out);
        }
        return 500.0 * s.len() as f64;
    };
    let mut advance = 0.0;
    for c in s.chunks(font.code_len()) {
        let k = code(c);
        advance += font.width(k);
        match &font.cmap {
            Some(cm) => match cm.map.get(&k) {
                // Private-use code points are glyph names nobody can search for.
                Some(t) => out.push_str(&t.chars().filter(|ch| !('\u{e000}'..='\u{f8ff}').contains(ch)).collect::<String>()),
                None if c.len() == 1 => push_byte(c[0], out),
                None => {}
            },
            None if font.composite => {}
            None => push_byte(c[0], out),
        }
    }
    advance
}

fn push_byte(b: u8, out: &mut Out) {
    match b {
        b'\n' | b'\r' | b'\t' => out.push(' '),
        0x20..=0x7e | 0xa0..=0xff => out.push(b as char),
        0x80..=0x9f => out.push(CP1252_HIGH[(b - 0x80) as usize]),
        _ => {}
    }
}

// ── Content streams ──

fn interpret(content: &[u8], fonts: &HashMap<String, Font>, out: &mut Out) {
    let mut lx = Lexer::new(content);
    let mut ops: Vec<Value> = Vec::new();
    let mut font: Option<&Font> = None;
    let mut size = 12.0_f64;
    // Text line origin and how far text has advanced from it (text space units).
    let (mut line_x, mut line_y, mut advanced) = (0.0_f64, 0.0_f64, 0.0_f64);
    let mut started = false;
    // Marked content: an `ActualText` span replaces the glyphs inside it.
    let (mut depth, mut actual_at): (usize, Option<usize>) = (0, None);
    let mut scratch = Out::new(usize::MAX);
    macro_rules! sink {
        () => {
            if actual_at.is_some() {
                &mut scratch
            } else {
                &mut *out
            }
        };
    }
    let space = |out: &mut Out| {
        if !out.s.is_empty() && !out.s.ends_with([' ', '\n']) {
            out.push(' ');
        }
    };
    // A move to (x, y): a new line when y changes, a word gap when x jumps past the text.
    let mut move_to = |x: f64, y: f64, line_x: &mut f64, line_y: &mut f64, advanced: &mut f64, size: f64, out: &mut Out| {
        if started {
            if (y - *line_y).abs() > size.abs() * 0.5 {
                if !out.s.ends_with('\n') {
                    out.push('\n');
                }
            } else if x - (*line_x + *advanced) > size.abs() * 0.15 || x < *line_x + *advanced - size.abs() * 2.0 {
                space(out);
            }
        }
        started = true;
        (*line_x, *line_y, *advanced) = (x, y, 0.0);
    };
    while let Some(tok) = lx.token() {
        if out.full() {
            return;
        }
        let Some(v) = lx.value_from(tok, 0) else { break };
        let Value::Op(op) = v else {
            ops.push(v);
            continue;
        };
        let num = |i: usize| ops.len().checked_sub(i + 1).and_then(|k| ops[k].as_num());
        match op.as_str() {
            "Tf" => {
                if let Some(Value::Name(n)) = ops.iter().rev().nth(1) {
                    font = fonts.get(n);
                }
                size = num(0).unwrap_or(size);
            }
            "Tj" => {
                if let Some(Value::Str(s)) = ops.last() {
                    advanced += decode(s, font, sink!()) / 1000.0 * size;
                }
            }
            "'" | "\"" => {
                out.push('\n');
                if let Some(Value::Str(s)) = ops.last() {
                    advanced = decode(s, font, sink!()) / 1000.0 * size;
                }
            }
            "TJ" => {
                if let Some(Value::Arr(a)) = ops.last() {
                    for item in a {
                        match item {
                            Value::Str(s) => advanced += decode(s, font, sink!()) / 1000.0 * size,
                            Value::Num(n) => {
                                // A large negative adjustment is a gap between words.
                                if *n < -200.0 {
                                    space(out);
                                }
                                advanced -= n / 1000.0 * size;
                            }
                            _ => {}
                        }
                    }
                }
            }
            "Td" | "TD" => {
                let (tx, ty) = (num(1).unwrap_or(0.0), num(0).unwrap_or(0.0));
                let (x, y) = (line_x + tx, line_y + ty);
                move_to(x, y, &mut line_x, &mut line_y, &mut advanced, size, out);
            }
            "T*" => {
                out.push('\n');
                advanced = 0.0;
            }
            "Tm" => {
                let (x, y) = (num(1).unwrap_or(0.0), num(0).unwrap_or(0.0));
                move_to(x, y, &mut line_x, &mut line_y, &mut advanced, size, out);
            }
            "BMC" => depth += 1,
            "BDC" => {
                depth += 1;
                let text = match ops.last() {
                    Some(Value::Dict(d)) => d.get("ActualText"),
                    _ => None,
                };
                if let (Some(Value::Str(t)), None) = (text, actual_at) {
                    out.push_str(&text_string(t));
                    actual_at = Some(depth);
                }
            }
            "EMC" => {
                if actual_at == Some(depth) {
                    actual_at = None;
                    scratch = Out::new(usize::MAX);
                }
                depth = depth.saturating_sub(1);
            }
            "ID" => {
                // Inline image data runs until `EI` after whitespace.
                let s = lx.s;
                let mut p = lx.pos + 1;
                while p + 2 < s.len() && !(is_ws(s[p]) && &s[p + 1..p + 3] == b"EI" && s.get(p + 3).map_or(true, |&b| is_ws(b))) {
                    p += 1;
                }
                lx.pos = (p + 3).min(s.len());
            }
            _ => {}
        }
        ops.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zlib(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    fn stream(dict: &str, data: &[u8]) -> Vec<u8> {
        let mut v = format!("<< {dict} /Length {} >>\nstream\n", data.len()).into_bytes();
        v.extend_from_slice(data);
        v.extend_from_slice(b"\nendstream");
        v
    }

    /// A PDF from numbered object bodies (no cross-reference table: the reader doesn't need one).
    fn pdf(objects: &[(u32, Vec<u8>)], trailer: &str) -> Vec<u8> {
        let mut v = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\n".to_vec();
        for (n, body) in objects {
            v.extend_from_slice(format!("{n} 0 obj\n").as_bytes());
            v.extend_from_slice(body);
            v.extend_from_slice(b"\nendobj\n");
        }
        v.extend_from_slice(format!("trailer\n<< {trailer} >>\n%%EOF\n").as_bytes());
        v
    }

    fn text(raw: &[u8]) -> Option<String> {
        let mut out = Out::new(100_000);
        pdf_text(raw, &mut out).unwrap().then(|| out.finish())
    }

    #[test]
    fn simple_fonts_pages_in_tree_order_and_string_escapes() {
        let page1 = b"BT /F1 12 Tf 72 700 Td (Hello \\(PDF\\) caf\\351) Tj 0 -14 Td [(Second)-250(line)] TJ ET".to_vec();
        let page2 = zlib(b"BT /F1 12 Tf 1 0 0 1 72 700 Tm (Page two) Tj T* (\\224quoted\\224) Tj ET");
        let raw = pdf(
            &[
                (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
                // Kids out of object order: the tree decides reading order.
                (2, b"<< /Type /Pages /Kids [4 0 R 3 0 R] /Count 2 /Resources << /Font << /F1 7 0 R >> >> >>".to_vec()),
                (3, b"<< /Type /Page /Parent 2 0 R /Contents 6 0 R >>".to_vec()),
                (4, b"<< /Type /Page /Parent 2 0 R /Contents [5 0 R] >>".to_vec()),
                (5, stream("", &page1)),
                (6, stream("/Filter /FlateDecode", &page2)),
                (7, b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec()),
            ],
            "/Root 1 0 R",
        );
        assert_eq!(text(&raw).unwrap(), "Hello (PDF) café\nSecond line\n\nPage two\n\u{201d}quoted\u{201d}");
    }

    #[test]
    fn composite_fonts_through_to_unicode_with_word_gaps_from_widths() {
        let cmap = b"/CIDInit /ProcSet findresource begin begincmap\n1 begincodespacerange <0000> <FFFF> endcodespacerange\n\
2 beginbfchar <0001> <0048> <0002> <00E9> endbfchar\n\
2 beginbfrange <0010> <0012> <0061> <0020> <0021> [<D83DDE00> <0066006C>] endbfrange\nendcmap end";
        // "Hé" then each glyph of "abc" placed separately, a word gap, then the ligature.
        let content = zlib(b"BT /F2 10 Tf 1 0 0 1 50 600 Tm <00010002> Tj 14 0 Td <0010> Tj 5 0 Td <0011> Tj 5 0 Td <0012> Tj 20 0 Td <00200021> Tj ET");
        let raw = pdf(
            &[
                (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
                (2, b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec()),
                (3, b"<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F2 5 0 R >> >> >>".to_vec()),
                (4, stream("/Filter [/FlateDecode]", &content)),
                (5, b"<< /Type /Font /Subtype /Type0 /Encoding /Identity-H /ToUnicode 6 0 R /DescendantFonts [7 0 R] >>".to_vec()),
                (6, stream("/Filter /FlateDecode", &zlib(cmap))),
                (7, b"<< /Type /Font /Subtype /CIDFontType2 /DW 500 /W [1 [600 600] 16 18 500] >>".to_vec()),
            ],
            "/Root 1 0 R",
        );
        // Widths: "Hé" advances 12, so a 14 move leaves a gap; 5-unit moves after 5-unit glyphs leave none; 20 after 5 does.
        assert_eq!(text(&raw).unwrap(), "Hé abc \u{1F600}fl");
    }

    #[test]
    fn objects_inside_object_streams_and_ascii85() {
        let content = b"BT /F1 9 Tf 10 10 Td (packed away) Tj ET";
        let objs = "<< /Type /Pages /Kids [3 0 R] /Count 1 >> << /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> << /Type /Font /Subtype /Type1 /BaseFont /Times-Roman >>";
        let first_len = "<< /Type /Pages /Kids [3 0 R] /Count 1 >> ".len();
        let second_len = "<< /Type /Page /Parent 2 0 R /Contents 4 0 R /Resources << /Font << /F1 5 0 R >> >> >> ".len();
        let header = format!("2 0 3 {first_len} 5 {} ", first_len + second_len);
        let packed = format!("{header}{objs}");
        let raw = pdf(
            &[
                (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
                (4, stream("/Filter [/ASCII85Decode /FlateDecode]", &a85(&zlib(content)))),
                (8, stream(&format!("/Type /ObjStm /N 3 /First {} /Filter /FlateDecode", header.len()), &zlib(packed.as_bytes()))),
            ],
            "/Root 1 0 R",
        );
        assert_eq!(text(&raw).unwrap(), "packed away");
    }

    fn a85(data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        for chunk in data.chunks(4) {
            let mut b = [0u8; 4];
            b[..chunk.len()].copy_from_slice(chunk);
            let mut n = u32::from_be_bytes(b) as u64;
            let mut digits = [0u8; 5];
            for d in digits.iter_mut().rev() {
                *d = (n % 85) as u8 + b'!';
                n /= 85;
            }
            out.extend_from_slice(&digits[..chunk.len() + 1]);
        }
        out.extend_from_slice(b"~>");
        out
    }

    #[test]
    fn ascii85_round_trips_partial_groups_and_z() {
        for data in [&b""[..], b"a", b"abcd", b"hello, world!"] {
            assert_eq!(ascii85(&a85(data)).unwrap(), data);
        }
        assert_eq!(ascii85(b"z~>").unwrap(), vec![0; 4]);
        assert!(ascii85(b"\x7f").is_none());
    }

    #[test]
    fn encrypted_files_give_no_text_and_non_pdfs_are_errors() {
        let raw = pdf(&[(1, b"<< /Type /Catalog >>".to_vec())], "/Root 1 0 R /Encrypt 9 0 R");
        let mut out = Out::new(100);
        assert!(!pdf_text(&raw, &mut out).unwrap());
        assert!(pdf_text(b"PK\x03\x04 not a pdf", &mut out).is_err());
    }

    #[test]
    fn hostile_input_is_bounded() {
        // Deep nesting, a reference loop, an unterminated string and a stream that isn't zlib.
        let deep = format!("<< /Type /Pages /Kids [3 0 R] /X {}{} >>", "[".repeat(10_000), "]".repeat(10_000));
        let raw = pdf(
            &[
                (1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()),
                (2, deep.into_bytes()),
                (3, b"<< /Type /Page /Contents 4 0 R /Resources 3 0 R >>".to_vec()),
                (4, stream("/Filter /FlateDecode", b"garbage")),
                (5, b"(never closed".to_vec()),
            ],
            "/Root 1 0 R",
        );
        let mut out = Out::new(100);
        assert!(pdf_text(&raw, &mut out).unwrap());
        // The output limit stops a long page.
        let long = format!("BT /F1 1 Tf ({}) Tj ET", "x".repeat(5000));
        let raw = pdf(
            &[(1, b"<< /Type /Catalog /Pages 2 0 R >>".to_vec()), (2, b"<< /Type /Pages /Kids [3 0 R] >>".to_vec()), (3, b"<< /Type /Page /Contents 4 0 R >>".to_vec()), (4, stream("", long.as_bytes()))],
            "/Root 1 0 R",
        );
        let mut out = Out::new(100);
        pdf_text(&raw, &mut out).unwrap();
        assert_eq!(out.finish().chars().count(), 100);
    }
}
