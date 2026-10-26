//! A small WebSocket client (RFC 6455) for Slack's Socket Mode: the
//! handshake, masked client frames, and reading text messages (answering
//! pings, joining fragments). It speaks `ws://` and `wss://`.
//!
//! The frame codec is symmetric (`encode_frame` can mask or not,
//! `decode_frame` accepts either), which lets the tests run a real server
//! on a local socket.

use base64::Engine;
use sha1::{Digest, Sha1};
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpStream;

/// Largest message accepted from the server.
pub const MAX_MESSAGE: usize = 4 * 1024 * 1024;
const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub trait Io: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Io for T {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opcode {
    Continuation,
    Text,
    Binary,
    Close,
    Ping,
    Pong,
}

impl Opcode {
    fn from_u8(b: u8) -> Option<Self> {
        Some(match b {
            0x0 => Self::Continuation,
            0x1 => Self::Text,
            0x2 => Self::Binary,
            0x8 => Self::Close,
            0x9 => Self::Ping,
            0xA => Self::Pong,
            _ => return None,
        })
    }
    fn code(self) -> u8 {
        match self {
            Self::Continuation => 0x0,
            Self::Text => 0x1,
            Self::Binary => 0x2,
            Self::Close => 0x8,
            Self::Ping => 0x9,
            Self::Pong => 0xA,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub fin: bool,
    pub opcode: Opcode,
    pub payload: Vec<u8>,
}

/// Encode one frame. Client frames must be masked (`mask: Some`), server
/// frames must not.
pub fn encode_frame(fin: bool, opcode: Opcode, payload: &[u8], mask: Option<[u8; 4]>) -> Vec<u8> {
    let mut out = Vec::with_capacity(payload.len() + 14);
    out.push(if fin { 0x80 } else { 0 } | opcode.code());
    let m = if mask.is_some() { 0x80 } else { 0 };
    match payload.len() {
        n if n < 126 => out.push(m | n as u8),
        n if n <= u16::MAX as usize => {
            out.push(m | 126);
            out.extend_from_slice(&(n as u16).to_be_bytes());
        }
        n => {
            out.push(m | 127);
            out.extend_from_slice(&(n as u64).to_be_bytes());
        }
    }
    match mask {
        Some(key) => {
            out.extend_from_slice(&key);
            out.extend(payload.iter().enumerate().map(|(i, b)| b ^ key[i % 4]));
        }
        None => out.extend_from_slice(payload),
    }
    out
}

/// Why a frame could not be read.
#[derive(Debug, PartialEq, Eq)]
pub enum FrameError {
    /// Not a frame this client accepts (reserved bits, unknown opcode, oversize, bad control frame).
    Invalid(&'static str),
}

/// Decode the frame at the start of `buf`: `Ok(Some((frame, bytes used)))`,
/// or `Ok(None)` when more bytes are needed.
pub fn decode_frame(buf: &[u8], max: usize) -> Result<Option<(Frame, usize)>, FrameError> {
    if buf.len() < 2 {
        return Ok(None);
    }
    let (b0, b1) = (buf[0], buf[1]);
    if b0 & 0x70 != 0 {
        return Err(FrameError::Invalid("reserved bits set"));
    }
    let opcode = Opcode::from_u8(b0 & 0x0f).ok_or(FrameError::Invalid("unknown opcode"))?;
    let masked = b1 & 0x80 != 0;
    let mut pos = 2;
    let len = match b1 & 0x7f {
        126 => {
            if buf.len() < pos + 2 {
                return Ok(None);
            }
            let n = u16::from_be_bytes([buf[pos], buf[pos + 1]]) as usize;
            pos += 2;
            n
        }
        127 => {
            if buf.len() < pos + 8 {
                return Ok(None);
            }
            let mut b = [0u8; 8];
            b.copy_from_slice(&buf[pos..pos + 8]);
            pos += 8;
            u64::from_be_bytes(b).min(usize::MAX as u64) as usize
        }
        n => n as usize,
    };
    if len > max {
        return Err(FrameError::Invalid("frame too large"));
    }
    let is_control = matches!(opcode, Opcode::Close | Opcode::Ping | Opcode::Pong);
    if is_control && (len > 125 || b0 & 0x80 == 0) {
        return Err(FrameError::Invalid("bad control frame"));
    }
    let key = if masked {
        if buf.len() < pos + 4 {
            return Ok(None);
        }
        let k = [buf[pos], buf[pos + 1], buf[pos + 2], buf[pos + 3]];
        pos += 4;
        Some(k)
    } else {
        None
    };
    if buf.len() < pos + len {
        return Ok(None);
    }
    let mut payload = buf[pos..pos + len].to_vec();
    if let Some(k) = key {
        payload.iter_mut().enumerate().for_each(|(i, b)| *b ^= k[i % 4]);
    }
    Ok(Some((Frame { fin: b0 & 0x80 != 0, opcode, payload }, pos + len)))
}

/// `Sec-WebSocket-Accept` for a request key.
pub fn accept_key(key: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(Sha1::digest(format!("{key}{GUID}").as_bytes()))
}

/// What a read produced.
#[derive(Debug, PartialEq, Eq)]
pub enum Message {
    Text(String),
    /// The server closed the connection.
    Closed,
}

/// A connected WebSocket.
pub struct WsClient {
    io: Box<dyn Io>,
    buf: Vec<u8>,
    partial: Vec<u8>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

impl WsClient {
    /// Connect to `url` (`ws://` or `wss://`) and complete the handshake.
    /// `tls` is used for `wss`.
    pub async fn connect(url: &str, tls: Arc<rustls::ClientConfig>, timeout: Duration) -> Result<Self, String> {
        let u = url::Url::parse(url).map_err(|_| "The WebSocket address is not a valid URL".to_string())?;
        let secure = match u.scheme() {
            "wss" => true,
            "ws" => false,
            other => return Err(format!("Unsupported WebSocket scheme '{other}'")),
        };
        let host = u.host_str().ok_or("The WebSocket address has no host")?.to_string();
        let port = u.port_or_known_default().unwrap_or(if secure { 443 } else { 80 });
        let tcp = tokio::time::timeout(timeout, TcpStream::connect((host.as_str(), port))).await.map_err(|_| format!("Connecting to {host} timed out"))?.map_err(|e| format!("Connecting to {host}: {e}"))?;
        let mut io: Box<dyn Io> = if secure {
            let name = rustls::pki_types::ServerName::try_from(host.clone()).map_err(|_| format!("Invalid server name '{host}'"))?;
            let s = tokio::time::timeout(timeout, tokio_rustls::TlsConnector::from(tls).connect(name, tcp)).await.map_err(|_| format!("TLS with {host} timed out"))?.map_err(|e| format!("TLS with {host} failed: {e}"))?;
            Box::new(s)
        } else {
            Box::new(tcp)
        };
        let key = {
            use rand::RngCore;
            let mut raw = [0u8; 16];
            rand::thread_rng().fill_bytes(&mut raw);
            base64::engine::general_purpose::STANDARD.encode(raw)
        };
        let mut path = u.path().to_string();
        if let Some(q) = u.query() {
            path.push('?');
            path.push_str(q);
        }
        let req = format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: {key}\r\nSec-WebSocket-Version: 13\r\nUser-Agent: ufop-remote-control\r\n\r\n");
        io.write_all(req.as_bytes()).await.map_err(err)?;
        // Read the response head.
        let mut head = Vec::new();
        let mut chunk = [0u8; 1024];
        let end = loop {
            if let Some(i) = head.windows(4).position(|w| w == b"\r\n\r\n") {
                break i + 4;
            }
            if head.len() > 16 * 1024 {
                return Err("The WebSocket handshake response is too long".into());
            }
            let n = tokio::time::timeout(timeout, io.read(&mut chunk)).await.map_err(|_| "The WebSocket handshake timed out".to_string())?.map_err(err)?;
            if n == 0 {
                return Err("The server closed the connection during the WebSocket handshake".into());
            }
            head.extend_from_slice(&chunk[..n]);
        };
        let text = String::from_utf8_lossy(&head[..end]).into_owned();
        let status_ok = text.lines().next().is_some_and(|l| l.split_whitespace().nth(1) == Some("101"));
        if !status_ok {
            return Err(format!("The server refused the WebSocket upgrade: {}", text.lines().next().unwrap_or("")));
        }
        let accept = text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.trim().eq_ignore_ascii_case("sec-websocket-accept")).map(|(_, v)| v.trim().to_string()));
        if accept.as_deref() != Some(accept_key(&key).as_str()) {
            return Err("The server's WebSocket handshake answer is wrong".into());
        }
        Ok(Self { io, buf: head[end..].to_vec(), partial: Vec::new() })
    }

    /// Wrap an already-upgraded stream (tests).
    #[cfg(test)]
    pub fn from_stream(io: Box<dyn Io>) -> Self {
        Self { io, buf: Vec::new(), partial: Vec::new() }
    }

    async fn write_frame(&mut self, opcode: Opcode, payload: &[u8]) -> Result<(), String> {
        use rand::RngCore;
        let mut key = [0u8; 4];
        rand::thread_rng().fill_bytes(&mut key);
        self.io.write_all(&encode_frame(true, opcode, payload, Some(key))).await.map_err(err)
    }

    pub async fn send_text(&mut self, text: &str) -> Result<(), String> {
        self.write_frame(Opcode::Text, text.as_bytes()).await
    }

    pub async fn close(&mut self) {
        let _ = self.write_frame(Opcode::Close, &[]).await;
    }

    /// The next text message. Pings are answered, pongs and binary frames
    /// skipped, fragments joined.
    pub async fn recv(&mut self) -> Result<Message, String> {
        loop {
            while let Some((frame, used)) = decode_frame(&self.buf, MAX_MESSAGE).map_err(|FrameError::Invalid(m)| format!("Invalid WebSocket frame: {m}"))? {
                self.buf.drain(..used);
                match frame.opcode {
                    Opcode::Ping => self.write_frame(Opcode::Pong, &frame.payload).await?,
                    Opcode::Pong | Opcode::Binary => {}
                    Opcode::Close => {
                        let _ = self.write_frame(Opcode::Close, &frame.payload).await;
                        return Ok(Message::Closed);
                    }
                    Opcode::Text | Opcode::Continuation => {
                        if frame.opcode == Opcode::Text {
                            self.partial.clear();
                        }
                        self.partial.extend_from_slice(&frame.payload);
                        if self.partial.len() > MAX_MESSAGE {
                            return Err("A WebSocket message was too large".into());
                        }
                        if frame.fin {
                            let text = String::from_utf8(std::mem::take(&mut self.partial)).map_err(|_| "A WebSocket message was not valid UTF-8".to_string())?;
                            return Ok(Message::Text(text));
                        }
                    }
                }
            }
            let mut chunk = [0u8; 8192];
            let n = self.io.read(&mut chunk).await.map_err(err)?;
            if n == 0 {
                return Ok(Message::Closed);
            }
            self.buf.extend_from_slice(&chunk[..n]);
        }
    }
}

#[cfg(test)]
pub mod testing {
    //! A WebSocket server on a local port for tests.
    use super::*;
    use tokio::net::TcpListener;

    /// Accept one connection, complete the handshake, and return the
    /// server side: `(read next client frame, write unmasked frame)`.
    pub struct ServerSide {
        pub io: TcpStream,
        buf: Vec<u8>,
    }

    pub async fn bind() -> (TcpListener, String) {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}/link", l.local_addr().unwrap());
        (l, url)
    }

    pub async fn accept(l: &TcpListener) -> ServerSide {
        let (mut io, _) = l.accept().await.unwrap();
        let mut head = Vec::new();
        let mut chunk = [0u8; 1024];
        while !head.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = io.read(&mut chunk).await.unwrap();
            assert!(n > 0, "client closed during handshake");
            head.extend_from_slice(&chunk[..n]);
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let key = text.lines().find_map(|l| l.split_once(':').filter(|(k, _)| k.eq_ignore_ascii_case("sec-websocket-key")).map(|(_, v)| v.trim().to_string())).expect("key header");
        let resp = format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", accept_key(&key));
        io.write_all(resp.as_bytes()).await.unwrap();
        ServerSide { io, buf: Vec::new() }
    }

    impl ServerSide {
        pub async fn send(&mut self, text: &str) {
            self.io.write_all(&encode_frame(true, Opcode::Text, text.as_bytes(), None)).await.unwrap();
        }

        pub async fn send_frame(&mut self, fin: bool, opcode: Opcode, payload: &[u8]) {
            self.io.write_all(&encode_frame(fin, opcode, payload, None)).await.unwrap();
        }

        /// The next frame from the client (must be masked).
        pub async fn next(&mut self) -> Frame {
            loop {
                if let Some((f, used)) = decode_frame(&self.buf, MAX_MESSAGE).unwrap() {
                    self.buf.drain(..used);
                    return f;
                }
                let mut chunk = [0u8; 4096];
                let n = self.io.read(&mut chunk).await.unwrap();
                assert!(n > 0, "client closed");
                self.buf.extend_from_slice(&chunk[..n]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn tls() -> Arc<rustls::ClientConfig> {
        Arc::new(crate::security::tls::client_config(false, None, &[]).unwrap())
    }

    #[test]
    fn the_accept_key_matches_the_rfc_example() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn frames_round_trip_masked_and_unmasked_at_every_length_class() {
        for len in [0usize, 1, 125, 126, 65535, 65536, 70000] {
            let payload: Vec<u8> = (0..len).map(|i| (i % 251) as u8).collect();
            for mask in [None, Some([1u8, 2, 3, 4])] {
                let bytes = encode_frame(true, Opcode::Binary, &payload, mask);
                let (f, used) = decode_frame(&bytes, 1 << 20).unwrap().unwrap();
                assert_eq!((f.fin, f.opcode, f.payload.len(), used), (true, Opcode::Binary, len, bytes.len()));
                assert_eq!(f.payload, payload);
                // Every strict prefix asks for more bytes.
                if len < 300 {
                    for cut in 0..bytes.len() {
                        assert_eq!(decode_frame(&bytes[..cut], 1 << 20), Ok(None), "len {len} cut {cut}");
                    }
                }
            }
        }
    }

    #[test]
    fn bad_frames_are_refused() {
        assert!(decode_frame(&[0xC1, 0x00], 100).is_err(), "reserved bit");
        assert!(decode_frame(&[0x83, 0x00], 100).is_err(), "unknown opcode");
        assert!(decode_frame(&[0x09, 0x00], 100).is_err(), "fragmented ping");
        let mut big_ping = vec![0x89, 126, 0, 200];
        big_ping.extend(vec![0u8; 200]);
        assert!(decode_frame(&big_ping, 1000).is_err(), "control frame over 125 bytes");
        assert!(decode_frame(&encode_frame(true, Opcode::Text, &[0u8; 50], None), 10).is_err(), "over the limit");
        assert!(decode_frame(&[0x81, 127, 255, 255, 255, 255, 255, 255, 255, 255], 1 << 20).is_err(), "absurd length");
    }

    #[tokio::test]
    async fn messages_pings_fragments_and_close_over_a_real_socket() {
        let (l, url) = bind().await;
        let server = tokio::spawn(async move {
            let mut s = accept(&l).await;
            s.send("hello").await;
            // Ping: the client must pong with the same payload (masked).
            s.send_frame(true, Opcode::Ping, b"are-you-there").await;
            // A text message in three fragments with a ping in between.
            s.send_frame(false, Opcode::Text, b"frag").await;
            s.send_frame(true, Opcode::Ping, b"mid").await;
            s.send_frame(false, Opcode::Continuation, b"men").await;
            s.send_frame(true, Opcode::Continuation, b"ted").await;
            let first = s.next().await;
            let second = s.next().await;
            let ack = s.next().await;
            assert_eq!((ack.opcode, ack.payload.as_slice()), (Opcode::Text, &b"ack"[..]), "client frames arrive unmasked by the codec");
            s.send_frame(true, Opcode::Close, &[]).await;
            let echoed_close = s.next().await;
            (first, second, echoed_close)
        });
        let mut c = WsClient::connect(&url, tls(), Duration::from_secs(5)).await.unwrap();
        assert_eq!(c.recv().await.unwrap(), Message::Text("hello".into()));
        assert_eq!(c.recv().await.unwrap(), Message::Text("fragmented".into()));
        c.send_text("ack").await.unwrap();
        assert_eq!(c.recv().await.unwrap(), Message::Closed);
        let (first, second, close) = server.await.unwrap();
        assert_eq!((first.opcode, first.payload.as_slice()), (Opcode::Pong, &b"are-you-there"[..]));
        assert_eq!((second.opcode, second.payload.as_slice()), (Opcode::Pong, &b"mid"[..]));
        assert_eq!(close.opcode, Opcode::Close);
    }

    #[tokio::test]
    async fn the_handshake_is_checked() {
        // A server that answers 200, and one that answers 101 with a wrong key.
        for reply in ["HTTP/1.1 200 OK\r\n\r\n", "HTTP/1.1 101 Switching Protocols\r\nSec-WebSocket-Accept: AAAA\r\n\r\n"] {
            let (l, url) = bind().await;
            tokio::spawn(async move {
                let (mut io, _) = l.accept().await.unwrap();
                let mut b = [0u8; 2048];
                let _ = io.read(&mut b).await;
                io.write_all(reply.as_bytes()).await.unwrap();
            });
            let e = WsClient::connect(&url, tls(), Duration::from_secs(5)).await.err().unwrap();
            assert!(e.contains("refused") || e.contains("wrong"), "{e}");
        }
        assert!(WsClient::connect("http://x/y", tls(), Duration::from_secs(1)).await.err().unwrap().contains("Unsupported"));
        assert!(WsClient::connect("not a url", tls(), Duration::from_secs(1)).await.is_err());
    }
}
