//! Peer-to-peer transport: device identity, mutual TLS and framing.
//!
//! - Each device has a persistent id and a self-signed certificate; its
//!   SHA-256 fingerprint is the device's identity. Both ends present their
//!   certificate (mutual TLS), so each side knows who it talks to, and a
//!   saved device whose certificate changes is refused.
//! - Frames are `[kind u8][len u32 BE][payload]`: kind 0 a JSON
//!   [`PeerMessage`](super::peer::PeerMessage), kind 1 file bytes.

use crate::core::error::AppError;
use crate::security::tls::cert_fingerprint;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use uuid::Uuid;

/// Name in every peer certificate (verification is by fingerprint, not name).
pub const PEER_CERT_NAME: &str = "ufop-peer";
/// Largest JSON frame accepted.
pub const MAX_JSON_FRAME: usize = 8 * 1024 * 1024;
/// Largest data frame sent or accepted.
pub const MAX_DATA_FRAME: usize = 4 * 1024 * 1024;

const KIND_JSON: u8 = 0;
const KIND_DATA: u8 = 1;

/// This device's identity.
#[derive(Clone, Serialize, Deserialize)]
pub struct PeerIdentity {
    pub id: Uuid,
    pub cert_pem: String,
    pub key_pem: String,
}

impl std::fmt::Debug for PeerIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PeerIdentity").field("id", &self.id).field("fingerprint", &self.fingerprint()).finish_non_exhaustive()
    }
}

impl PeerIdentity {
    pub fn generate() -> Result<Self, AppError> {
        let ck = rcgen::generate_simple_self_signed(vec![PEER_CERT_NAME.to_string()]).map_err(|e| AppError::internal(format!("Peer certificate: {e}")))?;
        Ok(Self { id: Uuid::new_v4(), cert_pem: ck.cert.pem(), key_pem: ck.key_pair.serialize_pem() })
    }

    /// Load `path`, or create it (owner-only) on first use.
    pub fn load_or_create(path: &Path) -> Result<Self, AppError> {
        if let Ok(raw) = std::fs::read(path) {
            if let Ok(id) = serde_json::from_slice::<Self>(&raw) {
                if id.cert_der().is_ok() && id.key_der().is_ok() {
                    return Ok(id);
                }
            }
            tracing::warn!("Peer identity at {} is unreadable; creating a new one", path.display());
        }
        let id = Self::generate()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_private(path, &serde_json::to_vec_pretty(&id).unwrap_or_default())?;
        Ok(id)
    }

    pub fn cert_der(&self) -> Result<CertificateDer<'static>, AppError> {
        rustls_pemfile::certs(&mut self.cert_pem.as_bytes()).next().and_then(Result::ok).ok_or_else(|| AppError::internal("Peer certificate is not valid PEM"))
    }

    fn key_der(&self) -> Result<PrivateKeyDer<'static>, AppError> {
        rustls_pemfile::private_key(&mut self.key_pem.as_bytes()).ok().flatten().ok_or_else(|| AppError::internal("Peer key is not valid PEM"))
    }

    pub fn fingerprint(&self) -> String {
        self.cert_der().map(|c| cert_fingerprint(c.as_ref())).unwrap_or_default()
    }

    /// TLS server config: our certificate, and any client certificate
    /// (the connection handler checks its fingerprint).
    pub fn server_config(&self) -> Result<Arc<rustls::ServerConfig>, AppError> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cfg = rustls::ServerConfig::builder()
            .with_client_cert_verifier(Arc::new(AnyClientCert(Arc::new(rustls::crypto::ring::default_provider()))))
            .with_single_cert(vec![self.cert_der()?], self.key_der()?)
            .map_err(|e| AppError::internal(format!("Peer TLS: {e}")))?;
        Ok(Arc::new(cfg))
    }

    /// TLS client config: our certificate, and the server's pinned to
    /// `expected` (any when `None`: trust on first use).
    pub fn client_config(&self, expected: Option<&str>) -> Result<Arc<rustls::ClientConfig>, AppError> {
        let pins: Vec<String> = expected.into_iter().map(String::from).collect();
        let cfg = rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(crate::security::tls::server_cert_verifier(&pins)?)
            .with_client_auth_cert(vec![self.cert_der()?], self.key_der()?)
            .map_err(|e| AppError::internal(format!("Peer TLS: {e}")))?;
        Ok(Arc::new(cfg))
    }
}

/// Write `data` readable by the owner only.
pub fn write_private(path: &Path, data: &[u8]) -> Result<(), AppError> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)?;
    Ok(())
}

/// Accepts any client certificate whose handshake signature verifies.
#[derive(Debug)]
struct AnyClientCert(Arc<rustls::crypto::CryptoProvider>);

impl rustls::server::danger::ClientCertVerifier for AnyClientCert {
    fn root_hint_subjects(&self) -> &[rustls::DistinguishedName] {
        &[]
    }
    fn verify_client_cert(&self, _: &CertificateDer<'_>, _: &[CertificateDer<'_>], _: UnixTime) -> Result<rustls::server::danger::ClientCertVerified, rustls::Error> {
        Ok(rustls::server::danger::ClientCertVerified::assertion())
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.0.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Fingerprint of the certificate the other end presented.
pub fn peer_fingerprint(certs: Option<&[CertificateDer<'_>]>) -> Option<String> {
    certs.and_then(|c| c.first()).map(|c| cert_fingerprint(c.as_ref()))
}

pub fn server_name() -> ServerName<'static> {
    ServerName::try_from(PEER_CERT_NAME).expect("static DNS name")
}

/// One frame read off the wire.
#[derive(Debug)]
pub enum Frame<M> {
    Msg(M),
    Data(Vec<u8>),
}

pub async fn write_msg<W: AsyncWrite + Unpin, M: Serialize>(w: &mut W, msg: &M) -> Result<(), AppError> {
    let body = serde_json::to_vec(msg).map_err(|e| AppError::internal(format!("Peer message: {e}")))?;
    write_frame(w, KIND_JSON, &body).await
}

pub async fn write_data<W: AsyncWrite + Unpin>(w: &mut W, data: &[u8]) -> Result<(), AppError> {
    write_frame(w, KIND_DATA, data).await
}

async fn write_frame<W: AsyncWrite + Unpin>(w: &mut W, kind: u8, body: &[u8]) -> Result<(), AppError> {
    let mut head = [0u8; 5];
    head[0] = kind;
    head[1..].copy_from_slice(&(body.len() as u32).to_be_bytes());
    w.write_all(&head).await.map_err(io_err)?;
    w.write_all(body).await.map_err(io_err)?;
    w.flush().await.map_err(io_err)
}

/// Read one frame; `None` when the other end closed the connection.
pub async fn read_frame<R: AsyncRead + Unpin, M: for<'de> Deserialize<'de>>(r: &mut R) -> Result<Option<Frame<M>>, AppError> {
    let mut head = [0u8; 5];
    match r.read_exact(&mut head).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(io_err(e)),
    }
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    let limit = if head[0] == KIND_JSON { MAX_JSON_FRAME } else { MAX_DATA_FRAME };
    if len > limit || head[0] > KIND_DATA {
        return Err(protocol("oversized or unknown frame"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await.map_err(io_err)?;
    if head[0] == KIND_DATA {
        return Ok(Some(Frame::Data(body)));
    }
    serde_json::from_slice(&body).map(|m| Some(Frame::Msg(m))).map_err(|e| protocol(&format!("bad message: {e}")))
}

/// Read the next frame, which must be a message.
pub async fn read_msg<R: AsyncRead + Unpin, M: for<'de> Deserialize<'de>>(r: &mut R) -> Result<M, AppError> {
    match read_frame(r).await? {
        Some(Frame::Msg(m)) => Ok(m),
        Some(Frame::Data(_)) => Err(protocol("unexpected file data")),
        None => Err(AppError::connection("The other computer closed the connection", "Check it is still running UFOP and try again.")),
    }
}

pub fn io_err(e: std::io::Error) -> AppError {
    AppError::connection(format!("Peer connection: {e}"), "Check both computers are on the same network and try again.")
}

pub fn protocol(what: &str) -> AppError {
    AppError::connection(format!("Peer protocol error: {what}"), "Make sure both computers run a current UFOP version.")
}

/// A relative path from a peer, made safe to join under a folder: no
/// absolute paths, drive prefixes or `..`; `\` is treated as a separator.
pub fn safe_relative(p: &str) -> Option<std::path::PathBuf> {
    let mut out = std::path::PathBuf::new();
    for part in p.split(['/', '\\']) {
        match part {
            "" | "." => continue,
            ".." => return None,
            s if s.contains(':') || s.contains('\0') => return None,
            s => out.push(s),
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_cannot_escape() {
        assert_eq!(safe_relative("a/b\\c.txt"), Some(["a", "b", "c.txt"].iter().collect()));
        assert_eq!(safe_relative("./a//b"), Some(["a", "b"].iter().collect()));
        for bad in ["../x", "a/../../x", "C:\\Windows\\x", "", "/", "a\0b"] {
            assert_eq!(safe_relative(bad), None, "{bad:?}");
        }
        // A leading slash is dropped, not honoured.
        assert_eq!(safe_relative("/etc/passwd"), Some(["etc", "passwd"].iter().collect()));
    }

    #[test]
    fn identity_persists_owner_only() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("peer/identity.json");
        let a = PeerIdentity::load_or_create(&p).unwrap();
        let b = PeerIdentity::load_or_create(&p).unwrap();
        assert_eq!((a.id, a.fingerprint()), (b.id, b.fingerprint()));
        assert_eq!(a.fingerprint().len(), 64);
        assert!(!format!("{a:?}").contains("PRIVATE"), "the key never appears in logs");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::write(&p, "garbage").unwrap();
        assert_ne!(PeerIdentity::load_or_create(&p).unwrap().id, a.id);
    }

    #[tokio::test]
    async fn frames_round_trip_and_limits_hold() {
        let (mut a, mut b) = tokio::io::duplex(64 * 1024);
        write_msg(&mut a, &serde_json::json!({ "type": "ping" })).await.unwrap();
        write_data(&mut a, b"chunk").await.unwrap();
        drop(a);
        assert!(matches!(read_frame::<_, serde_json::Value>(&mut b).await.unwrap(), Some(Frame::Msg(v)) if v["type"] == "ping"));
        assert!(matches!(read_frame::<_, serde_json::Value>(&mut b).await.unwrap(), Some(Frame::Data(d)) if d == b"chunk"));
        assert!(read_frame::<_, serde_json::Value>(&mut b).await.unwrap().is_none());

        let (mut a, mut b) = tokio::io::duplex(64);
        a.write_all(&[1, 0xff, 0xff, 0xff, 0xff]).await.unwrap();
        assert!(read_frame::<_, serde_json::Value>(&mut b).await.is_err(), "oversized frames are refused before allocating");
    }
}
