//! TLS enforcement helpers for in-transit encryption (T-047).
//!
//! Provides utilities for enforcing encrypted transport across all protocols:
//! - TLS 1.3 enforcement for HTTPS connections
//! - FTPS auto-upgrade from FTP
//! - SMB3 encryption preference
//! - HTTPS enforcement for all cloud API calls
//!
//! These helpers are used by connectors to verify and configure transport
//! security before establishing connections.

use serde::{Deserialize, Serialize};

use crate::core::error::AppError;
use crate::core::types::ConnectionProtocol;

/// Minimum acceptable TLS version.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum TlsVersion {
    /// TLS 1.2 (minimum for most protocols).
    Tls12 = 12,
    /// TLS 1.3 (preferred, required for new connections).
    Tls13 = 13,
}

impl TlsVersion {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Tls12 => "tls_1.2",
            Self::Tls13 => "tls_1.3",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "tls_1.2" | "1.2" => Self::Tls12,
            "tls_1.3" | "1.3" => Self::Tls13,
            _ => Self::Tls12,
        }
    }
}

/// Transport security requirements for a connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportSecurity {
    /// Minimum TLS version required.
    pub min_tls_version: TlsVersion,
    /// Whether to enforce encrypted transport (reject plaintext).
    pub require_encryption: bool,
    /// Whether to auto-upgrade from unencrypted to encrypted (e.g., FTP -> FTPS).
    pub auto_upgrade: bool,
    /// Whether to verify TLS certificates.
    pub verify_certificates: bool,
    /// Optional custom CA certificate path.
    pub ca_cert_path: Option<String>,
    /// Whether to use certificate pinning.
    pub certificate_pinning: bool,
    /// Pinned certificate fingerprints (SHA-256 hex).
    pub pinned_fingerprints: Vec<String>,
}

impl Default for TransportSecurity {
    fn default() -> Self {
        Self {
            min_tls_version: TlsVersion::Tls12,
            require_encryption: true,
            auto_upgrade: true,
            verify_certificates: true,
            ca_cert_path: None,
            certificate_pinning: false,
            pinned_fingerprints: Vec::new(),
        }
    }
}

impl TransportSecurity {
    /// Strict security profile (TLS 1.3 only, certificate verification required).
    pub fn strict() -> Self {
        Self {
            min_tls_version: TlsVersion::Tls13,
            require_encryption: true,
            auto_upgrade: true,
            verify_certificates: true,
            ca_cert_path: None,
            certificate_pinning: false,
            pinned_fingerprints: Vec::new(),
        }
    }
}

/// Result of a transport security check.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransportSecurityCheck {
    /// Whether the connection meets security requirements.
    pub secure: bool,
    /// The effective protocol that will be used.
    pub effective_protocol: String,
    /// Whether an upgrade was applied.
    pub upgraded: bool,
    /// Security warnings (non-fatal issues).
    pub warnings: Vec<String>,
    /// Security errors (fatal issues blocking the connection).
    pub errors: Vec<String>,
    /// Recommended actions for the user.
    pub recommendations: Vec<String>,
}

/// Check transport security for a given protocol.
///
/// Returns a security assessment including whether the connection is secure,
/// whether it was auto-upgraded, and any warnings or errors.
pub fn check_transport_security(
    protocol: ConnectionProtocol,
    host: &str,
    port: Option<u16>,
    security: &TransportSecurity,
) -> TransportSecurityCheck {
    let mut check = TransportSecurityCheck {
        secure: true,
        effective_protocol: protocol.as_str().to_string(),
        upgraded: false,
        warnings: Vec::new(),
        errors: Vec::new(),
        recommendations: Vec::new(),
    };

    match protocol {
        ConnectionProtocol::Sftp => {
            // SFTP is always encrypted (SSH tunnel)
            check.secure = true;
        }

        ConnectionProtocol::Ftp => {
            if security.require_encryption {
                if security.auto_upgrade {
                    // Auto-upgrade to FTPS
                    check.effective_protocol = "ftps".to_string();
                    check.upgraded = true;
                    check.warnings.push(
                        "FTP connection auto-upgraded to FTPS for encryption.".to_string(),
                    );
                } else {
                    check.secure = false;
                    check.errors.push(
                        "FTP transmits data in plaintext. Encryption is required.".to_string(),
                    );
                    check.recommendations.push(
                        "Use SFTP or FTPS instead of FTP.".to_string(),
                    );
                }
            } else {
                check.warnings.push(
                    "FTP transmits credentials and data in plaintext.".to_string(),
                );
                check.recommendations.push(
                    "Consider using SFTP or FTPS for encrypted transfers.".to_string(),
                );
            }
        }

        ConnectionProtocol::Ftps => {
            // FTPS uses TLS
            check.secure = true;
            if security.min_tls_version == TlsVersion::Tls13 {
                check.warnings.push(
                    "TLS 1.3 is preferred but FTPS server compatibility varies.".to_string(),
                );
            }
        }

        ConnectionProtocol::WebDav => {
            // WebDAV security depends on HTTPS
            let effective_port = port.unwrap_or(443);
            if (effective_port == 80 || host.starts_with("http://"))
                && security.require_encryption {
                    if security.auto_upgrade {
                        check.effective_protocol = "webdav_https".to_string();
                        check.upgraded = true;
                        check.warnings.push(
                            "WebDAV connection upgraded to HTTPS.".to_string(),
                        );
                    } else {
                        check.secure = false;
                        check.errors.push(
                            "WebDAV over HTTP transmits data in plaintext.".to_string(),
                        );
                        check.recommendations.push(
                            "Use HTTPS for WebDAV connections.".to_string(),
                        );
                    }
                }
        }

        ConnectionProtocol::Smb => {
            // Prefer SMB3 with encryption
            check.recommendations.push(
                "SMB3 encryption is preferred. Ensure the server supports SMB3.".to_string(),
            );
            if security.min_tls_version == TlsVersion::Tls13 {
                check.warnings.push(
                    "SMB uses its own encryption, not TLS. SMB3 encryption is the equivalent.".to_string(),
                );
            }
        }

        ConnectionProtocol::Nfs => {
            if security.require_encryption {
                check.warnings.push(
                    "NFS does not natively support encryption. Consider using NFS over VPN or stunnel.".to_string(),
                );
                check.recommendations.push(
                    "Use Kerberos authentication (sec=krb5p) for NFS encryption.".to_string(),
                );
            }
        }

        ConnectionProtocol::S3
        | ConnectionProtocol::GoogleDrive
        | ConnectionProtocol::Dropbox
        | ConnectionProtocol::OneDrive
        | ConnectionProtocol::AzureBlob
        | ConnectionProtocol::Swift => {
            // Cloud APIs always use HTTPS
            check.secure = true;
            if !security.verify_certificates {
                check.warnings.push(
                    "Certificate verification is disabled. This may allow man-in-the-middle attacks.".to_string(),
                );
            }
        }

        ConnectionProtocol::Local => {
            // Local filesystem doesn't use network transport
            check.secure = true;
        }
    }

    if security.certificate_pinning {
        let bad: Vec<&String> = security.pinned_fingerprints.iter().filter(|f| normalize_fingerprint(f).is_none()).collect();
        if security.pinned_fingerprints.is_empty() {
            check.secure = false;
            check.errors.push("Certificate pinning is on but no fingerprint is pinned".into());
            check.recommendations.push("Fetch the server's certificate fingerprint and pin it.".into());
        } else if !bad.is_empty() {
            check.secure = false;
            check.errors.push(format!("Invalid pinned fingerprint(s): {}", bad.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")));
        }
    }

    check
}

/// Get the recommended transport security settings for a protocol.
pub fn recommended_security(protocol: ConnectionProtocol) -> TransportSecurity {
    match protocol {
        ConnectionProtocol::Sftp => TransportSecurity::default(),
        ConnectionProtocol::Ftp => TransportSecurity {
            auto_upgrade: true,
            require_encryption: true,
            ..TransportSecurity::default()
        },
        ConnectionProtocol::Ftps => TransportSecurity::default(),
        ConnectionProtocol::WebDav => TransportSecurity {
            min_tls_version: TlsVersion::Tls12,
            ..TransportSecurity::default()
        },
        ConnectionProtocol::Smb => TransportSecurity::default(),
        ConnectionProtocol::Nfs => TransportSecurity {
            require_encryption: false, // NFS encryption is complex
            ..TransportSecurity::default()
        },
        ConnectionProtocol::S3
        | ConnectionProtocol::GoogleDrive
        | ConnectionProtocol::Dropbox
        | ConnectionProtocol::OneDrive
        | ConnectionProtocol::AzureBlob
        | ConnectionProtocol::Swift => TransportSecurity::strict(),
        ConnectionProtocol::Local => TransportSecurity {
            require_encryption: false,
            ..TransportSecurity::default()
        },
    }
}

/// Validate that a URL uses HTTPS (for cloud API calls).
pub fn enforce_https(url: &str) -> Result<String, AppError> {
    if url.starts_with("https://") {
        Ok(url.to_string())
    } else if let Some(rest) = url.strip_prefix("http://") {
        // Auto-upgrade to HTTPS
        let upgraded = format!("https://{rest}");
        tracing::warn!("Auto-upgrading HTTP to HTTPS: {url} -> {upgraded}");
        Ok(upgraded)
    } else if url.contains("://") {
        Err(AppError::encryption(
            "Unsupported URL scheme",
            "Only HTTPS URLs are supported for cloud API calls.",
        ))
    } else {
        // No scheme, assume HTTPS
        Ok(format!("https://{url}"))
    }
}

/// Check if a port number indicates an encrypted connection.
pub fn is_encrypted_port(protocol: ConnectionProtocol, port: u16) -> bool {
    match protocol {
        ConnectionProtocol::Ftp => port == 990, // Implicit FTPS
        ConnectionProtocol::WebDav => port == 443,
        ConnectionProtocol::Sftp => true, // All SFTP is encrypted
        ConnectionProtocol::Ftps => true,
        _ => true,
    }
}

// ── rustls client configuration: CA trust and certificate pinning ──

/// Normalize a SHA-256 certificate fingerprint (`AB:CD:…`, spaces, any case)
/// to 64 lowercase hex characters; `None` if it is not one.
pub fn normalize_fingerprint(s: &str) -> Option<String> {
    let h: String = s.chars().filter(|c| !matches!(c, ':' | ' ' | '-')).collect::<String>().to_ascii_lowercase();
    (h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit())).then_some(h)
}

/// SHA-256 of a DER certificate, lowercase hex.
pub fn cert_fingerprint(der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(der))
}

fn provider() -> std::sync::Arc<rustls::crypto::CryptoProvider> {
    std::sync::Arc::new(rustls::crypto::ring::default_provider())
}

/// Accepts only certificates whose SHA-256 fingerprint is pinned; the
/// handshake signature is still verified against the presented key.
#[derive(Debug)]
struct PinVerifier {
    pins: Vec<String>,
    provider: std::sync::Arc<rustls::crypto::CryptoProvider>,
    /// When set, the presented leaf's fingerprint is recorded (probe mode).
    seen: Option<std::sync::Arc<std::sync::Mutex<Option<String>>>>,
}

impl rustls::client::danger::ServerCertVerifier for PinVerifier {
    fn verify_server_cert(
        &self,
        ee: &rustls::pki_types::CertificateDer<'_>,
        _: &[rustls::pki_types::CertificateDer<'_>],
        _: &rustls::pki_types::ServerName<'_>,
        _: &[u8],
        _: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        let fp = cert_fingerprint(ee.as_ref());
        if let Some(seen) = &self.seen {
            if let Ok(mut g) = seen.lock() {
                *g = Some(fp.clone());
            }
            return Ok(rustls::client::danger::ServerCertVerified::assertion());
        }
        if self.pins.contains(&fp) {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(format!("certificate fingerprint {fp} does not match any pinned fingerprint")))
        }
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(m, c, d, &self.provider.signature_verification_algorithms)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

/// Accepts the exact certificate(s) from a PEM bundle; otherwise normal
/// validation (covers self-signed leaves given as "trusted CA").
#[derive(Debug)]
struct ExactOrWebPki {
    exact: Vec<Vec<u8>>,
    inner: std::sync::Arc<rustls::client::WebPkiServerVerifier>,
}

impl rustls::client::danger::ServerCertVerifier for ExactOrWebPki {
    fn verify_server_cert(
        &self,
        ee: &rustls::pki_types::CertificateDer<'_>,
        im: &[rustls::pki_types::CertificateDer<'_>],
        name: &rustls::pki_types::ServerName<'_>,
        ocsp: &[u8],
        now: rustls::pki_types::UnixTime,
    ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
        if self.exact.iter().any(|p| p.as_slice() == ee.as_ref()) {
            return Ok(rustls::client::danger::ServerCertVerified::assertion());
        }
        self.inner.verify_server_cert(ee, im, name, ocsp, now)
    }
    fn verify_tls12_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls12_signature(m, c, d)
    }
    fn verify_tls13_signature(&self, m: &[u8], c: &rustls::pki_types::CertificateDer<'_>, d: &rustls::DigitallySignedStruct) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
        self.inner.verify_tls13_signature(m, c, d)
    }
    fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

/// Build a rustls client configuration.
///
/// - `pins` non-empty: only certificates with one of these SHA-256
///   fingerprints are accepted (strict pinning, chain not consulted);
/// - `accept_invalid`: any certificate (test setups only);
/// - otherwise web PKI roots plus the certificates in `ca_pem`.
pub fn client_config(accept_invalid: bool, ca_pem: Option<&str>, pins: &[String]) -> Result<rustls::ClientConfig, AppError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    if !pins.is_empty() {
        let pins: Vec<String> = pins
            .iter()
            .map(|p| normalize_fingerprint(p).ok_or_else(|| AppError::validation(format!("'{p}' is not a SHA-256 certificate fingerprint (64 hex digits)"))))
            .collect::<Result<_, _>>()?;
        return Ok(rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(PinVerifier { pins, provider: provider(), seen: None }))
            .with_no_client_auth());
    }
    if accept_invalid {
        return Ok(rustls::ClientConfig::builder()
            .dangerous()
            .with_custom_certificate_verifier(std::sync::Arc::new(PinVerifier { pins: vec![], provider: provider(), seen: Some(Default::default()) }))
            .with_no_client_auth());
    }
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let mut exact: Vec<Vec<u8>> = Vec::new();
    if let Some(pem) = ca_pem.filter(|p| !p.trim().is_empty()) {
        for c in rustls_pemfile::certs(&mut pem.as_bytes()).flatten() {
            exact.push(c.as_ref().to_vec());
            let _ = roots.add(c);
        }
        if exact.is_empty() {
            return Err(AppError::validation("The trusted certificate is not valid PEM"));
        }
    }
    let webpki = rustls::client::WebPkiServerVerifier::builder(std::sync::Arc::new(roots)).build().map_err(|e| AppError::internal(format!("TLS setup: {e}")))?;
    if exact.is_empty() {
        return Ok(rustls::ClientConfig::builder().with_webpki_verifier(webpki).with_no_client_auth());
    }
    Ok(rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(ExactOrWebPki { exact, inner: webpki }))
        .with_no_client_auth())
}

/// Connect to `host:port`, complete a TLS handshake without validating the
/// certificate, and return the server certificate's SHA-256 fingerprint —
/// for the user to compare and pin.
pub async fn fetch_fingerprint(host: &str, port: u16, timeout: std::time::Duration) -> Result<String, AppError> {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let seen: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    let cfg = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(std::sync::Arc::new(PinVerifier { pins: vec![], provider: provider(), seen: Some(seen.clone()) }))
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| AppError::validation(format!("Invalid server name '{host}'")))?;
    let conn = async {
        let tcp = tokio::net::TcpStream::connect((host, port)).await.map_err(|e| AppError::connection(format!("Connecting to {host}:{port}: {e}"), "Check the host and port."))?;
        tokio_rustls::TlsConnector::from(std::sync::Arc::new(cfg)).connect(name, tcp).await.map_err(|e| AppError::connection(format!("TLS with {host}:{port} failed: {e}"), "Check that the port speaks TLS."))
    };
    tokio::time::timeout(timeout, conn).await.map_err(|_| AppError::connection(format!("{host}:{port} timed out"), "Check the host and port."))??;
    let fp = seen.lock().ok().and_then(|g| g.clone());
    fp.ok_or_else(|| AppError::connection(format!("{host}:{port} sent no certificate"), "Check the host and port."))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tls_version_ordering() {
        assert!(TlsVersion::Tls13 > TlsVersion::Tls12);
    }

    #[test]
    fn test_tls_version_roundtrip() {
        assert_eq!(
            TlsVersion::Tls12,
            TlsVersion::from_str_lossy(TlsVersion::Tls12.as_str())
        );
        assert_eq!(
            TlsVersion::Tls13,
            TlsVersion::from_str_lossy(TlsVersion::Tls13.as_str())
        );
    }

    #[test]
    fn test_sftp_always_secure() {
        let security = TransportSecurity::default();
        let check = check_transport_security(
            ConnectionProtocol::Sftp,
            "example.com",
            Some(22),
            &security,
        );
        assert!(check.secure);
        assert!(check.errors.is_empty());
    }

    #[test]
    fn test_ftp_auto_upgrade() {
        let security = TransportSecurity {
            require_encryption: true,
            auto_upgrade: true,
            ..TransportSecurity::default()
        };
        let check = check_transport_security(
            ConnectionProtocol::Ftp,
            "example.com",
            Some(21),
            &security,
        );
        assert!(check.secure);
        assert!(check.upgraded);
        assert_eq!(check.effective_protocol, "ftps");
    }

    #[test]
    fn test_ftp_no_auto_upgrade() {
        let security = TransportSecurity {
            require_encryption: true,
            auto_upgrade: false,
            ..TransportSecurity::default()
        };
        let check = check_transport_security(
            ConnectionProtocol::Ftp,
            "example.com",
            Some(21),
            &security,
        );
        assert!(!check.secure);
        assert!(!check.errors.is_empty());
    }

    #[test]
    fn test_ftp_no_encryption_required() {
        let security = TransportSecurity {
            require_encryption: false,
            ..TransportSecurity::default()
        };
        let check = check_transport_security(
            ConnectionProtocol::Ftp,
            "example.com",
            Some(21),
            &security,
        );
        // Not blocked, but should have warnings
        assert!(!check.warnings.is_empty());
    }

    #[test]
    fn test_cloud_always_secure() {
        let security = TransportSecurity::default();

        for protocol in [
            ConnectionProtocol::S3,
            ConnectionProtocol::GoogleDrive,
            ConnectionProtocol::Dropbox,
            ConnectionProtocol::OneDrive,
        ] {
            let check = check_transport_security(protocol, "api.cloud.com", None, &security);
            assert!(check.secure, "Protocol {:?} should be secure", protocol);
        }
    }

    #[test]
    fn test_webdav_http_upgrade() {
        let security = TransportSecurity {
            require_encryption: true,
            auto_upgrade: true,
            ..TransportSecurity::default()
        };
        let check = check_transport_security(
            ConnectionProtocol::WebDav,
            "http://example.com",
            Some(80),
            &security,
        );
        assert!(check.upgraded);
    }

    #[test]
    fn test_enforce_https() {
        assert_eq!(
            enforce_https("https://api.example.com").unwrap(),
            "https://api.example.com"
        );
        assert_eq!(
            enforce_https("http://api.example.com").unwrap(),
            "https://api.example.com"
        );
        assert_eq!(
            enforce_https("api.example.com").unwrap(),
            "https://api.example.com"
        );
        assert!(enforce_https("ftp://example.com").is_err());
    }

    #[test]
    fn test_is_encrypted_port() {
        assert!(is_encrypted_port(ConnectionProtocol::Sftp, 22));
        assert!(is_encrypted_port(ConnectionProtocol::Ftp, 990));
        assert!(!is_encrypted_port(ConnectionProtocol::Ftp, 21));
        assert!(is_encrypted_port(ConnectionProtocol::WebDav, 443));
        assert!(!is_encrypted_port(ConnectionProtocol::WebDav, 80));
    }

    #[test]
    fn test_recommended_security_cloud_strict() {
        let sec = recommended_security(ConnectionProtocol::S3);
        assert_eq!(sec.min_tls_version, TlsVersion::Tls13);
        assert!(sec.require_encryption);
    }

    #[test]
    fn test_recommended_security_local_no_encryption() {
        let sec = recommended_security(ConnectionProtocol::Local);
        assert!(!sec.require_encryption);
    }

    #[test]
    fn test_transport_security_serialization() {
        let ts = TransportSecurity::default();
        let json = serde_json::to_string(&ts).unwrap();
        let restored: TransportSecurity = serde_json::from_str(&json).unwrap();
        assert_eq!(ts.min_tls_version, restored.min_tls_version);
        assert_eq!(ts.require_encryption, restored.require_encryption);
    }
}

#[cfg(test)]
mod pinning_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn pins_are_enforced_for_http_clients() {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()]).unwrap();
        let der = cert.cert.der().to_vec();
        let fp = cert_fingerprint(&der);
        let cfg = axum_server::tls_rustls::RustlsConfig::from_pem(cert.cert.pem().into_bytes(), cert.key_pair.serialize_pem().into_bytes()).await.unwrap();
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let app = axum::Router::new().route("/", axum::routing::get(|| async { "pinned-ok" }));
        tokio::spawn(axum_server::from_tcp_rustls(l, cfg).serve(app.into_make_service()));

        assert_eq!(fetch_fingerprint("localhost", port, std::time::Duration::from_secs(5)).await.unwrap(), fp);
        let url = format!("https://localhost:{port}/");
        let get = |pins: Vec<String>| {
            let url = url.clone();
            async move {
                let c = crate::backup_engine::stores::http::client(&crate::backup_engine::stores::http::TlsOptions { pinned_sha256: pins, ..Default::default() })?;
                c.get(&url).send().await.map_err(|e| AppError::internal(e.to_string()))?.text().await.map_err(|e| AppError::internal(e.to_string()))
            }
        };
        // Pinned (colon form accepted): the self-signed server is trusted.
        let colon = fp.as_bytes().chunks(2).map(|c| std::str::from_utf8(c).unwrap().to_ascii_uppercase()).collect::<Vec<_>>().join(":");
        assert_eq!(get(vec![colon]).await.unwrap(), "pinned-ok");
        // Another pin, or no pin (web PKI), is refused.
        assert!(get(vec!["00".repeat(32)]).await.is_err());
        assert!(get(vec![]).await.is_err());
        assert!(client_config(false, None, &["nope".into()]).is_err());

        let sec = TransportSecurity { certificate_pinning: true, ..TransportSecurity::default() };
        let check = check_transport_security(ConnectionProtocol::WebDav, "localhost", Some(port), &sec);
        assert!(!check.secure && check.errors.iter().any(|e| e.contains("no fingerprint")));
        let sec = TransportSecurity { certificate_pinning: true, pinned_fingerprints: vec![fp], ..TransportSecurity::default() };
        assert!(check_transport_security(ConnectionProtocol::WebDav, "https://localhost", Some(443), &sec).errors.is_empty());
    }
}
