//! HTTPS for the server: either an imported PFX/P12 certificate, or a
//! locally generated CA plus server certificate. The generated server
//! certificate is renewed (re-signed by the same CA, so clients that
//! trust the CA keep working) when it is within 30 days of expiry.

use crate::core::error::AppError;
use rcgen::{BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, KeyPair, KeyUsagePurpose, SanType};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const RENEW_BEFORE_DAYS: i64 = 30;
const SERVER_VALID_DAYS: i64 = 397;
const CA_VALID_DAYS: i64 = 3650;

/// PEM certificate chain + private key.
#[derive(Debug, Clone)]
pub struct PemPair {
    pub cert_chain_pem: String,
    pub key_pem: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Meta {
    server_expires: String,
    hostnames: Vec<String>,
}

fn tls_err(e: impl std::fmt::Display) -> AppError {
    AppError::Configuration { message: format!("TLS certificate error: {e}"), advice: "Check the certificate files or regenerate them.".into() }
}

fn offset(days: i64) -> time::OffsetDateTime {
    time::OffsetDateTime::now_utc() + time::Duration::days(days)
}

/// Generated certificate files in `dir`.
pub struct GeneratedCerts {
    dir: PathBuf,
}

impl GeneratedCerts {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    fn p(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// The CA certificate users install to trust the server.
    pub fn ca_cert_path(&self) -> PathBuf {
        self.p("ufop-ca.pem")
    }

    fn ensure_ca(&self) -> Result<(String, String), AppError> {
        if let (Ok(c), Ok(k)) = (std::fs::read_to_string(self.ca_cert_path()), std::fs::read_to_string(self.p("ufop-ca.key"))) {
            return Ok((c, k));
        }
        std::fs::create_dir_all(&self.dir)?;
        let key = KeyPair::generate().map_err(tls_err)?;
        let mut params = CertificateParams::new(Vec::<String>::new()).map_err(tls_err)?;
        params.distinguished_name.push(DnType::CommonName, "UFOP local CA");
        params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
        params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        params.not_before = offset(-1);
        params.not_after = offset(CA_VALID_DAYS);
        let cert = params.self_signed(&key).map_err(tls_err)?;
        let (c, k) = (cert.pem(), key.serialize_pem());
        std::fs::write(self.ca_cert_path(), &c)?;
        write_private(&self.p("ufop-ca.key"), &k)?;
        Ok((c, k))
    }

    fn needs_renewal(&self, hostnames: &[String]) -> bool {
        let Ok(raw) = std::fs::read_to_string(self.p("server.json")) else { return true };
        let Ok(m) = serde_json::from_str::<Meta>(&raw) else { return true };
        if m.hostnames != hostnames || !self.p("server.pem").exists() || !self.p("server.key").exists() {
            return true;
        }
        chrono::DateTime::parse_from_rfc3339(&m.server_expires)
            .map(|e| e.with_timezone(&chrono::Utc) - chrono::Utc::now() < chrono::Duration::days(RENEW_BEFORE_DAYS))
            .unwrap_or(true)
    }

    /// Load the server certificate, creating or renewing it as needed.
    pub fn load_or_create(&self, hostnames: &[String]) -> Result<PemPair, AppError> {
        let (ca_pem, ca_key_pem) = self.ensure_ca()?;
        if self.needs_renewal(hostnames) {
            let ca_key = KeyPair::from_pem(&ca_key_pem).map_err(tls_err)?;
            let ca_params = CertificateParams::from_ca_cert_pem(&ca_pem).map_err(tls_err)?;
            let ca = ca_params.self_signed(&ca_key).map_err(tls_err)?;
            let key = KeyPair::generate().map_err(tls_err)?;
            let mut params = CertificateParams::new(hostnames.to_vec()).map_err(tls_err)?;
            for h in hostnames {
                if let Ok(ip) = h.parse() {
                    params.subject_alt_names.push(SanType::IpAddress(ip));
                }
            }
            params.distinguished_name.push(DnType::CommonName, hostnames.first().cloned().unwrap_or_else(|| "localhost".into()));
            params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
            params.not_before = offset(-1);
            params.not_after = offset(SERVER_VALID_DAYS);
            let cert = params.signed_by(&key, &ca, &ca_key).map_err(tls_err)?;
            std::fs::write(self.p("server.pem"), format!("{}{}", cert.pem(), ca_pem))?;
            write_private(&self.p("server.key"), &key.serialize_pem())?;
            let meta = Meta { server_expires: (chrono::Utc::now() + chrono::Duration::days(SERVER_VALID_DAYS)).to_rfc3339(), hostnames: hostnames.to_vec() };
            std::fs::write(self.p("server.json"), serde_json::to_vec_pretty(&meta)?)?;
            tracing::info!("Generated HTTPS certificate for {:?}; trust {} on clients", hostnames, self.ca_cert_path().display());
        }
        Ok(PemPair { cert_chain_pem: std::fs::read_to_string(self.p("server.pem"))?, key_pem: std::fs::read_to_string(self.p("server.key"))? })
    }
}

fn write_private(path: &Path, content: &str) -> Result<(), AppError> {
    std::fs::write(path, content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

/// Lowest TLS protocol version the server accepts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MinTls {
    #[default]
    Tls12,
    Tls13,
}

impl std::str::FromStr for MinTls {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().trim_start_matches("tls").trim_start_matches(['v', '-', '_']) {
            "1.2" | "12" => Ok(Self::Tls12),
            "1.3" | "13" => Ok(Self::Tls13),
            other => Err(format!("unsupported minimum TLS version '{other}' (1.2 or 1.3)")),
        }
    }
}

/// A rustls server configuration with the protocol floor applied (TLS 1.0/1.1
/// are never offered), HTTP/2 + HTTP/1.1 ALPN and no client certificates.
pub fn server_config(chain: Vec<Vec<u8>>, key: Vec<u8>, min: MinTls) -> Result<rustls::ServerConfig, AppError> {
    use rustls::pki_types::{CertificateDer, PrivateKeyDer};
    let versions: &[&'static rustls::SupportedProtocolVersion] = match min {
        MinTls::Tls12 => &[&rustls::version::TLS13, &rustls::version::TLS12],
        MinTls::Tls13 => &[&rustls::version::TLS13],
    };
    let provider = std::sync::Arc::new(rustls::crypto::ring::default_provider());
    let key = PrivateKeyDer::try_from(key).map_err(tls_err)?;
    let mut cfg = rustls::ServerConfig::builder_with_provider(provider)
        .with_protocol_versions(versions)
        .map_err(tls_err)?
        .with_no_client_auth()
        .with_single_cert(chain.into_iter().map(CertificateDer::from).collect(), key)
        .map_err(tls_err)?;
    cfg.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Ok(cfg)
}

/// PEM chain + key → DER, for `server_config`.
pub fn pem_to_der(p: &PemPair) -> Result<(Vec<Vec<u8>>, Vec<u8>), AppError> {
    let chain = rustls_pemfile::certs(&mut p.cert_chain_pem.as_bytes()).map(|c| c.map(|c| c.to_vec())).collect::<Result<Vec<_>, _>>().map_err(tls_err)?;
    let key = rustls_pemfile::private_key(&mut p.key_pem.as_bytes()).map_err(tls_err)?.ok_or_else(|| tls_err("no private key in the PEM file"))?;
    Ok((chain, key.secret_der().to_vec()))
}

/// Read a PFX/P12 bundle into DER chain + PKCS#8 key.
pub fn load_pfx(path: &Path, password: &str) -> Result<(Vec<Vec<u8>>, Vec<u8>), AppError> {
    let data = std::fs::read(path)?;
    let ks = p12_keystore::KeyStore::from_pkcs12(&data, password).map_err(tls_err)?;
    let (_, chain) = ks.private_key_chain().ok_or_else(|| tls_err("the PFX file contains no private key"))?;
    Ok((chain.chain().iter().map(|c| c.as_der().to_vec()).collect(), chain.key().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Handshake a client limited to `client` versions against a server with `min`.
    async fn handshake(min: MinTls, client: &[&'static rustls::SupportedProtocolVersion]) -> Result<rustls::ProtocolVersion, String> {
        let d = tempfile::tempdir().unwrap();
        let g = GeneratedCerts::new(d.path());
        let pair = g.load_or_create(&["localhost".to_string()]).unwrap();
        let (chain, key) = pem_to_der(&pair).unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(std::sync::Arc::new(server_config(chain, key, min).unwrap()));
        let mut roots = rustls::RootCertStore::empty();
        for c in rustls_pemfile::certs(&mut std::fs::read(g.ca_cert_path()).unwrap().as_slice()) {
            roots.add(c.unwrap()).unwrap();
        }
        let ccfg = rustls::ClientConfig::builder_with_provider(std::sync::Arc::new(rustls::crypto::ring::default_provider()))
            .with_protocol_versions(client)
            .unwrap()
            .with_root_certificates(roots)
            .with_no_client_auth();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (sock, _) = listener.accept().await.unwrap();
            let _ = acceptor.accept(sock).await;
        });
        let sock = tokio::net::TcpStream::connect(addr).await.unwrap();
        let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
        let tls = tokio_rustls::TlsConnector::from(std::sync::Arc::new(ccfg)).connect(name, sock).await.map_err(|e| e.to_string())?;
        Ok(tls.get_ref().1.protocol_version().unwrap())
    }

    #[tokio::test]
    async fn minimum_tls_version_is_enforced() {
        use rustls::version::{TLS12, TLS13};
        assert_eq!(handshake(MinTls::Tls12, &[&TLS12]).await.unwrap(), rustls::ProtocolVersion::TLSv1_2);
        assert_eq!(handshake(MinTls::Tls12, &[&TLS13, &TLS12]).await.unwrap(), rustls::ProtocolVersion::TLSv1_3);
        assert!(handshake(MinTls::Tls13, &[&TLS12]).await.is_err(), "a TLS 1.2-only client is refused");
        assert_eq!(handshake(MinTls::Tls13, &[&TLS13]).await.unwrap(), rustls::ProtocolVersion::TLSv1_3);
        assert_eq!("1.3".parse::<MinTls>().unwrap(), MinTls::Tls13);
        assert_eq!("TLSv1.2".parse::<MinTls>().unwrap(), MinTls::Tls12);
        assert!("1.1".parse::<MinTls>().is_err());
    }

    #[test]
    fn generates_and_reuses_until_renewal() {
        let d = tempfile::tempdir().unwrap();
        let g = GeneratedCerts::new(d.path());
        let hosts = vec!["localhost".to_string(), "127.0.0.1".to_string()];
        let a = g.load_or_create(&hosts).unwrap();
        assert!(a.cert_chain_pem.contains("BEGIN CERTIFICATE") && a.key_pem.contains("PRIVATE KEY"));
        let b = g.load_or_create(&hosts).unwrap();
        assert_eq!(a.cert_chain_pem, b.cert_chain_pem, "reused while valid");
        // Force renewal: same CA, new leaf.
        let ca = std::fs::read_to_string(g.ca_cert_path()).unwrap();
        std::fs::write(d.path().join("server.json"), r#"{"server_expires":"2000-01-01T00:00:00Z","hostnames":["localhost","127.0.0.1"]}"#).unwrap();
        let c = g.load_or_create(&hosts).unwrap();
        assert_ne!(a.cert_chain_pem, c.cert_chain_pem);
        assert_eq!(ca, std::fs::read_to_string(g.ca_cert_path()).unwrap());
        assert!(c.cert_chain_pem.ends_with(&ca));
    }
}
