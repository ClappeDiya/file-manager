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
    send_email(name, port.parse().ok(), TlsMode::StartTls, "mailer", Some("m41l"), "UFOP <ufop@example.org>", &["ops@example.org".into(), "b@example.org".into()], "Backup ✓ done", "Résumé: 3 files\nline 2", &cert).await.unwrap();
    let files: Vec<_> = std::fs::read_dir(p[1]).unwrap().flatten().collect();
    assert_eq!(files.len(), before + 1);
    let newest = files.iter().max_by_key(|f| f.metadata().unwrap().modified().unwrap()).unwrap();
    let raw = std::fs::read_to_string(newest.path()).unwrap();
    assert!(raw.contains("X-Rcpt: ops@example.org,b@example.org"));
    assert!(raw.contains("Subject: =?UTF-8?B?"));
    let body = raw.split("\r\n\r\n").nth(1).unwrap().replace("\r\n", "");
    assert_eq!(String::from_utf8(base64::engine::general_purpose::STANDARD.decode(body.trim()).unwrap()).unwrap(), "Résumé: 3 files\nline 2");
    // Wrong password and untrusted certificate are refused.
    assert!(send_email(name, port.parse().ok(), TlsMode::StartTls, "mailer", Some("nope"), "a@b.c", &["x@y.z".into()], "s", "b", &cert).await.unwrap_err().user_message().contains("authentication"));
    assert!(send_email(name, port.parse().ok(), TlsMode::StartTls, "mailer", Some("m41l"), "a@b.c", &["x@y.z".into()], "s", "b", &CertPolicy::default()).await.unwrap_err().user_message().contains("TLS"));
    assert!(send_email(name, port.parse().ok(), TlsMode::StartTls, "", None, "a@b.c", &[], "s", "b", &cert).await.is_err());
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
