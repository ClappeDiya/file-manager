//! Peer tests with two real managers talking TLS over loopback: the
//! handshake and certificate pinning, approval (accept, deny, block,
//! trusted auto-accept), file and folder transfers with verification,
//! cancels, refusal of tampered or unsafe transfers, persistence, and
//! mDNS discovery between two daemons.

use super::*;
use std::time::Duration;

struct Side {
    mgr: PeerManager,
    _dir: tempfile::TempDir,
    downloads: PathBuf,
}

fn side(name: &str) -> Side {
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("downloads");
    let mgr = PeerManager::open(dir.path().join("peer")).with_port(0).with_downloads_dir(downloads.clone()).with_hostname(name);
    Side { mgr, _dir: dir, downloads }
}

async fn wait_for<F, Fut>(what: &str, mut f: F)
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..200 {
        if f().await {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("timed out waiting for {what}");
}

async fn status_of(m: &PeerManager, id: Uuid) -> Option<PeerTransferStatus> {
    m.get_transfer_progress(id).await.map(|p| p.status)
}

async fn wait_status(m: &PeerManager, id: Uuid, want: PeerTransferStatus) -> PeerTransferProgress {
    wait_for(&format!("{want:?}"), || async { status_of(m, id).await == Some(want) }).await;
    m.get_transfer_progress(id).await.unwrap()
}

/// `a` connects to `b` by address; returns `b` as `a` sees it.
async fn pair(a: &Side, b: &Side) -> PeerDevice {
    let port = b.mgr.start_listener().await.unwrap();
    a.mgr.connect_manual(ManualPeerConfig { host: "127.0.0.1".into(), port, display_name: None }).await.unwrap()
}

fn payload(len: usize, seed: u32) -> Vec<u8> {
    (0..len as u32).map(|i| (i.wrapping_add(seed).wrapping_mul(2654435761) >> 24) as u8).collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn handshake_learns_identity_both_ways() {
    let (a, b) = (side("alpha"), side("beta"));
    let seen = pair(&a, &b).await;
    assert_eq!((seen.id, seen.hostname.as_str()), (b.mgr.local_peer_id(), "beta"));
    assert_eq!(seen.tls_fingerprint.as_deref(), Some(b.mgr.local_fingerprint().as_str()));
    assert_eq!(seen.status, PeerStatus::Online);
    // The listener side learned who connected, with the client certificate's fingerprint.
    let back = b.mgr.get_peer(a.mgr.local_peer_id()).await.unwrap();
    assert_eq!(back.hostname, "alpha");
    assert_eq!(back.tls_fingerprint, Some(a.mgr.local_fingerprint()));
    assert_eq!(back.addresses, vec!["127.0.0.1".parse::<IpAddr>().unwrap()]);

    // Host names resolve too; nothing listening is a clear error.
    let port = b.mgr.listening_port();
    assert_eq!(a.mgr.connect_manual(ManualPeerConfig { host: "localhost".into(), port, display_name: Some("Beta box".into()) }).await.unwrap().display_name.as_deref(), Some("Beta box"));
    b.mgr.stop_listener().await.unwrap();
    assert_eq!(b.mgr.listening_port(), 0);
    assert!(a.mgr.connect_manual(ManualPeerConfig { host: "127.0.0.1".into(), port, display_name: None }).await.is_err());
    assert!(a.mgr.connect_manual(ManualPeerConfig { host: " ".into(), port, display_name: None }).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn approved_transfer_of_files_and_folders_arrives_intact() {
    let (a, b) = (side("alpha"), side("beta"));
    let peer_b = pair(&a, &b).await;
    let src = tempfile::tempdir().unwrap();
    let big = payload(9 * 1024 * 1024 + 123, 1); // several data frames
    std::fs::write(src.path().join("big.bin"), &big).unwrap();
    std::fs::create_dir_all(src.path().join("album/2024/empty")).unwrap();
    std::fs::write(src.path().join("album/2024/a.jpg"), payload(1000, 2)).unwrap();
    std::fs::write(src.path().join("album/notes.txt"), b"notes").unwrap();
    let old = std::time::SystemTime::UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    std::fs::File::options().write(true).open(src.path().join("album/notes.txt")).unwrap().set_modified(old).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink("/etc/passwd", src.path().join("album/link")).unwrap();

    let paths = vec![src.path().join("big.bin").to_string_lossy().into_owned(), src.path().join("album").to_string_lossy().into_owned()];
    let req = a.mgr.request_transfer(peer_b.id, paths.clone()).await.unwrap();
    let names: Vec<&str> = req.files.iter().map(|f| f.relative_path.as_str()).collect();
    assert_eq!(names, ["big.bin", "album", "album/2024", "album/2024/a.jpg", "album/2024/empty", "album/notes.txt"], "symbolic links are skipped");
    assert_eq!(req.total_bytes, big.len() as u64 + 1005);

    // The receiver is asked first; the sender waits.
    wait_for("the request", || async { !b.mgr.list_pending_requests().await.is_empty() }).await;
    let pending = b.mgr.list_pending_requests().await;
    assert_eq!((pending[0].request_id, pending[0].sender_hostname.as_str(), pending[0].file_count), (req.request_id, "alpha", 6));
    assert_eq!(status_of(&a.mgr, req.request_id).await, Some(PeerTransferStatus::Pending));
    let save = b.downloads.join("from-alpha");
    assert!(b.mgr.respond_to_transfer(req.request_id, TransferResponse::Accepted, Some("relative/dir".into())).await.is_err());
    b.mgr.respond_to_transfer(req.request_id, TransferResponse::Accepted, Some(save.to_string_lossy().into_owned())).await.unwrap();

    let sent = wait_status(&a.mgr, req.request_id, PeerTransferStatus::Completed).await;
    assert_eq!((sent.transferred_bytes, sent.files_completed, sent.direction), (req.total_bytes, 6, PeerTransferDirection::Outgoing));
    let got = wait_status(&b.mgr, req.request_id, PeerTransferStatus::Completed).await;
    assert_eq!((got.direction, got.save_path.as_deref(), got.peer_name.as_str()), (PeerTransferDirection::Incoming, Some(save.to_str().unwrap()), "alpha"));
    assert_eq!(std::fs::read(save.join("big.bin")).unwrap(), big);
    assert_eq!(std::fs::read(save.join("album/2024/a.jpg")).unwrap(), payload(1000, 2));
    assert!(save.join("album/2024/empty").is_dir());
    assert!(!save.join("album/link").exists());
    assert_eq!(std::fs::metadata(save.join("album/notes.txt")).unwrap().modified().unwrap(), old, "modification times are kept");
    assert!(std::fs::read_dir(save.join("album")).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(super::super::STAGING_SUFFIX)));

    // Sending again never overwrites: the copies get new names.
    let again = a.mgr.request_transfer(peer_b.id, vec![paths[0].clone()]).await.unwrap();
    wait_for("the second request", || async { !b.mgr.list_pending_requests().await.is_empty() }).await;
    b.mgr.respond_to_transfer(again.request_id, TransferResponse::Accepted, Some(save.to_string_lossy().into_owned())).await.unwrap();
    wait_status(&a.mgr, again.request_id, PeerTransferStatus::Completed).await;
    assert_eq!(std::fs::read(save.join("big (1).bin")).unwrap(), big);

    assert_eq!(a.mgr.clear_finished_transfers().await, 2);
    assert!(a.mgr.list_transfers().await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn deny_block_and_trust() {
    let (a, b) = (side("alpha"), side("beta"));
    let peer_b = pair(&a, &b).await;
    let src = tempfile::tempdir().unwrap();
    let f = src.path().join("x.txt");
    std::fs::write(&f, b"x").unwrap();
    let paths = vec![f.to_string_lossy().into_owned()];

    let r = a.mgr.request_transfer(peer_b.id, paths.clone()).await.unwrap();
    wait_for("the request", || async { !b.mgr.list_pending_requests().await.is_empty() }).await;
    b.mgr.respond_to_transfer(r.request_id, TransferResponse::Denied, None).await.unwrap();
    wait_status(&a.mgr, r.request_id, PeerTransferStatus::Denied).await;
    assert!(!b.downloads.exists(), "nothing is written for a denied transfer");
    assert!(b.mgr.respond_to_transfer(r.request_id, TransferResponse::Accepted, None).await.is_err(), "a request is answered once");

    // Trusted senders are accepted without asking, into the default folder.
    b.mgr.set_trust(a.mgr.local_peer_id(), TrustLevel::Trusted).await.unwrap();
    let r = a.mgr.request_transfer(peer_b.id, paths.clone()).await.unwrap();
    wait_status(&a.mgr, r.request_id, PeerTransferStatus::Completed).await;
    assert_eq!(std::fs::read(b.downloads.join("x.txt")).unwrap(), b"x");
    assert!(b.mgr.list_pending_requests().await.is_empty());

    // Block via the answer: the sender is then refused at the handshake.
    b.mgr.set_trust(a.mgr.local_peer_id(), TrustLevel::Untrusted).await.unwrap();
    let r = a.mgr.request_transfer(peer_b.id, paths.clone()).await.unwrap();
    wait_for("the request", || async { !b.mgr.list_pending_requests().await.is_empty() }).await;
    b.mgr.respond_to_transfer(r.request_id, TransferResponse::Blocked, None).await.unwrap();
    wait_status(&a.mgr, r.request_id, PeerTransferStatus::Denied).await;
    assert_eq!(b.mgr.get_peer(a.mgr.local_peer_id()).await.unwrap().trust, TrustLevel::Blocked);
    let r = a.mgr.request_transfer(peer_b.id, paths.clone()).await.unwrap();
    let failed = wait_status(&a.mgr, r.request_id, PeerTransferStatus::Failed).await;
    assert!(failed.error.unwrap().contains("does not accept"));

    // And blocking on our side stops sending at once.
    a.mgr.set_trust(peer_b.id, TrustLevel::Blocked).await.unwrap();
    assert!(a.mgr.request_transfer(peer_b.id, paths).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn cancels_reach_the_other_side() {
    let (a, b) = (side("alpha"), side("beta"));
    let peer_b = pair(&a, &b).await;
    let src = tempfile::tempdir().unwrap();
    let f = src.path().join("x.txt");
    std::fs::write(&f, b"x").unwrap();

    // The sender gives up while the receiver is still deciding.
    let r = a.mgr.request_transfer(peer_b.id, vec![f.to_string_lossy().into_owned()]).await.unwrap();
    wait_for("the request", || async { !b.mgr.list_pending_requests().await.is_empty() }).await;
    a.mgr.cancel_transfer(r.request_id).await.unwrap();
    assert_eq!(status_of(&a.mgr, r.request_id).await, Some(PeerTransferStatus::Cancelled));
    wait_for("the request to be withdrawn", || async { b.mgr.list_pending_requests().await.is_empty() }).await;
    assert!(b.mgr.respond_to_transfer(r.request_id, TransferResponse::Accepted, None).await.is_err());
    assert!(a.mgr.cancel_transfer(r.request_id).await.is_err(), "a finished transfer cannot be cancelled");
    assert!(a.mgr.cancel_transfer(Uuid::new_v4()).await.is_err());
}

#[tokio::test(flavor = "multi_thread")]
async fn receiver_cancel_mid_transfer_leaves_no_partial_file() {
    let (a, b) = (side("alpha"), side("beta"));
    let peer_b = pair(&a, &b).await;
    b.mgr.set_trust(a.mgr.local_peer_id(), TrustLevel::Trusted).await.unwrap();
    // A sender that pauses after the first frame, so the cancel lands mid-file.
    let (tls, _) = a.mgr.dial(&PeerManager::addrs_of(&peer_b), peer_b.tls_fingerprint.as_deref()).await.unwrap();
    let (mut rd, mut wr) = tokio::io::split(tls);
    let req = PeerTransferRequest {
        request_id: Uuid::new_v4(),
        sender_id: a.mgr.local_peer_id(),
        sender_hostname: "alpha".into(),
        sender_os: PeerOs::Linux,
        files: vec![PeerFileInfo { relative_path: "slow.bin".into(), size: 10, is_dir: false, modified: None }],
        total_bytes: 10,
        file_count: 1,
        requested_at: Utc::now(),
    };
    wire::write_msg(&mut wr, &PeerMessage::TransferRequest(req.clone())).await.unwrap();
    assert!(matches!(wire::read_msg::<_, PeerMessage>(&mut rd).await.unwrap(), PeerMessage::TransferResponse { response: TransferResponse::Accepted, .. }));
    wire::write_msg(&mut wr, &PeerMessage::FileStart { request_id: req.request_id, index: 0 }).await.unwrap();
    wire::write_data(&mut wr, b"12345").await.unwrap();
    wait_for("the first bytes", || async { b.mgr.get_transfer_progress(req.request_id).await.is_some_and(|p| p.transferred_bytes == 5) }).await;
    b.mgr.cancel_transfer(req.request_id).await.unwrap();
    wire::write_data(&mut wr, b"67890").await.unwrap();
    // The receiver says so and hangs up.
    assert!(matches!(wire::read_msg::<_, PeerMessage>(&mut rd).await, Ok(PeerMessage::Error { message, .. }) if message == "cancelled"));
    assert_eq!(status_of(&b.mgr, req.request_id).await, Some(PeerTransferStatus::Cancelled));
    assert_eq!(std::fs::read_dir(&b.downloads).unwrap().count(), 0, "the partial file is removed");
}

#[tokio::test(flavor = "multi_thread")]
async fn tampered_or_unsafe_transfers_are_refused() {
    let (a, b) = (side("alpha"), side("beta"));
    let peer_b = pair(&a, &b).await;
    b.mgr.set_trust(a.mgr.local_peer_id(), TrustLevel::Trusted).await.unwrap();
    let request = |path: &str, size: u64| PeerTransferRequest {
        request_id: Uuid::new_v4(),
        sender_id: a.mgr.local_peer_id(),
        sender_hostname: "alpha".into(),
        sender_os: PeerOs::Linux,
        files: vec![PeerFileInfo { relative_path: path.into(), size, is_dir: false, modified: None }],
        total_bytes: size,
        file_count: 1,
        requested_at: Utc::now(),
    };

    // Wrong checksum: refused, nothing kept.
    let (tls, _) = a.mgr.dial(&PeerManager::addrs_of(&peer_b), None).await.unwrap();
    let (mut rd, mut wr) = tokio::io::split(tls);
    let req = request("a.txt", 3);
    wire::write_msg(&mut wr, &PeerMessage::TransferRequest(req.clone())).await.unwrap();
    wire::read_msg::<_, PeerMessage>(&mut rd).await.unwrap();
    wire::write_msg(&mut wr, &PeerMessage::FileStart { request_id: req.request_id, index: 0 }).await.unwrap();
    wire::write_data(&mut wr, b"abc").await.unwrap();
    wire::write_msg(&mut wr, &PeerMessage::FileEnd { request_id: req.request_id, index: 0, sha256: "00".repeat(32) }).await.unwrap();
    assert!(matches!(wire::read_msg::<_, PeerMessage>(&mut rd).await.unwrap(), PeerMessage::Error { message, .. } if message.contains("intact")));
    let p = wait_status(&b.mgr, req.request_id, PeerTransferStatus::Failed).await;
    assert!(p.error.unwrap().contains("intact"));
    assert!(!b.downloads.join("a.txt").exists());
    assert_eq!(std::fs::read_dir(&b.downloads).unwrap().count(), 0);

    // More bytes than announced.
    let (tls, _) = a.mgr.dial(&PeerManager::addrs_of(&peer_b), None).await.unwrap();
    let (mut rd, mut wr) = tokio::io::split(tls);
    let req = request("b.txt", 1);
    wire::write_msg(&mut wr, &PeerMessage::TransferRequest(req.clone())).await.unwrap();
    wire::read_msg::<_, PeerMessage>(&mut rd).await.unwrap();
    wire::write_msg(&mut wr, &PeerMessage::FileStart { request_id: req.request_id, index: 0 }).await.unwrap();
    wire::write_data(&mut wr, b"too long").await.unwrap();
    wait_status(&b.mgr, req.request_id, PeerTransferStatus::Failed).await;

    // A path that climbs out of the folder is refused before anything is accepted.
    let (tls, _) = a.mgr.dial(&PeerManager::addrs_of(&peer_b), None).await.unwrap();
    let (mut rd, mut wr) = tokio::io::split(tls);
    let req = request("../../evil.sh", 1);
    wire::write_msg(&mut wr, &PeerMessage::TransferRequest(req.clone())).await.unwrap();
    assert!(matches!(wire::read_msg::<_, PeerMessage>(&mut rd).await.unwrap(), PeerMessage::Error { .. }));
    assert!(b.mgr.get_transfer_progress(req.request_id).await.is_none());

    // A request on behalf of another device is refused too.
    let (tls, _) = a.mgr.dial(&PeerManager::addrs_of(&peer_b), None).await.unwrap();
    let (mut rd, mut wr) = tokio::io::split(tls);
    let req = PeerTransferRequest { sender_id: Uuid::new_v4(), ..request("c.txt", 1) };
    wire::write_msg(&mut wr, &PeerMessage::TransferRequest(req)).await.unwrap();
    assert!(matches!(wire::read_msg::<_, PeerMessage>(&mut rd).await.unwrap(), PeerMessage::Error { .. }));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_saved_device_with_a_new_certificate_is_refused() {
    let (a, b) = (side("alpha"), side("beta"));
    let peer_b = pair(&a, &b).await;
    a.mgr.save_peer(peer_b.id).await.unwrap();
    // Something else answers with beta's id but its own certificate.
    let mut fake_id = PeerIdentity::generate().unwrap();
    fake_id.id = peer_b.id;
    let fake = PeerManager::with_identity(fake_id, None).with_port(0).with_hostname("beta");
    let port = fake.start_listener().await.unwrap();
    // By address (no pin), the handshake reveals the mismatch.
    let err = a.mgr.connect_manual(ManualPeerConfig { host: "127.0.0.1".into(), port, display_name: None }).await.unwrap_err();
    assert!(err.to_string().contains("identity has changed"), "{err}");
    // To the saved device, the pinned certificate fails the TLS handshake itself.
    let err = a.mgr.dial(&[SocketAddr::from(([127, 0, 0, 1], port))], peer_b.tls_fingerprint.as_deref()).await.err().unwrap();
    assert!(err.to_string().contains("Secure connection"), "{err}");
    // The impostor also cannot pose as alpha towards beta... beta refuses a changed alpha.
    b.mgr.save_peer(a.mgr.local_peer_id()).await.unwrap();
    let mut fake_a = PeerIdentity::generate().unwrap();
    fake_a.id = a.mgr.local_peer_id();
    let impostor = PeerManager::with_identity(fake_a, None);
    let err = impostor.dial(&[SocketAddr::from(([127, 0, 0, 1], b.mgr.listening_port()))], None).await.err().unwrap();
    assert!(err.to_string().contains("identity has changed") || err.to_string().contains("hello"), "{err}");
}

#[tokio::test(flavor = "multi_thread")]
async fn saved_devices_persist_with_their_pins() {
    let dir = tempfile::tempdir().unwrap();
    let b = side("beta");
    let id;
    let fp;
    {
        let a = PeerManager::open(dir.path().to_path_buf()).with_port(0);
        id = a.local_peer_id();
        let port = b.mgr.start_listener().await.unwrap();
        let peer = a.connect_manual(ManualPeerConfig { host: "127.0.0.1".into(), port, display_name: None }).await.unwrap();
        a.set_trust(peer.id, TrustLevel::Trusted).await.unwrap();
        a.set_peer_display_name(peer.id, "Living room".into()).await.unwrap();
        fp = peer.tls_fingerprint.unwrap();
    }
    let a = PeerManager::open(dir.path().to_path_buf());
    assert_eq!(a.local_peer_id(), id, "the device keeps its identity");
    let saved = a.list_saved_peers().await;
    assert_eq!(saved.len(), 1);
    assert_eq!((saved[0].trust, saved[0].display_name.as_deref(), saved[0].tls_fingerprint.as_deref()), (TrustLevel::Trusted, Some("Living room"), Some(fp.as_str())));
    // Listed (offline until seen) so files can be sent straight away.
    let listed = a.get_peer(saved[0].id).await.unwrap();
    assert_eq!((listed.status, listed.port), (PeerStatus::Offline, b.mgr.listening_port()));
    a.remove_saved_peer(saved[0].id).await.unwrap();
    assert!(PeerManager::open(dir.path().to_path_buf()).list_saved_peers().await.is_empty());
    assert!(a.get_peer(saved[0].id).await.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn trust_and_names_need_a_known_peer() {
    let m = PeerManager::new();
    assert!(m.set_trust(Uuid::new_v4(), TrustLevel::Trusted).await.is_err());
    assert!(m.save_peer(Uuid::new_v4()).await.is_err());
    assert!(m.request_transfer(Uuid::new_v4(), vec![]).await.is_err());
    let p = m
        .on_peer_discovered(PeerSighting { id: Uuid::new_v4(), hostname: "h".into(), os: PeerOs::Windows, addresses: vec![], port: 1, app_version: "1".into(), fingerprint: None })
        .await;
    assert!(m.request_transfer(p.id, vec![]).await.unwrap_err().to_string().contains("Choose files"));
    m.set_peer_display_name(p.id, "  ".into()).await.unwrap();
    assert_eq!(m.get_peer(p.id).await.unwrap().display_name, None);
    m.on_peer_offline(p.id).await;
    assert!(m.list_online_peers().await.is_empty());
    m.on_peer_heartbeat(p.id).await;
    assert_eq!(m.list_online_peers().await.len(), 1);
}

#[test]
fn unique_names_and_os_detection() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("a.txt");
    assert_eq!(unique_path(&p), p);
    std::fs::write(&p, "").unwrap();
    std::fs::write(d.path().join("a (1).txt"), "").unwrap();
    assert_eq!(unique_path(&p), d.path().join("a (2).txt"));
    assert_eq!(PeerOs::from_str_lossy(PeerOs::current().as_str()), PeerOs::current());
    assert_eq!(TrustLevel::from_str_lossy("bogus"), TrustLevel::Unknown);
}

/// Two devices find each other over mDNS (needs a multicast-capable
/// interface; skipped with a note when the network has none).
#[tokio::test(flavor = "multi_thread")]
async fn mdns_discovery_finds_the_other_device() {
    let (a, b) = (side("alpha"), side("beta"));
    if let Err(e) = a.mgr.start_discovery().await {
        eprintln!("mDNS unavailable here, skipping: {e}");
        return;
    }
    b.mgr.start_discovery().await.unwrap();
    assert!(a.mgr.is_discovering() && a.mgr.listening_port() != 0);
    let (ida, idb) = (a.mgr.local_peer_id(), b.mgr.local_peer_id());
    let mut found = false;
    for _ in 0..200 {
        if a.mgr.get_peer(idb).await.is_some() && b.mgr.get_peer(ida).await.is_some() {
            found = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    if !found && std::env::var("UFOP_TEST_MDNS").is_err() {
        eprintln!("mDNS advertisements did not arrive (no multicast here); set UFOP_TEST_MDNS=1 to require it");
        return;
    }
    assert!(found, "both devices should discover each other");
    let seen = a.mgr.get_peer(idb).await.unwrap();
    assert_eq!((seen.hostname.as_str(), seen.port, seen.tls_fingerprint.clone()), ("beta", b.mgr.listening_port(), Some(b.mgr.local_fingerprint())));
    // The advertised address and pinned certificate are enough to send.
    let src = tempfile::tempdir().unwrap();
    std::fs::write(src.path().join("hi.txt"), b"hi").unwrap();
    b.mgr.set_trust(ida, TrustLevel::Trusted).await.unwrap();
    let r = a.mgr.request_transfer(idb, vec![src.path().join("hi.txt").to_string_lossy().into_owned()]).await.unwrap();
    wait_status(&a.mgr, r.request_id, PeerTransferStatus::Completed).await;
    assert_eq!(std::fs::read(b.downloads.join("hi.txt")).unwrap(), b"hi");
    // Stopping sends a goodbye: the other side marks it offline.
    b.mgr.stop_discovery().await.unwrap();
    assert!(!b.mgr.is_discovering());
    wait_for("the goodbye", || async { a.mgr.get_peer(idb).await.is_some_and(|p| p.status == PeerStatus::Offline) }).await;
    a.mgr.stop_discovery().await.unwrap();
}
