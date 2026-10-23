//! Microsoft Entra ID (Azure AD) access tokens for Azure services: the
//! client-credentials grant with a client secret, a certificate (signed
//! JWT assertion) or a federated token file (workload identity), and
//! managed identities (App Service / Functions `IDENTITY_ENDPOINT`, or the
//! VM instance metadata service). Tokens are cached per scope and fetched
//! again shortly before they expire.
//!
//! Used by the backup Azure store (scope `https://storage.azure.com/.default`)
//! and the Key Vault secret provider (`https://vault.azure.net/.default`).
//! Endpoints follow the Azure SDK variables: `AZURE_AUTHORITY_HOST`,
//! `IDENTITY_ENDPOINT` / `IDENTITY_HEADER`, `AZURE_POD_IDENTITY_AUTHORITY_HOST`
//! (instance metadata host).

use super::aws_chain::ChainEnv;
use crate::backup_engine::stores::http;
use crate::core::error::AppError;
use base64::Engine;
use std::collections::HashMap;
use std::time::{Duration, Instant};

pub const STORAGE_SCOPE: &str = "https://storage.azure.com/.default";
pub const VAULT_SCOPE: &str = "https://vault.azure.net/.default";
const AUTHORITY: &str = "https://login.microsoftonline.com";
const IMDS: &str = "http://169.254.169.254";
/// Fetch a new token this long before the current one expires.
pub const REFRESH_EARLY: Duration = Duration::from_secs(5 * 60);

/// How a client signs in to Entra ID.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AadCredential {
    /// Client secret (`client_secret`).
    Secret { tenant: String, client_id: String, secret: String },
    /// Certificate: PEM with the private key and certificate, or a PFX/P12
    /// file (`password` for the PFX).
    Certificate { tenant: String, client_id: String, path: String, password: Option<String> },
    /// Federated credential: a token file (AKS workload identity, GitHub
    /// OIDC, …) sent as the client assertion; read again on every refresh.
    Federated { tenant: String, client_id: String, token_file: String },
    /// Managed identity (system-assigned, or user-assigned by client id).
    Managed { client_id: Option<String> },
}

impl AadCredential {
    fn validate(&self) -> Result<(), AppError> {
        let need = |v: &str, what: &str| if v.trim().is_empty() { Err(AppError::validation(format!("Enter the {what}"))) } else { Ok(()) };
        match self {
            Self::Secret { tenant, client_id, secret } => need(tenant, "tenant id").and(need(client_id, "client (application) id")).and(need(secret, "client secret")),
            Self::Certificate { tenant, client_id, path, .. } => need(tenant, "tenant id").and(need(client_id, "client (application) id")).and(need(path, "certificate file")),
            Self::Federated { tenant, client_id, token_file } => need(tenant, "tenant id").and(need(client_id, "client (application) id")).and(need(token_file, "federated token file")),
            Self::Managed { .. } => Ok(()),
        }
    }
}

fn b64url(d: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(d)
}

/// Private key (PKCS#8 or PKCS#1 DER) and certificate DER from a PEM or PFX file.
fn load_certificate(path: &str, password: Option<&str>) -> Result<(ring::signature::RsaKeyPair, Vec<u8>), AppError> {
    let data = std::fs::read(path).map_err(|e| AppError::Configuration { message: format!("Cannot read the certificate {path}: {e}"), advice: "Check the path and that the app may read it.".into() })?;
    let (key, cert) = if data.starts_with(b"-----") || data.windows(5).any(|w| w == b"-----") {
        let mut rd = data.as_slice();
        let cert = rustls_pemfile::certs(&mut rd).next().and_then(Result::ok).map(|c| c.to_vec()).ok_or_else(|| AppError::validation("The PEM file holds no certificate (it needs the private key and the certificate)"))?;
        let mut rd = data.as_slice();
        let key = rustls_pemfile::private_key(&mut rd).ok().flatten().ok_or_else(|| AppError::validation("The PEM file holds no private key"))?;
        (key.secret_der().to_vec(), cert)
    } else {
        let (chain, key) = crate::server::tls::load_pfx(std::path::Path::new(path), password.unwrap_or(""))?;
        (key, chain.into_iter().next().ok_or_else(|| AppError::validation("The PFX file holds no certificate"))?)
    };
    let kp = ring::signature::RsaKeyPair::from_pkcs8(&key).or_else(|_| ring::signature::RsaKeyPair::from_der(&key)).map_err(|_| AppError::validation("The certificate key must be an RSA key (2048 bits or more)"))?;
    Ok((kp, cert))
}

/// RS256 client assertion for the token endpoint `aud`, with the
/// certificate's SHA-1 thumbprint (`x5t`) as Entra ID requires.
pub fn certificate_assertion(path: &str, password: Option<&str>, client_id: &str, aud: &str) -> Result<String, AppError> {
    use sha1::Digest;
    let (key, cert) = load_certificate(path, password)?;
    let now = chrono::Utc::now().timestamp();
    let header = serde_json::json!({ "alg": "RS256", "typ": "JWT", "x5t": b64url(&sha1::Sha1::digest(&cert)) });
    let claims = serde_json::json!({ "aud": aud, "iss": client_id, "sub": client_id, "jti": uuid::Uuid::new_v4().to_string(), "nbf": now - 60, "exp": now + 600 });
    let input = format!("{}.{}", b64url(header.to_string().as_bytes()), b64url(claims.to_string().as_bytes()));
    let mut sig = vec![0u8; key.public().modulus_len()];
    key.sign(&ring::signature::RSA_PKCS1_SHA256, &ring::rand::SystemRandom::new(), input.as_bytes(), &mut sig).map_err(|_| AppError::internal("JWT signing failed"))?;
    Ok(format!("{input}.{}", b64url(&sig)))
}

/// Cached access tokens for one credential.
pub struct AadTokenSource {
    cred: AadCredential,
    authority: String,
    env: ChainEnv,
    http: reqwest::Client,
    cache: tokio::sync::Mutex<HashMap<String, (String, Instant)>>,
}

impl AadTokenSource {
    /// `authority` overrides `https://login.microsoftonline.com` (sovereign
    /// clouds: `https://login.microsoftonline.us`, tests).
    pub fn new(cred: AadCredential, authority: Option<&str>, http: reqwest::Client, env: ChainEnv) -> Result<Self, AppError> {
        cred.validate()?;
        let authority = authority.map(str::trim).filter(|a| !a.is_empty()).map(String::from).or_else(|| env.var("AZURE_AUTHORITY_HOST")).unwrap_or_else(|| AUTHORITY.into());
        Ok(Self { cred, authority: authority.trim_end_matches('/').to_string(), env, http, cache: tokio::sync::Mutex::new(HashMap::new()) })
    }

    /// A valid token for `scope`, fetched when missing or about to expire.
    pub async fn token(&self, scope: &str) -> Result<String, AppError> {
        let mut cache = self.cache.lock().await;
        if let Some((t, exp)) = cache.get(scope) {
            if *exp > Instant::now() + REFRESH_EARLY {
                return Ok(t.clone());
            }
        }
        let (t, ttl) = self.fetch(scope).await?;
        cache.insert(scope.to_string(), (t.clone(), Instant::now() + Duration::from_secs(ttl)));
        Ok(t)
    }

    /// Forget cached tokens (after the service rejected one).
    pub async fn invalidate(&self) {
        self.cache.lock().await.clear();
    }

    fn token_url(&self, tenant: &str) -> String {
        format!("{}/{}/oauth2/v2.0/token", self.authority, tenant.trim())
    }

    async fn fetch(&self, scope: &str) -> Result<(String, u64), AppError> {
        let form: Vec<(&str, String)> = match &self.cred {
            AadCredential::Secret { client_id, secret, .. } => vec![("grant_type", "client_credentials".into()), ("client_id", client_id.trim().into()), ("client_secret", secret.clone()), ("scope", scope.into())],
            AadCredential::Certificate { tenant, client_id, path, password } => {
                let assertion = certificate_assertion(path, password.as_deref(), client_id.trim(), &self.token_url(tenant))?;
                vec![
                    ("grant_type", "client_credentials".into()),
                    ("client_id", client_id.trim().into()),
                    ("client_assertion_type", "urn:ietf:params:oauth:client-assertion-type:jwt-bearer".into()),
                    ("client_assertion", assertion),
                    ("scope", scope.into()),
                ]
            }
            AadCredential::Federated { client_id, token_file, .. } => {
                let tok = tokio::fs::read_to_string(token_file.trim())
                    .await
                    .map_err(|e| AppError::Configuration { message: format!("Cannot read the federated token file {token_file}: {e}"), advice: "Check the path (AKS: AZURE_FEDERATED_TOKEN_FILE).".into() })?;
                vec![
                    ("grant_type", "client_credentials".into()),
                    ("client_id", client_id.trim().into()),
                    ("client_assertion_type", "urn:ietf:params:oauth:client-assertion-type:jwt-bearer".into()),
                    ("client_assertion", tok.trim().to_string()),
                    ("scope", scope.into()),
                ]
            }
            AadCredential::Managed { client_id } => return self.managed(scope, client_id.as_deref()).await,
        };
        let tenant = match &self.cred {
            AadCredential::Secret { tenant, .. } | AadCredential::Certificate { tenant, .. } | AadCredential::Federated { tenant, .. } => tenant.clone(),
            AadCredential::Managed { .. } => unreachable!("handled above"),
        };
        let r = self.http.post(self.token_url(&tenant)).form(&form).send().await.map_err(|e| http::net("Microsoft Entra sign-in", e))?;
        let status = r.status();
        let v: serde_json::Value = r.json().await.unwrap_or_default();
        if !status.is_success() {
            let desc = v["error_description"].as_str().or(v["error"].as_str()).unwrap_or("").lines().next().unwrap_or("").to_string();
            return Err(AppError::connection(format!("Microsoft Entra sign-in failed ({status}): {desc}"), "Check the tenant, client id and credential, and that the app registration may use this resource."));
        }
        parse_token(&v)
    }

    async fn managed(&self, scope: &str, client_id: Option<&str>) -> Result<(String, u64), AppError> {
        let resource = scope.trim_end_matches("/.default");
        let client_id = client_id.filter(|c| !c.trim().is_empty());
        let req = if let (Some(ep), Some(hdr)) = (self.env.var("IDENTITY_ENDPOINT"), self.env.var("IDENTITY_HEADER")) {
            let mut q = vec![("api-version", "2019-08-01"), ("resource", resource)];
            if let Some(c) = client_id {
                q.push(("client_id", c));
            }
            self.http.get(ep).query(&q).header("X-IDENTITY-HEADER", hdr)
        } else {
            let host = self.env.var("AZURE_POD_IDENTITY_AUTHORITY_HOST").unwrap_or_else(|| IMDS.into());
            let mut q = vec![("api-version", "2018-02-01"), ("resource", resource)];
            if let Some(c) = client_id {
                q.push(("client_id", c));
            }
            self.http.get(format!("{}/metadata/identity/oauth2/token", host.trim_end_matches('/'))).query(&q).header("Metadata", "true").timeout(Duration::from_secs(10))
        };
        let r = req.send().await.map_err(|e| AppError::connection(format!("Managed identity: {e}"), "Run on Azure with a managed identity assigned, or pick another sign-in method."))?;
        let v: serde_json::Value = http::check(r, "Managed identity token").await?.json().await.map_err(|e| http::net("Managed identity token", e))?;
        parse_token(&v)
    }
}

/// `{access_token, expires_in}` (number or string; managed identity sends strings).
fn parse_token(v: &serde_json::Value) -> Result<(String, u64), AppError> {
    let t = v["access_token"].as_str().ok_or_else(|| AppError::connection("Microsoft Entra returned no access token", "Check the sign-in settings."))?;
    let ttl = v["expires_in"].as_u64().or_else(|| v["expires_in"].as_str().and_then(|s| s.parse().ok())).unwrap_or(3600);
    Ok((t.to_string(), ttl))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn certificate_assertion_is_signed_with_the_thumbprint() {
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/src/backup_engine/stores/testdata");
        let tmp = tempfile::tempdir().unwrap();
        let pem = tmp.path().join("combined.pem").to_string_lossy().into_owned();
        std::fs::write(&pem, format!("{}{}", std::fs::read_to_string(format!("{dir}/test-rsa-key.pem")).unwrap(), std::fs::read_to_string(format!("{dir}/test-rsa-cert.pem")).unwrap())).unwrap();
        let a = certificate_assertion(&pem, None, "app-1", "https://login/t/oauth2/v2.0/token").unwrap();
        let parts: Vec<&str> = a.split('.').collect();
        let header: serde_json::Value = serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
        let claims: serde_json::Value = serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
        assert_eq!((header["alg"].as_str(), claims["iss"].as_str(), claims["aud"].as_str()), (Some("RS256"), Some("app-1"), Some("https://login/t/oauth2/v2.0/token")));
        assert_eq!(header["x5t"].as_str().unwrap().len(), 27, "base64url SHA-1");
        // The PFX of the same key signs the same way (thumbprint identical).
        let b = certificate_assertion(&format!("{dir}/test-rsa.pfx"), Some("pfxpass"), "app-1", "aud").unwrap();
        let hb: serde_json::Value = serde_json::from_slice(&base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(b.split('.').next().unwrap()).unwrap()).unwrap();
        assert_eq!(hb["x5t"], header["x5t"]);
        assert!(certificate_assertion(&format!("{dir}/test-rsa.pfx"), Some("wrong"), "app-1", "aud").is_err());
        assert!(certificate_assertion(&format!("{dir}/test-rsa-cert.pem"), None, "a", "b").unwrap_err().user_message().contains("private key"));
    }

    #[test]
    fn credentials_are_validated() {
        let http = reqwest::Client::new();
        assert!(AadTokenSource::new(AadCredential::Secret { tenant: "t".into(), client_id: String::new(), secret: "s".into() }, None, http.clone(), ChainEnv::process()).is_err());
        let s = AadTokenSource::new(AadCredential::Managed { client_id: None }, Some("https://login.microsoftonline.us/"), http, ChainEnv::process()).unwrap();
        assert_eq!(s.token_url("tid"), "https://login.microsoftonline.us/tid/oauth2/v2.0/token");
        assert_eq!(parse_token(&serde_json::json!({ "access_token": "a", "expires_in": "120" })).unwrap(), ("a".into(), 120));
    }
}
