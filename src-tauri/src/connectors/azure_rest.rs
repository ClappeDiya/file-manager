//! Shared Azure Storage REST client for the Blob, Data Lake (ADLS Gen2) and
//! Files services: endpoints, Shared Key signing, SAS tokens and service SAS
//! links. Used by the `azure_blob`, `azure_adls` and `azure_files`
//! connectors and by the backup engine's Azure store.

use crate::backup_engine::stores::http::{self, TlsOptions};
use crate::core::error::AppError;
use base64::Engine;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use std::collections::BTreeMap;

/// REST version sent with every request (supports Files rename and DFS).
pub const VERSION: &str = "2021-08-06";
/// Version of generated SAS links.
const SAS_VERSION: &str = "2021-08-06";
/// Largest body sent in one request (Files ranges are limited to 4 MiB).
pub const CHUNK: usize = 4 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Blob,
    /// Data Lake Storage Gen2 (hierarchical namespace).
    Dfs,
    File,
}

impl Service {
    fn host_part(self) -> &'static str {
        match self {
            Service::Blob => "blob",
            Service::Dfs => "dfs",
            Service::File => "file",
        }
    }
}

enum Auth {
    SharedKey(Vec<u8>),
    Sas(String),
}

pub struct AzureClient {
    pub account: String,
    pub service: Service,
    auth: Auth,
    http: reqwest::Client,
    /// Endpoint without trailing slash, and its URL path (for signing).
    base: String,
    base_path: String,
}

/// RFC 1123 date as Azure wants it.
pub fn http_date(t: chrono::DateTime<chrono::Utc>) -> String {
    t.format("%a, %d %b %Y %H:%M:%S GMT").to_string()
}

/// Percent-encode a path, keeping `/`.
pub fn encode_path(p: &str) -> String {
    crate::backup_engine::stores::s3::uri_encode(p, true)
}

impl AzureClient {
    /// `endpoint`: empty = `https://<account>.<service>.core.windows.net`;
    /// a bare host (`myacct.blob.core.usgovcloudapi.net`) gets `https://`;
    /// a URL is used as given (Azurite: `http://127.0.0.1:10000/<account>`).
    /// `secret` = base64 account key, or a SAS token (`sv=…` / `?sv=…`).
    pub fn new(account: &str, endpoint: &str, service: Service, secret: &str, tls: &TlsOptions) -> Result<Self, AppError> {
        let account = account.trim();
        if account.is_empty() {
            return Err(AppError::validation("Enter the storage account name"));
        }
        let ep = endpoint.trim().trim_end_matches('/');
        let base = if ep.is_empty() {
            format!("https://{account}.{}.core.windows.net", service.host_part())
        } else if ep.starts_with("http://") || ep.starts_with("https://") {
            ep.to_string()
        } else {
            format!("https://{ep}")
        };
        let u = url::Url::parse(&base).map_err(|e| AppError::validation(format!("Invalid Azure endpoint: {e}")))?;
        let s = secret.trim();
        if s.is_empty() {
            return Err(AppError::validation("Enter the account key or a SAS token"));
        }
        let auth = if s.starts_with("sv=") || s.starts_with("?sv=") || s.contains("&sig=") {
            Auth::Sas(s.trim_start_matches('?').to_string())
        } else {
            Auth::SharedKey(base64::engine::general_purpose::STANDARD.decode(s).map_err(|_| AppError::validation("The Azure account key is not valid base64"))?)
        };
        Ok(Self { account: account.to_string(), service, auth, http: http::client(tls)?, base_path: u.path().trim_end_matches('/').to_string(), base })
    }

    pub fn base(&self) -> &str {
        &self.base
    }

    pub fn uses_sas(&self) -> bool {
        matches!(self.auth, Auth::Sas(_))
    }

    /// Shared Key signature (`Authorization` value) for a request; `None` with SAS.
    fn sign(&self, method: &str, path: &str, query: &[(&str, String)], headers: &BTreeMap<String, String>, content_length: usize) -> Option<String> {
        let Auth::SharedKey(key) = &self.auth else { return None };
        let h = |n: &str| headers.get(n).cloned().unwrap_or_default();
        let len = if content_length == 0 { String::new() } else { content_length.to_string() };
        let canon_headers: String = headers.iter().filter(|(k, _)| k.starts_with("x-ms-")).map(|(k, v)| format!("{k}:{}\n", v.trim())).collect();
        // Parameters with the same name are joined with commas (values and names sorted).
        let mut q: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (k, v) in query {
            q.entry(k.to_lowercase()).or_default().push(v.clone());
        }
        let canon_query: String = q
            .iter_mut()
            .map(|(k, v)| {
                v.sort();
                format!("\n{k}:{}", v.join(","))
            })
            .collect();
        let resource = format!("/{}{}{}", self.account, self.base_path, path);
        let sts = format!(
            "{method}\n{}\n{}\n{len}\n{}\n{}\n\n{}\n{}\n{}\n{}\n{}\n{canon_headers}{resource}{canon_query}",
            h("content-encoding"),
            h("content-language"),
            h("content-md5"),
            h("content-type"),
            h("if-modified-since"),
            h("if-match"),
            h("if-none-match"),
            h("if-unmodified-since"),
            h("range"),
        );
        Some(format!("SharedKey {}:{}", self.account, hmac_b64(key, &sts)))
    }

    /// Send a request. `path` is the encoded path below the endpoint
    /// (`/container/blob`), `query` unencoded pairs.
    pub async fn send(&self, method: reqwest::Method, path: &str, query: &[(&str, String)], extra: &[(&str, String)], body: Vec<u8>) -> Result<reqwest::Response, AppError> {
        let mut headers: BTreeMap<String, String> = BTreeMap::new();
        headers.insert("x-ms-date".into(), http_date(chrono::Utc::now()));
        headers.insert("x-ms-version".into(), VERSION.into());
        for (k, v) in extra {
            headers.insert(k.to_lowercase(), v.clone());
        }
        let mut qs: Vec<String> = query.iter().map(|(k, v)| format!("{k}={}", urlencoding::encode(v))).collect();
        if let Auth::Sas(sas) = &self.auth {
            qs.push(sas.clone());
        }
        let path = if path.is_empty() { "/" } else { path };
        let url = format!("{}{path}{}", self.base, if qs.is_empty() { String::new() } else { format!("?{}", qs.join("&")) });
        let mut req = self.http.request(method.clone(), &url);
        if let Some(a) = self.sign(method.as_str(), path, query, &headers, body.len()) {
            req = req.header("authorization", a);
        }
        for (k, v) in &headers {
            req = req.header(k.as_str(), v.as_str());
        }
        req.body(body).send().await.map_err(|e| http::net("Azure request", e))
    }

    /// `send` + fail on a non-success status.
    pub async fn call(&self, what: &str, method: reqwest::Method, path: &str, query: &[(&str, String)], extra: &[(&str, String)], body: Vec<u8>) -> Result<reqwest::Response, AppError> {
        http::check(self.send(method, path, query, extra, body).await?, what).await
    }

    /// A read-only link to one blob or file: a service SAS signed with the
    /// account key, or the connection's own SAS token appended.
    /// `path` is unencoded (`container/blob` or `share/dir/file`).
    pub fn read_link(&self, path: &str, hours: u32) -> Result<String, AppError> {
        let path = path.trim_start_matches('/');
        let url = format!("{}/{}", self.base, encode_path(path));
        let key = match &self.auth {
            Auth::Sas(sas) => return Ok(format!("{url}?{sas}")),
            Auth::SharedKey(k) => k,
        };
        let now = chrono::Utc::now();
        let st = (now - chrono::Duration::minutes(5)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let se = (now + chrono::Duration::hours(hours as i64)).format("%Y-%m-%dT%H:%M:%SZ").to_string();
        let spr = if self.base.starts_with("https://") { "https" } else { "https,http" };
        let (sts, sr) = match self.service {
            // Data Lake files are blobs: links go through the blob service.
            Service::Blob | Service::Dfs => (format!("r\n{st}\n{se}\n/blob/{}/{path}\n\n\n{spr}\n{SAS_VERSION}\nb\n\n\n\n\n\n\n", self.account), "b"),
            Service::File => (format!("r\n{st}\n{se}\n/file/{}/{path}\n\n\n{spr}\n{SAS_VERSION}\n\n\n\n\n", self.account), "f"),
        };
        let sig = urlencoding::encode(&hmac_b64(key, &sts)).into_owned();
        let url = if self.service == Service::Dfs { url.replacen(".dfs.", ".blob.", 1) } else { url };
        Ok(format!("{url}?sv={SAS_VERSION}&st={st}&se={se}&sr={sr}&sp=r&spr={}&sig={sig}", urlencoding::encode(spr)))
    }
}

fn hmac_b64(key: &[u8], s: &str) -> String {
    let mut m = Hmac::<Sha256>::new_from_slice(key).expect("any key length");
    m.update(s.as_bytes());
    base64::engine::general_purpose::STANDARD.encode(m.finalize().into_bytes())
}

/// Parse an RFC 1123 (`Last-Modified`) or RFC 3339 time.
pub fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    chrono::DateTime::parse_from_rfc2822(s).or_else(|_| chrono::DateTime::parse_from_rfc3339(s)).ok().map(|t| t.with_timezone(&chrono::Utc))
}

/// `/a/b/c` → (`a`, `b/c`); `/a` → (`a`, ``); `/` → (``, ``).
pub fn split_root(path: &str) -> (String, String) {
    let t = path.trim_matches('/');
    match t.split_once('/') {
        Some((a, b)) => (a.to_string(), b.trim_matches('/').to_string()),
        None => (t.to_string(), String::new()),
    }
}

/// Connection details from a profile: account (user name), endpoint (host:
/// empty, a bare host, or a URL), and whether the host is the default one.
pub fn profile_endpoint(host: &str, service: Service) -> String {
    let h = host.trim();
    let default = format!("{}.core.windows.net", service.host_part());
    if h.is_empty() || h == default {
        String::new()
    } else {
        h.to_string()
    }
}

/// Azurite's well-known development account and key.
pub const DEV_ACCOUNT: &str = "devstoreaccount1";
pub const DEV_KEY: &str = "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==";

/// Account, endpoint and secret from an Azure connection string
/// (`DefaultEndpointsProtocol=…;AccountName=…;AccountKey=…;EndpointSuffix=…`,
/// `…;SharedAccessSignature=…`, explicit `BlobEndpoint`/`FileEndpoint`/
/// `DfsEndpoint`, or `UseDevelopmentStorage=true`).
pub fn parse_connection_string(cs: &str, service: Service) -> Result<(String, String, String), AppError> {
    let kv: std::collections::HashMap<String, String> = cs
        .split(';')
        .filter_map(|p| p.split_once('=').map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string())))
        .collect();
    if kv.get("usedevelopmentstorage").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
        let port = match service {
            Service::Blob | Service::Dfs => 10000,
            Service::File => 10004,
        };
        return Ok((DEV_ACCOUNT.into(), format!("http://127.0.0.1:{port}/{DEV_ACCOUNT}"), DEV_KEY.into()));
    }
    let account = kv.get("accountname").cloned().unwrap_or_default();
    let secret = kv.get("accountkey").or_else(|| kv.get("sharedaccesssignature")).cloned().unwrap_or_default();
    if secret.is_empty() {
        return Err(AppError::validation("The connection string has no AccountKey or SharedAccessSignature"));
    }
    let explicit = match service {
        Service::Blob => kv.get("blobendpoint"),
        Service::Dfs => kv.get("dfsendpoint"),
        Service::File => kv.get("fileendpoint"),
    };
    let endpoint = match (explicit, kv.get("endpointsuffix")) {
        (Some(e), _) => e.clone(),
        (None, Some(suffix)) => format!("{}://{account}.{}.{suffix}", kv.get("defaultendpointsprotocol").map(String::as_str).unwrap_or("https"), service.host_part()),
        (None, None) => String::new(),
    };
    let account = if account.is_empty() {
        // SAS-only strings name the account in the endpoint host.
        url::Url::parse(&endpoint).ok().and_then(|u| u.host_str().and_then(|h| h.split('.').next()).map(str::to_string)).unwrap_or_default()
    } else {
        account
    };
    Ok((account, endpoint, secret))
}

/// A client for a saved connection: the account is the user name, the
/// endpoint the host (empty for the public cloud), the secret an account
/// key, a SAS token or a whole connection string.
pub fn client_for_profile(profile: &crate::core::types::ConnectionProfile, secret: Option<String>, service: Service) -> Result<AzureClient, AppError> {
    let secret = secret.filter(|s| !s.trim().is_empty()).ok_or_else(|| AppError::validation("Enter the account key, a SAS token or a connection string"))?;
    let (account, endpoint, secret) = if secret.contains("AccountName=") || secret.contains("UseDevelopmentStorage=") || secret.contains("Endpoint=") {
        let (a, e, s) = parse_connection_string(&secret, service)?;
        let account = profile.username.clone().filter(|u| !u.trim().is_empty()).unwrap_or(a);
        let endpoint = if profile_endpoint(&profile.host, service).is_empty() { e } else { profile.host.clone() };
        (account, endpoint, s)
    } else {
        (profile.username.clone().unwrap_or_default(), profile_endpoint(&profile.host, service), secret)
    };
    AzureClient::new(&account, &endpoint, service, &secret, &TlsOptions::default())
}

/// A connector's live client (`None` while disconnected).
#[derive(Default)]
pub struct Slot(tokio::sync::RwLock<Option<std::sync::Arc<AzureClient>>>);

impl Slot {
    pub async fn get(&self, what: &str) -> Result<std::sync::Arc<AzureClient>, AppError> {
        self.0.read().await.clone().ok_or_else(|| not_connected(what))
    }
    pub async fn set(&self, c: Option<AzureClient>) {
        *self.0.write().await = c.map(std::sync::Arc::new);
    }
    pub fn is_set(&self) -> bool {
        self.0.try_read().map(|s| s.is_some()).unwrap_or(false)
    }
}

/// Whole hours for a read-only link (default one day); password, edit and
/// organization-only links are not available for SAS links.
pub fn sas_hours(o: &super::ShareOptions) -> Result<u32, AppError> {
    if o.password.as_deref().is_some_and(|p| !p.is_empty()) {
        return Err(super::share_unsupported("set a password"));
    }
    if o.allow_edit {
        return Err(super::share_unsupported("allow editing"));
    }
    if o.scope != super::ShareScope::Anyone {
        return Err(super::share_unsupported("limit links to your organization"));
    }
    match o.expires_in_secs {
        None => Ok(24),
        Some(0) => Err(AppError::validation("Choose an expiry time")),
        Some(s) => u32::try_from(s.div_ceil(3600)).map_err(|_| AppError::validation("Expiry too far in the future")),
    }
}

/// A share link for `path` (unencoded `root/…`).
pub async fn share_link(slot: &Slot, what: &str, provider: &str, path: &str, o: &super::ShareOptions) -> Result<super::ShareLink, AppError> {
    let hours = sas_hours(o)?;
    let (root, rest) = split_root(path);
    if root.is_empty() || rest.is_empty() {
        return Err(AppError::validation("Choose a file to share"));
    }
    let c = slot.get(what).await?;
    let fixed = c.uses_sas();
    Ok(super::ShareLink {
        url: c.read_link(path, hours)?,
        expires_at: (!fixed).then(|| chrono::Utc::now() + chrono::Duration::hours(hours as i64)),
        scope: super::ShareScope::Anyone,
        provider: provider.into(),
        note: Some(if fixed { "Uses this connection's SAS token: it expires when that token does.".into() } else { "Read-only SAS link: anyone holding it can download until it expires.".into() }),
    })
}

/// Error for an operation on a disconnected connector.
pub fn not_connected(what: &str) -> AppError {
    AppError::connection(format!("Not connected to {what}"), "Connect first.")
}

/// Removing a whole container, file system or share deletes everything in it.
pub fn refuse_root_delete(kind: &str) -> AppError {
    AppError::validation(format!("Deleting a whole {kind} removes everything in it; do that in the Azure portal or CLI."))
}

/// Build a file entry.
pub fn entry(parent: &str, name: &str, is_dir: bool, size: u64, modified: Option<chrono::DateTime<chrono::Utc>>) -> crate::core::types::FileEntry {
    let parent = parent.trim_end_matches('/');
    crate::core::types::FileEntry {
        name: name.to_string(),
        path: format!("{parent}/{name}"),
        is_dir,
        is_symlink: false,
        size,
        modified,
        created: None,
        is_hidden: name.starts_with('.'),
        extension: if is_dir { None } else { std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_lowercase()) },
        permissions: None,
    }
}

/// Stream a response body into a local file; returns bytes written.
pub async fn save_body(mut resp: reqwest::Response, local: &str) -> Result<u64, AppError> {
    use tokio::io::AsyncWriteExt;
    let part = format!("{local}.ufop-part");
    let mut f = tokio::fs::File::create(&part).await?;
    let mut n = 0u64;
    let r: Result<(), AppError> = async {
        while let Some(chunk) = resp.chunk().await.map_err(|e| http::net("Azure download", e))? {
            f.write_all(&chunk).await?;
            n += chunk.len() as u64;
        }
        f.sync_all().await?;
        Ok(())
    }
    .await;
    drop(f);
    if let Err(e) = r {
        let _ = tokio::fs::remove_file(&part).await;
        return Err(e);
    }
    tokio::fs::rename(&part, local).await?;
    Ok(n)
}

/// Read a local file in `CHUNK`-sized pieces, calling `f(offset, bytes)`.
pub async fn for_each_chunk<F, Fut>(local: &str, mut f: F) -> Result<u64, AppError>
where
    F: FnMut(u64, Vec<u8>) -> Fut,
    Fut: std::future::Future<Output = Result<(), AppError>>,
{
    use tokio::io::AsyncReadExt;
    let mut file = tokio::fs::File::open(local).await?;
    let mut off = 0u64;
    loop {
        let mut buf = Vec::with_capacity(CHUNK);
        (&mut file).take(CHUNK as u64).read_to_end(&mut buf).await?;
        if buf.is_empty() {
            break;
        }
        let n = buf.len() as u64;
        f(off, buf).await?;
        off += n;
    }
    Ok(off)
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: &str = DEV_KEY;

    #[test]
    fn connection_strings() {
        let (a, e, s) = parse_connection_string("DefaultEndpointsProtocol=https;AccountName=acct;AccountKey=abc==;EndpointSuffix=core.usgovcloudapi.net", Service::File).unwrap();
        assert_eq!((a.as_str(), e.as_str(), s.as_str()), ("acct", "https://acct.file.core.usgovcloudapi.net", "abc=="));
        let (a, e, s) = parse_connection_string("BlobEndpoint=https://acct.blob.core.windows.net/;SharedAccessSignature=sv=2021&sig=x", Service::Blob).unwrap();
        assert_eq!((a.as_str(), e.as_str(), s.as_str()), ("acct", "https://acct.blob.core.windows.net/", "sv=2021&sig=x"));
        let (a, e, _) = parse_connection_string("UseDevelopmentStorage=true", Service::Blob).unwrap();
        assert_eq!((a.as_str(), e.as_str()), (DEV_ACCOUNT, "http://127.0.0.1:10000/devstoreaccount1"));
        assert!(parse_connection_string("AccountName=a", Service::Blob).is_err());
    }

    #[test]
    fn endpoints_and_secrets() {
        let t = TlsOptions::default();
        assert_eq!(AzureClient::new("acct", "", Service::Dfs, KEY, &t).unwrap().base(), "https://acct.dfs.core.windows.net");
        assert_eq!(AzureClient::new("acct", "acct.file.core.chinacloudapi.cn", Service::File, KEY, &t).unwrap().base(), "https://acct.file.core.chinacloudapi.cn");
        assert_eq!(AzureClient::new("devstoreaccount1", "http://127.0.0.1:10000/devstoreaccount1/", Service::Blob, KEY, &t).unwrap().base(), "http://127.0.0.1:10000/devstoreaccount1");
        assert!(AzureClient::new("acct", "", Service::Blob, "?sv=2021&sig=x", &t).unwrap().uses_sas());
        assert!(AzureClient::new("acct", "", Service::Blob, "not base64!", &t).is_err());
        assert!(AzureClient::new("", "", Service::Blob, KEY, &t).is_err());
        assert!(AzureClient::new("acct", "", Service::Blob, " ", &t).is_err());
        assert_eq!(profile_endpoint("blob.core.windows.net", Service::Blob), "");
        assert_eq!(profile_endpoint("x.example", Service::File), "x.example");
        assert_eq!(split_root("/fs/a/b/"), ("fs".into(), "a/b".into()));
        assert_eq!(split_root("/"), (String::new(), String::new()));
    }

    #[test]
    fn shared_key_string_to_sign_is_canonical() {
        let c = AzureClient::new("acct", "", Service::Blob, KEY, &TlsOptions::default()).unwrap();
        let mut h = BTreeMap::new();
        h.insert("x-ms-version".to_string(), VERSION.to_string());
        h.insert("x-ms-date".to_string(), "Mon, 01 Jan 2024 00:00:00 GMT".to_string());
        // Query order and case do not change the signature; repeated names join.
        let a = c.sign("GET", "/c", &[("restype", "container".into()), ("COMP", "list".into())], &h, 0).unwrap();
        let b = c.sign("GET", "/c", &[("comp", "list".into()), ("restype", "container".into())], &h, 0).unwrap();
        assert_eq!(a, b);
        assert!(a.starts_with("SharedKey acct:"));
        assert_ne!(a, c.sign("GET", "/c", &[("comp", "list".into())], &h, 0).unwrap());
    }

    #[test]
    fn read_links() {
        let c = AzureClient::new("acct", "", Service::File, KEY, &TlsOptions::default()).unwrap();
        let l = c.read_link("/share/dir/a b.txt", 2).unwrap();
        assert!(l.starts_with("https://acct.file.core.windows.net/share/dir/a%20b.txt?sv="), "{l}");
        assert!(l.contains("&sr=f&sp=r&spr=https&sig="));
        let d = AzureClient::new("acct", "", Service::Dfs, KEY, &TlsOptions::default()).unwrap().read_link("fs/x.bin", 1).unwrap();
        assert!(d.starts_with("https://acct.blob.core.windows.net/fs/x.bin?") && d.contains("&sr=b&"), "{d}");
        let s = AzureClient::new("acct", "", Service::Blob, "sv=2021&sig=abc", &TlsOptions::default()).unwrap().read_link("c/b", 1).unwrap();
        assert_eq!(s, "https://acct.blob.core.windows.net/c/b?sv=2021&sig=abc");
    }
}
