//! Real-server tests run when configured:
//! `UFOP_TEST_SMTP=127.0.0.1:2525|<maildir>|<ca.pem>` (aiosmtpd, STARTTLS + AUTH mailer/m41l),
//! `UFOP_TEST_XMPP=127.0.0.1:5222|<prosody data dir>|<ca.pem>` (users ufopbot/botpass, admin).

use super::*;

fn env(k: &str) -> Option<String> {
    std::env::var(k).ok().filter(|v| !v.is_empty())
}

fn ca(path: &str) -> CertPolicy {
    CertPolicy { accept_invalid_certs: false, ca_pem: Some(std::fs::read_to_string(path).unwrap()), pinned_sha256: vec![] }
}

#[tokio::test]
async fn smtp_starttls_auth_delivery() {
    let Some(spec) = env("UFOP_TEST_SMTP") else { return };
    let p: Vec<&str> = spec.split('|').collect();
    let (host, port) = p[0].split_once(':').unwrap();
    let cert = ca(p[2]);
    let before = std::fs::read_dir(p[1]).unwrap().count();
    // Server names in certificates: connect by name, not IP.
    let name = if host == "127.0.0.1" { "localhost" } else { host };
    send_email(name, port.parse().ok(), TlsMode::StartTls, "mailer", Some("m41l"), "UFOP <ufop@example.org>", &["ops@example.org".into(), "b@example.org".into()], "Backup ✓ done", "Résumé: 3 files\nline 2", None, &cert).await.unwrap();
    let files: Vec<_> = std::fs::read_dir(p[1]).unwrap().flatten().collect();
    assert_eq!(files.len(), before + 1);
    let newest = files.iter().max_by_key(|f| f.metadata().unwrap().modified().unwrap()).unwrap();
    let raw = std::fs::read_to_string(newest.path()).unwrap();
    assert!(raw.contains("X-Rcpt: ops@example.org,b@example.org"));
    assert!(raw.contains("Subject: =?UTF-8?B?"));
    let body = raw.split("\r\n\r\n").nth(1).unwrap().replace("\r\n", "");
    assert_eq!(String::from_utf8(base64::engine::general_purpose::STANDARD.decode(body.trim()).unwrap()).unwrap(), "Résumé: 3 files\nline 2");
    // Wrong password and untrusted certificate are refused.
    assert!(send_email(name, port.parse().ok(), TlsMode::StartTls, "mailer", Some("nope"), "a@b.c", &["x@y.z".into()], "s", "b", None, &cert).await.unwrap_err().user_message().contains("authentication"));
    assert!(send_email(name, port.parse().ok(), TlsMode::StartTls, "mailer", Some("m41l"), "a@b.c", &["x@y.z".into()], "s", "b", None, &CertPolicy::default()).await.unwrap_err().user_message().contains("TLS"));
    assert!(send_email(name, port.parse().ok(), TlsMode::StartTls, "", None, "a@b.c", &[], "s", "b", None, &cert).await.is_err());
}

#[tokio::test]
async fn xmpp_starttls_sasl_message() {
    let Some(spec) = env("UFOP_TEST_XMPP") else { return };
    let p: Vec<&str> = spec.split('|').collect();
    let (host, port) = p[0].split_once(':').unwrap();
    let marker = format!("backup finished {}", uuid::Uuid::new_v4().simple());
    send_xmpp("ufopbot@localhost", "botpass", "admin@localhost", host, port.parse().ok(), TlsMode::StartTls, &format!("{marker} <ok> & \"done\""), &ca(p[2])).await.unwrap();
    // Delivered to the offline store of the recipient.
    let mut found = false;
    for _ in 0..30 {
        let walk = walk(std::path::Path::new(p[1]));
        if walk.iter().any(|f| std::fs::read_to_string(f).unwrap_or_default().contains(&marker)) {
            found = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
    assert!(found, "message stored for admin@localhost");
    assert!(send_xmpp("ufopbot@localhost", "wrong", "admin@localhost", host, port.parse().ok(), TlsMode::StartTls, "x", &ca(p[2])).await.is_err());
}

fn walk(d: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(d).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[tokio::test]
async fn telegram_api_mock_and_channel_filter() {
    use axum::{extract::Path, routing::post, Json, Router};
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
    let s2 = seen.clone();
    let app = Router::new().route(
        "/{bot}/sendMessage",
        post(move |Path(bot): Path<String>, Json(v): Json<serde_json::Value>| {
            let s = s2.clone();
            async move {
                if bot != "bot123:ABC" {
                    return Json(serde_json::json!({ "ok": false, "description": "Unauthorized" }));
                }
                s.lock().unwrap().push(v);
                Json(serde_json::json!({ "ok": true }))
            }
        }),
    );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    let n = Notification {
        channel: Channel::Telegram { chat_id: "-1001".into(), api_base: base.clone() },
        send_on: vec!["error".into()],
        credential_id: String::new(),
        template: "Job {{name}}: {{result}}".into(),
        cert: CertPolicy::default(),
    };
    let mut vars = BTreeMap::new();
    vars.insert("name".to_string(), "Docs".to_string());
    vars.insert("result".to_string(), "error".to_string());
    // Filtered out: only errors are sent.
    assert!(deliver(&[(n.clone(), Some("123:ABC".into()))], "success", &vars, "body").await.is_empty());
    assert!(seen.lock().unwrap().is_empty());
    assert!(deliver(&[(n.clone(), Some("123:ABC".into()))], "error", &vars, "details").await.is_empty());
    let msg = seen.lock().unwrap()[0].clone();
    assert_eq!(msg["chat_id"], "-1001");
    assert_eq!(msg["text"], "Job Docs: error\n\ndetails");
    let errs = deliver(&[(n, Some("bad".into()))], "error", &vars, "x").await;
    assert!(errs[0].contains("Unauthorized"));
}

#[test]
fn path_redaction() {
    let roots = vec!["/home/alice/Documents".to_string()];
    let out = redact_paths("Cannot read /home/alice/Documents/taxes/2026.pdf; skipped /etc/secret/key.pem and C:\\Users\\bob\\notes.txt", &roots);
    assert!(!out.contains("alice") && !out.contains("bob") && !out.contains("secret"), "{out}");
    assert!(out.contains("[source:") && out.contains("…/key.pem") && out.contains("…/notes.txt"), "{out}");
    assert_eq!(redact_paths("no paths here", &roots), "no paths here");
}

#[test]
fn channel_serde_shape() {
    let n: Notification = serde_json::from_value(serde_json::json!({ "type": "email", "host": "smtp.example.org", "from": "a@b.c", "to": ["x@y.z"], "credential_id": "c1" })).unwrap();
    assert_eq!(n.send_on, vec!["warning", "error"]);
    assert!(matches!(n.channel, Channel::Email { tls: TlsMode::StartTls, .. }));
    let v = serde_json::to_value(&n).unwrap();
    assert_eq!(v["type"], "email");
    assert_eq!(v["subject"], "UFOP {{operation}} {{result}}: {{name}}");
}

// ── K104: webhooks, presets and template escaping ──

fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
}

#[test]
fn template_escaping_per_context() {
    use webhook::{render, Escape};
    let v = vars(&[("name", "Say \"hi\"\n<b>&"), ("n", "a b&c=d/é")]);
    assert_eq!(render(r#"{"t":"{{name}}"}"#, &v, Escape::Json), r#"{"t":"Say \"hi\"\n<b>&"}"#);
    assert_eq!(render(r#"{"t":{{json name}}}"#, &v, Escape::Json), r#"{"t":"Say \"hi\"\n<b>&"}"#);
    assert!(serde_json::from_str::<serde_json::Value>(&render(r#"{"t":"{{name}}","u":{{json n}}}"#, &v, Escape::Json)).is_ok());
    assert_eq!(render("x={{n}}", &v, Escape::Form), "x=a+b%26c%3Dd%2F%C3%A9");
    assert_eq!(render("/t/{{n}}", &v, Escape::Url), "/t/a%20b%26c%3Dd%2F%C3%A9");
    assert_eq!(render("<p>{{name}}</p>", &v, Escape::Html), "<p>Say &quot;hi&quot;\n&lt;b&gt;&amp;</p>");
    assert_eq!(render("{{raw name}}", &v, Escape::Header), "Say \"hi\" <b>&", "no CR/LF in headers, even raw");
    assert_eq!(render("{{ name }} {{missing}} {{raw n}}", &v, Escape::None), "Say \"hi\"\n<b>& {{missing}} a b&c=d/é");
    assert_eq!(render("open {{name", &v, Escape::None), "open {{name");
    assert!(webhook::uses_var("a {{raw secret}}", "secret") && webhook::uses_var("{{ secret }}", "secret") && !webhook::uses_var("{{secrets}}", "secret"));
}

#[test]
fn header_and_method_validation() {
    use webhook::{validate_headers, validate_url_and_method, WebhookHeader};
    let h = |n: &str, v: &str| WebhookHeader { name: n.into(), value: v.into(), ..Default::default() };
    assert!(validate_headers(&[h("X-Api-Key", "abc"), h("Authorization", "Bearer {{secret}}")]).is_ok());
    for bad in [h("Bad Name", "x"), h("X:Y", "x"), h("", "x"), h("X-Ok", "a\r\nX-Injected: 1"), h("Content-Length", "1"), h("host", "evil")] {
        assert!(validate_headers(&[bad.clone()]).is_err(), "{bad:?}");
    }
    assert!(validate_headers(&[WebhookHeader { name: "X-Token".into(), secret: true, ..Default::default() }]).is_err(), "a secret header needs a value");
    assert!(validate_url_and_method("https://x.example/hook", "patch").is_ok());
    assert!(validate_url_and_method("https://x.example/hook", "DELETE").is_err());
    assert!(validate_url_and_method("ftp://x.example/hook", "POST").is_err());
    assert!(validate_url_and_method("{{raw secret}}", "POST").is_ok());
    assert!(validate_url_and_method("https://g.example/message?token={{secret}}", "POST").is_ok());
}

type Seen = std::sync::Arc<std::sync::Mutex<Vec<(String, String, BTreeMap<String, String>, String)>>>;

/// A server recording every request: (method, path?query, headers, body).
async fn recorder() -> (String, Seen) {
    use axum::{extract::Request, Router};
    let seen: Seen = Default::default();
    let s2 = seen.clone();
    let app = Router::new().fallback(move |req: Request| {
        let s = s2.clone();
        async move {
            let (parts, body) = req.into_parts();
            let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap_or_default();
            let headers = parts.headers.iter().map(|(k, v)| (k.as_str().to_string(), v.to_str().unwrap_or_default().to_string())).collect();
            s.lock().unwrap().push((parts.method.to_string(), parts.uri.to_string(), headers, String::from_utf8_lossy(&bytes).into_owned()));
            "{\"ok\":true}"
        }
    });
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (base, seen)
}

fn run_vars(result: &str, previous: &str) -> BTreeMap<String, String> {
    vars(&[("name", "Docs \"main\""), ("operation", "backup"), ("result", result), ("machine", "pc1"), ("backup_id", "job-1"), ("previous_result", previous)])
}

#[tokio::test]
async fn webhook_presets_send_exact_requests() {
    use webhook::{preset, PresetParams, WebhookPreset};
    let (base, seen) = recorder().await;
    let take = || std::mem::take(&mut *seen.lock().unwrap());
    let send = |n: Notification, secret: &str, result: &str, previous: &str| {
        let n2 = n.clone();
        let s = secret.to_string();
        let (r, p) = (result.to_string(), previous.to_string());
        async move { deliver(&[(n2, Some(s).filter(|s| !s.is_empty()))], &r, &run_vars(&r, &p), "3 files\nline \"2\"").await }
    };

    // Pushover: form POST with token, user, title, message and priority.
    let n = preset(WebhookPreset::Pushover, &PresetParams { server: base.clone(), user: "u123".into(), ..Default::default() }).unwrap();
    assert!(send(n.clone(), "TOK", "error", "success").await.is_empty());
    let r = take().remove(0);
    assert_eq!((r.0.as_str(), r.1.as_str()), ("POST", "/1/messages.json"));
    assert_eq!(r.2["content-type"], "application/x-www-form-urlencoded");
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(r.3.as_bytes()).into_owned().collect();
    assert_eq!(form["token"], "TOK");
    assert_eq!(form["user"], "u123");
    assert_eq!(form["title"], "UFOP backup error: Docs \"main\"");
    assert_eq!(form["message"], "3 files\nline \"2\"");
    assert_eq!(form["priority"], "1");
    assert!(preset(WebhookPreset::Pushover, &PresetParams::default()).is_err());
    // Missing token: refused before sending.
    assert!(send(n, "", "error", "").await[0].contains("token"));
    assert!(take().is_empty());

    // ntfy: text body, Title/Priority/Tags headers and a bearer token.
    let n = preset(WebhookPreset::Ntfy, &PresetParams { server: format!("{base}/"), topic: "backups".into(), bearer: true, ..Default::default() }).unwrap();
    assert!(send(n.clone(), "tk_1", "warning", "").await.is_empty());
    let r = take().remove(0);
    assert_eq!((r.0.as_str(), r.1.as_str()), ("POST", "/backups"));
    assert_eq!(r.2["title"], "UFOP backup warning: Docs \"main\"");
    assert_eq!(r.2["priority"], "3");
    assert_eq!(r.2["tags"], "warning,ufop");
    assert_eq!(r.2["authorization"], "Bearer tk_1");
    assert_eq!(r.2["content-type"], "text/plain; charset=utf-8");
    assert_eq!(r.3, "3 files\nline \"2\"");
    assert!(preset(WebhookPreset::Ntfy, &PresetParams { topic: "a/b".into(), ..Default::default() }).is_err());

    // Discord: the webhook URL is the secret; JSON with content and embeds.
    let n = preset(WebhookPreset::Discord, &PresetParams::default()).unwrap();
    assert!(send(n, &format!("{base}/api/webhooks/1/abc"), "error", "").await.is_empty());
    let r = take().remove(0);
    assert_eq!(r.1, "/api/webhooks/1/abc");
    let v: serde_json::Value = serde_json::from_str(&r.3).unwrap();
    assert_eq!(v["content"], "**UFOP backup error: Docs \"main\"**");
    assert_eq!(v["embeds"][0]["description"], "3 files\nline \"2\"");
    assert_eq!(v["embeds"][0]["color"], 0xE53935);

    // PagerDuty: trigger on errors, nothing on a success after a success, resolve on recovery.
    let n = preset(WebhookPreset::Pagerduty, &PresetParams { server: base.clone(), ..Default::default() }).unwrap();
    assert_eq!(n.send_on, vec!["success", "warning", "error"]);
    assert!(send(n.clone(), "RK", "success", "success").await.is_empty());
    assert!(take().is_empty(), "no resolve without an open problem");
    assert!(send(n.clone(), "RK", "error", "success").await.is_empty());
    assert!(send(n.clone(), "RK", "success", "error").await.is_empty());
    let reqs = take();
    assert_eq!(reqs.len(), 2);
    let (t, res): (serde_json::Value, serde_json::Value) = (serde_json::from_str(&reqs[0].3).unwrap(), serde_json::from_str(&reqs[1].3).unwrap());
    assert_eq!(reqs[0].1, "/v2/enqueue");
    assert_eq!((t["routing_key"].as_str(), t["event_action"].as_str(), t["dedup_key"].as_str()), (Some("RK"), Some("trigger"), Some("ufop-job-1")));
    assert_eq!(t["payload"]["severity"], "critical");
    assert_eq!(t["payload"]["source"], "pc1");
    assert_eq!((res["event_action"].as_str(), res["dedup_key"].as_str()), (Some("resolve"), Some("ufop-job-1")));
    // A test triggers and then resolves.
    let mut tv = run_vars("test", "");
    tv.insert("backup_id".into(), "test".into());
    assert!(deliver(&[(Notification { send_on: vec!["test".into()], ..n }, Some("RK".into()))], "test", &tv, "t").await.is_empty());
    let reqs = take();
    assert_eq!(reqs.iter().map(|r| serde_json::from_str::<serde_json::Value>(&r.3).unwrap()["event_action"].as_str().unwrap().to_string()).collect::<Vec<_>>(), vec!["trigger", "resolve"]);

    // Gotify: token in the query string, JSON body with a numeric priority.
    let n = preset(WebhookPreset::Gotify, &PresetParams { server: base.clone(), ..Default::default() }).unwrap();
    assert!(send(n, "A&B", "error", "").await.is_empty());
    let r = take().remove(0);
    assert_eq!(r.1, "/message?token=A%26B");
    let v: serde_json::Value = serde_json::from_str(&r.3).unwrap();
    assert_eq!((v["title"].as_str(), v["priority"].as_i64()), (Some("UFOP backup error: Docs \"main\""), Some(8)));
    assert!(preset(WebhookPreset::Gotify, &PresetParams::default()).is_err());

    // Generic: PATCH, custom content type, headers (one secret, one non-ASCII) and a JSON template.
    let mut n = preset(WebhookPreset::Generic, &PresetParams { server: format!("{base}/hook/{{{{name}}}}"), ..Default::default() }).unwrap();
    if let Channel::Webhook { method, headers, body_template, content_type, .. } = &mut n.channel {
        *method = "PATCH".into();
        *content_type = "application/vnd.ufop+json".into();
        headers.push(webhook::WebhookHeader { name: "X-Api-Key".into(), value: "s3cr3t".into(), secret: true, credential_id: "h1".into() });
        headers.push(webhook::WebhookHeader { name: "X-Job".into(), value: "{{name}} ✓".into(), ..Default::default() });
        *body_template = r#"{"text":"{{name}}: {{result}}","detail":{{json message}}}"#.into();
    }
    assert!(send(n.clone(), "", "error", "").await.is_empty());
    let r = take().remove(0);
    assert_eq!((r.0.as_str(), r.1.as_str()), ("PATCH", "/hook/Docs%20%22main%22"));
    assert_eq!(r.2["x-api-key"], "s3cr3t");
    assert_eq!(r.2["x-job"], "=?UTF-8?B?RG9jcyAibWFpbiIg4pyT?=");
    assert_eq!(r.2["content-type"], "application/vnd.ufop+json");
    let v: serde_json::Value = serde_json::from_str(&r.3).unwrap();
    assert_eq!(v["text"], "Docs \"main\": error");
    assert_eq!(v["detail"], "3 files\nline \"2\"");
    // Default body: the run's values as JSON, never the secret.
    if let Channel::Webhook { body_template, url, .. } = &mut n.channel {
        body_template.clear();
        *url = format!("{base}/x?k={{{{secret}}}}");
    }
    assert!(send(n, "zz", "error", "").await.is_empty());
    let v: serde_json::Value = serde_json::from_str(&take()[0].3).unwrap();
    assert_eq!(v["name"], "Docs \"main\"");
    assert_eq!(v["message"], "3 files\nline \"2\"");
    assert!(v.get("secret").is_none());
}

#[test]
fn webhook_preview_masks_secrets() {
    let mut n = webhook::preset(webhook::WebhookPreset::Gotify, &webhook::PresetParams { server: "https://push.example.org".into(), ..Default::default() }).unwrap();
    if let Channel::Webhook { headers, .. } = &mut n.channel {
        headers.push(webhook::WebhookHeader { name: "X-K".into(), value: "real".into(), secret: true, credential_id: "c".into() });
    }
    let p = preview(&n, "error", &run_vars("error", ""), "body");
    let req = p.request.unwrap();
    assert!(req.starts_with("POST https://push.example.org/message?token="), "{req}");
    assert!(!req.contains("real") && req.contains("X-K: ******"), "{req}");
}

// ── K105: HTML email ──

fn parse_headers(block: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut last = String::new();
    for line in block.split("\r\n") {
        if line.starts_with(' ') || line.starts_with('\t') {
            if let Some(v) = out.get_mut(&last) {
                *v = format!("{v}{line}");
            }
        } else if let Some((k, v)) = line.split_once(':') {
            last = k.to_ascii_lowercase();
            out.insert(last.clone(), v.trim().to_string());
        }
    }
    out
}

fn decode_b64(s: &str) -> String {
    String::from_utf8(base64::engine::general_purpose::STANDARD.decode(s.replace("\r\n", "").trim()).unwrap()).unwrap()
}

fn decode_words(v: &str) -> String {
    v.split_whitespace().map(|w| decode_b64(w.trim_start_matches("=?UTF-8?B?").trim_end_matches("?="))).collect()
}

/// Parse a multipart/alternative message: (headers, [(part headers, decoded body)]).
fn parse_mime(raw: &str) -> (BTreeMap<String, String>, Vec<(BTreeMap<String, String>, String)>) {
    let (head, body) = raw.split_once("\r\n\r\n").unwrap();
    let h = parse_headers(head);
    let ct = h["content-type"].clone();
    let Some(b) = ct.split("boundary=\"").nth(1).map(|b| b.trim_end_matches('"').to_string()) else {
        return (h, vec![(BTreeMap::new(), decode_b64(body))]);
    };
    assert!(body.ends_with(&format!("--{b}--\r\n")), "closing boundary");
    let sep = format!("--{b}");
    let parts = body.split(&sep).skip(1).filter(|p| !p.starts_with("--")).map(|p| {
        let (ph, pb) = p.trim_start_matches("\r\n").split_once("\r\n\r\n").unwrap();
        (parse_headers(ph), decode_b64(pb))
    });
    (h, parts.collect())
}

#[test]
fn multipart_message_structure_and_escaping() {
    let mut v = run_vars("error", "");
    v.insert("name".into(), "<script>alert(1)</script> & Ünïcode".into());
    v.insert("message".into(), "line <1>\nline 2".into());
    let html = mime::render_html("", &v);
    assert!(html.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; Ünïcode") && !html.contains("<script>"));
    assert!(html.contains("line &lt;1&gt;") && html.contains("#c62828"));
    let subject = "Sauvegarde terminée ✓ — Résumé très long pour vérifier le découpage des mots encodés";
    let to = vec!["a@x.org".to_string(), "b@x.org".to_string()];
    let raw = mime::build_message(&mime::Mail { from: "UFOP <u@x.org>", to: &to, subject, text: "Résumé\nok", html: Some(&html), domain: "x.org" }, "Fri, 23 Oct 2026 10:00:00 +0000", "abc", "=_ufop_b1");
    for line in raw.split("\r\n") {
        assert!(line.len() <= 78, "line too long: {line}");
        assert!(!line.starts_with('.'), "no dot-stuffing needed");
    }
    let (h, parts) = parse_mime(&raw);
    assert_eq!(h["mime-version"], "1.0");
    assert_eq!(h["content-type"], "multipart/alternative; boundary=\"=_ufop_b1\"");
    assert_eq!(h["to"], "a@x.org, b@x.org");
    assert_eq!(decode_words(&h["subject"]), subject);
    assert!(h["subject"].split_whitespace().all(|w| w.len() <= 75 && w.starts_with("=?UTF-8?B?")));
    assert_eq!(parts.len(), 2);
    assert_eq!(parts[0].0["content-type"], "text/plain; charset=utf-8");
    assert_eq!(parts[0].0["content-transfer-encoding"], "base64");
    assert_eq!(parts[0].1, "Résumé\nok");
    assert_eq!(parts[1].0["content-type"], "text/html; charset=utf-8");
    assert_eq!(parts[1].1, html);
    // Text only without HTML.
    let raw = mime::build_message(&mime::Mail { from: "u@x.org", to: &to, subject: "plain", text: "hi", html: None, domain: "x.org" }, "d", "id", "b");
    let (h, parts) = parse_mime(&raw);
    assert_eq!((h["content-type"].as_str(), h["subject"].as_str(), parts[0].1.as_str()), ("text/plain; charset=utf-8", "plain", "hi"));
    // A custom template: values escaped, raw on request.
    assert_eq!(mime::render_html("<b>{{name}}</b>{{raw message}}", &vars(&[("name", "<i>"), ("message", "<hr>")])), "<b>&lt;i&gt;</b><hr>");
}

/// A minimal SMTP server (no TLS, no auth) that returns the DATA it received.
async fn smtp_mock() -> (u16, tokio::sync::oneshot::Receiver<String>) {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = l.local_addr().unwrap().port();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut s, _) = l.accept().await.unwrap();
        let mut buf = Vec::new();
        s.write_all(b"220 mock ESMTP\r\n").await.unwrap();
        let mut in_data = false;
        let mut data = String::new();
        loop {
            let mut chunk = [0u8; 8192];
            let n = s.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            loop {
                if in_data {
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    let Some(end) = text.find("\r\n.\r\n") else { break };
                    data = text[..end + 2].to_string();
                    buf.drain(..end + 5);
                    in_data = false;
                    s.write_all(b"250 queued\r\n").await.unwrap();
                    continue;
                }
                let Some(pos) = buf.windows(2).position(|w| w == b"\r\n") else { break };
                let line = String::from_utf8_lossy(&buf[..pos]).to_uppercase();
                buf.drain(..pos + 2);
                let reply: &[u8] = if line.starts_with("EHLO") {
                    b"250-mock\r\n250 8BITMIME\r\n"
                } else if line == "DATA" {
                    in_data = true;
                    b"354 go ahead\r\n"
                } else if line == "QUIT" {
                    s.write_all(b"221 bye\r\n").await.unwrap();
                    let _ = tx.send(data);
                    return;
                } else {
                    b"250 ok\r\n"
                };
                s.write_all(reply).await.unwrap();
            }
        }
    });
    (port, rx)
}

#[tokio::test]
async fn smtp_sends_html_alternative_through_a_mock_server() {
    let (port, rx) = smtp_mock().await;
    let n = Notification {
        channel: Channel::Email {
            host: "127.0.0.1".into(),
            port: Some(port),
            tls: TlsMode::None,
            username: String::new(),
            from: "UFOP <ufop@example.org>".into(),
            to: vec!["ops@example.org".into()],
            subject: default_subject(),
            html: true,
            html_template: String::new(),
        },
        send_on: vec!["error".into()],
        credential_id: String::new(),
        template: String::new(),
        cert: CertPolicy::default(),
    };
    assert!(deliver(&[(n.clone(), None)], "error", &run_vars("error", ""), "Report <with> markup").await.is_empty());
    let raw = rx.await.unwrap();
    let (h, parts) = parse_mime(&raw);
    assert!(h["content-type"].starts_with("multipart/alternative; boundary=\"=_ufop_"));
    assert_eq!(h["subject"], "UFOP backup error: Docs \"main\"");
    assert_eq!(parts[0].1, "Report <with> markup");
    assert!(parts[1].1.contains("Report &lt;with&gt; markup") && parts[1].1.contains("Docs &quot;main&quot;"));
    // Older channels (html off) stay text-only.
    let (port, rx) = smtp_mock().await;
    let mut old = n;
    if let Channel::Email { port: p, html, .. } = &mut old.channel {
        *p = Some(port);
        *html = false;
    }
    assert!(deliver(&[(old, None)], "error", &run_vars("error", ""), "plain").await.is_empty());
    let raw = rx.await.unwrap();
    assert!(raw.contains("Content-Type: text/plain; charset=utf-8") && !raw.contains("multipart"));
    // Channels saved before HTML mail parse with it off.
    let legacy: Notification = serde_json::from_value(serde_json::json!({ "type": "email", "host": "h", "from": "a@b.c", "to": ["x@y.z"] })).unwrap();
    assert!(matches!(legacy.channel, Channel::Email { html: false, .. }));
}

#[test]
fn email_preview_renders_html() {
    let n: Notification = serde_json::from_value(serde_json::json!({ "type": "email", "host": "h", "from": "a@b.c", "to": ["x@y.z"], "html": true, "html_template": "<h1>{{name}}</h1><p>{{message}}</p>" })).unwrap();
    let p = preview(&n, "success", &run_vars("success", ""), "all <good>");
    assert_eq!(p.subject, "UFOP backup success: Docs \"main\"");
    assert_eq!(p.html.as_deref(), Some("<h1>Docs &quot;main&quot;</h1><p>all &lt;good&gt;</p>"));
    assert!(p.request.is_none());
}

// ── K101: notification profiles in the manager ──

async fn manager(dir: &std::path::Path) -> crate::backup_engine::BackupManager {
    let repo = crate::storage::Repository::open_in_memory().await.unwrap();
    crate::backup_engine::BackupManager::with_kdf(repo.pool().clone(), std::sync::Arc::new(crate::security::CredentialStore::new()), None, None, dir.join("dbs"), ufop_core::backup::crypto::KdfParams::insecure_fast())
}

fn webhook_profile(name: &str, url: &str) -> profiles::NotificationProfile {
    let mut n = webhook::preset(webhook::WebhookPreset::Generic, &webhook::PresetParams { server: url.into(), ..Default::default() }).unwrap();
    if let Channel::Webhook { headers, .. } = &mut n.channel {
        headers.push(webhook::WebhookHeader { name: "X-Token".into(), value: "hdr-s3cret".into(), secret: true, credential_id: String::new() });
    }
    n.send_on = vec!["error".into()];
    profiles::NotificationProfile { id: String::new(), name: name.into(), description: String::new(), notification: n, min_severity: None, managed: false, created_at: String::new(), updated_at: String::new(), secret: Some("chan-s3cret".into()) }
}

#[tokio::test]
async fn profiles_crud_references_secrets_and_delete() {
    use crate::backup_engine::{templates, BackupDestination, BackupJob};
    let t = tempfile::tempdir().unwrap();
    let m = manager(t.path()).await;
    let (base, seen) = recorder().await;

    // Create: the secret and the secret header value go to the credential store.
    let p = m.save_notification_profile(webhook_profile("Ops hook", &format!("{base}/ops?k={{{{secret}}}}"))).await.unwrap();
    assert!(!p.id.is_empty() && !p.created_at.is_empty());
    let Channel::Webhook { headers, .. } = &p.notification.channel else { panic!() };
    assert!(headers[0].value.is_empty() && !headers[0].credential_id.is_empty());
    let stored = m.config_get(profiles::PROFILES_KEY).await.unwrap().unwrap();
    assert!(!stored.contains("hdr-s3cret") && !stored.contains("chan-s3cret"), "{stored}");
    assert!(m.has_destination_secret(&p.notification.credential_id).await.unwrap());
    assert!(m.save_notification_profile(webhook_profile("ops HOOK", &base)).await.is_err(), "duplicate name");

    // Test send resolves both secrets.
    m.test_notification_profile(&p.id).await.unwrap();
    let r = seen.lock().unwrap().remove(0);
    assert_eq!(r.1, "/ops?k=chan-s3cret");
    assert_eq!(r.2["x-token"], "hdr-s3cret");

    // Update keeps created_at and the stored secrets.
    let mut upd = p.clone();
    upd.min_severity = Some(profiles::Severity::Warning);
    let upd = m.save_notification_profile(upd).await.unwrap();
    assert_eq!(upd.created_at, p.created_at);
    assert_eq!(m.get_notification_profile(&p.id).await.unwrap().min_severity, Some(profiles::Severity::Warning));

    // A job referencing it (and a template that sets it).
    std::fs::create_dir_all(t.path().join("src")).unwrap();
    let mut job = BackupJob { name: "Docs".into(), encrypted: false, sources: vec![t.path().join("src").to_string_lossy().into()], ..Default::default() };
    job.destination = BackupDestination::Local { path: t.path().join("dst").to_string_lossy().into(), alternate_paths: vec![], marker_file: None };
    job.notification_profile_ids = vec!["missing".into()];
    assert!(m.save_job(job.clone(), None).await.is_err(), "unknown profile id");
    job.notification_profile_ids = vec![p.id.clone()];
    let saved = m.save_job(job, None).await.unwrap();
    let (list, warns) = m.job_notifications(&m.resolve_job(&saved).await.unwrap().job).await;
    assert_eq!((list.len(), warns.len()), (1, 0));
    assert_eq!(list[0].send_on, vec!["warning", "error"], "minimum severity applied");

    let p2 = m.save_notification_profile(webhook_profile("Second", &format!("{base}/two"))).await.unwrap();
    let tpl = m
        .save_template(templates::PolicyTemplate { name: "With alerts".into(), settings: templates::SettingsPatch { notification_profile_ids: Some(vec![p2.id.clone()]), ..Default::default() }, ..Default::default() })
        .await
        .unwrap();
    assert!(m.save_template(templates::PolicyTemplate { name: "Bad".into(), settings: templates::SettingsPatch { notification_profile_ids: Some(vec!["nope".into()]), ..Default::default() }, ..Default::default() }).await.is_err());
    let mut based = BackupJob { template_ids: vec![tpl.id.clone()], ..saved.clone() };
    based.overrides = Some(vec![]);
    let r = m.resolve_job(&based).await.unwrap();
    assert_eq!(r.job.notification_profile_ids, vec![p2.id.clone()], "the template sets the profile ids");

    // Delete: refused while referenced, forced removes references.
    let e = m.delete_notification_profile(&p.id, false).await.unwrap_err();
    assert!(e.user_message().contains("job 'Docs'"), "{}", e.user_message());
    let out = m.delete_notification_profile(&p.id, true).await.unwrap();
    assert_eq!(out.removed_from, vec!["job 'Docs'"]);
    assert_eq!(out.warnings.len(), 1);
    assert!(m.get_job(&saved.id).await.unwrap().notification_profile_ids.is_empty());
    assert!(!m.has_destination_secret(&p.notification.credential_id).await.unwrap(), "secrets removed with the profile");
    let out = m.delete_notification_profile(&p2.id, true).await.unwrap();
    assert_eq!(out.removed_from, vec!["template 'With alerts'"]);
    assert_eq!(m.get_template(&tpl.id).await.unwrap().settings.notification_profile_ids, Some(vec![]));
    assert!(m.delete_notification_profile(&p2.id, false).await.is_err(), "gone");

    // Organization profiles: read-only, referenced as org:<id>.
    let org = ufop_core::agent::ManagedNotificationProfile { id: "pager".into(), name: "Org pager".into(), notification: serde_json::json!({ "type": "webhook", "url": format!("{base}/org"), "credential_id": "central:pd" }), ..Default::default() };
    assert!(m.set_managed_notification_profiles(Some(&[org])).is_empty());
    let all = m.list_notification_profiles().await.unwrap();
    assert!(all.iter().any(|p| p.id == "org:pager" && p.managed));
    let mut o = m.get_notification_profile("org:pager").await.unwrap();
    o.name = "Mine now".into();
    assert!(m.save_notification_profile(o).await.is_err());
    assert!(m.delete_notification_profile("org:pager", true).await.is_err());
    let bad = ufop_core::agent::ManagedNotificationProfile { id: "x".into(), name: "Plain".into(), notification: serde_json::json!({ "type": "slack", "credential_id": "local" }), ..Default::default() };
    assert_eq!(m.set_managed_notification_profiles(Some(&[bad])).len(), 1);

    // Preview with sample values.
    let pv = m.preview_notification(&webhook_profile("P", "https://h.example/x").notification, "error");
    assert!(pv.request.unwrap().contains("X-Token: ******"));
}
