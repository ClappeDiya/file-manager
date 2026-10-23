//! File gateway relay (`ufop_core::gateway`): on-premises agents poll
//! `/api/v1/gateway/poll` and serve the jobs clients queue through
//! `/api/v1/gateways/{name}/…`. Data streams from one side to the other and
//! is never stored here.
//!
//! - A gateway is the set of nodes polling with its name; a job goes to the
//!   first node that polls, so extra nodes add capacity and failover.
//! - Agents need the write scope (they push data into client responses);
//!   clients need read to browse and download and write to upload.
//! - With no node online a request fails at once (503); a node that takes a
//!   job but does not answer in time fails it (504).

use super::api_tokens::{Principal, Scope};
use super::ServerState;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path as AxPath, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post, put};
use axum::{Extension, Json, Router};
use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::json;
use std::collections::{HashMap, VecDeque};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{oneshot, Notify};
use ufop_core::gateway::{valid_name, ErrorBody, Hello, Job, Op, NODE_TIMEOUT_SECS, POLL_WAIT_SECS, SIZE_HEADER, STATUS_HEADER};

/// How long a client waits for a node to take its job and start answering.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
const MAX_JSON_REPLY: usize = 16 << 20;
const MAX_QUEUE: usize = 1000;
/// A refused upload's body is read (and discarded) up to this much before
/// the answer goes out, so the client, still sending, reads the refusal
/// instead of a broken pipe. Beyond it the connection is closed.
const DRAIN_LIMIT: usize = 64 << 20;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

struct Node {
    hello: Hello,
    ip: String,
    last_seen: DateTime<Utc>,
    jobs: u64,
}

#[derive(Default)]
struct Gateway {
    nodes: HashMap<String, Node>,
    queue: VecDeque<Job>,
    notify: Arc<Notify>,
}

/// An agent's answer, handed to the waiting client request.
struct Reply {
    ok: bool,
    size: Option<u64>,
    body: Body,
}

struct Pending {
    reply: Option<oneshot::Sender<Reply>>,
    upload: Option<Body>,
}

#[derive(Default)]
struct HubState {
    gateways: HashMap<String, Gateway>,
    pending: HashMap<String, Pending>,
}

/// Gateways and their jobs (in memory: nodes re-register by polling).
#[derive(Clone, Default)]
pub struct Hub(Arc<Mutex<HubState>>);

fn online(n: &Node) -> bool {
    (Utc::now() - n.last_seen).num_seconds() < NODE_TIMEOUT_SECS
}

fn fail(code: StatusCode, message: &str, advice: &str) -> Response {
    (code, Json(json!({ "error_type": "gateway", "message": message, "advice": advice }))).into_response()
}

/// A body that resolves `done` once it has been read to the end or dropped.
fn guarded(body: Body, done: oneshot::Sender<()>) -> Body {
    Body::from_stream(body.into_data_stream().map(move |chunk| {
        let _keep = &done;
        chunk
    }))
}

impl Hub {
    fn lock(&self) -> std::sync::MutexGuard<'_, HubState> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Queue a job for `gateway` and wait for a node's answer. Also returns
    /// the upload body when no node fetched it (the job was refused).
    async fn submit(&self, gateway: &str, op: Op, upload: Option<Body>) -> (Result<Reply, Response>, Option<Body>) {
        let mut upload = upload;
        let r = self.submit_inner(gateway, op, &mut upload).await;
        (r, upload)
    }

    async fn submit_inner(&self, gateway: &str, op: Op, upload: &mut Option<Body>) -> Result<Reply, Response> {
        let needs_write = matches!(op, Op::Upload { .. });
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (tx, rx) = oneshot::channel();
        {
            let mut s = self.lock();
            let Some(g) = s.gateways.get_mut(gateway) else {
                return Err(fail(StatusCode::NOT_FOUND, &format!("No gateway named {gateway} has connected"), "Start `ufop gateway serve` on the machine with the files."));
            };
            let nodes: Vec<&Node> = g.nodes.values().filter(|n| online(n)).collect();
            if nodes.is_empty() {
                return Err(fail(StatusCode::SERVICE_UNAVAILABLE, &format!("No node of gateway {gateway} is online"), "Check that the gateway agent is running and can reach this server."));
            }
            if needs_write && !nodes.iter().any(|n| n.hello.writable) {
                return Err(fail(StatusCode::FORBIDDEN, &format!("Gateway {gateway} is read-only"), "Start the agent with --writable to accept uploads."));
            }
            if g.queue.len() >= MAX_QUEUE {
                return Err(fail(StatusCode::SERVICE_UNAVAILABLE, "The gateway is busy", "Try again shortly."));
            }
            g.queue.push_back(Job { id: id.clone(), op });
            // Every waiting node looks; the first that can take the job does.
            g.notify.notify_waiters();
            s.pending.insert(id.clone(), Pending { reply: Some(tx), upload: upload.take() });
        }
        let r = tokio::time::timeout(ANSWER_TIMEOUT, rx).await;
        let mut s = self.lock();
        *upload = s.pending.remove(&id).and_then(|p| p.upload);
        if let Some(g) = s.gateways.get_mut(gateway) {
            g.queue.retain(|j| j.id != id);
        }
        match r {
            Ok(Ok(reply)) => Ok(reply),
            _ => Err(fail(StatusCode::GATEWAY_TIMEOUT, &format!("Gateway {gateway} did not answer"), "The node may have stopped; try again.")),
        }
    }

    /// Register a polling node and hand it the next job (waits up to `POLL_WAIT_SECS`).
    async fn poll(&self, hello: Hello, ip: String) -> Option<Job> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(POLL_WAIT_SECS);
        let notify = self.lock().gateways.entry(hello.gateway.clone()).or_default().notify.clone();
        loop {
            // Listen before looking, so a job queued in between is not missed.
            let notified = notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let mut s = self.lock();
                let g = s.gateways.entry(hello.gateway.clone()).or_default();
                let n = g.nodes.entry(hello.node.clone()).or_insert_with(|| Node { hello: hello.clone(), ip: ip.clone(), last_seen: Utc::now(), jobs: 0 });
                n.hello = hello.clone();
                n.ip = ip.clone();
                n.last_seen = Utc::now();
                // A read-only node leaves uploads to writable ones.
                let pos = g.queue.iter().position(|j| hello.writable || !matches!(j.op, Op::Upload { .. }));
                if let Some(job) = pos.and_then(|p| g.queue.remove(p)) {
                    if let Some(n) = g.nodes.get_mut(&hello.node) {
                        n.jobs += 1;
                    }
                    return Some(job);
                }
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                return None;
            }
        }
    }

    fn leave(&self, gateway: &str, node: &str) -> bool {
        let mut s = self.lock();
        let Some(g) = s.gateways.get_mut(gateway) else { return false };
        let gone = g.nodes.remove(node).is_some();
        if g.nodes.is_empty() && g.queue.is_empty() {
            s.gateways.remove(gateway);
        }
        gone
    }

    fn take_reply(&self, id: &str) -> Option<oneshot::Sender<Reply>> {
        self.lock().pending.get_mut(id).and_then(|p| p.reply.take())
    }

    fn take_upload(&self, id: &str) -> Option<Body> {
        self.lock().pending.get_mut(id).and_then(|p| p.upload.take())
    }

    fn describe(&self) -> serde_json::Value {
        let s = self.lock();
        let mut gws: Vec<serde_json::Value> = s
            .gateways
            .iter()
            .map(|(name, g)| {
                let mut nodes: Vec<serde_json::Value> = g
                    .nodes
                    .iter()
                    .map(|(id, n)| json!({ "node": id, "host": n.hello.host, "ip": n.ip, "roots": n.hello.roots, "writable": n.hello.writable, "version": n.hello.version, "last_seen": n.last_seen.to_rfc3339(), "online": online(n), "jobs": n.jobs }))
                    .collect();
                nodes.sort_by(|a, b| a["node"].as_str().cmp(&b["node"].as_str()));
                let up = g.nodes.values().filter(|n| online(n)).count();
                json!({ "name": name, "online_nodes": up, "online": up > 0, "writable": g.nodes.values().any(|n| online(n) && n.hello.writable), "queued": g.queue.len(), "nodes": nodes })
            })
            .collect();
        gws.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        json!({ "gateways": gws })
    }
}

/// `Some(refusal)` unless the caller may act as a gateway agent.
fn deny_unless_writer(p: &Principal) -> Option<Response> {
    match p {
        Principal::Session => None,
        Principal::ApiToken { scope, .. } if *scope >= Scope::Write => None,
        Principal::ApiToken { .. } => Some(fail(StatusCode::UNAUTHORIZED, "Gateway agents need an API token with the write scope", "Create one with `ufop server tokens create --scope write`.")),
    }
}

/// The agent's error body → a client response.
async fn agent_error(reply: Reply) -> Response {
    let bytes = axum::body::to_bytes(reply.body, MAX_JSON_REPLY).await.unwrap_or_default();
    let e: ErrorBody = serde_json::from_slice(&bytes).unwrap_or(ErrorBody { message: "The gateway failed".into(), kind: "failed".into() });
    let code = match e.kind.as_str() {
        "not_found" => StatusCode::NOT_FOUND,
        "forbidden" => StatusCode::FORBIDDEN,
        "invalid" => StatusCode::BAD_REQUEST,
        _ => StatusCode::BAD_GATEWAY,
    };
    fail(code, &e.message, "")
}

#[derive(Deserialize)]
struct PathQ {
    #[serde(default)]
    path: String,
}

fn deny_bad_name(name: &str) -> Option<Response> {
    (!valid_name(name)).then(|| fail(StatusCode::BAD_REQUEST, "Invalid gateway name", ""))
}

// ── Agent side ──

async fn poll(State(st): State<ServerState>, Extension(p): Extension<Principal>, ConnectInfo(peer): ConnectInfo<SocketAddr>, Json(hello): Json<Hello>) -> Response {
    if let Some(r) = deny_unless_writer(&p) {
        return r;
    }
    if !valid_name(&hello.gateway) || !valid_name(&hello.node) {
        return fail(StatusCode::BAD_REQUEST, "Invalid gateway or node name", "Use letters, digits, '-', '_' and '.'.");
    }
    match st.gateways.poll(hello, peer.ip().to_string()).await {
        Some(job) => Json(job).into_response(),
        None => StatusCode::NO_CONTENT.into_response(),
    }
}

async fn leave(State(st): State<ServerState>, Extension(p): Extension<Principal>, Json(hello): Json<Hello>) -> Response {
    if let Some(r) = deny_unless_writer(&p) {
        return r;
    }
    Json(json!({ "ok": st.gateways.leave(&hello.gateway, &hello.node) })).into_response()
}

async fn reply(State(st): State<ServerState>, Extension(p): Extension<Principal>, AxPath(id): AxPath<String>, headers: HeaderMap, body: Body) -> Response {
    if let Some(r) = deny_unless_writer(&p) {
        return r;
    }
    let Some(tx) = st.gateways.take_reply(&id) else {
        return fail(StatusCode::GONE, "The request is no longer waiting", "");
    };
    let ok = headers.get(STATUS_HEADER).and_then(|v| v.to_str().ok()) == Some("ok");
    let size = headers.get(SIZE_HEADER).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    let (done_tx, done_rx) = oneshot::channel::<()>();
    if tx.send(Reply { ok, size, body: guarded(body, done_tx) }).is_err() {
        return fail(StatusCode::GONE, "The request is no longer waiting", "");
    }
    // Keep this request open while the client reads the body.
    let _ = tokio::time::timeout(Duration::from_secs(24 * 3600), done_rx).await;
    Json(json!({ "ok": true })).into_response()
}

async fn job_body(State(st): State<ServerState>, Extension(p): Extension<Principal>, AxPath(id): AxPath<String>) -> Response {
    if let Some(r) = deny_unless_writer(&p) {
        return r;
    }
    match st.gateways.take_upload(&id) {
        Some(b) => Response::new(b),
        None => fail(StatusCode::GONE, "No upload is waiting for this job", ""),
    }
}

// ── Client side ──

async fn list_gateways(State(st): State<ServerState>) -> Response {
    Json(st.gateways.describe()).into_response()
}

async fn files(State(st): State<ServerState>, AxPath(name): AxPath<String>, Query(q): Query<PathQ>) -> Response {
    if let Some(r) = deny_bad_name(&name) {
        return r;
    }
    let path = if q.path.is_empty() { "/".to_string() } else { q.path };
    let reply = match st.gateways.submit(&name, Op::List { path }, None).await.0 {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !reply.ok {
        return agent_error(reply).await;
    }
    match axum::body::to_bytes(reply.body, MAX_JSON_REPLY).await {
        Ok(b) => ([(header::CONTENT_TYPE, "application/json")], b).into_response(),
        Err(_) => fail(StatusCode::BAD_GATEWAY, "The gateway's listing was cut off", ""),
    }
}

async fn download(State(st): State<ServerState>, AxPath(name): AxPath<String>, Query(q): Query<PathQ>) -> Response {
    if let Some(r) = deny_bad_name(&name) {
        return r;
    }
    let file = q.path.rsplit('/').next().unwrap_or("download").to_string();
    let reply = match st.gateways.submit(&name, Op::Download { path: q.path }, None).await.0 {
        Ok(r) => r,
        Err(r) => return r,
    };
    if !reply.ok {
        return agent_error(reply).await;
    }
    let size = reply.size;
    let mut r = Response::new(reply.body);
    let h = r.headers_mut();
    h.insert(header::CONTENT_TYPE, HeaderValue::from_static("application/octet-stream"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    if let Some(n) = size {
        h.insert(header::CONTENT_LENGTH, HeaderValue::from(n));
    }
    let ascii: String = file.chars().map(|c| if c.is_ascii_graphic() && c != '"' && c != '\\' { c } else { '_' }).collect();
    if let Ok(v) = HeaderValue::from_str(&format!("attachment; filename=\"{ascii}\"")) {
        h.insert(header::CONTENT_DISPOSITION, v);
    }
    r
}

async fn upload(State(st): State<ServerState>, AxPath(name): AxPath<String>, Query(q): Query<PathQ>, headers: HeaderMap, body: Body) -> Response {
    if let Some(r) = deny_bad_name(&name) {
        return r;
    }
    let size = headers.get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).and_then(|v| v.parse().ok());
    let (result, unread) = st.gateways.submit(&name, Op::Upload { path: q.path, size }, Some(body)).await;
    let answer = match result {
        Err(r) => r,
        Ok(reply) if !reply.ok => agent_error(reply).await,
        Ok(reply) => match axum::body::to_bytes(reply.body, MAX_JSON_REPLY).await {
            Ok(b) => ([(header::CONTENT_TYPE, "application/json")], b).into_response(),
            Err(_) => fail(StatusCode::BAD_GATEWAY, "The gateway's answer was cut off", ""),
        },
    };
    match unread {
        Some(b) => after_draining(b, answer).await,
        None => answer,
    }
}

/// Answer a refused upload once its body has been received (see
/// [`DRAIN_LIMIT`]); a larger or stalled body closes the connection.
async fn after_draining(body: Body, mut answer: Response) -> Response {
    let drained = tokio::time::timeout(DRAIN_TIMEOUT, async {
        let mut stream = body.into_data_stream();
        let mut n = 0usize;
        while let Some(chunk) = stream.next().await {
            n += chunk.map_err(|_| ())?.len();
            if n > DRAIN_LIMIT {
                return Err(());
            }
        }
        Ok(())
    })
    .await;
    if !matches!(drained, Ok(Ok(()))) {
        answer.headers_mut().insert(header::CONNECTION, HeaderValue::from_static("close"));
    }
    answer
}

pub fn protected_routes() -> Router<ServerState> {
    Router::new()
        .route("/api/v1/gateway/poll", post(poll))
        .route("/api/v1/gateway/leave", post(leave))
        .route("/api/v1/gateway/jobs/{id}/reply", post(reply))
        .route("/api/v1/gateway/jobs/{id}/body", get(job_body))
        .route("/api/v1/gateways", get(list_gateways))
        .route("/api/v1/gateways/{name}/files", get(files))
        .route("/api/v1/gateways/{name}/download", get(download))
        .route("/api/v1/gateways/{name}/upload", put(upload))
        // Uploads stream through to the agent; its own limit applies.
        .layer(axum::extract::DefaultBodyLimit::disable())
}
