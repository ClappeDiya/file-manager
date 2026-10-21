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
