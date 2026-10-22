use super::*;

#[test]
fn bypass_rules() {
    let l = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert!(bypassed("api.example.com", &l(&["example.com"])));
    assert!(bypassed("example.com", &l(&[".example.com"])));
    assert!(!bypassed("badexample.com", &l(&["example.com"])));
    assert!(bypassed("10.1.2.3", &l(&["10.*"])));
    assert!(bypassed("intranet", &l(&["<local>"])));
    assert!(!bypassed("intranet.corp", &l(&["<local>"])));
    assert!(bypassed("anything", &l(&["*"])));
    assert!(bypassed("[::1]", &l(&["::1"])));
}

#[test]
fn parses_os_settings() {
    let mac = parse_scutil(
        "<dictionary> {\n  ExceptionsList : <array> {\n    0 : *.local\n    1 : 169.254/16\n  }\n  HTTPEnable : 1\n  HTTPPort : 3128\n  HTTPProxy : proxy.corp\n  HTTPSEnable : 0\n  ProxyAutoConfigEnable : 1\n  ProxyAutoConfigURLString : http://wpad/wpad.dat\n}\n",
    );
    assert_eq!(mac.http.as_deref(), Some("http://proxy.corp:3128"));
    assert_eq!(mac.https, None);
    assert_eq!(mac.bypass, vec!["*.local", "169.254/16"]);
    assert_eq!(mac.pac_url.as_deref(), Some("http://wpad/wpad.dat"));

    let w = parse_windows(true, "proxy:8080", "*.corp;<local>", "");
    assert_eq!((w.http.as_deref(), w.https.as_deref()), (Some("http://proxy:8080"), Some("http://proxy:8080")));
    assert_eq!(w.bypass, vec!["*.corp", "<local>"]);
    let w = parse_windows(true, "http=a:1;https=b:2", "", "http://pac/x.pac");
    assert_eq!((w.http.as_deref(), w.https.as_deref(), w.pac_url.as_deref()), (Some("http://a:1"), Some("http://b:2"), Some("http://pac/x.pac")));
    assert_eq!(parse_windows(false, "proxy:8080", "", "").http, None);
}

const PAC: &str = r#"
function FindProxyForURL(url, host) {
  if (isPlainHostName(host) || dnsDomainIs(host, ".corp.example")) return "DIRECT";
  if (shExpMatch(host, "*.media.example")) return "SOCKS5 socks.example:1080";
  if (isInNet(host, "10.0.0.0", "255.0.0.0")) return "DIRECT";
  if (url.substring(0, 6) == "https:") return "PROXY secure.example:8443; DIRECT";
  return "PROXY proxy.example:3128";
}
"#;

#[test]
fn pac_evaluation() {
    let p = pac::PacScript::from_source(PAC).unwrap();
    assert_eq!(p.find("http://intranet/", "intranet"), None);
    assert_eq!(p.find("http://wiki.corp.example/", "wiki.corp.example"), None);
    assert_eq!(p.find("http://cdn.media.example/x", "cdn.media.example").as_deref(), Some("socks5://socks.example:1080"));
    assert_eq!(p.find("http://10.2.3.4/", "10.2.3.4"), None);
    assert_eq!(p.find("https://api.example.org/", "api.example.org").as_deref(), Some("http://secure.example:8443"));
    assert_eq!(p.find("http://www.example.org/", "www.example.org").as_deref(), Some("http://proxy.example:3128"));
    assert!(pac::PacScript::from_source("function nope(){}").is_err());
    assert!(pac::PacScript::from_source("function FindProxyForURL(u,h){ return ").is_err());
    // A throwing script goes direct rather than failing the request.
    let bad = pac::PacScript::from_source("function FindProxyForURL(u,h){ throw 'x'; }").unwrap();
    assert_eq!(bad.find("http://a/", "a"), None);
    assert_eq!(pac::first_proxy("BOGUS x; HTTPS h:1"), Some("https://h:1".into()));
    // Regression: unresolvable hosts made isInNet throw (undefined vs null).
    let r = pac::PacScript::from_source(r#"function FindProxyForURL(u,h){ return dnsResolve("no-such-host.invalid") === null && !isResolvable("no-such-host.invalid") ? "PROXY ok:1" : "DIRECT"; }"#).unwrap();
    assert_eq!(r.eval("http://x/", "x").unwrap(), "PROXY ok:1");
}

#[test]
fn pac_time_helpers_run() {
    let p = pac::PacScript::from_source(
        r#"function FindProxyForURL(u,h){ return (weekdayRange("SUN","SAT") && timeRange(0, 24) && dateRange("JAN","DEC")) ? "PROXY a:1" : "DIRECT"; }"#,
    )
    .unwrap();
    assert_eq!(p.find("http://x/", "x").as_deref(), Some("http://a:1"));
}

/// Mode resolution; one test since the active settings are process-wide.
#[tokio::test]
async fn modes_resolve_and_carry_credentials() {
    let _guard = MODE_LOCK.lock().await;
    let url = |s: &str| reqwest::Url::parse(s).unwrap();
    apply(
        ProxySettings { mode: ProxyMode::Manual, url: "http://proxy.local:3128".into(), no_proxy: vec!["internal.example".into()], username: "me".into(), ..Default::default() },
        Some("s3cret".into()),
    )
    .unwrap();
    let p = proxy_for(&url("https://api.example.com/x")).unwrap();
    assert_eq!((p.host_str(), p.port(), p.username(), p.password()), (Some("proxy.local"), Some(3128), "me", Some("s3cret")));
    assert_eq!(proxy_for(&url("https://svc.internal.example/")), None);

    apply(ProxySettings { mode: ProxyMode::None, ..Default::default() }, None).unwrap();
    assert_eq!(proxy_for(&url("https://api.example.com/")), None);

    let dir = tempfile::tempdir().unwrap();
    let pac_path = dir.path().join("p.pac");
    std::fs::write(&pac_path, PAC).unwrap();
    apply(ProxySettings { mode: ProxyMode::Pac, pac_url: pac_path.to_string_lossy().into(), ..Default::default() }, None).unwrap();
    assert_eq!(proxy_for(&url("http://www.example.org/")).unwrap().as_str(), "http://proxy.example:3128/");
    assert_eq!(current().mode, ProxyMode::Pac);

    // Invalid settings are refused and leave the previous ones in force.
    assert!(apply(ProxySettings { mode: ProxyMode::Manual, url: "not a url".into(), ..Default::default() }, None).is_err());
    assert!(apply(ProxySettings { mode: ProxyMode::Pac, pac_url: "".into(), ..Default::default() }, None).is_err());
    assert_eq!(current().mode, ProxyMode::Pac);
    apply(ProxySettings::default(), None).unwrap();
}

#[tokio::test]
async fn clients_route_through_the_manual_proxy() {
    // A tiny HTTP proxy: records the request line, answers 200.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = std::sync::Arc::new(tokio::sync::Mutex::new(String::new()));
    let seen2 = seen.clone();
    tokio::spawn(async move {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let (mut s, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 4096];
        let n = s.read(&mut buf).await.unwrap();
        *seen2.lock().await = String::from_utf8_lossy(&buf[..n]).to_string();
        s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok").await.unwrap();
    });
    // Build the client first: settings applied later still take effect.
    let c = client();
    let _guard = MODE_LOCK.lock().await;
    apply(ProxySettings { mode: ProxyMode::Manual, url: format!("http://{addr}"), ..Default::default() }, None).unwrap();
    let body = c.get("http://upstream.invalid/hello").send().await.unwrap().text().await.unwrap();
    apply(ProxySettings::default(), None).unwrap();
    assert_eq!(body, "ok");
    assert!(seen.lock().await.starts_with("GET http://upstream.invalid/hello HTTP/1.1"), "{}", seen.lock().await);
}

/// Serialises the tests that change the process-wide proxy.
static MODE_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[test]
fn loopback_and_metadata_never_proxied() {
    for h in ["localhost", "127.0.0.1", "169.254.169.254", "[::1]", "fe80::1"] {
        assert!(always_direct(h), "{h}");
    }
    assert!(!always_direct("10.0.0.1") && !always_direct("example.com"));
}
