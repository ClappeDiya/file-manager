//! Browser sign-in for native apps (RFC 8252): a one-shot HTTP listener on
//! `127.0.0.1` receives the redirect with the authorization code, and PKCE
//! (RFC 7636, S256) binds that code to this process.

use crate::core::error::AppError;
use base64::Engine as _;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

pub const CALLBACK_PATH: &str = "/callback";

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn random(n: usize) -> String {
    let mut b = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut b);
    b64url(&b)
}

/// PKCE verifier (43 chars) and its S256 challenge.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

pub fn pkce() -> Pkce {
    let verifier = random(32);
    let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
    Pkce { verifier, challenge }
}

pub fn new_state() -> String {
    random(16)
}

/// Bound listener waiting for the browser to come back.
pub struct Loopback {
    listener: TcpListener,
    pub redirect_uri: String,
}

fn page(title: &str, body: &str) -> String {
    let html = format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title><body style=\"font:16px system-ui;margin:4rem auto;max-width:32rem\"><h1 style=\"font-size:1.3rem\">{title}</h1><p>{body}</p>"
    );
    format!("HTTP/1.1 200 OK\r\ncontent-type: text/html; charset=utf-8\r\ncache-control: no-store\r\nconnection: close\r\ncontent-length: {}\r\n\r\n{html}", html.len())
}

/// `GET /callback?a=b HTTP/1.1` → path and decoded query.
fn parse_target(request: &str) -> Option<(String, HashMap<String, String>)> {
    let line = request.lines().next()?;
    let mut parts = line.split_whitespace();
    if parts.next()? != "GET" {
        return None;
    }
    let url = reqwest::Url::parse(&format!("http://127.0.0.1{}", parts.next()?)).ok()?;
    Some((url.path().to_string(), url.query_pairs().into_owned().collect()))
}

impl Loopback {
    pub async fn bind() -> Result<Self, AppError> {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.map_err(|e| AppError::internal(format!("Cannot listen for the sign-in on this computer: {e}")))?;
        let port = listener.local_addr()?.port();
        Ok(Self { listener, redirect_uri: format!("http://127.0.0.1:{port}{CALLBACK_PATH}") })
    }

    /// Listen on `port` (0 = any free port) and name the redirect URI with
    /// `host` (`localhost` for providers that register it by name, e.g.
    /// Dropbox, whose registered redirect URIs must match exactly).
    pub async fn bind_on(port: u16, host: &str) -> Result<Self, AppError> {
        let listener = TcpListener::bind(("127.0.0.1", port)).await.map_err(|e| AppError::State {
            message: format!("Cannot listen for the sign-in on port {port}: {e}"),
            advice: "Another sign-in may be in progress, or another app uses the port; finish or cancel it and try again.".into(),
        })?;
        let port = listener.local_addr()?.port();
        Ok(Self { listener, redirect_uri: format!("http://{host}:{port}{CALLBACK_PATH}") })
    }

    /// Wait for the redirect carrying `state`; returns its `code`. Other
    /// requests (favicon, stray probes) are answered and ignored.
    pub async fn wait_for_code(self, state: &str, timeout: Duration) -> Result<String, AppError> {
        let fut = async {
            loop {
                let (mut sock, _) = self.listener.accept().await?;
                let mut buf = vec![0u8; 8192];
                let n = match tokio::time::timeout(Duration::from_secs(10), sock.read(&mut buf)).await {
                    Ok(Ok(n)) => n,
                    _ => continue,
                };
                let req = String::from_utf8_lossy(&buf[..n]).to_string();
                let Some((_, q)) = parse_target(&req).filter(|(p, _)| p == CALLBACK_PATH) else {
                    let _ = sock.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
                    continue;
                };
                if q.get("state").map(String::as_str) != Some(state) {
                    // Not our sign-in (or a forged redirect): keep waiting.
                    let _ = sock.write_all(page("Sign-in not recognised", "This link does not belong to the sign-in UFOP started. Return to the app and try again.").as_bytes()).await;
                    continue;
                }
                if let Some(err) = q.get("error") {
                    let _ = sock.write_all(page("Sign-in cancelled", "You can close this tab and return to UFOP.").as_bytes()).await;
                    return Err(AppError::PermissionDenied {
                        message: if err == "access_denied" { "The sign-in was denied in the browser".into() } else { format!("The sign-in page returned an error: {err}") },
                        advice: "Start the sign-in again.".into(),
                    });
                }
                let Some(code) = q.get("code").filter(|c| !c.is_empty()).cloned() else {
                    let _ = sock.write_all(page("Sign-in incomplete", "No sign-in code came back. Return to the app and try again.").as_bytes()).await;
                    return Err(AppError::validation("The sign-in page did not return a code"));
                };
                let _ = sock.write_all(page("UFOP is connecting", "You can close this tab and return to UFOP.").as_bytes()).await;
                return Ok::<String, AppError>(code);
            }
        };
        tokio::time::timeout(timeout, fut)
            .await
            .map_err(|_| AppError::state("The browser sign-in was not completed in time", "Start the sign-in again and approve it in the browser."))?
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn get(uri: &str, target: &str) -> String {
        let port = reqwest::Url::parse(uri).unwrap().port().unwrap();
        let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(format!("GET {target} HTTP/1.1\r\nhost: 127.0.0.1\r\n\r\n").as_bytes()).await.unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).await.unwrap();
        out
    }

    #[test]
    fn pkce_challenge_is_s256_of_the_verifier() {
        let p = pkce();
        assert_eq!(p.verifier.len(), 43);
        assert_eq!(p.challenge, b64url(&Sha256::digest(p.verifier.as_bytes())));
        assert_ne!(pkce().verifier, p.verifier);
        // RFC 7636 appendix B test vector.
        assert_eq!(b64url(&Sha256::digest(b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk")), "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    }

    #[tokio::test]
    async fn receives_the_code_for_its_state_and_ignores_the_rest() {
        let lb = Loopback::bind().await.unwrap();
        let uri = lb.redirect_uri.clone();
        assert!(uri.starts_with("http://127.0.0.1:") && uri.ends_with(CALLBACK_PATH));
        let wait = tokio::spawn(lb.wait_for_code_owned("good".into()));
        assert!(get(&uri, "/favicon.ico").await.starts_with("HTTP/1.1 404"));
        assert!(get(&uri, "/callback?code=evil&state=forged").await.contains("not recognised"));
        assert!(get(&uri, "/callback?code=ufop_connect_1&state=good").await.contains("You can close this tab"));
        assert_eq!(wait.await.unwrap().unwrap(), "ufop_connect_1");
    }

    #[tokio::test]
    async fn denial_and_timeout_are_errors() {
        let lb = Loopback::bind().await.unwrap();
        let uri = lb.redirect_uri.clone();
        let wait = tokio::spawn(lb.wait_for_code_owned("s".into()));
        get(&uri, "/callback?error=access_denied&state=s").await;
        assert!(wait.await.unwrap().unwrap_err().user_message().contains("denied"));

        let lb = Loopback::bind().await.unwrap();
        let e = lb.wait_for_code("s", Duration::from_millis(50)).await.unwrap_err();
        assert!(e.user_message().contains("not completed in time"));
    }

    impl Loopback {
        async fn wait_for_code_owned(self, state: String) -> Result<String, AppError> {
            self.wait_for_code(&state, Duration::from_secs(5)).await
        }
    }
}
