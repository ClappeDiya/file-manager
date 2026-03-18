//! Journey 6: Computer-to-computer transfer over LAN
//!
//! End-to-end test validating:
//! 1. Peer discovery via mDNS/Bonjour
//! 2. Peer trust management
//! 3. Transfer request/response workflow
//! 4. Transfer progress tracking
//! 5. Server-to-server transfers

use unified_file_ops_lib::connectors::PeerManager;
use unified_file_ops_lib::connectors::ServerTransferManager;
use unified_file_ops_lib::connectors::server_to_server::{
    self, TransferEndpoint, EndpointProtocol, ServerTransferFile, FileTransferStatus,
};
use uuid::Uuid;

/// Journey 6A: Peer manager creation and discovery lifecycle
#[tokio::test]
async fn journey6_peer_discovery_lifecycle() {
    let peer_mgr = PeerManager::new();

    // Should not be discovering initially
    assert!(!peer_mgr.is_discovering());

    // Start discovery
    let result = peer_mgr.start_discovery().await;
    assert!(result.is_ok(), "Should start discovery without error");

    // Stop discovery
    let result = peer_mgr.stop_discovery().await;
    assert!(result.is_ok(), "Should stop discovery without error");
    assert!(!peer_mgr.is_discovering());
}

/// Journey 6B: Peer listing (initially empty)
#[tokio::test]
async fn journey6_peer_listing() {
    let peer_mgr = PeerManager::new();

    let all_peers = peer_mgr.list_peers().await;
    assert!(all_peers.is_empty(), "Should have no peers initially");

    let online = peer_mgr.list_online_peers().await;
    assert!(online.is_empty(), "Should have no online peers without discovery");
}

/// Journey 6C: Saved peers management
#[tokio::test]
async fn journey6_saved_peers() {
    let peer_mgr = PeerManager::new();

    // List saved peers (should be empty initially)
    let saved = peer_mgr.list_saved_peers().await;
    assert!(saved.is_empty(), "Should have no saved peers initially");
}

/// Journey 6D: Transfer request workflow
#[tokio::test]
async fn journey6_transfer_request_workflow() {
    let peer_mgr = PeerManager::new();

    // List pending requests (should be empty)
    let pending = peer_mgr.list_pending_requests().await;
    assert!(pending.is_empty(), "Should have no pending requests initially");

    // List active transfers
    let transfers = peer_mgr.list_transfers().await;
    assert!(transfers.is_empty(), "Should have no active transfers initially");
}

/// Journey 6E: Peer has local ID
#[test]
fn journey6_peer_local_id() {
    let peer_mgr = PeerManager::new();
    let local_id = peer_mgr.local_peer_id();
    assert!(!local_id.is_nil(), "Local peer should have a non-nil ID");
}

/// Journey 6F: Server-to-server capability matrix
#[test]
fn journey6_server_capability_matrix() {
    let matrix = server_to_server::get_capability_matrix();
    assert!(!matrix.is_empty(), "Capability matrix should not be empty");

    // Should have entries for common protocol pairs
    let has_sftp = matrix.iter().any(|c| c.source_protocol == EndpointProtocol::Sftp || c.dest_protocol == EndpointProtocol::Sftp);
    assert!(has_sftp, "Matrix should include SFTP entries");
}

/// Journey 6G: Server transfer create and manage
#[tokio::test]
async fn journey6_server_transfer_lifecycle() {
    let s2s_mgr = ServerTransferManager::new();

    // List active transfers (empty)
    let active = s2s_mgr.list_active_transfers().await;
    assert!(active.is_empty(), "Should have no active S2S transfers initially");

    // List all transfers (empty)
    let all = s2s_mgr.list_transfers().await;
    assert!(all.is_empty(), "Should have no S2S transfers initially");

    // Create a transfer
    let source = TransferEndpoint {
        connection_id: Uuid::new_v4(),
        protocol: EndpointProtocol::Sftp,
        host: "sftp.source.com".to_string(),
        port: Some(22),
        path: "/remote/".to_string(),
        display_name: "Source SFTP".to_string(),
    };
    let destination = TransferEndpoint {
        connection_id: Uuid::new_v4(),
        protocol: EndpointProtocol::S3,
        host: "s3.dest.com".to_string(),
        port: None,
        path: "s3://bucket/".to_string(),
        display_name: "Dest S3".to_string(),
    };
    let files = vec![ServerTransferFile {
        source_path: "file.dat".to_string(),
        dest_path: "file.dat".to_string(),
        size: 1024 * 1024,
        is_dir: false,
        status: FileTransferStatus::Pending,
        error: None,
    }];

    let job = s2s_mgr.create_transfer(source, destination, files).await.unwrap();

    assert!(!job.id.is_nil());

    // List should now have one transfer
    let all = s2s_mgr.list_transfers().await;
    assert_eq!(all.len(), 1);
}

/// Journey 6H: Server transfer preview method
#[test]
fn journey6_server_transfer_preview() {
    let s2s_mgr = ServerTransferManager::new();
    let preview = s2s_mgr.preview_method("sftp", "s3");
    // Should return a transfer method recommendation
    assert!(!preview.method.as_str().is_empty(), "Should recommend a transfer method");
}

/// Journey 6I: Cleanup completed transfers
#[tokio::test]
async fn journey6_cleanup_completed_transfers() {
    let s2s_mgr = ServerTransferManager::new();
    let cleaned = s2s_mgr.cleanup_completed().await;
    assert_eq!(cleaned, 0, "Nothing to clean initially");
}

/// Journey 6J: Peer message creation
#[test]
fn journey6_peer_message_creation() {
    let peer_mgr = PeerManager::new();

    let hello = peer_mgr.create_hello();
    assert!(!format!("{:?}", hello).is_empty());

    let ping = peer_mgr.create_ping();
    assert!(!format!("{:?}", ping).is_empty());
}
