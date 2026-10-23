//! Peer discovery and computer-to-computer transfer (T-031).
//!
//! - Discovery: each running app advertises `_ufop._tcp` over mDNS
//!   (Bonjour/Avahi compatible) with its id, name, OS, version and
//!   certificate fingerprint, and browses for the others.
//! - Transport: TLS with both ends presenting their device certificate
//!   (see [`peer_wire`](super::peer_wire)); a saved device whose certificate
//!   changes is refused.
//! - Transfers: the sender asks, the receiver approves (trusted devices
//!   are accepted automatically, blocked ones refused), then files stream
//!   in 4 MiB frames, each file checked with SHA-256 and written under a
//!   hidden name until complete. Either side can cancel.
//! - Manual connection by host name or IP when discovery is unavailable.
//! - Saved devices (trust, names, pinned fingerprints) persist across runs.

use super::peer_wire::{self as wire, Frame, PeerIdentity};
use crate::core::error::AppError;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::Digest;
use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::AsyncWriteExt;
use tokio::sync::{oneshot, RwLock};
use uuid::Uuid;

// ── Constants ──

/// mDNS service type for UFOP peer discovery.
const MDNS_SERVICE_TYPE: &str = "_ufop._tcp.local.";
/// Default port for peer-to-peer transfers.
pub const DEFAULT_PEER_PORT: u16 = 43210;
/// How long a receiver has to approve a transfer.
const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);
/// Connect and handshake time limit.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Most entries one transfer may list.
const MAX_FILES: usize = 100_000;

// ── Types ──


/// Operating system type of a discovered peer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerOs {
    MacOs,
    Windows,
    Linux,
    Unknown,
}

impl PeerOs {
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(target_os = "windows") {
            Self::Windows
        } else if cfg!(target_os = "linux") {
            Self::Linux
        } else {
            Self::Unknown
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::MacOs => "macos",
            Self::Windows => "windows",
            Self::Linux => "linux",
            Self::Unknown => "unknown",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "macos" => Self::MacOs,
            "windows" => Self::Windows,
            "linux" => Self::Linux,
            _ => Self::Unknown,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            Self::MacOs => "macOS",
            Self::Windows => "Windows",
            Self::Linux => "Linux",
            Self::Unknown => "Unknown OS",
        }
    }
}

/// Online/offline status of a discovered peer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerStatus {
    Online,
    Offline,
    Connecting,
    Transferring,
}

impl PeerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Connecting => "connecting",
            Self::Transferring => "transferring",
        }
    }
}

/// Trust level for a peer device.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TrustLevel {
    /// Never seen before.
    Unknown,
    /// Seen once, user denied or hasn't decided.
    Untrusted,
    /// User explicitly trusted this device.
    Trusted,
    /// Blocked by user.
    Blocked,
}

impl TrustLevel {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Untrusted => "untrusted",
            Self::Trusted => "trusted",
            Self::Blocked => "blocked",
        }
    }

    pub fn from_str_lossy(s: &str) -> Self {
        match s {
            "unknown" => Self::Unknown,
            "untrusted" => Self::Untrusted,
            "trusted" => Self::Trusted,
            "blocked" => Self::Blocked,
            _ => Self::Unknown,
        }
    }
}

/// A discovered or saved peer device.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerDevice {
    /// Device id (persistent on that device).
    pub id: Uuid,
    pub hostname: String,
    pub os: PeerOs,
    /// Addresses it was seen at (IPv4 and/or IPv6).
    pub addresses: Vec<IpAddr>,
    /// Port its transfer listener uses.
    pub port: u16,
    pub app_version: String,
    pub status: PeerStatus,
    pub trust: TrustLevel,
    pub discovered_at: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    /// User-assigned display name.
    pub display_name: Option<String>,
    /// SHA-256 fingerprint of the device certificate.
    pub tls_fingerprint: Option<String>,
}

/// Transfer request sent from one peer to another.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerTransferRequest {
    pub request_id: Uuid,
    pub sender_id: Uuid,
    pub sender_hostname: String,
    pub sender_os: PeerOs,
    pub files: Vec<PeerFileInfo>,
    pub total_bytes: u64,
    pub file_count: u64,
    pub requested_at: DateTime<Utc>,
}

/// A file or folder in a peer transfer.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PeerFileInfo {
    /// Path below the transfer root, `/`-separated (`photos/2024/a.jpg`).
    pub relative_path: String,
    pub size: u64,
    pub is_dir: bool,
    pub modified: Option<DateTime<Utc>>,
}

/// Response to a transfer request.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TransferResponse {
    Accepted,
    Denied,
    /// Denied and the sender blocked.
    Blocked,
}

/// Which way a transfer goes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerTransferDirection {
    Outgoing,
    Incoming,
}

/// Progress of a peer-to-peer transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerTransferProgress {
    pub request_id: Uuid,
    pub peer_id: Uuid,
    pub peer_name: String,
    pub direction: PeerTransferDirection,
    pub transferred_bytes: u64,
    pub total_bytes: u64,
    pub current_file: String,
    pub files_completed: u64,
    pub files_total: u64,
    pub speed_bps: u64,
    pub eta_seconds: Option<u64>,
    pub status: PeerTransferStatus,
    /// Incoming: the folder files are saved in.
    pub save_path: Option<String>,
    /// Why it failed or was refused.
    pub error: Option<String>,
}

/// Status of a peer transfer.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PeerTransferStatus {
    /// Waiting for the receiver to approve.
    Pending,
    Connecting,
    Transferring,
    Verifying,
    Completed,
    Failed,
    Cancelled,
    Denied,
}

impl PeerTransferStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Connecting => "connecting",
            Self::Transferring => "transferring",
            Self::Verifying => "verifying",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Denied => "denied",
        }
    }

    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled | Self::Denied)
    }
}

/// Manual peer connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManualPeerConfig {
    /// IP address or host name.
    pub host: String,
    pub port: u16,
    pub display_name: Option<String>,
}

/// Messages of the peer protocol (JSON frames; file bytes travel as data frames).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PeerMessage {
    /// First message each way: who is talking.
    Hello { peer_id: Uuid, hostname: String, os: PeerOs, app_version: String, port: u16 },
    Ping { peer_id: Uuid, timestamp: i64 },
    Pong { peer_id: Uuid, timestamp: i64 },
    TransferRequest(PeerTransferRequest),
    TransferResponse { request_id: Uuid, response: TransferResponse },
    /// Entry `index` of the request follows (its bytes as data frames).
    FileStart { request_id: Uuid, index: u64 },
    /// Entry `index` is complete; its SHA-256.
    FileEnd { request_id: Uuid, index: u64, sha256: String },
    TransferComplete { request_id: Uuid, files_transferred: u64, bytes_transferred: u64, verification_passed: bool },
    Error { request_id: Option<Uuid>, message: String },
    /// End of the conversation.
    Bye { peer_id: Uuid },
}

/// Saved peer data persisted to disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedPeer {
    pub id: Uuid,
    pub hostname: String,
    pub os: PeerOs,
    pub last_addresses: Vec<IpAddr>,
    pub port: u16,
    pub trust: TrustLevel,
    pub display_name: Option<String>,
    pub tls_fingerprint: Option<String>,
    pub last_seen: DateTime<Utc>,
    pub saved_at: DateTime<Utc>,
}

/// A peer as seen by discovery or a handshake.
#[derive(Debug, Clone)]
pub struct PeerSighting {
    pub id: Uuid,
    pub hostname: String,
    pub os: PeerOs,
    pub addresses: Vec<IpAddr>,
    pub port: u16,
    pub app_version: String,
    pub fingerprint: Option<String>,
}

type Decision = (TransferResponse, Option<String>);
type Tls<S> = tokio_rustls::TlsStream<S>;

/// The default folder for received files.
fn default_downloads_dir() -> PathBuf {
    directories::UserDirs::new()
        .and_then(|d| d.download_dir().map(Path::to_path_buf).or_else(|| Some(d.home_dir().join("Downloads"))))
        .unwrap_or_else(|| std::env::temp_dir().join("ufop-received"))
}

/// The default folder for this device's peer identity and saved peers.
pub fn default_peer_dir() -> PathBuf {
    directories::ProjectDirs::from("com", "ufop", "unified-file-ops")
        .map(|d| d.data_dir().join("peer"))
        .unwrap_or_else(|| ufop_core::session::user_temp_subdir("peer"))
}

fn lock<T>(m: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Stopped by a cancel on either side (the status says which).
fn cancelled() -> AppError {
    AppError::transfer("The transfer was cancelled", "Send it again if needed.")
}

fn not_found(peer_id: Uuid) -> AppError {
    AppError::connection(format!("Peer {peer_id} not found"), "The peer may have gone offline.")
}

// ── Peer Manager ──

/// Discovery, connections and transfers. Cloning shares the state.
#[derive(Clone)]
pub struct PeerManager {
    identity: Arc<PeerIdentity>,
    local_hostname: String,
    local_os: PeerOs,
    app_version: String,
    /// Port to listen on (0: any free port).
    preferred_port: u16,
    /// Port actually listened on (0 while not listening).
    local_port: Arc<AtomicU16>,
    /// Where saved peers are kept (`None`: not persisted).
    data_dir: Option<PathBuf>,
    downloads_dir: PathBuf,

    peers: Arc<RwLock<HashMap<Uuid, PeerDevice>>>,
    saved_peers: Arc<RwLock<HashMap<Uuid, SavedPeer>>>,
    transfers: Arc<RwLock<HashMap<Uuid, PeerTransferProgress>>>,
    pending_requests: Arc<RwLock<HashMap<Uuid, PeerTransferRequest>>>,
    /// Pending requests' connections waiting for the user's answer.
    responders: Arc<std::sync::Mutex<HashMap<Uuid, oneshot::Sender<Decision>>>>,
    cancels: Arc<std::sync::Mutex<HashMap<Uuid, Arc<AtomicBool>>>>,

    listener: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>>,
    mdns: Arc<std::sync::Mutex<Option<(mdns_sd::ServiceDaemon, String)>>>,
    discovery_active: Arc<AtomicBool>,
}

impl Default for PeerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl PeerManager {
    /// A manager with a new identity that persists nothing.
    pub fn new() -> Self {
        let identity = PeerIdentity::generate().expect("generating a self-signed certificate");
        Self::with_identity(identity, None)
    }

    /// A manager whose identity and saved peers live in `dir`.
    pub fn open(dir: PathBuf) -> Self {
        let identity = PeerIdentity::load_or_create(&dir.join("identity.json")).unwrap_or_else(|e| {
            tracing::warn!("Peer identity could not be stored ({e}); using a temporary one");
            PeerIdentity::generate().expect("generating a self-signed certificate")
        });
        let mgr = Self::with_identity(identity, Some(dir));
        mgr.load_saved();
        mgr
    }

    fn with_identity(identity: PeerIdentity, data_dir: Option<PathBuf>) -> Self {
        let hostname = hostname::get().map(|h| h.to_string_lossy().to_string()).unwrap_or_else(|_| "unknown-host".to_string());
        Self {
            identity: Arc::new(identity),
            local_hostname: hostname,
            local_os: PeerOs::current(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            preferred_port: DEFAULT_PEER_PORT,
            local_port: Arc::new(AtomicU16::new(0)),
            data_dir,
            downloads_dir: default_downloads_dir(),
            peers: Default::default(),
            saved_peers: Default::default(),
            transfers: Default::default(),
            pending_requests: Default::default(),
            responders: Default::default(),
            cancels: Default::default(),
            listener: Default::default(),
            mdns: Default::default(),
            discovery_active: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Listen on `port` (0: any free port).
    pub fn with_port(mut self, port: u16) -> Self {
        self.preferred_port = port;
        self
    }

    /// Save accepted files in `dir` unless the user picks another folder.
    pub fn with_downloads_dir(mut self, dir: PathBuf) -> Self {
        self.downloads_dir = dir;
        self
    }

    /// Name this device announces.
    pub fn with_hostname(mut self, name: &str) -> Self {
        self.local_hostname = name.to_string();
        self
    }

    pub fn local_peer_id(&self) -> Uuid {
        self.identity.id
    }

    /// This device's certificate fingerprint, for comparing with the other computer.
    pub fn local_fingerprint(&self) -> String {
        self.identity.fingerprint()
    }

    /// The listening port (0 while not listening).
    pub fn listening_port(&self) -> u16 {
        self.local_port.load(Ordering::Relaxed)
    }

    // ── Saved peers ──

    fn saved_file(&self) -> Option<PathBuf> {
        self.data_dir.as_ref().map(|d| d.join("saved-peers.json"))
    }

    fn load_saved(&self) {
        let Some(path) = self.saved_file() else { return };
        let Ok(raw) = std::fs::read(&path) else { return };
        let list: Vec<SavedPeer> = match serde_json::from_slice(&raw) {
            Ok(l) => l,
            Err(e) => {
                tracing::warn!("Saved peers at {} are unreadable: {e}", path.display());
                return;
            }
        };
        // Saved devices are listed (offline until seen) so they can be sent to directly.
        let (Ok(mut saved), Ok(mut peers)) = (self.saved_peers.try_write(), self.peers.try_write()) else { return };
        for s in list {
            peers.insert(
                s.id,
                PeerDevice {
                    id: s.id,
                    hostname: s.hostname.clone(),
                    os: s.os,
                    addresses: s.last_addresses.clone(),
                    port: s.port,
                    app_version: String::new(),
                    status: PeerStatus::Offline,
                    trust: s.trust,
                    discovered_at: s.saved_at,
                    last_seen: s.last_seen,
                    display_name: s.display_name.clone(),
                    tls_fingerprint: s.tls_fingerprint.clone(),
                },
            );
            saved.insert(s.id, s);
        }
    }

    async fn persist_saved(&self) {
        let Some(path) = self.saved_file() else { return };
        let list: Vec<SavedPeer> = self.saved_peers.read().await.values().cloned().collect();
        let r = path.parent().map(std::fs::create_dir_all).transpose().map_err(AppError::from).and_then(|_| wire::write_private(&path, &serde_json::to_vec_pretty(&list).unwrap_or_default()));
        if let Err(e) = r {
            tracing::warn!("Could not save the peer list: {e}");
        }
    }

    fn saved_from(peer: &PeerDevice, trust: TrustLevel) -> SavedPeer {
        SavedPeer {
            id: peer.id,
            hostname: peer.hostname.clone(),
            os: peer.os,
            last_addresses: peer.addresses.clone(),
            port: peer.port,
            trust,
            display_name: peer.display_name.clone(),
            tls_fingerprint: peer.tls_fingerprint.clone(),
            last_seen: peer.last_seen,
            saved_at: Utc::now(),
        }
    }

    // ── Discovery ──

    /// Advertise this device over mDNS and browse for others. Starts the
    /// listener first (the advertisement names its port).
    pub async fn start_discovery(&self) -> Result<(), AppError> {
        if self.discovery_active.load(Ordering::Relaxed) {
            return Ok(());
        }
        let port = self.start_listener().await?;
        let mdns_err = |e: mdns_sd::Error| AppError::connection(format!("Network discovery is unavailable: {e}"), "Connect to the other computer by its address instead.");
        let daemon = mdns_sd::ServiceDaemon::new().map_err(mdns_err)?;
        let host: String = self.local_hostname.chars().map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '-' }).collect();
        let instance = self.identity.id.simple().to_string();
        let props = [
            ("id", self.identity.id.to_string()),
            ("name", self.local_hostname.clone()),
            ("os", self.local_os.as_str().to_string()),
            ("ver", self.app_version.clone()),
            ("fp", self.identity.fingerprint()),
        ];
        let info = mdns_sd::ServiceInfo::new(MDNS_SERVICE_TYPE, &instance, &format!("{host}.local."), "", port, &props[..]).map_err(mdns_err)?.enable_addr_auto();
        let fullname = info.get_fullname().to_string();
        daemon.register(info).map_err(mdns_err)?;
        let events = daemon.browse(MDNS_SERVICE_TYPE).map_err(mdns_err)?;
        *lock(&self.mdns) = Some((daemon, fullname));
        self.discovery_active.store(true, Ordering::Relaxed);

        let me = self.clone();
        tokio::spawn(async move {
            while let Ok(ev) = events.recv_async().await {
                match ev {
                    mdns_sd::ServiceEvent::ServiceResolved(info) => {
                        if let Some(s) = sighting_from_mdns(&info) {
                            if s.id != me.identity.id {
                                me.on_mdns_sighting(s).await;
                            }
                        }
                    }
                    mdns_sd::ServiceEvent::ServiceRemoved(_, fullname) => {
                        if let Some(id) = fullname.split('.').next().and_then(|i| Uuid::parse_str(i).ok()) {
                            me.on_peer_offline(id).await;
                        }
                    }
                    mdns_sd::ServiceEvent::SearchStopped(_) => break,
                    _ => {}
                }
            }
        });
        tracing::info!("Peer discovery started as {} on port {port}", self.local_hostname);
        Ok(())
    }

    async fn on_mdns_sighting(&self, s: PeerSighting) {
        // A saved device's pinned certificate is not replaced by an advertisement.
        if let Some(fp) = &s.fingerprint {
            if let Some(saved) = self.saved_peers.read().await.get(&s.id).and_then(|p| p.tls_fingerprint.clone()) {
                if &saved != fp {
                    tracing::warn!("Ignoring an mDNS advertisement for saved peer {} with a different certificate", s.id);
                    return;
                }
            }
        }
        self.on_peer_discovered(s).await;
    }

    /// Stop advertising and browsing (sends an mDNS goodbye).
    pub async fn stop_discovery(&self) -> Result<(), AppError> {
        self.discovery_active.store(false, Ordering::Relaxed);
        if let Some((daemon, fullname)) = lock(&self.mdns).take() {
            if let Ok(rx) = daemon.unregister(&fullname) {
                let _ = rx.recv_timeout(Duration::from_secs(1));
            }
            let _ = daemon.stop_browse(MDNS_SERVICE_TYPE);
            let _ = daemon.shutdown();
        }
        Ok(())
    }

    pub fn is_discovering(&self) -> bool {
        self.discovery_active.load(Ordering::Relaxed)
    }

    // ── Peer Management ──

    pub async fn list_peers(&self) -> Vec<PeerDevice> {
        self.peers.read().await.values().cloned().collect()
    }

    pub async fn list_online_peers(&self) -> Vec<PeerDevice> {
        self.peers.read().await.values().filter(|p| p.status == PeerStatus::Online || p.status == PeerStatus::Transferring).cloned().collect()
    }

    pub async fn get_peer(&self, peer_id: Uuid) -> Option<PeerDevice> {
        self.peers.read().await.get(&peer_id).cloned()
    }

    /// Record a peer seen by discovery or a handshake.
    pub async fn on_peer_discovered(&self, s: PeerSighting) -> PeerDevice {
        let now = Utc::now();
        let saved = self.saved_peers.read().await.get(&s.id).cloned();
        let mut peers = self.peers.write().await;
        let peer = peers.entry(s.id).or_insert_with(|| PeerDevice {
            id: s.id,
            hostname: s.hostname.clone(),
            os: s.os,
            addresses: Vec::new(),
            port: s.port,
            app_version: String::new(),
            status: PeerStatus::Online,
            trust: saved.as_ref().map(|p| p.trust).unwrap_or(TrustLevel::Unknown),
            discovered_at: now,
            last_seen: now,
            display_name: saved.as_ref().and_then(|p| p.display_name.clone()),
            tls_fingerprint: saved.as_ref().and_then(|p| p.tls_fingerprint.clone()),
        });
        peer.hostname = s.hostname;
        peer.os = s.os;
        if !s.addresses.is_empty() {
            peer.addresses = s.addresses;
        }
        peer.port = s.port;
        peer.app_version = s.app_version;
        if peer.status == PeerStatus::Offline {
            peer.status = PeerStatus::Online;
        }
        peer.last_seen = now;
        if peer.tls_fingerprint.is_none() {
            peer.tls_fingerprint = s.fingerprint;
        }
        let peer = peer.clone();
        drop(peers);
        if saved.is_some() {
            let mut saved = self.saved_peers.write().await;
            if let Some(sp) = saved.get_mut(&peer.id) {
                sp.last_addresses = peer.addresses.clone();
                sp.port = peer.port;
                sp.last_seen = now;
                sp.hostname = peer.hostname.clone();
                if sp.tls_fingerprint.is_none() {
                    sp.tls_fingerprint = peer.tls_fingerprint.clone();
                }
            }
            drop(saved);
            self.persist_saved().await;
        }
        peer
    }

    pub async fn on_peer_heartbeat(&self, peer_id: Uuid) {
        if let Some(peer) = self.peers.write().await.get_mut(&peer_id) {
            peer.last_seen = Utc::now();
            if peer.status == PeerStatus::Offline {
                peer.status = PeerStatus::Online;
            }
        }
    }

    pub async fn on_peer_offline(&self, peer_id: Uuid) {
        if let Some(peer) = self.peers.write().await.get_mut(&peer_id) {
            peer.status = PeerStatus::Offline;
        }
    }

    async fn set_status(&self, peer_id: Uuid, status: PeerStatus) {
        if let Some(peer) = self.peers.write().await.get_mut(&peer_id) {
            peer.status = status;
        }
    }

    /// Refuse a device whose certificate differs from the one on record.
    async fn check_identity(&self, peer_id: Uuid, fingerprint: &str) -> Result<(), AppError> {
        let known = match self.saved_peers.read().await.get(&peer_id).and_then(|p| p.tls_fingerprint.clone()) {
            Some(f) => Some(f),
            None => self.peers.read().await.get(&peer_id).and_then(|p| p.tls_fingerprint.clone()),
        };
        match known {
            Some(f) if f != fingerprint => Err(AppError::connection(
                "This computer's identity has changed since it was last seen, so the connection was refused",
                "If it was reinstalled, remove it from saved devices and connect again; otherwise someone may be impersonating it.",
            )),
            _ => Ok(()),
        }
    }

    // ── Trust Management ──

    /// Set the trust level (saving the peer, with its certificate pinned).
    pub async fn set_trust(&self, peer_id: Uuid, trust: TrustLevel) -> Result<(), AppError> {
        let peer = {
            let mut peers = self.peers.write().await;
            peers.get_mut(&peer_id).map(|p| {
                p.trust = trust;
                p.clone()
            })
        };
        {
            let mut saved = self.saved_peers.write().await;
            match (saved.get_mut(&peer_id), peer) {
                (Some(sp), _) => sp.trust = trust,
                (None, Some(p)) => {
                    saved.insert(peer_id, Self::saved_from(&p, trust));
                }
                (None, None) => return Err(not_found(peer_id)),
            }
        }
        self.persist_saved().await;
        Ok(())
    }

    pub async fn save_peer(&self, peer_id: Uuid) -> Result<SavedPeer, AppError> {
        let peer = self.peers.read().await.get(&peer_id).cloned().ok_or_else(|| not_found(peer_id))?;
        let saved = Self::saved_from(&peer, peer.trust);
        self.saved_peers.write().await.insert(peer_id, saved.clone());
        self.persist_saved().await;
        Ok(saved)
    }

    /// Forget a saved peer (and its pinned certificate).
    pub async fn remove_saved_peer(&self, peer_id: Uuid) -> Result<(), AppError> {
        self.saved_peers.write().await.remove(&peer_id);
        let mut peers = self.peers.write().await;
        if peers.get(&peer_id).is_some_and(|p| p.status == PeerStatus::Offline) {
            peers.remove(&peer_id);
        } else if let Some(p) = peers.get_mut(&peer_id) {
            p.trust = TrustLevel::Unknown;
            p.tls_fingerprint = None;
        }
        drop(peers);
        self.persist_saved().await;
        Ok(())
    }

    pub async fn list_saved_peers(&self) -> Vec<SavedPeer> {
        self.saved_peers.read().await.values().cloned().collect()
    }

    pub async fn set_peer_display_name(&self, peer_id: Uuid, name: String) -> Result<(), AppError> {
        let name = Some(name.trim().to_string()).filter(|n| !n.is_empty());
        if let Some(peer) = self.peers.write().await.get_mut(&peer_id) {
            peer.display_name = name.clone();
        }
        if let Some(sp) = self.saved_peers.write().await.get_mut(&peer_id) {
            sp.display_name = name;
        }
        self.persist_saved().await;
        Ok(())
    }

    // ── Connections ──

    fn hello(&self) -> PeerMessage {
        PeerMessage::Hello {
            peer_id: self.identity.id,
            hostname: self.local_hostname.clone(),
            os: self.local_os,
            app_version: self.app_version.clone(),
            port: self.listening_port(),
        }
    }

    /// Open a TLS connection to one of `addrs` and exchange Hellos. With
    /// `expected` the other end's certificate must match it.
    async fn dial(&self, addrs: &[SocketAddr], expected: Option<&str>) -> Result<(Tls<tokio::net::TcpStream>, PeerDevice), AppError> {
        let connector = tokio_rustls::TlsConnector::from(self.identity.client_config(expected)?);
        let mut last = AppError::connection("The computer has no known address", "Connect to it by address.");
        for addr in addrs {
            let attempt = async {
                let tcp = tokio::net::TcpStream::connect(addr).await.map_err(wire::io_err)?;
                let _ = tcp.set_nodelay(true);
                let tls = connector.connect(wire::server_name(), tcp).await.map_err(|e| {
                    AppError::connection(format!("Secure connection to {addr} failed: {e}"), "If the computer was reinstalled, remove it from saved devices and connect again.")
                })?;
                let fp = wire::peer_fingerprint(tls.get_ref().1.peer_certificates()).ok_or_else(|| wire::protocol("no certificate"))?;
                let mut tls = Tls::from(tls);
                wire::write_msg(&mut tls, &self.hello()).await?;
                let (peer_id, hostname, os, app_version, port) = match wire::read_msg(&mut tls).await? {
                    PeerMessage::Hello { peer_id, hostname, os, app_version, port } => (peer_id, hostname, os, app_version, port),
                    PeerMessage::Error { message, .. } => return Err(AppError::connection(message, "Ask the other computer's user to allow it.")),
                    _ => return Err(wire::protocol("expected a hello")),
                };
                self.check_identity(peer_id, &fp).await?;
                let peer = self.on_peer_discovered(PeerSighting { id: peer_id, hostname, os, addresses: vec![addr.ip()], port, app_version, fingerprint: Some(fp) }).await;
                Ok::<_, AppError>((tls, peer))
            };
            match tokio::time::timeout(CONNECT_TIMEOUT, attempt).await {
                Ok(Ok(r)) => return Ok(r),
                Ok(Err(e)) => last = e,
                Err(_) => last = AppError::connection(format!("{addr} did not answer in time"), "Check the other computer is running UFOP and on the same network."),
            }
        }
        Err(last)
    }

    /// Socket addresses to try for `peer`, LAN IPv4 first.
    fn addrs_of(peer: &PeerDevice) -> Vec<SocketAddr> {
        let mut ips = peer.addresses.clone();
        ips.sort_by_key(|ip| (ip.is_loopback(), ip.is_ipv6()));
        ips.into_iter().map(|ip| SocketAddr::new(ip, peer.port)).collect()
    }

    /// Connect to a computer by address when discovery does not find it;
    /// its identity is learned from the handshake.
    pub async fn connect_manual(&self, config: ManualPeerConfig) -> Result<PeerDevice, AppError> {
        let host = config.host.trim().trim_start_matches('[').trim_end_matches(']').to_string();
        if host.is_empty() {
            return Err(AppError::validation("Enter the other computer's address"));
        }
        let addrs: Vec<SocketAddr> = match host.parse::<IpAddr>() {
            Ok(ip) => vec![SocketAddr::new(ip, config.port)],
            Err(_) => tokio::net::lookup_host((host.as_str(), config.port))
                .await
                .map_err(|e| AppError::connection(format!("Cannot find {host}: {e}"), "Check the name, or enter its IP address."))?
                .collect(),
        };
        let (mut tls, peer) = self.dial(&addrs, None).await?;
        let _ = wire::write_msg(&mut tls, &PeerMessage::Bye { peer_id: self.identity.id }).await;
        let _ = tls.shutdown().await;
        if let Some(name) = config.display_name.filter(|n| !n.trim().is_empty()) {
            self.set_peer_display_name(peer.id, name).await?;
        }
        Ok(self.get_peer(peer.id).await.unwrap_or(peer))
    }

    // ── Listener ──

    /// Listen for peers (the preferred port, else any free one); returns the port.
    pub async fn start_listener(&self) -> Result<u16, AppError> {
        if lock(&self.listener).is_some() {
            return Ok(self.listening_port());
        }
        let bind = |port: u16| tokio::net::TcpListener::bind(SocketAddr::from(([0, 0, 0, 0], port)));
        let listener = match bind(self.preferred_port).await {
            Ok(l) => l,
            Err(e) if self.preferred_port != 0 => {
                tracing::warn!("Peer port {} is unavailable ({e}); using another", self.preferred_port);
                bind(0).await.map_err(wire::io_err)?
            }
            Err(e) => return Err(wire::io_err(e)),
        };
        let port = listener.local_addr().map_err(wire::io_err)?.port();
        let acceptor = tokio_rustls::TlsAcceptor::from(self.identity.server_config()?);
        let me = self.clone();
        let task = tokio::spawn(async move {
            loop {
                let Ok((tcp, remote)) = listener.accept().await else { continue };
                let (me, acceptor) = (me.clone(), acceptor.clone());
                tokio::spawn(async move {
                    let _ = tcp.set_nodelay(true);
                    let tls = match tokio::time::timeout(CONNECT_TIMEOUT, acceptor.accept(tcp)).await {
                        Ok(Ok(t)) => t,
                        _ => return,
                    };
                    if let Err(e) = me.serve(Tls::from(tls), remote).await {
                        tracing::info!("Peer connection from {remote} ended: {e}");
                    }
                });
            }
        });
        *lock(&self.listener) = Some(task);
        self.local_port.store(port, Ordering::Relaxed);
        tracing::info!("Peer listener on port {port}");
        Ok(port)
    }

    /// Stop listening (and discovery, which advertises the listener).
    pub async fn stop_listener(&self) -> Result<(), AppError> {
        self.stop_discovery().await?;
        if let Some(t) = lock(&self.listener).take() {
            t.abort();
        }
        self.local_port.store(0, Ordering::Relaxed);
        Ok(())
    }

    /// One incoming connection.
    async fn serve<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send>(&self, mut tls: Tls<S>, remote: SocketAddr) -> Result<(), AppError> {
        let fp = wire::peer_fingerprint(tls.get_ref().1.peer_certificates()).ok_or_else(|| wire::protocol("no client certificate"))?;
        let first = tokio::time::timeout(CONNECT_TIMEOUT, wire::read_msg(&mut tls)).await.map_err(|_| wire::protocol("no hello"))??;
        let PeerMessage::Hello { peer_id, hostname, os, app_version, port } = first else { return Err(wire::protocol("expected a hello")) };
        let refuse = |message: String| PeerMessage::Error { request_id: None, message };
        if let Err(e) = self.check_identity(peer_id, &fp).await {
            let _ = wire::write_msg(&mut tls, &refuse(e.user_message())).await;
            return Err(e);
        }
        if self.peers.read().await.get(&peer_id).is_some_and(|p| p.trust == TrustLevel::Blocked) {
            let _ = wire::write_msg(&mut tls, &refuse("This computer does not accept connections from you.".into())).await;
            return Ok(());
        }
        let peer = self.on_peer_discovered(PeerSighting { id: peer_id, hostname, os, addresses: vec![remote.ip()], port, app_version, fingerprint: Some(fp) }).await;
        wire::write_msg(&mut tls, &self.hello()).await?;
        loop {
            match wire::read_frame::<_, PeerMessage>(&mut tls).await? {
                None | Some(Frame::Msg(PeerMessage::Bye { .. })) => return Ok(()),
                Some(Frame::Msg(PeerMessage::Ping { timestamp, .. })) => {
                    self.on_peer_heartbeat(peer_id).await;
                    wire::write_msg(&mut tls, &PeerMessage::Pong { peer_id: self.identity.id, timestamp }).await?;
                }
                Some(Frame::Msg(PeerMessage::TransferRequest(req))) => return self.receive(tls, &peer, req).await,
                Some(_) => return Err(wire::protocol("unexpected message")),
            }
        }
    }

    // ── Transfers: receiving ──

    /// Answer a transfer request (asking the user unless the peer is trusted), then receive it.
    async fn receive<S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin + Send>(&self, tls: Tls<S>, peer: &PeerDevice, req: PeerTransferRequest) -> Result<(), AppError> {
        let (mut rd, mut wr) = tokio::io::split(tls);
        let request_id = req.request_id;
        let bad = req.sender_id != peer.id
            || req.files.len() > MAX_FILES
            || req.files.iter().any(|f| wire::safe_relative(&f.relative_path).is_none())
            || self.transfers.read().await.contains_key(&request_id);
        if bad {
            let _ = wire::write_msg(&mut wr, &PeerMessage::Error { request_id: Some(request_id), message: "The transfer request is not valid.".into() }).await;
            return Err(wire::protocol("invalid transfer request"));
        }
        let (response, save_path) = if peer.trust == TrustLevel::Trusted {
            (TransferResponse::Accepted, None)
        } else {
            let (tx, rx) = oneshot::channel();
            lock(&self.responders).insert(request_id, tx);
            self.pending_requests.write().await.insert(request_id, req.clone());
            // The sender says nothing while waiting; a frame from it means it gave up.
            let gave_up = wire::read_frame::<_, PeerMessage>(&mut rd);
            let decision = tokio::select! {
                d = rx => d.ok(),
                _ = tokio::time::sleep(APPROVAL_TIMEOUT) => Some((TransferResponse::Denied, None)),
                _ = gave_up => None,
            };
            lock(&self.responders).remove(&request_id);
            self.pending_requests.write().await.remove(&request_id);
            match decision {
                Some(d) => d,
                None => return Ok(()),
            }
        };
        if response == TransferResponse::Blocked {
            let _ = self.set_trust(peer.id, TrustLevel::Blocked).await;
        }
        wire::write_msg(&mut wr, &PeerMessage::TransferResponse { request_id, response }).await?;
        if response != TransferResponse::Accepted {
            return Ok(());
        }
        let dest = save_path.map(PathBuf::from).unwrap_or_else(|| self.downloads_dir.clone());
        let cancel = self.track(&req, peer, PeerTransferDirection::Incoming, Some(dest.to_string_lossy().into_owned())).await;
        self.set_status(peer.id, PeerStatus::Transferring).await;
        let result = self.receive_files(&mut rd, &mut wr, &req, &dest, &cancel).await;
        self.set_status(peer.id, PeerStatus::Online).await;
        self.finish(request_id, result.as_ref().err().map(AppError::user_message), &cancel).await;
        result
    }

    async fn receive_files<R, W>(&self, rd: &mut R, wr: &mut W, req: &PeerTransferRequest, dest: &Path, cancel: &AtomicBool) -> Result<(), AppError>
    where
        R: tokio::io::AsyncRead + Unpin,
        W: tokio::io::AsyncWrite + Unpin,
    {
        tokio::fs::create_dir_all(dest).await?;
        let start = Instant::now();
        let mut bytes = 0u64;
        let mut done = 0u64;
        // The file being received: (entry, part file, writer, hash, bytes left, final path).
        let mut current: Option<(u64, PathBuf, tokio::fs::File, sha2::Sha256, u64, PathBuf)> = None;
        let cleanup = |c: &Option<(u64, PathBuf, tokio::fs::File, sha2::Sha256, u64, PathBuf)>| {
            if let Some((_, part, ..)) = c {
                let _ = std::fs::remove_file(part);
            }
        };
        loop {
            if cancel.load(Ordering::Relaxed) {
                cleanup(&current);
                let _ = wire::write_msg(wr, &PeerMessage::Error { request_id: Some(req.request_id), message: "cancelled".into() }).await;
                return Err(cancelled());
            }
            let frame = match wire::read_frame::<_, PeerMessage>(rd).await {
                Ok(Some(f)) => f,
                Ok(None) => {
                    cleanup(&current);
                    return Err(AppError::connection("The sender disconnected before the transfer finished", "Ask them to send it again."));
                }
                Err(e) => {
                    cleanup(&current);
                    return Err(e);
                }
            };
            let step: Result<bool, AppError> = async {
                match frame {
                    Frame::Msg(PeerMessage::FileStart { index, .. }) if current.is_none() => {
                        let entry = req.files.get(index as usize).ok_or_else(|| wire::protocol("unknown file"))?;
                        let rel = wire::safe_relative(&entry.relative_path).ok_or_else(|| wire::protocol("unsafe path"))?;
                        let target = dest.join(&rel);
                        self.update(req.request_id, |p| p.current_file = entry.relative_path.clone()).await;
                        if entry.is_dir {
                            tokio::fs::create_dir_all(&target).await?;
                            done += 1;
                            return Ok(false);
                        }
                        if let Some(parent) = target.parent() {
                            tokio::fs::create_dir_all(parent).await?;
                        }
                        let target = unique_path(&target);
                        let part = target.with_file_name(format!(".{}{}", target.file_name().unwrap_or_default().to_string_lossy(), super::STAGING_SUFFIX));
                        let f = tokio::fs::File::create(&part).await?;
                        current = Some((index, part, f, sha2::Sha256::new(), entry.size, target));
                        Ok(false)
                    }
                    Frame::Data(d) => {
                        let (_, _, f, h, left, _) = current.as_mut().ok_or_else(|| wire::protocol("file data outside a file"))?;
                        if d.len() as u64 > *left {
                            return Err(wire::protocol("more data than announced"));
                        }
                        f.write_all(&d).await?;
                        h.update(&d);
                        *left -= d.len() as u64;
                        bytes += d.len() as u64;
                        self.progress(req.request_id, bytes, done, start).await;
                        Ok(false)
                    }
                    Frame::Msg(PeerMessage::FileEnd { index, sha256, .. }) => {
                        let (i, part, mut f, h, left, target) = current.take().ok_or_else(|| wire::protocol("file end outside a file"))?;
                        f.flush().await?;
                        f.sync_all().await?;
                        drop(f);
                        if i != index || left != 0 || hex::encode(h.finalize()) != sha256 {
                            let _ = tokio::fs::remove_file(&part).await;
                            return Err(AppError::transfer(format!("{} did not arrive intact", req.files[i as usize].relative_path), "Ask the sender to send it again."));
                        }
                        tokio::fs::rename(&part, &target).await?;
                        if let Some(m) = req.files[i as usize].modified {
                            let _ = std::fs::File::options().write(true).open(&target).and_then(|f| f.set_modified(std::time::SystemTime::from(m)));
                        }
                        done += 1;
                        self.progress(req.request_id, bytes, done, start).await;
                        Ok(false)
                    }
                    Frame::Msg(PeerMessage::TransferComplete { .. }) if current.is_none() => {
                        let ok = done == req.files.len() as u64 && bytes == req.total_bytes;
                        wire::write_msg(wr, &PeerMessage::TransferComplete { request_id: req.request_id, files_transferred: done, bytes_transferred: bytes, verification_passed: ok }).await?;
                        if ok { Ok(true) } else { Err(AppError::transfer("Some files were missing from the transfer", "Ask the sender to send it again.")) }
                    }
                    Frame::Msg(PeerMessage::Error { message, .. }) if message == "cancelled" => {
                        cancel.store(true, Ordering::Relaxed);
                        Err(cancelled())
                    }
                    Frame::Msg(PeerMessage::Error { message, .. }) => Err(AppError::transfer(format!("The sender stopped: {message}"), "Ask them to send it again.")),
                    _ => Err(wire::protocol("unexpected message during the transfer")),
                }
            }
            .await;
            match step {
                Ok(true) => return Ok(()),
                Ok(false) => {}
                Err(e) => {
                    cleanup(&current);
                    if !cancel.load(Ordering::Relaxed) {
                        let _ = wire::write_msg(wr, &PeerMessage::Error { request_id: Some(req.request_id), message: e.user_message() }).await;
                    }
                    return Err(e);
                }
            }
        }
    }

    /// List pending incoming transfer requests.
    pub async fn list_pending_requests(&self) -> Vec<PeerTransferRequest> {
        self.pending_requests.read().await.values().cloned().collect()
    }

    /// Accept, deny or block a pending incoming request. Accepted files go
    /// to `save_path` (default: the Downloads folder).
    pub async fn respond_to_transfer(&self, request_id: Uuid, response: TransferResponse, save_path: Option<String>) -> Result<(), AppError> {
        let save_path = match save_path.filter(|p| !p.trim().is_empty()) {
            Some(p) if !Path::new(&p).is_absolute() => return Err(AppError::validation("Choose a folder to save the files in")),
            p => p,
        };
        let tx = lock(&self.responders).remove(&request_id);
        self.pending_requests.write().await.remove(&request_id);
        let tx = tx.ok_or_else(|| AppError::transfer(format!("Transfer request {request_id} not found"), "The sender may have cancelled it, or it expired."))?;
        tx.send((response, save_path)).map_err(|_| AppError::transfer("The sender is no longer waiting", "Ask them to send it again."))
    }

    // ── Transfers: sending ──

    /// Ask `peer_id` to accept `paths` (files or folders) and send them
    /// once accepted. Returns at once; progress shows in [`Self::list_transfers`].
    pub async fn request_transfer(&self, peer_id: Uuid, paths: Vec<String>) -> Result<PeerTransferRequest, AppError> {
        let peer = self.get_peer(peer_id).await.ok_or_else(|| not_found(peer_id))?;
        if peer.trust == TrustLevel::Blocked {
            return Err(AppError::connection(format!("Peer {} is blocked", peer.hostname), "Unblock the peer to transfer files."));
        }
        let entries = tokio::task::spawn_blocking(move || collect_entries(&paths)).await.map_err(|e| AppError::internal(e.to_string()))??;
        if entries.is_empty() {
            return Err(AppError::validation("Choose files or folders to send"));
        }
        if entries.len() > MAX_FILES {
            return Err(AppError::validation(format!("Send at most {MAX_FILES} files at a time")));
        }
        let files: Vec<PeerFileInfo> = entries.iter().map(|(f, _)| f.clone()).collect();
        let request = PeerTransferRequest {
            request_id: Uuid::new_v4(),
            sender_id: self.identity.id,
            sender_hostname: self.local_hostname.clone(),
            sender_os: self.local_os,
            total_bytes: files.iter().map(|f| f.size).sum(),
            file_count: files.len() as u64,
            files,
            requested_at: Utc::now(),
        };
        let cancel = self.track(&request, &peer, PeerTransferDirection::Outgoing, None).await;
        self.update(request.request_id, |p| p.status = PeerTransferStatus::Connecting).await;
        let (me, req) = (self.clone(), request.clone());
        tokio::spawn(async move {
            let result = me.send(&peer, &req, &entries, &cancel).await;
            me.set_status(peer.id, PeerStatus::Online).await;
            me.finish(req.request_id, result.err().map(|e| e.user_message()), &cancel).await;
        });
        Ok(request)
    }

    async fn send(&self, peer: &PeerDevice, req: &PeerTransferRequest, entries: &[(PeerFileInfo, PathBuf)], cancel: &AtomicBool) -> Result<(), AppError> {
        if self.listening_port() == 0 {
            // Our Hello names a port the receiver could answer on; not required to send.
            let _ = self.start_listener().await;
        }
        let (tls, _) = self.dial(&Self::addrs_of(peer), peer.tls_fingerprint.as_deref()).await?;
        let (mut rd, mut wr) = tokio::io::split(tls);
        wire::write_msg(&mut wr, &PeerMessage::TransferRequest(req.clone())).await?;
        self.update(req.request_id, |p| p.status = PeerTransferStatus::Pending).await;

        // Wait for the answer, watching for a cancel meanwhile.
        let answer = {
            let answer = wire::read_msg::<_, PeerMessage>(&mut rd);
            tokio::pin!(answer);
            loop {
                tokio::select! {
                    m = &mut answer => break m?,
                    _ = tokio::time::sleep(Duration::from_millis(200)) => {
                        if cancel.load(Ordering::Relaxed) {
                            let _ = wire::write_msg(&mut wr, &PeerMessage::Error { request_id: Some(req.request_id), message: "cancelled".into() }).await;
                            return Err(cancelled());
                        }
                    }
                }
            }
        };
        match answer {
            PeerMessage::TransferResponse { response: TransferResponse::Accepted, request_id } if request_id == req.request_id => {}
            PeerMessage::TransferResponse { .. } => {
                self.update(req.request_id, |p| p.status = PeerTransferStatus::Denied).await;
                return Err(AppError::transfer("The other computer declined the transfer", "Ask its user to accept it."));
            }
            PeerMessage::Error { message, .. } => return Err(AppError::transfer(message, "Check the transfer on the other computer.")),
            _ => return Err(wire::protocol("unexpected answer")),
        }

        self.update(req.request_id, |p| p.status = PeerTransferStatus::Transferring).await;
        self.set_status(peer.id, PeerStatus::Transferring).await;
        let start = Instant::now();
        let (mut bytes, mut done) = (0u64, 0u64);
        let mut buf = vec![0u8; wire::MAX_DATA_FRAME];
        for (index, (info, local)) in entries.iter().enumerate() {
            self.update(req.request_id, |p| p.current_file = info.relative_path.clone()).await;
            wire::write_msg(&mut wr, &PeerMessage::FileStart { request_id: req.request_id, index: index as u64 }).await?;
            if !info.is_dir {
                use tokio::io::AsyncReadExt;
                let mut f = tokio::fs::File::open(local).await?;
                let mut h = sha2::Sha256::new();
                let mut left = info.size;
                while left > 0 {
                    if cancel.load(Ordering::Relaxed) {
                        let _ = wire::write_msg(&mut wr, &PeerMessage::Error { request_id: Some(req.request_id), message: "cancelled".into() }).await;
                        return Err(cancelled());
                    }
                    let want = left.min(buf.len() as u64) as usize;
                    let n = f.read(&mut buf[..want]).await?;
                    if n == 0 {
                        let msg = format!("{} changed while it was being sent", info.relative_path);
                        let _ = wire::write_msg(&mut wr, &PeerMessage::Error { request_id: Some(req.request_id), message: msg.clone() }).await;
                        return Err(AppError::transfer(msg, "Send it again once it is no longer being written."));
                    }
                    h.update(&buf[..n]);
                    wire::write_data(&mut wr, &buf[..n]).await.map_err(|e| self.receiver_gone(e))?;
                    left -= n as u64;
                    bytes += n as u64;
                    self.progress(req.request_id, bytes, done, start).await;
                }
                wire::write_msg(&mut wr, &PeerMessage::FileEnd { request_id: req.request_id, index: index as u64, sha256: hex::encode(h.finalize()) }).await?;
            }
            done += 1;
            self.progress(req.request_id, bytes, done, start).await;
        }
        self.update(req.request_id, |p| p.status = PeerTransferStatus::Verifying).await;
        wire::write_msg(&mut wr, &PeerMessage::TransferComplete { request_id: req.request_id, files_transferred: done, bytes_transferred: bytes, verification_passed: true }).await?;
        match wire::read_msg::<_, PeerMessage>(&mut rd).await? {
            PeerMessage::TransferComplete { verification_passed: true, .. } => {
                let _ = wr.shutdown().await;
                Ok(())
            }
            PeerMessage::Error { message, .. } => Err(AppError::transfer(message, "Send it again.")),
            _ => Err(AppError::transfer("The other computer could not verify the files", "Send them again.")),
        }
    }

    fn receiver_gone(&self, e: AppError) -> AppError {
        AppError::transfer(format!("The other computer stopped receiving ({e})"), "It may have cancelled the transfer.")
    }

    // ── Transfer bookkeeping ──

    async fn track(&self, req: &PeerTransferRequest, peer: &PeerDevice, direction: PeerTransferDirection, save_path: Option<String>) -> Arc<AtomicBool> {
        let cancel = Arc::new(AtomicBool::new(false));
        lock(&self.cancels).insert(req.request_id, cancel.clone());
        self.transfers.write().await.insert(
            req.request_id,
            PeerTransferProgress {
                request_id: req.request_id,
                peer_id: peer.id,
                peer_name: peer.display_name.clone().unwrap_or_else(|| peer.hostname.clone()),
                direction,
                transferred_bytes: 0,
                total_bytes: req.total_bytes,
                current_file: String::new(),
                files_completed: 0,
                files_total: req.file_count,
                speed_bps: 0,
                eta_seconds: None,
                status: PeerTransferStatus::Transferring,
                save_path,
                error: None,
            },
        );
        cancel
    }

    async fn update(&self, id: Uuid, f: impl FnOnce(&mut PeerTransferProgress)) {
        if let Some(p) = self.transfers.write().await.get_mut(&id) {
            if !p.status.is_finished() {
                f(p);
            }
        }
    }

    async fn progress(&self, id: Uuid, bytes: u64, done: u64, start: Instant) {
        let secs = start.elapsed().as_secs_f64().max(0.001);
        self.update(id, |p| {
            p.transferred_bytes = bytes;
            p.files_completed = done;
            p.speed_bps = (bytes as f64 / secs) as u64;
            p.eta_seconds = (p.speed_bps > 0).then(|| p.total_bytes.saturating_sub(bytes) / p.speed_bps);
        })
        .await;
    }

    async fn finish(&self, id: Uuid, error: Option<String>, cancel: &AtomicBool) {
        lock(&self.cancels).remove(&id);
        if let Some(p) = self.transfers.write().await.get_mut(&id) {
            if p.status.is_finished() {
                return;
            }
            match error {
                None => {
                    p.status = PeerTransferStatus::Completed;
                    p.eta_seconds = Some(0);
                }
                Some(_) if cancel.load(Ordering::Relaxed) => p.status = PeerTransferStatus::Cancelled,
                Some(e) => {
                    p.status = PeerTransferStatus::Failed;
                    p.error = Some(e);
                }
            }
        }
    }

    pub async fn get_transfer_progress(&self, request_id: Uuid) -> Option<PeerTransferProgress> {
        self.transfers.read().await.get(&request_id).cloned()
    }

    pub async fn list_transfers(&self) -> Vec<PeerTransferProgress> {
        self.transfers.read().await.values().cloned().collect()
    }

    /// Cancel a transfer in either direction.
    pub async fn cancel_transfer(&self, request_id: Uuid) -> Result<(), AppError> {
        let flag = lock(&self.cancels).get(&request_id).cloned();
        let mut transfers = self.transfers.write().await;
        match (transfers.get_mut(&request_id), flag) {
            (Some(t), Some(flag)) if !t.status.is_finished() => {
                flag.store(true, Ordering::Relaxed);
                t.status = PeerTransferStatus::Cancelled;
                Ok(())
            }
            (Some(_), _) => Err(AppError::transfer("This transfer has already finished", "Nothing to cancel.")),
            (None, _) => Err(AppError::transfer(format!("Transfer {request_id} not found"), "The transfer may have already completed.")),
        }
    }

    /// Forget finished transfers; returns how many were removed.
    pub async fn clear_finished_transfers(&self) -> usize {
        let mut t = self.transfers.write().await;
        let before = t.len();
        t.retain(|_, p| !p.status.is_finished());
        before - t.len()
    }
}

/// Parse an mDNS advertisement of another UFOP device.
fn sighting_from_mdns(info: &mdns_sd::ServiceInfo) -> Option<PeerSighting> {
    let prop = |k: &str| info.get_property_val_str(k).map(str::to_string);
    let id = Uuid::parse_str(&prop("id")?).ok()?;
    let mut addresses: Vec<IpAddr> = info.get_addresses().iter().copied().collect();
    addresses.sort();
    Some(PeerSighting {
        id,
        hostname: prop("name").unwrap_or_else(|| info.get_hostname().trim_end_matches(".local.").to_string()),
        os: PeerOs::from_str_lossy(&prop("os").unwrap_or_default()),
        addresses,
        port: info.get_port(),
        app_version: prop("ver").unwrap_or_default(),
        fingerprint: prop("fp").and_then(|f| crate::security::tls::normalize_fingerprint(&f)),
    })
}

/// `path`, or `name (1).ext`, `name (2).ext`… if it exists.
fn unique_path(path: &Path) -> PathBuf {
    if !path.exists() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = path.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (1..).map(|i| path.with_file_name(format!("{stem} ({i}){ext}"))).find(|p| !p.exists()).unwrap_or_else(|| path.to_path_buf())
}

/// The entries to send for `paths`: each file, and each folder with
/// everything under it (symbolic links are skipped), named relative to
/// the chosen item's parent.
fn collect_entries(paths: &[String]) -> Result<Vec<(PeerFileInfo, PathBuf)>, AppError> {
    fn info(rel: String, meta: &std::fs::Metadata) -> PeerFileInfo {
        PeerFileInfo { relative_path: rel, size: if meta.is_dir() { 0 } else { meta.len() }, is_dir: meta.is_dir(), modified: meta.modified().ok().map(DateTime::<Utc>::from) }
    }
    fn walk(dir: &Path, rel: &str, out: &mut Vec<(PeerFileInfo, PathBuf)>) -> Result<(), AppError> {
        let mut children: Vec<_> = std::fs::read_dir(dir)?.filter_map(Result::ok).collect();
        children.sort_by_key(|e| e.file_name());
        for c in children {
            let meta = std::fs::symlink_metadata(c.path())?;
            if meta.file_type().is_symlink() {
                continue;
            }
            let r = format!("{rel}/{}", c.file_name().to_string_lossy());
            out.push((info(r.clone(), &meta), c.path()));
            if meta.is_dir() {
                walk(&c.path(), &r, out)?;
            }
            if out.len() > MAX_FILES {
                return Ok(());
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    for p in paths {
        let path = PathBuf::from(p);
        let meta = std::fs::symlink_metadata(&path).map_err(|e| AppError::validation(format!("Cannot read {p}: {e}")))?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).ok_or_else(|| AppError::validation(format!("Cannot send {p}")))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        out.push((info(name.clone(), &meta), path.clone()));
        if meta.is_dir() {
            walk(&path, &name, &mut out)?;
        }
    }
    Ok(out)
}

#[cfg(test)]
#[path = "peer_tests.rs"]
mod tests;
