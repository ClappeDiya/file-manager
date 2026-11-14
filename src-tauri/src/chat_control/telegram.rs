//! Telegram transport: long polling (`getUpdates`) and replies (`sendMessage`).
//!
//! On start the backlog is skipped: commands sent while the app was not
//! listening are never run late. Bot-token errors never include the request
//! URL (it contains the token).

use super::handler::{Handler, Incoming};
use super::core::Dialect;
use super::ChannelStatus;
use serde_json::{json, Value};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

pub type Stop = tokio::sync::watch::Receiver<bool>;

pub struct TelegramConfig {
    /// Bot API base without a trailing slash, e.g. `https://api.telegram.org`.
    pub api_base: String,
    pub token: String,
    /// Long-poll length in seconds.
    pub poll_secs: u64,
    pub http: reqwest::Client,
}

/// The base to use for a settings value (empty = the real service).
pub fn base_of(configured: &str) -> String {
    let b = configured.trim().trim_end_matches('/');
    if b.is_empty() { "https://api.telegram.org".to_string() } else { b.to_string() }
}

async fn call(cfg: &TelegramConfig, method: &str, body: Value) -> Result<Value, String> {
    let r = cfg
        .http
        .post(format!("{}/bot{}/{method}", cfg.api_base, cfg.token))
        .json(&body)
        .timeout(Duration::from_secs(cfg.poll_secs + 15))
        .send()
        .await
        .map_err(|e| format!("Telegram {method}: {}", e.without_url()))?;
    let status = r.status();
    let v: Value = r.json().await.map_err(|e| format!("Telegram {method}: unreadable answer ({status}): {}", e.without_url()))?;
    if v["ok"] != true {
        return Err(format!("Telegram refused {method} ({status}): {}", v["description"].as_str().unwrap_or("no reason given")));
    }
    Ok(v)
}

/// The bot's own handle (without `@`), which also proves the token works.
pub async fn get_me(cfg: &TelegramConfig) -> Result<String, String> {
    let v = call(cfg, "getMe", json!({})).await?;
    Ok(v["result"]["username"].as_str().unwrap_or_default().to_string())
}

/// One update from `getUpdates`.
#[derive(Debug, PartialEq, Eq)]
pub struct Update {
    pub id: i64,
    /// A text message from a person; `None` for anything else.
    pub incoming: Option<Incoming>,
}

/// Read the `result` of a `getUpdates` answer.
pub fn parse_updates(v: &Value) -> Vec<Update> {
    v["result"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|u| {
                    let id = u["update_id"].as_i64()?;
                    let m = &u["message"];
                    let incoming = (|| {
                        if m["from"]["is_bot"].as_bool() == Some(true) {
                            return None;
                        }
                        Some(Incoming {
                            dialect: Dialect::Telegram,
                            sender: m["from"]["id"].as_i64()?.to_string(),
                            chat: m["chat"]["id"].as_i64()?.to_string(),
                            text: m["text"].as_str()?.to_string(),
                        })
                    })();
                    Some(Update { id, incoming })
                })
                .collect()
        })
        .unwrap_or_default()
}

pub async fn send_message(cfg: &TelegramConfig, chat: &str, text: &str) -> Result<(), String> {
    // Plain text, no parse mode: nothing in a reply can become markup. Telegram caps messages at 4096 characters.
    let text: String = text.chars().take(4000).collect();
    call(cfg, "sendMessage", json!({ "chat_id": chat, "text": text, "disable_web_page_preview": true })).await.map(|_| ())
}

/// Sleep for `d`; returns true if the stop signal came first.
pub async fn sleep_or_stop(d: Duration, stop: &mut Stop) -> bool {
    tokio::select! {
        _ = tokio::time::sleep(d) => false,
        _ = stop.changed() => true,
    }
}

/// The offset past everything already waiting, retrying with a growing pause
/// until Telegram answers. `None` when stopped first.
async fn skip_backlog(cfg: &TelegramConfig, stop: &mut Stop, status: &ChannelStatus) -> Option<i64> {
    let mut pause = Duration::from_millis(500);
    loop {
        if *stop.borrow() {
            return None;
        }
        match call(cfg, "getUpdates", json!({ "offset": -1, "timeout": 0 })).await {
            Ok(v) => return Some(parse_updates(&v).iter().map(|u| u.id + 1).max().unwrap_or(0)),
            Err(e) => {
                tracing::warn!("Remote control: {e}");
                status.set_error(e);
                if sleep_or_stop(pause, stop).await {
                    return None;
                }
                pause = (pause * 2).min(Duration::from_secs(60));
            }
        }
    }
}

/// Run until `stop` fires.
pub async fn run(cfg: TelegramConfig, handler: Arc<Handler>, mut stop: Stop, status: Arc<ChannelStatus>) {
    // Skip whatever was said while nobody listened. Until that has worked
    // (the app may have started offline) nothing is processed: starting from
    // offset 0 would run up to a day of old commands.
    let mut offset = match skip_backlog(&cfg, &mut stop, &status).await {
        Some(o) => o,
        None => {
            status.connected.store(false, Ordering::SeqCst);
            return;
        }
    };
    let mut backoff = 1u64;
    loop {
        if *stop.borrow() {
            break;
        }
        let poll = call(&cfg, "getUpdates", json!({ "offset": offset, "timeout": cfg.poll_secs, "allowed_updates": ["message"] }));
        let result = tokio::select! {
            r = poll => r,
            _ = stop.changed() => break,
        };
        match result {
            Ok(v) => {
                status.clear_error();
                backoff = 1;
                for u in parse_updates(&v) {
                    offset = offset.max(u.id + 1);
                    let Some(incoming) = u.incoming else { continue };
                    if let Some(reply) = handler.handle(&incoming).await {
                        status.handled.fetch_add(1, Ordering::SeqCst);
                        if let Err(e) = send_message(&cfg, &incoming.chat, &reply).await {
                            tracing::warn!("Remote control: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                tracing::warn!("Remote control: {e}");
                status.set_error(e);
                if sleep_or_stop(Duration::from_secs(backoff), &mut stop).await {
                    break;
                }
                backoff = (backoff * 2).min(60);
            }
        }
    }
    status.connected.store(false, Ordering::SeqCst);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat_control::core::JobKind;
    use crate::chat_control::handler::mock::{job, MockAudit, MockBackend};
    use std::sync::Mutex;

    #[test]
    fn updates_are_read_defensively() {
        let v = json!({ "ok": true, "result": [
            { "update_id": 10, "message": { "from": { "id": 42, "is_bot": false }, "chat": { "id": -100, "type": "group" }, "text": "/run Tidy" } },
            { "update_id": 11, "message": { "from": { "id": 7, "is_bot": true }, "chat": { "id": 7 }, "text": "/run Tidy" } },
            { "update_id": 12, "message": { "from": { "id": 42 }, "chat": { "id": 42 }, "photo": [] } },
            { "update_id": 13, "edited_message": { "text": "x" } },
            { "no_id": true }
        ] });
        let u = parse_updates(&v);
        assert_eq!(u.iter().map(|u| u.id).collect::<Vec<_>>(), vec![10, 11, 12, 13]);
        let first = u[0].incoming.as_ref().unwrap();
        assert_eq!((first.sender.as_str(), first.chat.as_str(), first.text.as_str()), ("42", "-100", "/run Tidy"));
        assert!(u[1].incoming.is_none(), "other bots are never obeyed");
        assert!(u[2].incoming.is_none() && u[3].incoming.is_none());
        assert!(parse_updates(&json!({})).is_empty() && parse_updates(&json!({ "result": "x" })).is_empty());
        assert_eq!(base_of(""), "https://api.telegram.org");
        assert_eq!(base_of(" http://127.0.0.1:1/ "), "http://127.0.0.1:1");
    }

    /// A mock Bot API: queued updates, recorded sendMessage calls.
    #[derive(Default)]
    struct Mock {
        queue: Mutex<Vec<Value>>,
        sent: Mutex<Vec<(String, String)>>,
        polls: Mutex<Vec<Value>>,
        fail_updates: Mutex<u32>,
    }

    async fn serve(mock: Arc<Mock>) -> String {
        use axum::{extract::{Request, State}, Json, Router};
        async fn handle(State(m): State<Arc<Mock>>, req: Request) -> Json<Value> {
            // `/bot<token>/<method>`
            let method = req.uri().path().rsplit('/').next().unwrap_or_default().to_string();
            let bytes = axum::body::to_bytes(req.into_body(), 1 << 20).await.unwrap_or_default();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            match method.as_str() {
                "getMe" => Json(json!({ "ok": true, "result": { "username": "ufopbot" } })),
                "getUpdates" => {
                    m.polls.lock().unwrap().push(body.clone());
                    {
                        let mut f = m.fail_updates.lock().unwrap();
                        if *f > 0 {
                            *f -= 1;
                            return Json(json!({ "ok": false, "description": "Bad Gateway" }));
                        }
                    }
                    let offset = body["offset"].as_i64().unwrap_or(0);
                    let result: Vec<Value> = {
                        let mut q = m.queue.lock().unwrap();
                        if offset == -1 {
                            q.last().cloned().into_iter().collect()
                        } else {
                            // Telegram forgets updates below the offset.
                            q.retain(|u| u["update_id"].as_i64().unwrap_or(0) >= offset);
                            q.clone()
                        }
                    };
                    if result.is_empty() {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                    Json(json!({ "ok": true, "result": result }))
                }
                "sendMessage" => {
                    m.sent.lock().unwrap().push((body["chat_id"].to_string().trim_matches('"').to_string(), body["text"].as_str().unwrap_or_default().to_string()));
                    Json(json!({ "ok": true, "result": {} }))
                }
                _ => Json(json!({ "ok": false, "description": "unknown method" })),
            }
        }
        let app = Router::new().fallback(handle).with_state(mock);
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        base
    }

    fn update(id: i64, user: i64, text: &str) -> Value {
        json!({ "update_id": id, "message": { "from": { "id": user, "is_bot": false }, "chat": { "id": user, "type": "private" }, "text": text } })
    }

    fn config(base: String) -> TelegramConfig {
        TelegramConfig { api_base: base, token: "123:SECRET".into(), poll_secs: 1, http: reqwest::Client::new() }
    }

    async fn eventually(what: &str, mut f: impl FnMut() -> bool) {
        for _ in 0..200 {
            if f() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!("timed out waiting for {what}");
    }

    #[tokio::test]
    async fn the_loop_answers_listed_senders_skips_the_backlog_and_stays_silent_for_strangers() {
        let mock = Arc::new(Mock::default());
        // Said while the app was not listening: must never run.
        mock.queue.lock().unwrap().extend([update(1, 42, "/run Tidy"), update(2, 42, "/run Tidy")]);
        let base = serve(mock.clone()).await;
        let backend = Arc::new(MockBackend::with(vec![job(JobKind::Rule, "r1", "Tidy")]));
        let audit = Arc::new(MockAudit::default());
        let cfg = config(base);
        let name = get_me(&cfg).await.unwrap();
        assert_eq!(name, "ufopbot");
        let handler = Arc::new(Handler::new(Dialect::Telegram, Some(name), &["42".to_string()], 10, backend.clone(), audit.clone()));
        let (tx, rx) = tokio::sync::watch::channel(false);
        let status = Arc::new(ChannelStatus::default());
        let task = tokio::spawn(run(cfg, handler, rx, status.clone()));

        // Wait until the loop is polling, then send new commands.
        eventually("the first long poll", || mock.polls.lock().unwrap().len() >= 2).await;
        assert!(backend.calls().is_empty(), "the backlog was skipped");
        mock.queue.lock().unwrap().extend([update(3, 99, "/run Tidy"), update(4, 99, "/jobs"), update(5, 42, "/jobs"), update(6, 42, "/run Tidy"), update(7, 42, "hello")]);
        eventually("two replies", || mock.sent.lock().unwrap().len() >= 2).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        let sent = mock.sent.lock().unwrap().clone();
        assert_eq!(sent.len(), 2, "{sent:?}: strangers and chatter got nothing");
        assert!(sent[0].1.contains("Automation rules") && sent[1].1 == "Started Tidy");
        assert!(sent.iter().all(|(chat, _)| chat == "42"), "replies go to the sender's chat");
        assert_eq!(backend.calls(), vec!["run rule:r1"]);
        assert_eq!(audit.entries.lock().unwrap().iter().map(|e| e.command.as_str()).collect::<Vec<_>>(), vec!["jobs", "run Tidy"]);
        // Offsets move past what was handled.
        let last_offset = mock.polls.lock().unwrap().last().unwrap()["offset"].as_i64().unwrap();
        assert!(last_offset >= 8, "{last_offset}");
        assert!(status.snapshot().connected && status.snapshot().handled == 2);
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        assert!(!status.snapshot().connected);
    }

    #[tokio::test]
    async fn a_failed_backlog_skip_is_retried_and_old_commands_never_run() {
        let mock = Arc::new(Mock::default());
        // Said while the app was off, and the first skip call fails (offline start).
        mock.queue.lock().unwrap().extend([update(1, 42, "/run Tidy"), update(2, 42, "/stop Tidy")]);
        *mock.fail_updates.lock().unwrap() = 1;
        let base = serve(mock.clone()).await;
        let backend = Arc::new(MockBackend::with(vec![job(JobKind::Rule, "r1", "Tidy")]));
        let handler = Arc::new(Handler::new(Dialect::Telegram, None, &["42".to_string()], 10, backend.clone(), Arc::new(MockAudit::default())));
        let (tx, rx) = tokio::sync::watch::channel(false);
        let status = Arc::new(ChannelStatus::default());
        let task = tokio::spawn(run(config(base), handler, rx, status.clone()));
        // The skip is retried (offset -1 again), then polling starts past the backlog.
        eventually("a normal poll", || mock.polls.lock().unwrap().iter().any(|p| p["offset"].as_i64().is_some_and(|o| o >= 0))).await;
        let polls = mock.polls.lock().unwrap().clone();
        assert!(polls.len() >= 3 && polls[0]["offset"] == -1 && polls[1]["offset"] == -1, "{polls:?}");
        assert_eq!(polls.iter().find(|p| p["offset"] != -1).unwrap()["offset"], 3, "polling starts after the last old update");
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(backend.calls().is_empty(), "{:?}: old commands were not run", backend.calls());
        assert!(mock.sent.lock().unwrap().is_empty());
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn stopping_while_the_skip_keeps_failing_ends_the_loop_without_running_anything() {
        let mock = Arc::new(Mock::default());
        mock.queue.lock().unwrap().push(update(1, 42, "/run Tidy"));
        *mock.fail_updates.lock().unwrap() = 1000;
        let base = serve(mock.clone()).await;
        let backend = Arc::new(MockBackend::with(vec![job(JobKind::Rule, "r1", "Tidy")]));
        let handler = Arc::new(Handler::new(Dialect::Telegram, None, &["42".to_string()], 10, backend.clone(), Arc::new(MockAudit::default())));
        let (tx, rx) = tokio::sync::watch::channel(false);
        let task = tokio::spawn(run(config(base), handler, rx, Arc::new(ChannelStatus::default())));
        eventually("a failing skip", || !mock.polls.lock().unwrap().is_empty()).await;
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        assert!(backend.calls().is_empty());
        assert!(mock.polls.lock().unwrap().iter().all(|p| p["offset"] == -1), "never moved on to normal polling");
    }

    #[tokio::test]
    async fn errors_are_recorded_without_the_token_and_the_loop_recovers() {
        let mock = Arc::new(Mock::default());
        *mock.fail_updates.lock().unwrap() = 2;
        let base = serve(mock.clone()).await;
        let cfg = config(base);
        let handler = Arc::new(Handler::new(Dialect::Telegram, None, &["42".to_string()], 10, Arc::new(MockBackend::default()), Arc::new(MockAudit::default())));
        let (tx, rx) = tokio::sync::watch::channel(false);
        let status = Arc::new(ChannelStatus::default());
        let task = tokio::spawn(run(cfg, handler, rx, status.clone()));
        eventually("the error", || status.snapshot().last_error.is_some()).await;
        let err = status.snapshot().last_error.unwrap();
        assert!(err.contains("Bad Gateway") && !err.contains("SECRET"), "{err}");
        eventually("recovery", || status.snapshot().connected).await;
        assert_eq!(status.snapshot().last_error, None);
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        // An unreachable server's error does not leak the token either.
        let dead = TelegramConfig { api_base: "http://127.0.0.1:1".into(), token: "123:SECRET".into(), poll_secs: 1, http: reqwest::Client::new() };
        let e = get_me(&dead).await.unwrap_err();
        assert!(!e.contains("SECRET"), "{e}");
    }
}
