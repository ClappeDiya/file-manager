//! In-process HTTP mock for connector tests: one handler closure sees every
//! request (path, query, headers, body) and keeps whatever state it needs.
//! Requests are logged so tests can assert on what the connector sent.

use axum::body::Bytes;
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use std::sync::{Arc, Mutex};

/// One request the mock received.
#[derive(Debug, Clone)]
pub struct Seen {
    pub path: String,
    pub query: String,
    pub headers: HeaderMap,
    pub body: Bytes,
}

impl Seen {
    pub fn header(&self, name: &str) -> Option<String> {
        self.headers.get(name).and_then(|v| v.to_str().ok()).map(String::from)
    }

    pub fn form(&self) -> std::collections::BTreeMap<String, String> {
        url::form_urlencoded::parse(&self.body).into_owned().collect()
    }

    pub fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or(serde_json::Value::Null)
    }

    pub fn query_param(&self, k: &str) -> Option<String> {
        url::form_urlencoded::parse(self.query.as_bytes()).find(|(a, _)| a == k).map(|(_, v)| v.into_owned())
    }
}

/// Running mock: its base URL and the request log.
pub struct Mock {
    pub base: String,
    pub log: Arc<Mutex<Vec<Seen>>>,
}

impl Mock {
    pub fn requests(&self, path: &str) -> Vec<Seen> {
        self.log.lock().unwrap().iter().filter(|s| s.path == path).cloned().collect()
    }
}

/// Serve `handler` on 127.0.0.1 (random port).
pub async fn serve<F>(handler: F) -> Mock
where
    F: Fn(&Seen) -> Response + Send + Sync + 'static,
{
    let log: Arc<Mutex<Vec<Seen>>> = Arc::default();
    let (h, l2) = (Arc::new(handler), log.clone());
    let app = axum::Router::new().fallback(move |u: Uri, headers: HeaderMap, body: Bytes| {
        let (h, log) = (h.clone(), l2.clone());
        async move {
            let seen = Seen { path: u.path().to_string(), query: u.query().unwrap_or("").to_string(), headers, body };
            log.lock().unwrap().push(seen.clone());
            h(&seen)
        }
    });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    Mock { base, log }
}

pub fn json(v: serde_json::Value) -> Response {
    axum::Json(v).into_response()
}

pub fn status(code: u16, body: &str) -> Response {
    (StatusCode::from_u16(code).unwrap(), body.to_string()).into_response()
}
