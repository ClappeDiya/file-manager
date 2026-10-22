//! Filen (filen.io): end-to-end encrypted cloud storage.
//!
//! Everything is encrypted on this machine in Filen's own format, so the
//! volumes can be opened in Filen's apps. The format follows the official
//! SDKs:
//!
//! - TypeScript SDK: <https://github.com/FilenCloudDienste/filen-sdk-ts>
//!   (`src/crypto/{utils,encrypt,decrypt}.ts`, `src/api/client.ts`,
//!   `src/api/v3/**`, `src/cloud/index.ts` `uploadLocalFile`, `src/index.ts` `login`)
//! - Rust SDK: <https://github.com/FilenCloudDienste/filen-rs>
//!   (`filen-sdk-rs/src/crypto/v2.rs`, `crypto/v3.rs`, `auth/v2.rs`, `fs/file/write.rs`)
//! - Duplicati's Filen backend (endpoint usage):
//!   <https://github.com/duplicati/duplicati/tree/master/Duplicati/Library/Backend/Filen>
//!
//! Supported versions:
//!
//! - **Auth version 2** (PBKDF2-SHA512, 200 000 rounds; the version the SDK
//!   registers new accounts with). Version 1 is deprecated and version 3
//!   (Argon2id + DEK) is refused: its file-name hash needs an HMAC key derived
//!   from the account's RSA private key, and the two official SDKs derive that
//!   key differently (HKDF over the PKCS#8 DER in filen-sdk-ts, over the RSA
//!   exponent `d` in filen-rs), so a v3 upload could not be verified correct.
//! - **Metadata**: writes version `002` (AES-256-GCM, key = PBKDF2(key, key, 1)),
//!   the SDK default; reads `002` and `003`.
//! - **File data**: writes version 2 (32-character key, per-chunk AES-256-GCM,
//!   `iv ‖ ciphertext ‖ tag`, 1 MiB plaintext chunks); reads versions 2 and 3.
//!
//! The secret is the account password, or JSON
//! `{"password": "...", "api_key": "...", "two_factor_code": "..."}`. The
//! password is always needed (the master key is derived from it); an API key
//! (e.g. from `filen-cli`) skips the login call, which avoids two-factor
//! prompts on unattended runs.

use super::http::{self, TlsOptions};
use super::{BoxFut, ObjectStore};
use crate::core::error::AppError;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use tokio::sync::Mutex;
use ufop_core::backup::backend::Quota;
use ufop_core::backup::RemoteFile;

/// Plaintext chunk size (`UPLOAD_CHUNK_SIZE` in the SDK).
pub const CHUNK: usize = 1024 * 1024;
const GATEWAY: &str = "https://gateway.filen.io";
const INGEST: &str = "https://ingest.filen.io";
const EGEST: &str = "https://egest.filen.io";
const MIME: &str = "application/octet-stream";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FilenConfig {
    /// Account e-mail address.
    pub email: String,
    /// Folder below the account's root, e.g. `Backups/laptop`.
    pub folder: String,
    /// Move deleted volumes to the Filen trash instead of deleting them permanently.
    pub use_trash: bool,
    /// Gateway / upload / download hosts (tests); empty = Filen's.
    pub gateway_url: String,
    pub ingest_url: String,
    pub egest_url: String,
    pub tls: TlsOptions,
}

/// Filen's client-side cryptography (auth version 2 / metadata 002+003 / data 2+3).
pub mod crypto {
    use aes_gcm::aead::{Aead, KeyInit};
    use aes_gcm::{Aes256Gcm, Nonce};
    use base64::Engine;
    use sha2::Digest;
    use std::num::NonZeroU32;

    /// `base64Charset` in filen-sdk-ts `crypto/utils.ts`.
    const CHARSET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    fn pbkdf2_sha512(password: &[u8], salt: &[u8], rounds: u32, out: &mut [u8]) {
        ring::pbkdf2::derive(ring::pbkdf2::PBKDF2_HMAC_SHA512, NonZeroU32::new(rounds).expect("rounds > 0"), salt, password, out);
    }

    /// `generateRandomString`: `n` characters from the SDK's charset.
    pub fn random_string(n: usize) -> String {
        use rand::RngCore;
        let mut b = vec![0u8; n];
        rand::rngs::OsRng.fill_bytes(&mut b);
        b.iter().map(|x| CHARSET[*x as usize % CHARSET.len()] as char).collect()
    }

    /// Auth version 2: `(master key, derived login password)`.
    pub fn derive_v2(password: &str, salt: &str) -> (String, String) {
        let mut out = [0u8; 64];
        pbkdf2_sha512(password.as_bytes(), salt.as_bytes(), 200_000, &mut out);
        let hex = hex::encode(out);
        let (mk, pw) = hex.split_at(64);
        (mk.to_string(), hex::encode(sha2::Sha512::digest(pw.as_bytes())))
    }

    /// `hashFileName` for auth versions 1 and 2: `sha1(hex(sha512(lowercase(name))))`.
    pub fn hash_name_v2(name: &str) -> String {
        let inner = hex::encode(sha2::Sha512::digest(name.to_lowercase().as_bytes()));
        hex::encode(sha1::Sha1::digest(inner.as_bytes()))
    }

    pub fn sha512_hex(b: &[u8]) -> String {
        hex::encode(sha2::Sha512::digest(b))
    }

    fn cipher(key: &[u8]) -> Result<Aes256Gcm, String> {
        Aes256Gcm::new_from_slice(key).map_err(|_| "invalid key length".to_string())
    }

    fn meta_key_v2(key: &str) -> [u8; 32] {
        let mut k = [0u8; 32];
        pbkdf2_sha512(key.as_bytes(), key.as_bytes(), 1, &mut k);
        k
    }

    /// Metadata version 002 with a caller-chosen 12-character IV (tests); see [`encrypt_meta`].
    pub fn encrypt_meta_with_iv(plain: &str, key: &str, iv: &str) -> String {
        debug_assert_eq!(iv.len(), 12);
        let ct = cipher(&meta_key_v2(key)).expect("32-byte key").encrypt(Nonce::from_slice(iv.as_bytes()), plain.as_bytes()).expect("AES-GCM encrypt");
        format!("002{iv}{}", base64::engine::general_purpose::STANDARD.encode(ct))
    }

    /// Metadata version 002: `"002" ‖ iv(12 chars) ‖ base64(ciphertext ‖ tag)`.
    pub fn encrypt_meta(plain: &str, key: &str) -> String {
        encrypt_meta_with_iv(plain, key, &random_string(12))
    }

    /// Decrypt metadata versions 002 and 003.
    pub fn decrypt_meta(meta: &str, key: &str) -> Result<String, String> {
        let b = meta.as_bytes();
        let b64 = base64::engine::general_purpose::STANDARD;
        let (k, iv, body): (Vec<u8>, Vec<u8>, &[u8]) = match b.get(..3) {
            Some(b"002") if b.len() >= 15 => (meta_key_v2(key).to_vec(), b[3..15].to_vec(), &b[15..]),
            Some(b"003") if b.len() >= 27 => {
                let k = hex::decode(key).ok().filter(|k| k.len() == 32).ok_or("version 003 needs a 64-character hex key")?;
                (k, hex::decode(&b[3..27]).map_err(|e| e.to_string())?, &b[27..])
            }
            _ if meta.starts_with("U2FsdGVk") => return Err("metadata version 1 (deprecated) is not supported".into()),
            _ => return Err(format!("unknown metadata version '{}'", String::from_utf8_lossy(&b[..b.len().min(3)]))),
        };
        let ct = b64.decode(body).map_err(|e| e.to_string())?;
        let pt = cipher(&k)?.decrypt(Nonce::from_slice(&iv), ct.as_slice()).map_err(|_| "authentication failed (wrong key)".to_string())?;
        String::from_utf8(pt).map_err(|e| e.to_string())
    }

    /// File data version 2 with a caller-chosen IV (tests); see [`encrypt_data`].
    pub fn encrypt_data_with_iv(data: &[u8], key: &str, iv: &[u8; 12]) -> Result<Vec<u8>, String> {
        if key.len() != 32 {
            return Err("version 2 file keys are 32 characters".into());
        }
        let ct = cipher(key.as_bytes())?.encrypt(Nonce::from_slice(iv), data).map_err(|_| "AES-GCM encrypt".to_string())?;
        let mut out = Vec::with_capacity(12 + ct.len());
        out.extend_from_slice(iv);
        out.extend_from_slice(&ct);
        Ok(out)
    }

    /// File data version 2: `iv(12) ‖ ciphertext ‖ tag(16)`, key = the 32 key characters.
    /// The IV is 12 charset characters, as `encrypt.data` generates it.
    pub fn encrypt_data(data: &[u8], key: &str) -> Result<Vec<u8>, String> {
        let iv: [u8; 12] = random_string(12).into_bytes().try_into().expect("12 bytes");
        encrypt_data_with_iv(data, key, &iv)
    }

    /// Decrypt file data versions 2 (32-character key) and 3 (64-character hex key).
    pub fn decrypt_data(version: u64, data: &[u8], key: &str) -> Result<Vec<u8>, String> {
        let k = match version {
            2 => key.as_bytes().to_vec(),
            3 => hex::decode(key).map_err(|e| e.to_string())?,
            v => return Err(format!("file encryption version {v} is not supported")),
        };
        if data.len() < 28 {
            return Err("encrypted chunk is truncated".into());
        }
        cipher(&k)?.decrypt(Nonce::from_slice(&data[..12]), &data[12..]).map_err(|_| "authentication failed (wrong key or corrupted chunk)".to_string())
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct FilenSecret {
    password: String,
    api_key: String,
    two_factor_code: String,
}

impl FilenSecret {
    fn parse(s: &str) -> Result<Self, AppError> {
        let t = s.trim();
        let v = if t.starts_with('{') {
            serde_json::from_str(t).map_err(|e| AppError::validation(format!("Invalid Filen credentials JSON: {e}")))?
        } else {
            FilenSecret { password: s.trim_end_matches(['\r', '\n']).to_string(), ..Default::default() }
        };
        if v.password.is_empty() {
            return Err(AppError::validation("Enter the Filen account password (it is needed to derive the encryption key)"));
        }
        Ok(v)
    }
}

#[derive(Clone)]
struct Session {
    api_key: String,
    /// Oldest first; new metadata is encrypted with the last one.
    master_keys: Vec<String>,
    base_folder: String,
}

impl Session {
    fn encrypt(&self, plain: &str) -> String {
        crypto::encrypt_meta(plain, self.master_keys.last().expect("at least one master key"))
    }
    fn decrypt(&self, meta: &str) -> Option<String> {
        self.master_keys.iter().rev().find_map(|k| crypto::decrypt_meta(meta, k).ok())
    }
}

#[derive(Debug, Clone)]
struct Entry {
    uuid: String,
    size: u64,
    key: String,
    region: String,
    bucket: String,
    chunks: u64,
    version: u64,
}

pub struct FilenStore {
    cfg: FilenConfig,
    secret: FilenSecret,
    http: reqwest::Client,
    session: Mutex<Session>,
    folder_uuid: Mutex<Option<String>>,
    cache: Mutex<Option<HashMap<String, Vec<Entry>>>>,
}

/// A failed gateway call: `auth` when the API key is no longer accepted.
struct CallErr {
    err: AppError,
    auth: bool,
}

impl From<AppError> for CallErr {
    fn from(err: AppError) -> Self {
        Self { err, auth: false }
    }
}

fn base(v: &str, d: &str) -> String {
    if v.trim().is_empty() { d.to_string() } else { v.trim().trim_end_matches('/').to_string() }
}

fn api_error(what: &str, v: &Value) -> CallErr {
    let code = v["code"].as_str().unwrap_or("");
    let msg = v["message"].as_str().unwrap_or("unknown error");
    let auth = matches!(code, "api_key_not_found" | "invalid_api_key");
    let err = match code {
        "enter_2fa" | "wrong_2fa" => AppError::connection(
            format!("{what}: {msg} ({code})"),
            "This account uses two-factor authentication: store the credentials as JSON with an \"api_key\" (from filen-cli) so unattended runs need no code.",
        ),
        "email_or_password_wrong" => AppError::connection(format!("{what}: {msg}"), "Check the Filen e-mail address and password."),
        _ if auth => AppError::connection(format!("{what}: {msg} ({code})"), "The Filen API key is no longer valid; sign in again."),
        "file_not_found" | "folder_not_found" => AppError::not_found(format!("{what}: {msg} ({code})")),
        _ => AppError::connection(format!("{what} failed: {msg} ({code})"), "Check the destination settings."),
    };
    CallErr { err, auth }
}

impl FilenStore {
    /// Sign in (or validate the stored API key) and load the master keys.
    pub async fn connect(cfg: FilenConfig, secret: String) -> Result<Self, AppError> {
        if cfg.email.trim().is_empty() {
            return Err(AppError::validation("Enter the Filen account e-mail address"));
        }
        let secret = FilenSecret::parse(&secret)?;
        let http = http::client(&cfg.tls)?;
        let s = match Self::sign_in(&http, &cfg, &secret, !secret.api_key.is_empty()).await {
            // A stored API key that no longer works falls back to a password login.
            Err(_) if !secret.api_key.is_empty() => Self::sign_in(&http, &cfg, &secret, false).await?,
            r => r?,
        };
        Ok(Self { cfg, secret, http, session: Mutex::new(s), folder_uuid: Mutex::new(None), cache: Mutex::new(None) })
    }

    /// POST/GET a gateway endpoint and unwrap the `{status, message, code, data}` envelope.
    /// JSON bodies carry the SDK's `Checksum` header (SHA-512 of the body).
    async fn raw(http: &reqwest::Client, gateway: &str, path: &str, body: Option<&Value>, api_key: Option<&str>, what: &str) -> Result<Value, CallErr> {
        let url = format!("{gateway}{path}");
        let mut rb = match body {
            Some(b) => {
                let bytes = serde_json::to_vec(b).map_err(|e| AppError::internal(e.to_string()))?;
                http.post(&url).header("content-type", "application/json").header("Checksum", crypto::sha512_hex(&bytes)).body(bytes)
            }
            None => http.get(&url),
        };
        if let Some(k) = api_key {
            rb = rb.bearer_auth(k);
        }
        let r = rb.header("accept", "application/json").send().await.map_err(|e| http::net(what, e))?;
        if r.status().as_u16() == 401 {
            return Err(CallErr { err: http::check(r, what).await.err().unwrap_or_else(|| AppError::connection(what.to_string(), "Sign in again.")), auth: true });
        }
        let v: Value = http::check(r, what).await?.json().await.map_err(|e| http::net(what, e))?;
        if v["status"].as_bool() != Some(true) {
            return Err(api_error(what, &v));
        }
        Ok(v["data"].clone())
    }

    async fn sign_in(http: &reqwest::Client, cfg: &FilenConfig, secret: &FilenSecret, use_api_key: bool) -> Result<Session, AppError> {
        let gw = base(&cfg.gateway_url, GATEWAY);
        let email = cfg.email.trim();
        let info = Self::raw(http, &gw, "/v3/auth/info", Some(&json!({ "email": email })), None, "Filen sign-in").await.map_err(|e| e.err)?;
        let version = info["authVersion"].as_u64().unwrap_or(0);
        if version != 2 {
            return Err(AppError::validation(format!(
                "This Filen account uses authentication version {version}; only version 2 is supported (version 3's file-name hashing cannot be verified against Filen's SDKs yet). Use the rclone bridge with rclone's filen backend for this account."
            )));
        }
        let (mk, derived_pw) = crypto::derive_v2(&secret.password, info["salt"].as_str().unwrap_or_default());
        let api_key = if use_api_key {
            secret.api_key.clone()
        } else {
            let tfa = if secret.two_factor_code.is_empty() { "XXXXXX" } else { secret.two_factor_code.as_str() };
            let body = json!({ "email": email, "password": derived_pw, "twoFactorCode": tfa, "authVersion": 2 });
            let v = Self::raw(http, &gw, "/v3/login", Some(&body), None, "Filen sign-in").await.map_err(|e| e.err)?;
            v["apiKey"].as_str().filter(|k| !k.is_empty()).ok_or_else(|| AppError::connection("Filen sign-in returned no API key", "Retry."))?.to_string()
        };
        // `_updateKeys` (auth v1/v2): send the known keys, encrypted with the
        // newest one; the reply holds every master key of the account.
        let enc = crypto::encrypt_meta(&mk, &mk);
        let v = Self::raw(http, &gw, "/v3/user/masterKeys", Some(&json!({ "masterKeys": enc })), Some(&api_key), "Filen master keys").await.map_err(|e| e.err)?;
        let mut keys = vec![mk.clone()];
        if let Some(all) = v["keys"].as_str().and_then(|k| crypto::decrypt_meta(k, &mk).ok()) {
            for k in all.split('|').filter(|k| !k.is_empty()) {
                if !keys.iter().any(|x| x == k) {
                    keys.push(k.to_string());
                }
            }
        } else {
            return Err(AppError::connection("The Filen master keys could not be decrypted", "Check the password: it must be the account's current password."));
        }
        let b = Self::raw(http, &gw, "/v3/user/baseFolder", None, Some(&api_key), "Filen root folder").await.map_err(|e| e.err)?;
        let base_folder = b["uuid"].as_str().ok_or_else(|| AppError::connection("Filen returned no root folder", "Retry."))?.to_string();
        Ok(Session { api_key, master_keys: keys, base_folder })
    }

    /// A gateway call with the session's API key; signs in again once when the key was rejected.
    async fn call(&self, path: &str, body: Option<Value>, what: &str) -> Result<Value, AppError> {
        let gw = base(&self.cfg.gateway_url, GATEWAY);
        let key = self.session.lock().await.api_key.clone();
        match Self::raw(&self.http, &gw, path, body.as_ref(), Some(&key), what).await {
            Ok(v) => Ok(v),
            Err(e) if e.auth => {
                // A rejected stored API key falls back to a password login.
                let s = Self::sign_in(&self.http, &self.cfg, &self.secret, false).await?;
                let key = s.api_key.clone();
                *self.session.lock().await = s;
                Self::raw(&self.http, &gw, path, body.as_ref(), Some(&key), what).await.map_err(|e| e.err)
            }
            Err(e) => Err(e.err),
        }
    }

    async fn session(&self) -> Session {
        self.session.lock().await.clone()
    }

    fn segments(&self) -> Vec<&str> {
        self.cfg.folder.split('/').filter(|s| !s.is_empty()).collect()
    }

    /// Sub-folders of `parent` by decrypted name.
    async fn subfolders(&self, parent: &str) -> Result<Vec<(String, String)>, AppError> {
        let v = self.call("/v3/dir/content", Some(json!({ "uuid": parent, "foldersOnly": true })), "Filen folder listing").await?;
        let s = self.session().await;
        Ok(v["folders"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| {
                let name: Value = serde_json::from_str(&s.decrypt(f["name"].as_str()?)?).ok()?;
                Some((name["name"].as_str()?.to_string(), f["uuid"].as_str()?.to_string()))
            })
            .collect())
    }

    /// Resolve (and with `create`, make) the configured folder.
    async fn folder(&self, create: bool) -> Result<String, AppError> {
        if let Some(u) = self.folder_uuid.lock().await.clone() {
            return Ok(u);
        }
        let mut cur = self.session().await.base_folder;
        for seg in self.segments() {
            let found = self.subfolders(&cur).await?.into_iter().find(|(n, _)| n == seg).map(|(_, u)| u);
            cur = match found {
                Some(u) => u,
                None if create => {
                    let s = self.session().await;
                    let uuid = uuid::Uuid::new_v4().to_string();
                    let body = json!({ "uuid": uuid, "name": s.encrypt(&json!({ "name": seg }).to_string()), "nameHashed": crypto::hash_name_v2(seg), "parent": cur });
                    let v = self.call("/v3/dir/create", Some(body), "Filen folder creation").await?;
                    v["uuid"].as_str().unwrap_or(&uuid).to_string()
                }
                None => return Err(AppError::not_found(format!("Filen folder '{}' does not exist", self.cfg.folder))),
            };
        }
        *self.folder_uuid.lock().await = Some(cur.clone());
        Ok(cur)
    }

    /// Files of the target folder, by name (a name can occur more than once).
    async fn entries(&self) -> Result<HashMap<String, Vec<Entry>>, AppError> {
        let folder = self.folder(false).await?;
        let v = self.call("/v3/dir/content", Some(json!({ "uuid": folder })), "Filen listing").await?;
        let s = self.session().await;
        let mut out: HashMap<String, Vec<Entry>> = HashMap::new();
        for f in v["uploads"].as_array().into_iter().flatten() {
            // Entries whose metadata no key opens are not ours; skip them.
            let Some(meta) = f["metadata"].as_str().and_then(|m| s.decrypt(m)).and_then(|m| serde_json::from_str::<Value>(&m).ok()) else { continue };
            let Some(name) = meta["name"].as_str() else { continue };
            out.entry(name.to_string()).or_default().push(Entry {
                uuid: f["uuid"].as_str().unwrap_or_default().to_string(),
                size: meta["size"].as_u64().or_else(|| f["size"].as_u64()).unwrap_or(0),
                key: meta["key"].as_str().unwrap_or_default().to_string(),
                region: f["region"].as_str().unwrap_or_default().to_string(),
                bucket: f["bucket"].as_str().unwrap_or_default().to_string(),
                chunks: f["chunks"].as_u64().unwrap_or(0),
                version: f["version"].as_u64().unwrap_or(2),
            });
        }
        *self.cache.lock().await = Some(out.clone());
        Ok(out)
    }

    async fn lookup(&self, name: &str) -> Result<Vec<Entry>, AppError> {
        if let Some(c) = self.cache.lock().await.as_ref() {
            if let Some(e) = c.get(name) {
                return Ok(e.clone());
            }
        }
        Ok(self.entries().await?.remove(name).unwrap_or_default())
    }

    async fn remove(&self, uuid: &str) -> Result<(), AppError> {
        let path = if self.cfg.use_trash { "/v3/file/trash" } else { "/v3/file/delete/permanent" };
        match self.call(path, Some(json!({ "uuid": uuid })), "Filen delete").await {
            Err(e) if e.user_message().contains("file_not_found") => Ok(()),
            r => r.map(|_| ()),
        }
    }

    async fn upload_chunk(&self, uuid: &str, index: usize, parent: &str, upload_key: &str, enc: Vec<u8>) -> Result<(), AppError> {
        let hash = crypto::sha512_hex(&enc);
        let idx = index.to_string();
        // `uploadChunkBuffer`: Checksum = SHA-512 of the JSON of the query parameters.
        let params = json!({ "uuid": uuid, "index": idx, "parent": parent, "uploadKey": upload_key, "hash": hash });
        let checksum = crypto::sha512_hex(format!(r#"{{"uuid":{},"index":{},"parent":{},"uploadKey":{},"hash":{}}}"#, params["uuid"], params["index"], params["parent"], params["uploadKey"], params["hash"]).as_bytes());
        let url = format!("{}/v3/upload", base(&self.cfg.ingest_url, INGEST));
        for attempt in 0..2 {
            let key = self.session().await.api_key;
            let r = self
                .http
                .post(&url)
                .query(&[("uuid", uuid), ("index", idx.as_str()), ("parent", parent), ("uploadKey", upload_key), ("hash", hash.as_str())])
                .bearer_auth(&key)
                .header("Checksum", &checksum)
                .body(enc.clone())
                .send()
                .await
                .map_err(|e| http::net("Filen chunk upload", e))?;
            if r.status().as_u16() == 401 && attempt == 0 {
                *self.session.lock().await = Self::sign_in(&self.http, &self.cfg, &self.secret, false).await?;
                continue;
            }
            let v: Value = http::check(r, "Filen chunk upload").await?.json().await.map_err(|e| http::net("Filen chunk upload", e))?;
            if v["status"].as_bool() != Some(true) {
                let e = api_error("Filen chunk upload", &v);
                if e.auth && attempt == 0 {
                    *self.session.lock().await = Self::sign_in(&self.http, &self.cfg, &self.secret, false).await?;
                    continue;
                }
                return Err(e.err);
            }
            return Ok(());
        }
        unreachable!()
    }
}

impl ObjectStore for FilenStore {
    fn describe(&self) -> String {
        format!("filen://{}/{}", self.cfg.email.trim(), self.cfg.folder.trim_matches('/'))
    }

    fn list(&self) -> BoxFut<'_, Vec<RemoteFile>> {
        Box::pin(async move {
            // The gateway returns a folder's whole content in one response (no paging).
            Ok(self.entries().await?.into_iter().filter_map(|(name, es)| es.last().map(|e| RemoteFile { name, size: e.size })).collect())
        })
    }

    fn put<'a>(&'a self, name: &'a str, data: Vec<u8>) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let parent = self.folder(false).await?;
            let previous = self.lookup(name).await?;
            let file_key = crypto::random_string(32);
            let uuid = uuid::Uuid::new_v4().to_string();
            let upload_key = crypto::random_string(32);
            let size = data.len() as u64;
            let mut chunks = 0usize;
            for (i, c) in data.chunks(CHUNK).enumerate() {
                let enc = crypto::encrypt_data(c, &file_key).map_err(AppError::internal)?;
                self.upload_chunk(&uuid, i, &parent, &upload_key, enc).await?;
                chunks += 1;
            }
            let s = self.session().await;
            let now = chrono::Utc::now().timestamp_millis();
            let meta = json!({ "name": name, "size": size, "mime": MIME, "key": file_key, "lastModified": now, "creation": now, "hash": crypto::sha512_hex(&data) });
            let mut body = json!({
                "uuid": uuid,
                "name": crypto::encrypt_meta(name, &file_key),
                "nameHashed": crypto::hash_name_v2(name),
                "size": crypto::encrypt_meta(&size.to_string(), &file_key),
                "mime": crypto::encrypt_meta(MIME, &file_key),
                "metadata": s.encrypt(&meta.to_string()),
                "version": 2,
            });
            let v = if chunks == 0 {
                body["parent"] = parent.clone().into();
                self.call("/v3/upload/empty", Some(body), "Filen upload").await?
            } else {
                body["chunks"] = chunks.into();
                body["rm"] = crypto::random_string(32).into();
                body["uploadKey"] = upload_key.into();
                self.call("/v3/upload/done", Some(body), "Filen upload").await?
            };
            if v["size"].as_u64().is_some_and(|n| n != size) {
                return Err(AppError::connection(format!("Filen stored {} bytes of {size} for {name}", v["size"]), "Retry the upload."));
            }
            // Filen keeps an overwritten file as an old version; remove any
            // same-named file that is still listed separately.
            for e in previous.iter().filter(|e| e.uuid != uuid) {
                self.remove(&e.uuid).await?;
            }
            *self.cache.lock().await = None;
            Ok(())
        })
    }

    fn get<'a>(&'a self, name: &'a str) -> BoxFut<'a, Vec<u8>> {
        Box::pin(async move {
            let mut es = self.lookup(name).await?;
            let Some(e) = es.pop() else { return Err(AppError::not_found(format!("Filen: {name} not found"))) };
            let egest = base(&self.cfg.egest_url, EGEST);
            let mut out = Vec::with_capacity(e.size as usize);
            for i in 0..e.chunks {
                let key = self.session().await.api_key;
                let r = self.http.get(format!("{egest}/{}/{}/{}/{i}", e.region, e.bucket, e.uuid)).bearer_auth(&key).send().await.map_err(|er| http::net("Filen download", er))?;
                let enc = http::check(r, "Filen download").await?.bytes().await.map_err(|er| http::net("Filen download", er))?;
                let plain = crypto::decrypt_data(e.version, &enc, &e.key).map_err(|m| AppError::connection(format!("Filen chunk {i} of {name}: {m}"), "The volume is damaged or was replaced; run a repair."))?;
                out.extend_from_slice(&plain);
            }
            if out.len() as u64 != e.size {
                return Err(AppError::connection(format!("Filen: {name} decrypted to {} bytes, expected {}", out.len(), e.size), "The volume is incomplete; run a repair."));
            }
            Ok(out)
        })
    }

    fn delete<'a>(&'a self, name: &'a str) -> BoxFut<'a, ()> {
        Box::pin(async move {
            let es = match self.lookup(name).await {
                Err(e) if http::is_not_found(&e) || e.user_message().contains("does not exist") => return Ok(()),
                r => r?,
            };
            for e in es {
                self.remove(&e.uuid).await?;
            }
            if let Some(c) = self.cache.lock().await.as_mut() {
                c.remove(name);
            }
            Ok(())
        })
    }

    fn create_folder(&self) -> BoxFut<'_, ()> {
        Box::pin(async move { self.folder(true).await.map(|_| ()) })
    }

    fn quota(&self) -> BoxFut<'_, Option<Quota>> {
        Box::pin(async move {
            let v = self.call("/v3/user/info", None, "Filen account info").await?;
            let (Some(total), Some(used)) = (v["maxStorage"].as_u64(), v["storageUsed"].as_u64()) else { return Ok(None) };
            Ok(Some(Quota { total, free: total.saturating_sub(used) }))
        })
    }
}
