//! Persistent SSH host-key trust store.
//!
//! Keys the user accepts (trust on first use) are written to a JSON file in
//! the app's data directory, so a server's identity survives restarts and a
//! changed key after a restart is reported as a possible MITM instead of
//! being silently re-trusted. The OpenSSH `known_hosts` file is consulted
//! read-only, so hosts already trusted by `ssh` are trusted here too; plain,
//! `[host]:port`, comma-separated, hashed (`|1|salt|hash`) and `@revoked`
//! entries are understood. The system file is never modified.

use base64::Engine as _;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

/// One trusted key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct KnownHostEntry {
    pub hostname: String,
    pub port: u16,
    pub key_type: String,
    pub fingerprint: String,
    pub added_at: DateTime<Utc>,
}

/// Outcome of checking a presented host key.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum HostKeyCheckResult {
    /// Host key matches a trusted key.
    Trusted,
    /// No key of this type is known for the host (first connection).
    NewHost { fingerprint: String, key_type: String },
    /// A different key of this type is trusted for the host, or the key is
    /// revoked (potential MITM).
    Changed {
        expected_fingerprint: String,
        actual_fingerprint: String,
        key_type: String,
    },
}

/// SHA-256 fingerprint of a raw public-key blob, in the `SHA256:<base64>`
/// form the connector uses everywhere.
pub fn fingerprint_of(key_blob: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(key_blob);
    format!("SHA256:{}", base64::engine::general_purpose::STANDARD.encode(hash))
}

/// Trusted host keys: app store (read-write) plus system files (read-only).
pub struct KnownHostsStore {
    /// App-managed store; `None` keeps accepted keys in memory only.
    path: Option<PathBuf>,
    /// OpenSSH-format files consulted read-only.
    system_files: Vec<PathBuf>,
    entries: RwLock<HashMap<String, KnownHostEntry>>,
}

fn store_key(host: &str, port: u16, key_type: &str) -> String {
    format!("{}:{port}:{key_type}", host.to_lowercase())
}

impl KnownHostsStore {
    /// Memory-only store that ignores system files (tests, AcceptAll callers).
    pub fn in_memory() -> Self {
        Self { path: None, system_files: Vec::new(), entries: RwLock::new(HashMap::new()) }
    }

    /// Store persisted at `path`, also trusting the given OpenSSH files.
    pub fn open(path: PathBuf, system_files: Vec<PathBuf>) -> Self {
        let entries = load_store(&path);
        Self { path: Some(path), system_files, entries: RwLock::new(entries) }
    }

    /// The process-wide store every SSH connector shares, so a key
    /// accepted through one connector is trusted by all and concurrent
    /// saves can't drop each other's entries.
    pub fn shared() -> Arc<Self> {
        static SHARED: OnceLock<Arc<KnownHostsStore>> = OnceLock::new();
        SHARED.get_or_init(|| Arc::new(Self::default_store())).clone()
    }

    /// Store for a new connector: the shared one, or an isolated in-memory
    /// store in unit tests.
    pub fn for_connector() -> Arc<Self> {
        if cfg!(test) {
            Arc::new(Self::in_memory())
        } else {
            Self::shared()
        }
    }

    /// `<data dir>/known_hosts.json` plus `~/.ssh/known_hosts` (in-memory
    /// under unit tests).
    fn default_store() -> Self {
        if cfg!(test) {
            return Self::in_memory();
        }
        let path = directories::ProjectDirs::from("com", "ufop", "unified-file-ops")
            .map(|d| d.data_dir().join("known_hosts.json"));
        let system = directories::UserDirs::new()
            .map(|d| vec![d.home_dir().join(".ssh").join("known_hosts")])
            .unwrap_or_default();
        match path {
            Some(p) => Self::open(p, system),
            None => Self { path: None, system_files: system, entries: RwLock::new(HashMap::new()) },
        }
    }

    /// Check a presented key. The app store wins; otherwise the system files
    /// decide; a revoked key is always reported as changed.
    pub async fn check(&self, host: &str, port: u16, key_type: &str, fingerprint: &str) -> HostKeyCheckResult {
        let system = self.system_keys(host, port);
        if system.iter().any(|k| k.revoked && k.fingerprint == fingerprint) {
            return HostKeyCheckResult::Changed {
                expected_fingerprint: "(revoked in known_hosts)".to_string(),
                actual_fingerprint: fingerprint.to_string(),
                key_type: key_type.to_string(),
            };
        }

        if let Some(entry) = self.entries.read().await.get(&store_key(host, port, key_type)) {
            return if entry.fingerprint == fingerprint {
                HostKeyCheckResult::Trusted
            } else {
                HostKeyCheckResult::Changed {
                    expected_fingerprint: entry.fingerprint.clone(),
                    actual_fingerprint: fingerprint.to_string(),
                    key_type: key_type.to_string(),
                }
            };
        }

        let same_type: Vec<&SystemKey> = system.iter().filter(|k| !k.revoked && k.key_type == key_type).collect();
        if same_type.iter().any(|k| k.fingerprint == fingerprint) {
            HostKeyCheckResult::Trusted
        } else if let Some(k) = same_type.first() {
            HostKeyCheckResult::Changed {
                expected_fingerprint: k.fingerprint.clone(),
                actual_fingerprint: fingerprint.to_string(),
                key_type: key_type.to_string(),
            }
        } else {
            HostKeyCheckResult::NewHost { fingerprint: fingerprint.to_string(), key_type: key_type.to_string() }
        }
    }

    /// Trust a key (replacing any previous key of the same type) and persist.
    pub async fn accept(&self, host: &str, port: u16, key_type: &str, fingerprint: &str) -> std::io::Result<()> {
        let entry = KnownHostEntry {
            hostname: host.to_lowercase(),
            port,
            key_type: key_type.to_string(),
            fingerprint: fingerprint.to_string(),
            added_at: Utc::now(),
        };
        let mut entries = self.entries.write().await;
        entries.insert(store_key(host, port, key_type), entry);
        match &self.path {
            Some(p) => save_store(p, &entries),
            None => Ok(()),
        }
    }

    /// Forget every app-trusted key for a host (e.g. after a legitimate
    /// server rebuild). System files are untouched.
    pub async fn forget(&self, host: &str, port: u16) -> std::io::Result<usize> {
        let prefix = format!("{}:{port}:", host.to_lowercase());
        let mut entries = self.entries.write().await;
        let before = entries.len();
        entries.retain(|k, _| !k.starts_with(&prefix));
        let removed = before - entries.len();
        if let Some(p) = &self.path {
            save_store(p, &entries)?;
        }
        Ok(removed)
    }

    /// All app-trusted keys, sorted by host.
    pub async fn list(&self) -> Vec<KnownHostEntry> {
        let mut v: Vec<_> = self.entries.read().await.values().cloned().collect();
        v.sort_by(|a, b| (&a.hostname, a.port, &a.key_type).cmp(&(&b.hostname, b.port, &b.key_type)));
        v
    }

    fn system_keys(&self, host: &str, port: u16) -> Vec<SystemKey> {
        self.system_files
            .iter()
            .filter_map(|f| std::fs::read_to_string(f).ok())
            .flat_map(|content| parse_known_hosts(&content, host, port))
            .collect()
    }
}

fn load_store(path: &Path) -> HashMap<String, KnownHostEntry> {
    let Ok(data) = std::fs::read(path) else { return HashMap::new() };
    match serde_json::from_slice::<Vec<KnownHostEntry>>(&data) {
        Ok(list) => list
            .into_iter()
            .map(|e| (store_key(&e.hostname, e.port, &e.key_type), e))
            .collect(),
        Err(e) => {
            // Never start trusting nothing silently on a corrupt file: keep
            // it for inspection and start empty (new hosts then prompt/TOFU).
            tracing::error!("Unreadable host-key store {}: {e}", path.display());
            let _ = std::fs::rename(path, path.with_extension("json.corrupt"));
            HashMap::new()
        }
    }
}

/// Atomic write: temp file + rename, owner-only permissions on Unix.
fn save_store(path: &Path, entries: &HashMap<String, KnownHostEntry>) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut list: Vec<&KnownHostEntry> = entries.values().collect();
    list.sort_by(|a, b| (&a.hostname, a.port, &a.key_type).cmp(&(&b.hostname, b.port, &b.key_type)));
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&list).map_err(std::io::Error::other)?)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(tmp, path)
}

/// A key from an OpenSSH known_hosts file that applies to the queried host.
#[derive(Debug, Clone, PartialEq)]
struct SystemKey {
    key_type: String,
    fingerprint: String,
    revoked: bool,
}

/// Keys in an OpenSSH known_hosts file that match `host`:`port`.
fn parse_known_hosts(content: &str, host: &str, port: u16) -> Vec<SystemKey> {
    let target = if port == 22 { host.to_lowercase() } else { format!("[{}]:{port}", host.to_lowercase()) };
    let mut keys = Vec::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let mut first = fields.next().unwrap_or("");
        let mut revoked = false;
        if first.starts_with('@') {
            match first {
                "@revoked" => revoked = true,
                _ => continue, // @cert-authority: CA keys, not host keys
            }
            first = fields.next().unwrap_or("");
        }
        let (Some(key_type), Some(blob_b64)) = (fields.next(), fields.next()) else { continue };
        if !hosts_field_matches(first, &target) {
            continue;
        }
        let Ok(blob) = base64::engine::general_purpose::STANDARD.decode(blob_b64) else { continue };
        keys.push(SystemKey { key_type: key_type.to_string(), fingerprint: fingerprint_of(&blob), revoked });
    }
    keys
}

/// Whether a known_hosts host field (comma list of names/patterns, or a
/// hashed `|1|salt|hash` entry) matches `target` (`host` or `[host]:port`).
/// A matching negated pattern (`!pattern`) excludes the line.
fn hosts_field_matches(field: &str, target: &str) -> bool {
    if let Some(rest) = field.strip_prefix("|1|") {
        return hashed_host_matches(rest, target);
    }
    let mut matched = false;
    for pattern in field.split(',') {
        let (negated, pat) = match pattern.strip_prefix('!') {
            Some(p) => (true, p),
            None => (false, pattern),
        };
        if wildcard_match(&pat.to_lowercase(), target) {
            if negated {
                return false;
            }
            matched = true;
        }
    }
    matched
}

fn hashed_host_matches(salt_and_hash: &str, target: &str) -> bool {
    use hmac::{Hmac, Mac};
    let mut parts = salt_and_hash.splitn(2, '|');
    let (Some(salt), Some(hash)) = (parts.next(), parts.next()) else { return false };
    let b64 = base64::engine::general_purpose::STANDARD;
    let (Ok(salt), Ok(hash)) = (b64.decode(salt), b64.decode(hash)) else { return false };
    let Ok(mut mac) = Hmac::<sha1::Sha1>::new_from_slice(&salt) else { return false };
    mac.update(target.as_bytes());
    mac.verify_slice(&hash).is_ok()
}

/// OpenSSH host patterns: `*` matches any run, `?` one character.
fn wildcard_match(pattern: &str, text: &str) -> bool {
    fn go(p: &[u8], t: &[u8]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some(b'*'), _) => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
            (Some(b'?'), Some(_)) => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) if a == b => go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    go(pattern.as_bytes(), text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real ed25519 public-key blob (from `ssh-keygen -t ed25519`).
    const KEY_A: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIOMqqnkVzrm0SdG6UOoqKLsabgH5C9okWi0dh2l9GKJl";
    const KEY_B: &str = "AAAAC3NzaC1lZDI1NTE5AAAAIB4Y0X5o3nJmJ8k2a7mVpJv3wq6Jt9cS0t8kQe1ZgP7q";

    fn fp(b64: &str) -> String {
        fingerprint_of(&base64::engine::general_purpose::STANDARD.decode(b64).unwrap())
    }

    /// Regression: accepted keys lived in memory only, so after a restart a
    /// changed key was silently re-trusted.
    #[tokio::test]
    async fn accepted_keys_survive_a_restart_and_detect_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("known_hosts.json");
        let store = KnownHostsStore::open(path.clone(), vec![]);
        assert!(matches!(store.check("Example.com", 22, "ssh-ed25519", "SHA256:a").await, HostKeyCheckResult::NewHost { .. }));
        store.accept("Example.com", 22, "ssh-ed25519", "SHA256:a").await.unwrap();

        let reopened = KnownHostsStore::open(path, vec![]);
        assert_eq!(reopened.check("example.com", 22, "ssh-ed25519", "SHA256:a").await, HostKeyCheckResult::Trusted);
        assert!(matches!(
            reopened.check("example.com", 22, "ssh-ed25519", "SHA256:evil").await,
            HostKeyCheckResult::Changed { .. }
        ));
        // Another key type is a new key, not a change.
        assert!(matches!(reopened.check("example.com", 22, "ssh-rsa", "SHA256:r").await, HostKeyCheckResult::NewHost { .. }));
        // Ports are distinct hosts.
        assert!(matches!(reopened.check("example.com", 2222, "ssh-ed25519", "SHA256:a").await, HostKeyCheckResult::NewHost { .. }));
    }

    #[tokio::test]
    async fn forget_removes_app_trust_only() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh.json");
        let store = KnownHostsStore::open(path.clone(), vec![]);
        store.accept("h", 22, "ssh-ed25519", "SHA256:a").await.unwrap();
        store.accept("h", 22, "ssh-rsa", "SHA256:b").await.unwrap();
        assert_eq!(store.forget("h", 22).await.unwrap(), 2);
        assert!(KnownHostsStore::open(path, vec![]).list().await.is_empty());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn store_file_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh.json");
        KnownHostsStore::open(path.clone(), vec![]).accept("h", 22, "t", "f").await.unwrap();
        assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
    }

    #[tokio::test]
    async fn corrupt_store_is_set_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kh.json");
        std::fs::write(&path, "not json").unwrap();
        let store = KnownHostsStore::open(path.clone(), vec![]);
        assert!(store.list().await.is_empty());
        assert!(path.with_extension("json.corrupt").exists());
    }

    #[tokio::test]
    async fn system_known_hosts_are_trusted_read_only() {
        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("known_hosts");
        std::fs::write(
            &system,
            format!(
                "# comment\nbuild.example.com,10.0.0.5 ssh-ed25519 {KEY_A}\n[git.example.com]:2222 ssh-ed25519 {KEY_A} user@host\n*.wild.example ssh-ed25519 {KEY_A}\n"
            ),
        )
        .unwrap();
        let before = std::fs::read(&system).unwrap();
        let store = KnownHostsStore::open(dir.path().join("app.json"), vec![system.clone()]);

        assert_eq!(store.check("10.0.0.5", 22, "ssh-ed25519", &fp(KEY_A)).await, HostKeyCheckResult::Trusted);
        assert_eq!(store.check("git.example.com", 2222, "ssh-ed25519", &fp(KEY_A)).await, HostKeyCheckResult::Trusted);
        assert_eq!(store.check("a.wild.example", 22, "ssh-ed25519", &fp(KEY_A)).await, HostKeyCheckResult::Trusted);
        assert!(matches!(store.check("git.example.com", 22, "ssh-ed25519", &fp(KEY_A)).await, HostKeyCheckResult::NewHost { .. }));
        assert!(matches!(
            store.check("build.example.com", 22, "ssh-ed25519", &fp(KEY_B)).await,
            HostKeyCheckResult::Changed { .. }
        ));
        store.accept("build.example.com", 22, "ssh-rsa", "SHA256:x").await.unwrap();
        assert_eq!(std::fs::read(&system).unwrap(), before, "system file must never be written");
    }

    #[tokio::test]
    async fn hashed_and_revoked_entries() {
        use hmac::{Hmac, Mac};
        let b64 = base64::engine::general_purpose::STANDARD;
        let salt = b"0123456789abcdefghij";
        let mut mac = Hmac::<sha1::Sha1>::new_from_slice(salt).unwrap();
        mac.update(b"secret.example.com");
        let hashed = format!("|1|{}|{}", b64.encode(salt), b64.encode(mac.finalize().into_bytes()));

        let dir = tempfile::tempdir().unwrap();
        let system = dir.path().join("known_hosts");
        std::fs::write(&system, format!("{hashed} ssh-ed25519 {KEY_A}\n@revoked * ssh-ed25519 {KEY_B}\n")).unwrap();
        let store = KnownHostsStore::open(dir.path().join("app.json"), vec![system]);

        assert_eq!(store.check("secret.example.com", 22, "ssh-ed25519", &fp(KEY_A)).await, HostKeyCheckResult::Trusted);
        assert!(matches!(store.check("other.example.com", 22, "ssh-ed25519", &fp(KEY_A)).await, HostKeyCheckResult::NewHost { .. }));
        // A revoked key is refused even if the user once accepted it.
        store.accept("any.example.com", 22, "ssh-ed25519", &fp(KEY_B)).await.unwrap();
        assert!(matches!(store.check("any.example.com", 22, "ssh-ed25519", &fp(KEY_B)).await, HostKeyCheckResult::Changed { .. }));
    }

    #[test]
    fn negated_patterns_exclude() {
        assert!(hosts_field_matches("*.example.com,!bad.example.com", "good.example.com"));
        assert!(!hosts_field_matches("*.example.com,!bad.example.com", "bad.example.com"));
        assert!(wildcard_match("web-??.lan", "web-01.lan"));
        assert!(!wildcard_match("web-??.lan", "web-1.lan"));
    }
}
