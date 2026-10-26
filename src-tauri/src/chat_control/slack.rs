//! Slack transport: Socket Mode (a WebSocket Slack opens for the app, no
//! public URL needed) and `chat.postMessage` for replies.
//!
//! Every event envelope is acknowledged first, as Slack requires; a repeated
//! delivery of the same event is not run twice. The bot ignores its own and
//! other bots' messages. One post that Slack reports twice (as `message` and
//! as `app_mention`, with different event ids) is run once.

use super::core::{slack_escape, Dialect};
use super::handler::{Handler, Incoming};
use super::telegram::{sleep_or_stop, Stop};
use super::ws::{Message, WsClient};
use super::ChannelStatus;
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

pub struct SlackConfig {
    /// Web API base without a trailing slash (`https://slack.com/api`).
    pub api_base: String,
    /// App-level token (`xapp-…`): opens the Socket Mode connection.
    pub app_token: String,
    /// Bot token (`xoxb-…`): posts replies.
    pub bot_token: String,
    pub http: reqwest::Client,
    pub tls: Arc<rustls::ClientConfig>,
}

pub fn base_of(configured: &str) -> String {
    let b = configured.trim().trim_end_matches('/');
    if b.is_empty() { "https://slack.com/api".to_string() } else { b.to_string() }
}

/// What a Socket Mode message is.
#[derive(Debug, PartialEq, Eq)]
pub enum Envelope {
    Hello,
    /// Slack asks for a new connection.
    Disconnect(String),
    /// An event to acknowledge, maybe a message for the bot.
    Event {
        envelope_id: String,
        event_id: Option<String>,
        /// `channel:ts` of the post: the same for the `message` and the
        /// `app_mention` event of one message.
        message_key: Option<String>,
        incoming: Option<Incoming>,
    },
    Other,
}

pub fn parse_envelope(text: &str) -> Envelope {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Envelope::Other };
    match v["type"].as_str() {
        Some("hello") => Envelope::Hello,
        Some("disconnect") => Envelope::Disconnect(v["reason"].as_str().unwrap_or("").to_string()),
        Some(kind) => {
            let Some(envelope_id) = v["envelope_id"].as_str().map(str::to_string) else { return Envelope::Other };
            let mut event_id = None;
            let mut message_key = None;
            let mut incoming = None;
            if kind == "events_api" {
                event_id = v["payload"]["event_id"].as_str().map(str::to_string);
                let e = &v["payload"]["event"];
                let is_message = e["type"] == "message" && e.get("subtype").map_or(true, Value::is_null);
                let is_mention = e["type"] == "app_mention";
                // Not our own or another bot's messages: that is how reply loops start.
                if (is_message || is_mention) && e.get("bot_id").map_or(true, Value::is_null) {
                    if let (Some(user), Some(channel), Some(text)) = (e["user"].as_str(), e["channel"].as_str(), e["text"].as_str()) {
                        message_key = e["ts"].as_str().map(|ts| format!("{channel}:{ts}"));
                        incoming = Some(Incoming { dialect: Dialect::Slack, sender: user.to_string(), chat: channel.to_string(), text: text.to_string() });
                    }
                }
            }
            Envelope::Event { envelope_id, event_id, message_key, incoming }
        }
        None => Envelope::Other,
    }
}

/// Ask Slack for a Socket Mode WebSocket URL.
pub async fn open_connection(cfg: &SlackConfig) -> Result<String, String> {
    let r = cfg
        .http
        .post(format!("{}/apps.connections.open", cfg.api_base))
        .bearer_auth(&cfg.app_token)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Slack apps.connections.open: {}", e.without_url()))?;
    let v: Value = r.json().await.map_err(|e| format!("Slack apps.connections.open: unreadable answer: {}", e.without_url()))?;
    if v["ok"] != true {
        return Err(format!("Slack refused the app token: {}", v["error"].as_str().unwrap_or("unknown error")));
    }
    v["url"].as_str().map(str::to_string).ok_or_else(|| "Slack gave no WebSocket address".to_string())
}

/// Post `text` to `channel`.
pub async fn post_message(cfg: &SlackConfig, channel: &str, text: &str) -> Result<(), String> {
    let text: String = slack_escape(text).chars().take(3900).collect();
    let r = cfg
        .http
        .post(format!("{}/chat.postMessage", cfg.api_base))
        .bearer_auth(&cfg.bot_token)
        .json(&json!({ "channel": channel, "text": text, "mrkdwn": false, "unfurl_links": false, "unfurl_media": false }))
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|e| format!("Slack chat.postMessage: {}", e.without_url()))?;
    let v: Value = r.json().await.map_err(|e| format!("Slack chat.postMessage: unreadable answer: {}", e.without_url()))?;
    if v["ok"] != true {
        return Err(format!("Slack refused the reply: {}", v["error"].as_str().unwrap_or("unknown error")));
    }
    Ok(())
}

/// Recent event ids, so a redelivered event is not run twice.
#[derive(Default)]
struct Seen(VecDeque<String>);

impl Seen {
    /// Whether `id` is new (and remember it).
    fn first_time(&mut self, id: &str) -> bool {
        if self.0.iter().any(|s| s == id) {
            return false;
        }
        self.0.push_back(id.to_string());
        if self.0.len() > 256 {
            self.0.pop_front();
        }
        true
    }
}

/// Whether this event repeats one already seen: a redelivery (same event id)
/// or the other event type of the same post (same channel and timestamp).
/// Both are remembered either way.
fn is_repeat(seen: &mut Seen, event_id: Option<&str>, message_key: Option<&str>) -> bool {
    let new_event = event_id.map_or(true, |id| seen.first_time(id));
    let new_message = message_key.map_or(true, |key| seen.first_time(&format!("msg:{key}")));
    !(new_event && new_message)
}

fn seen_event(seen: &mut Seen, event_id: Option<&str>) {
    if let Some(id) = event_id {
        seen.first_time(id);
    }
}

/// One connection's life. `Ok(true)` = Slack asked for a fresh connection.
async fn session(cfg: &SlackConfig, handler: &Handler, stop: &mut Stop, status: &ChannelStatus, seen: &mut Seen) -> Result<bool, String> {
    let url = open_connection(cfg).await?;
    let mut ws = WsClient::connect(&url, cfg.tls.clone(), Duration::from_secs(20)).await?;
    status.clear_error();
    loop {
        let msg = tokio::select! {
            m = ws.recv() => m?,
            _ = stop.changed() => {
                ws.close().await;
                return Ok(false);
            }
        };
        let Message::Text(text) = msg else { return Err("Slack closed the connection".into()) };
        match parse_envelope(&text) {
            Envelope::Hello | Envelope::Other => {}
            Envelope::Disconnect(reason) => {
                tracing::info!("Remote control: Slack asked for a new connection ({reason})");
                return Ok(true);
            }
            Envelope::Event { envelope_id, event_id, message_key, incoming } => {
                ws.send_text(&json!({ "envelope_id": envelope_id }).to_string()).await?;
                // Nothing to obey; still remember the id so a redelivery stays a repeat.
                let Some(incoming) = incoming else {
                    seen_event(seen, event_id.as_deref());
                    continue;
                };
                if is_repeat(seen, event_id.as_deref(), message_key.as_deref()) {
                    continue;
                }
                if let Some(reply) = handler.handle(&incoming).await {
                    status.handled.fetch_add(1, Ordering::SeqCst);
                    if let Err(e) = post_message(cfg, &incoming.chat, &reply).await {
                        tracing::warn!("Remote control: {e}");
                    }
                }
            }
        }
    }
}

/// Run until `stop` fires, reconnecting with a growing pause after failures.
pub async fn run(cfg: SlackConfig, handler: Arc<Handler>, mut stop: Stop, status: Arc<ChannelStatus>) {
    let mut backoff = 1u64;
    let mut seen = Seen::default();
    loop {
        if *stop.borrow() {
            break;
        }
        match session(&cfg, &handler, &mut stop, &status, &mut seen).await {
            Ok(false) => break,
            Ok(true) => backoff = 1,
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
    use crate::chat_control::ws::testing;
    use std::sync::Mutex;

    fn event(envelope: &str, event_id: &str, user: &str, text: &str, extra: Value) -> String {
        let mut e = json!({ "type": "message", "user": user, "channel": "D1", "text": text, "channel_type": "im" });
        if let (Some(o), Some(x)) = (e.as_object_mut(), extra.as_object()) {
            o.extend(x.clone());
        }
        json!({ "envelope_id": envelope, "type": "events_api", "payload": { "event_id": event_id, "event": e } }).to_string()
    }

    #[test]
    fn envelopes_are_read_defensively() {
        assert_eq!(parse_envelope(r#"{"type":"hello"}"#), Envelope::Hello);
        assert_eq!(parse_envelope(r#"{"type":"disconnect","reason":"refresh_requested"}"#), Envelope::Disconnect("refresh_requested".into()));
        assert_eq!(parse_envelope("not json"), Envelope::Other);
        assert_eq!(parse_envelope(r#"{"foo":1}"#), Envelope::Other);
        assert_eq!(parse_envelope(r#"{"type":"events_api"}"#), Envelope::Other, "no envelope id, nothing to acknowledge");
        match parse_envelope(&event("e1", "Ev1", "U1", "run Tidy", json!({}))) {
            Envelope::Event { envelope_id, event_id, incoming: Some(i), .. } => {
                assert_eq!((envelope_id.as_str(), event_id.as_deref()), ("e1", Some("Ev1")));
                assert_eq!((i.sender.as_str(), i.chat.as_str(), i.text.as_str(), i.dialect), ("U1", "D1", "run Tidy", Dialect::Slack));
            }
            other => panic!("{other:?}"),
        }
        // Edits, bot messages and other event types are acknowledged but never obeyed.
        for extra in [json!({ "subtype": "message_changed" }), json!({ "bot_id": "B1" }), json!({ "subtype": "bot_message" })] {
            assert!(matches!(parse_envelope(&event("e", "x", "U1", "run Tidy", extra)), Envelope::Event { incoming: None, .. }));
        }
        let mention = json!({ "envelope_id": "e2", "type": "events_api", "payload": { "event": { "type": "app_mention", "user": "U1", "channel": "C1", "text": "<@U0BOT> jobs" } } }).to_string();
        assert!(matches!(parse_envelope(&mention), Envelope::Event { incoming: Some(i), .. } if i.chat == "C1"));
        let slash = json!({ "envelope_id": "e3", "type": "slash_commands", "payload": { "command": "/x" } }).to_string();
        assert!(matches!(parse_envelope(&slash), Envelope::Event { incoming: None, .. }));
        let no_user = json!({ "envelope_id": "e4", "type": "events_api", "payload": { "event": { "type": "message", "channel": "D1", "text": "run" } } }).to_string();
        assert!(matches!(parse_envelope(&no_user), Envelope::Event { incoming: None, .. }));
        assert_eq!(base_of(""), "https://slack.com/api");
    }

    #[test]
    fn one_post_reported_as_message_and_as_mention_is_a_repeat() {
        let post = |kind: &str, event_id: &str, ts: &str| {
            json!({ "envelope_id": "e", "type": "events_api", "payload": { "event_id": event_id, "event": { "type": kind, "user": "U1", "channel": "C1", "ts": ts, "text": "<@U0BOT> run Tidy" } } }).to_string()
        };
        let keys: Vec<Option<String>> = [post("message", "Ev1", "1700.1"), post("app_mention", "Ev2", "1700.1"), post("app_mention", "Ev3", "1700.2")]
            .iter()
            .map(|t| match parse_envelope(t) {
                Envelope::Event { message_key, .. } => message_key,
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(keys[0], Some("C1:1700.1".into()));
        assert_eq!(keys[0], keys[1], "the same post has the same key whichever event reports it");
        assert_ne!(keys[0], keys[2]);
        let mut seen = Seen::default();
        assert!(!is_repeat(&mut seen, Some("Ev1"), keys[0].as_deref()));
        assert!(is_repeat(&mut seen, Some("Ev2"), keys[1].as_deref()), "second event of the same post");
        assert!(is_repeat(&mut seen, Some("Ev1"), None), "redelivery by event id");
        assert!(!is_repeat(&mut seen, Some("Ev3"), keys[2].as_deref()), "a new post is new");
        assert!(!is_repeat(&mut seen, None, None), "no ids at all: nothing to compare, obey");
    }

    #[test]
    fn redelivered_events_are_recognised() {
        let mut s = Seen::default();
        assert!(s.first_time("a") && !s.first_time("a") && s.first_time("b"));
        for i in 0..300 {
            s.first_time(&format!("id{i}"));
        }
        assert!(s.first_time("a"), "old ids are forgotten after 256 newer ones");
    }

    /// A Web API mock: `apps.connections.open` hands out the WebSocket URL,
    /// `chat.postMessage` is recorded.
    async fn web_api(ws_url: String) -> (String, Arc<Mutex<Vec<(String, Value)>>>) {
        use axum::{extract::{Request, State}, Json, Router};
        type Calls = Arc<Mutex<Vec<(String, Value)>>>;
        #[derive(Clone)]
        struct S {
            ws_url: String,
            calls: Calls,
        }
        async fn handle(State(s): State<S>, req: Request) -> Json<Value> {
            let path = req.uri().path().to_string();
            let auth = req.headers().get("authorization").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
            let bytes = axum::body::to_bytes(req.into_body(), 1 << 20).await.unwrap_or_default();
            let body: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            s.calls.lock().unwrap().push((format!("{path} {auth}"), body));
            if path.ends_with("apps.connections.open") {
                return Json(if auth == "Bearer xapp-good" { json!({ "ok": true, "url": s.ws_url }) } else { json!({ "ok": false, "error": "invalid_auth" }) });
            }
            Json(json!({ "ok": true }))
        }
        let calls: Calls = Default::default();
        let app = Router::new().fallback(handle).with_state(S { ws_url, calls: calls.clone() });
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", l.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (base, calls)
    }

    fn tls() -> Arc<rustls::ClientConfig> {
        Arc::new(crate::security::tls::client_config(false, None, &[]).unwrap())
    }

    #[tokio::test]
    async fn socket_mode_acknowledges_obeys_listed_users_and_replies_in_plain_text() {
        let (listener, ws_url) = testing::bind().await;
        let (base, calls) = web_api(ws_url).await;
        let backend = Arc::new(MockBackend::with(vec![job(JobKind::Rule, "r1", "Tidy <up> & co")]));
        let audit = Arc::new(MockAudit::default());
        let handler = Arc::new(Handler::new(Dialect::Slack, None, &["U0ADMIN".to_string()], 10, backend.clone(), audit.clone()));
        let cfg = SlackConfig { api_base: base, app_token: "xapp-good".into(), bot_token: "xoxb-bot".into(), http: reqwest::Client::new(), tls: tls() };
        let (tx, rx) = tokio::sync::watch::channel(false);
        let status = Arc::new(ChannelStatus::default());
        let task = tokio::spawn(run(cfg, handler, rx, status.clone()));

        let mut server = testing::accept(&listener).await;
        server.send(r#"{"type":"hello"}"#).await;
        server.send(&event("env-1", "Ev1", "U0ADMIN", "jobs", json!({}))).await;
        // Every envelope is acknowledged, whether or not it is obeyed.
        let ack = server.next().await;
        assert_eq!(serde_json::from_slice::<Value>(&ack.payload).unwrap(), json!({ "envelope_id": "env-1" }));
        server.send(&event("env-2", "Ev2", "U0STRANGER", "run Tidy <up> & co", json!({}))).await;
        assert_eq!(serde_json::from_slice::<Value>(&server.next().await.payload).unwrap(), json!({ "envelope_id": "env-2" }));
        server.send(&event("env-3", "Ev3", "U0ADMIN", "run Tidy <up> & co", json!({ "ts": "100.1" }))).await;
        assert_eq!(serde_json::from_slice::<Value>(&server.next().await.payload).unwrap(), json!({ "envelope_id": "env-3" }));
        // Slack redelivers the same event: acknowledged again, not run again.
        server.send(&event("env-3b", "Ev3", "U0ADMIN", "run Tidy <up> & co", json!({}))).await;
        assert_eq!(serde_json::from_slice::<Value>(&server.next().await.payload).unwrap(), json!({ "envelope_id": "env-3b" }));
        // The same post arriving again as an app_mention (new event id) is not run again either.
        server.send(&event("env-3c", "Ev3c", "U0ADMIN", "run Tidy <up> & co", json!({ "type": "app_mention", "ts": "100.1" }))).await;
        assert_eq!(serde_json::from_slice::<Value>(&server.next().await.payload).unwrap(), json!({ "envelope_id": "env-3c" }));
        // The bot's own reply arriving as an event is ignored.
        server.send(&event("env-4", "Ev4", "U0ADMIN", "Started Tidy", json!({ "bot_id": "B1" }))).await;
        assert_eq!(serde_json::from_slice::<Value>(&server.next().await.payload).unwrap(), json!({ "envelope_id": "env-4" }));

        for _ in 0..200 {
            if calls.lock().unwrap().iter().filter(|(p, _)| p.starts_with("/chat.postMessage")).count() >= 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        let posts: Vec<(String, Value)> = calls.lock().unwrap().iter().filter(|(p, _)| p.starts_with("/chat.postMessage")).cloned().collect();
        assert_eq!(posts.len(), 2, "{posts:?}: the stranger, the repeat and the bot's own message got nothing");
        assert!(posts[0].0.ends_with("Bearer xoxb-bot"));
        assert_eq!(posts[0].1["channel"], "D1");
        assert_eq!(posts[0].1["mrkdwn"], false);
        assert!(posts[0].1["text"].as_str().unwrap().contains("Tidy &lt;up&gt; &amp; co"), "{}", posts[0].1["text"]);
        assert_eq!(posts[1].1["text"], "Started Tidy &lt;up&gt; &amp; co");
        assert_eq!(backend.calls(), vec!["run rule:r1"], "run exactly once");
        assert_eq!(audit.entries.lock().unwrap().iter().map(|e| (e.channel, e.sender.as_str(), e.command.as_str())).collect::<Vec<_>>(), vec![("slack", "U0ADMIN", "jobs"), ("slack", "U0ADMIN", "run Tidy <up> & co")]);
        assert!(status.snapshot().connected);

        // Slack asks for a new connection: the bot reconnects on its own.
        server.send(r#"{"type":"disconnect","reason":"refresh_requested"}"#).await;
        let mut second = testing::accept(&listener).await;
        second.send(&event("env-5", "Ev5", "U0ADMIN", "status", json!({}))).await;
        assert_eq!(serde_json::from_slice::<Value>(&second.next().await.payload).unwrap(), json!({ "envelope_id": "env-5" }));
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        assert!(!status.snapshot().connected);
    }

    #[tokio::test]
    async fn a_refused_app_token_is_reported_without_the_token() {
        let (base, _calls) = web_api("ws://127.0.0.1:1/x".into()).await;
        let cfg = SlackConfig { api_base: base, app_token: "xapp-WRONG".into(), bot_token: "xoxb-bot".into(), http: reqwest::Client::new(), tls: tls() };
        let e = open_connection(&cfg).await.unwrap_err();
        assert!(e.contains("invalid_auth") && !e.contains("WRONG"), "{e}");
        let handler = Arc::new(Handler::new(Dialect::Slack, None, &["U0ADMIN".to_string()], 10, Arc::new(MockBackend::default()), Arc::new(MockAudit::default())));
        let (tx, rx) = tokio::sync::watch::channel(false);
        let status = Arc::new(ChannelStatus::default());
        let task = tokio::spawn(run(cfg, handler, rx, status.clone()));
        for _ in 0..200 {
            if status.snapshot().last_error.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let err = status.snapshot().last_error.unwrap();
        assert!(err.contains("invalid_auth") && !err.contains("WRONG"), "{err}");
        tx.send(true).unwrap();
        tokio::time::timeout(Duration::from_secs(5), task).await.unwrap().unwrap();
        // Unreachable servers do not leak the token.
        let dead = SlackConfig { api_base: "http://127.0.0.1:1".into(), app_token: "xapp-SECRET".into(), bot_token: "xoxb-SECRET".into(), http: reqwest::Client::new(), tls: tls() };
        assert!(!open_connection(&dead).await.unwrap_err().contains("SECRET"));
        assert!(!post_message(&dead, "D1", "x").await.unwrap_err().contains("SECRET"));
    }
}
