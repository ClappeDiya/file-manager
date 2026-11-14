//! Graceful shutdown of `ufop-server`.
//!
//! A stop request comes from SIGTERM / SIGINT (Ctrl-C, also on Windows), the
//! Windows service manager, or `POST /api/v1/server/shutdown {grace_secs}`.
//! Then the server
//!
//! 1. answers new mutating requests with `503` and `Retry-After`,
//! 2. stops the scheduler from starting work and refuses new operations,
//! 3. asks running backups to stop at their next safe point (the engine's
//!    cancel path writes a partial version and drops its repository lock),
//! 4. waits up to the grace period (default 60 s, `--shutdown-grace`),
//! 5. drains open HTTP connections, flushes telemetry and exits 0.
//!
//! A second request while draining stops at once.

use super::{ApiError, ServerState};
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Notify;

#[derive(Default)]
struct Inner {
    requested: AtomicBool,
    immediate: AtomicBool,
    grace_secs: AtomicU64,
    default_grace_secs: AtomicU64,
    wake: Notify,
}

/// Shared stop switch (cheap to clone).
#[derive(Clone)]
pub struct Lifecycle(Arc<Inner>);

impl Default for Lifecycle {
    fn default() -> Self {
        Self::new(crate::backup_engine::shutdown::DEFAULT_GRACE)
    }
}

/// What a stop request did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StopRequest {
    /// First request: graceful drain started.
    Graceful,
    /// Already draining: stop now.
    Immediate,
}

impl Lifecycle {
    pub fn new(default_grace: Duration) -> Self {
        let i = Inner::default();
        i.default_grace_secs.store(default_grace.as_secs(), Ordering::SeqCst);
        Self(Arc::new(i))
    }

    pub fn default_grace(&self) -> Duration {
        Duration::from_secs(self.0.default_grace_secs.load(Ordering::SeqCst))
    }

    /// Ask to stop (`grace` = None: the default). A second call means "now".
    pub fn request(&self, grace: Option<Duration>) -> StopRequest {
        let g = grace.unwrap_or_else(|| self.default_grace());
        if self.0.requested.swap(true, Ordering::SeqCst) {
            self.0.immediate.store(true, Ordering::SeqCst);
            self.0.wake.notify_waiters();
            return StopRequest::Immediate;
        }
        self.0.grace_secs.store(g.as_secs(), Ordering::SeqCst);
        self.0.wake.notify_waiters();
        StopRequest::Graceful
    }

    pub fn is_draining(&self) -> bool {
        self.0.requested.load(Ordering::SeqCst)
    }

    pub fn is_immediate(&self) -> bool {
        self.0.immediate.load(Ordering::SeqCst)
    }

    pub fn grace(&self) -> Duration {
        Duration::from_secs(self.0.grace_secs.load(Ordering::SeqCst))
    }

    /// Resolves once a stop was requested.
    pub async fn requested(&self) {
        loop {
            let n = self.0.wake.notified();
            if self.is_draining() {
                return;
            }
            n.await;
        }
    }

    /// Resolves once an immediate stop was requested.
    pub async fn immediate(&self) {
        loop {
            let n = self.0.wake.notified();
            if self.is_immediate() {
                return;
            }
            n.await;
        }
    }

    /// Turn SIGTERM / SIGINT (Ctrl-C on Windows) into stop requests.
    pub fn listen_for_signals(&self) {
        let me = self.clone();
        tokio::spawn(async move {
            loop {
                if !wait_signal().await {
                    return;
                }
                match me.request(None) {
                    StopRequest::Graceful => tracing::info!("Stop signal received: finishing running backups (up to {}s); send it again to stop now", me.grace().as_secs()),
                    StopRequest::Immediate => tracing::warn!("Second stop signal: stopping now"),
                }
            }
        });
    }
}

/// The next SIGTERM or SIGINT; false when signals cannot be watched.
async fn wait_signal() -> bool {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};
        let (Ok(mut term), Ok(mut int)) = (signal(SignalKind::terminate()), signal(SignalKind::interrupt())) else { return false };
        tokio::select! {
            _ = term.recv() => true,
            _ = int.recv() => true,
        }
    }
    #[cfg(not(unix))]
    {
        tokio::signal::ctrl_c().await.is_ok()
    }
}

/// Mutating requests while draining get `503 Service Unavailable`.
pub(crate) async fn drain_guard(State(st): State<ServerState>, req: Request, next: Next) -> Response {
    let mutating = !matches!(*req.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    // A second shutdown request must get through (it means "stop now").
    if st.life.is_draining() && mutating && req.uri().path() != "/api/v1/server/shutdown" {
        let retry = st.life.grace().as_secs().max(1) + 5;
        let mut r = (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error_type": "unavailable", "message": "The server is shutting down", "advice": "Retry after it has restarted."})),
        )
            .into_response();
        r.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(retry));
        return r;
    }
    next.run(req).await
}

#[derive(Deserialize, Default)]
pub(crate) struct ShutdownBody {
    grace_secs: Option<u64>,
}

/// `POST /api/v1/server/shutdown` (admin).
pub(crate) async fn shutdown_route(State(st): State<ServerState>, body: Option<Json<ShutdownBody>>) -> Result<Response, ApiError> {
    let grace = body.and_then(|b| b.0.grace_secs).map(Duration::from_secs);
    let running = st.mgr.active_task_count();
    let kind = st.life.request(grace);
    tracing::info!("Shutdown requested over the API ({kind:?})");
    let body = json!({"accepted": true, "immediate": kind == StopRequest::Immediate, "grace_secs": st.life.grace().as_secs(), "running_tasks": running});
    Ok((StatusCode::ACCEPTED, Json(body)).into_response())
}

/// Wait for a stop request, then drain the backup manager.
pub async fn drain_when_requested(st: ServerState) -> crate::backup_engine::shutdown::ShutdownReport {
    st.life.requested().await;
    st.mgr.begin_shutdown();
    let grace = st.life.grace();
    tokio::select! {
        r = st.mgr.shutdown(grace) => r,
        _ = st.life.immediate() => {
            let cancelled = st.mgr.cancel_all();
            crate::backup_engine::shutdown::ShutdownReport { cancelled, still_running: st.mgr.active_task_count(), waited_ms: 0 }
        }
    }
}
