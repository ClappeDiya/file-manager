//! A minimal in-process HTTP server standing in for the console in tests
//! (std only; one thread per connection, `Connection: close`).

use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone)]
pub struct MockRequest {
    pub method: String,
    pub path: String,
    pub authorization: Option<String>,
    pub body: Value,
}

impl MockRequest {
    pub fn bearer(&self) -> Option<&str> {
        self.authorization.as_deref().and_then(|a| a.strip_prefix("Bearer "))
    }
}

type Handler = dyn Fn(&MockRequest) -> (u16, Value) + Send + Sync;

pub struct MockConsole {
    pub url: String,
    pub requests: Arc<Mutex<Vec<MockRequest>>>,
}

impl MockConsole {
    /// Serve `handler` on a free loopback port.
    pub fn start(handler: impl Fn(&MockRequest) -> (u16, Value) + Send + Sync + 'static) -> std::io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}", listener.local_addr()?);
        let requests = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        let log = requests.clone();
        std::thread::spawn(move || {
            for s in listener.incoming().flatten() {
                let (h, log) = (handler.clone(), log.clone());
                std::thread::spawn(move || {
                    let _ = serve(s, &*h, &log);
                });
            }
        });
        Ok(Self { url, requests })
    }

    pub fn requests(&self) -> Vec<MockRequest> {
        self.requests.lock().map(|r| r.clone()).unwrap_or_default()
    }

    /// Requests to `path` (exact match).
    pub fn hits(&self, path: &str) -> Vec<MockRequest> {
        self.requests().into_iter().filter(|r| r.path == path).collect()
    }
}

fn serve(s: TcpStream, h: &Handler, log: &Mutex<Vec<MockRequest>>) -> std::io::Result<()> {
    let mut r = BufReader::new(s.try_clone()?);
    let mut line = String::new();
    r.read_line(&mut line)?;
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();
    let (mut len, mut auth) = (0usize, None);
    loop {
        let mut l = String::new();
        if r.read_line(&mut l)? == 0 || l == "\r\n" || l == "\n" {
            break;
        }
        if let Some((k, v)) = l.split_once(':') {
            match k.trim().to_ascii_lowercase().as_str() {
                "content-length" => len = v.trim().parse().unwrap_or(0),
                "authorization" => auth = Some(v.trim().to_string()),
                _ => {}
            }
        }
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    let req = MockRequest { method, path, authorization: auth, body: serde_json::from_slice(&body).unwrap_or(Value::Null) };
    if let Ok(mut l) = log.lock() {
        l.push(req.clone());
    }
    let (code, v) = h(&req);
    let out = v.to_string();
    let mut s = s;
    write!(s, "HTTP/1.1 {code} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{out}", out.len())?;
    s.flush()
}
