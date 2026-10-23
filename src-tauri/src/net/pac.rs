//! Proxy auto-config (PAC) files, evaluated with QuickJS.
//!
//! The standard helper functions are provided (`isPlainHostName`,
//! `dnsDomainIs`, `shExpMatch`, `isInNet`, `dnsResolve`, `myIpAddress`,
//! `weekdayRange`, `timeRange`…); `dateRange` is treated as always true.
//! Results are cached per host for a few minutes.

use rquickjs::{Context, Function, Runtime};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

const CACHE_FOR: Duration = Duration::from_secs(300);
const MAX_PAC_BYTES: usize = 1024 * 1024;

const HELPERS: &str = r#"
function isPlainHostName(h) { return h.indexOf('.') < 0; }
function dnsDomainIs(h, d) { return h.length >= d.length && h.substring(h.length - d.length) == d; }
function localHostOrDomainIs(h, hd) { return h == hd || hd.lastIndexOf(h + '.', 0) == 0; }
// Rust `None` arrives as undefined; PAC scripts expect null.
function dnsResolve(h) { var r = __dnsResolve(h); return r == null ? null : r; }
function isResolvable(h) { return dnsResolve(h) !== null; }
function myIpAddress() { return __myIp(); }
function dnsDomainLevels(h) { return h.split('.').length - 1; }
function shExpMatch(s, p) {
  var re = '^' + p.replace(/[.+^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*').replace(/\?/g, '.') + '$';
  return new RegExp(re).test(s);
}
function __addr(ip) { var b = ip.split('.'); return ((b[0] << 24) | (b[1] << 16) | (b[2] << 8) | b[3]) >>> 0; }
function isInNet(ip, pat, mask) {
  if (!/^\d+\.\d+\.\d+\.\d+$/.test(ip)) { ip = dnsResolve(ip); if (ip == null) return false; }
  return (__addr(ip) & __addr(mask)) === (__addr(pat) & __addr(mask));
}
var __days = ['SUN', 'MON', 'TUE', 'WED', 'THU', 'FRI', 'SAT'];
function weekdayRange(a, b, gmt) {
  if (b === 'GMT') { gmt = b; b = undefined; }
  var d = new Date(); var t = gmt === 'GMT' ? d.getUTCDay() : d.getDay();
  var x = __days.indexOf(a), y = b === undefined ? x : __days.indexOf(b);
  if (x < 0 || y < 0) return false;
  return x <= y ? (t >= x && t <= y) : (t >= x || t <= y);
}
function timeRange() {
  var a = Array.prototype.slice.call(arguments); var gmt = a[a.length - 1] === 'GMT'; if (gmt) a.pop();
  var d = new Date(); var h = gmt ? d.getUTCHours() : d.getHours(), m = gmt ? d.getUTCMinutes() : d.getMinutes();
  var now = h * 60 + m;
  if (a.length == 1) return h == a[0];
  var s, e;
  if (a.length == 2) { s = a[0] * 60; e = a[1] * 60 - 1; } else if (a.length >= 4) { s = a[0] * 60 + a[1]; e = a[2] * 60 + a[3]; } else return false;
  return s <= e ? (now >= s && now <= e) : (now >= s || now <= e);
}
function dateRange() { return true; }
"#;

/// Message of a JS exception (QuickJS reports only "Exception generated").
fn js_error(ctx: &rquickjs::Ctx<'_>, e: rquickjs::Error) -> String {
    if matches!(e, rquickjs::Error::Exception) {
        let v = ctx.catch();
        if let Some(ex) = v.as_exception() {
            return ex.message().unwrap_or_else(|| "exception".into());
        }
        if let Ok(s) = v.get::<String>() {
            return s;
        }
    }
    e.to_string()
}

fn dns_resolve(host: String) -> Option<String> {
    use std::net::ToSocketAddrs;
    (host.as_str(), 0).to_socket_addrs().ok()?.find(|a| a.is_ipv4()).map(|a| a.ip().to_string())
}

/// This machine's outward IPv4 address (no packet is sent).
fn my_ip() -> String {
    std::net::UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| s.connect("192.0.2.1:80").map(|_| s))
        .and_then(|s| s.local_addr())
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".into())
}

pub struct PacScript {
    // Field order: the context must drop before its runtime.
    ctx: Mutex<Context>,
    _rt: Runtime,
    cache: Mutex<HashMap<String, (Option<String>, Instant)>>,
}

impl PacScript {
    /// Load from an `http(s)://` URL, a `file://` URL or a path. Blocking.
    pub fn load(source: &str) -> Result<Self, String> {
        let text = if source.starts_with("http://") || source.starts_with("https://") {
            // Fetched directly: the PAC file decides the proxy, so it can't need one.
            let resp = reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(15))
                .build()
                .map_err(|e| e.to_string())?
                .get(source)
                .send()
                .map_err(|e| format!("Could not fetch the PAC file: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("Could not fetch the PAC file: HTTP {}", resp.status()));
            }
            resp.text().map_err(|e| e.to_string())?
        } else {
            let path = source.strip_prefix("file://").unwrap_or(source);
            std::fs::read_to_string(path).map_err(|e| format!("Could not read the PAC file {path}: {e}"))?
        };
        Self::from_source(&text)
    }

    pub fn from_source(src: &str) -> Result<Self, String> {
        if src.len() > MAX_PAC_BYTES {
            return Err("PAC file too large".into());
        }
        let rt = Runtime::new().map_err(|e| e.to_string())?;
        rt.set_memory_limit(32 * 1024 * 1024);
        rt.set_max_stack_size(512 * 1024);
        let ctx = Context::full(&rt).map_err(|e| e.to_string())?;
        ctx.with(|ctx| -> Result<(), String> {
            let g = ctx.globals();
            g.set("__dnsResolve", Function::new(ctx.clone(), dns_resolve).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            g.set("__myIp", Function::new(ctx.clone(), my_ip).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
            ctx.eval::<(), _>(HELPERS).map_err(|e| format!("PAC helpers failed: {}", js_error(&ctx, e)))?;
            ctx.eval::<(), _>(src).map_err(|e| format!("PAC file has an error: {}", js_error(&ctx, e)))?;
            g.get::<_, Function>("FindProxyForURL").map(|_| ()).map_err(|_| "PAC file defines no FindProxyForURL".to_string())
        })?;
        Ok(Self { ctx: Mutex::new(ctx), _rt: rt, cache: Mutex::new(HashMap::new()) })
    }

    /// Raw `FindProxyForURL` result.
    pub fn eval(&self, url: &str, host: &str) -> Result<String, String> {
        let ctx = self.ctx.lock().unwrap_or_else(|e| e.into_inner());
        ctx.with(|ctx| {
            let f: Function = ctx.globals().get("FindProxyForURL").map_err(|e| e.to_string())?;
            f.call::<_, String>((url.to_string(), host.to_string())).map_err(|e| format!("FindProxyForURL failed: {}", js_error(&ctx, e)))
        })
    }

    /// First usable proxy URL for the request, `None` = direct.
    pub fn find(&self, url: &str, host: &str) -> Option<String> {
        if let Some((v, at)) = self.cache.lock().unwrap_or_else(|e| e.into_inner()).get(host) {
            if at.elapsed() < CACHE_FOR {
                return v.clone();
            }
        }
        let v = match self.eval(url, host) {
            Ok(r) => first_proxy(&r),
            Err(e) => {
                tracing::warn!("{e}; going direct");
                None
            }
        };
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).insert(host.to_string(), (v.clone(), Instant::now()));
        v
    }
}

/// `"PROXY a:8080; SOCKS5 b:1080; DIRECT"` → first entry as a URL
/// (`None` when it is DIRECT).
pub fn first_proxy(result: &str) -> Option<String> {
    for part in result.split(';') {
        let mut it = part.split_whitespace();
        let (kind, addr) = (it.next().unwrap_or("").to_ascii_uppercase(), it.next());
        match (kind.as_str(), addr) {
            ("DIRECT", _) => return None,
            ("PROXY" | "HTTP", Some(a)) => return Some(format!("http://{a}")),
            ("HTTPS", Some(a)) => return Some(format!("https://{a}")),
            ("SOCKS" | "SOCKS5", Some(a)) => return Some(format!("socks5://{a}")),
            _ => continue,
        }
    }
    None
}
