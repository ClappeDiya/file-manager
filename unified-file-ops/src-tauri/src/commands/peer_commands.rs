//! Tauri IPC commands for peer discovery and computer-to-computer transfer (T-031)
//! and server-to-server direct transfer (T-033).

use crate::connectors::peer::{
    ManualPeerConfig, PeerDevice, PeerFileInfo, PeerManager, PeerTransferProgress,
    PeerTransferRequest, SavedPeer, TransferResponse, TrustLevel,
};
use crate::connectors::server_to_server::{
    DirectTransferCapability, ServerTransferFile, ServerTransferJob, ServerTransferManager,
    TransferEndpoint,
};
use crate::core::error::AppError;
use tauri::State;
use uuid::Uuid;

// ──────────────────────────────────────────────
// T-031: Peer Discovery Commands
// ──────────────────────────────────────────────

/// Start mDNS/Bonjour peer discovery on the local network.
#[tauri::command]
pub async fn peer_start_discovery(
    peer_mgr: State<'_, PeerManager>,
) -> Result<(), AppError> {
    peer_mgr.start_discovery().await
}

/// Stop mDNS peer discovery.
#[tauri::command]
pub async fn peer_stop_discovery(
    peer_mgr: State<'_, PeerManager>,
) -> Result<(), AppError> {
    peer_mgr.stop_discovery().await
}

/// Check if peer discovery is currently active.
#[tauri::command]
pub async fn peer_is_discovering(
    peer_mgr: State<'_, PeerManager>,
) -> Result<bool, AppError> {
    Ok(peer_mgr.is_discovering())
}

/// List all discovered peers on the network.
#[tauri::command]
pub async fn peer_list_peers(
    peer_mgr: State<'_, PeerManager>,
) -> Result<Vec<PeerDevice>, AppError> {
    Ok(peer_mgr.list_peers().await)
}

/// List only online peers.
#[tauri::command]
pub async fn peer_list_online(
    peer_mgr: State<'_, PeerManager>,
) -> Result<Vec<PeerDevice>, AppError> {
    Ok(peer_mgr.list_online_peers().await)
}

/// Get details for a specific peer.
#[tauri::command]
pub async fn peer_get_peer(
    peer_id: String,
    peer_mgr: State<'_, PeerManager>,
) -> Result<Option<PeerDevice>, AppError> {
    let id = Uuid::parse_str(&peer_id).map_err(|_| AppError::Connection {
        message: "Invalid peer ID format".to_string(),
        advice: "Peer IDs must be valid UUIDs.".to_string(),
    })?;
    Ok(peer_mgr.get_peer(id).await)
}

// ──────────────────────────────────────────────
// T-031: Trust Management
// ──────────────────────────────────────────────

/// Set the trust level for a peer (trusted, blocked, etc.).
#[tauri::command]
pub async fn peer_set_trust(
    peer_id: String,
    trust: String,
    peer_mgr: State<'_, PeerManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&peer_id).map_err(|_| AppError::Connection {
        message: "Invalid peer ID format".to_string(),
        advice: "Peer IDs must be valid UUIDs.".to_string(),
    })?;
    let trust_level = TrustLevel::from_str_lossy(&trust);
    peer_mgr.set_trust(id, trust_level).await
}

/// Save a peer for future sessions (appears in sidebar).
#[tauri::command]
pub async fn peer_save_peer(
    peer_id: String,
    peer_mgr: State<'_, PeerManager>,
) -> Result<SavedPeer, AppError> {
    let id = Uuid::parse_str(&peer_id).map_err(|_| AppError::Connection {
        message: "Invalid peer ID format".to_string(),
        advice: "Peer IDs must be valid UUIDs.".to_string(),
    })?;
    peer_mgr.save_peer(id).await
}

/// Remove a saved peer.
#[tauri::command]
pub async fn peer_remove_saved(
    peer_id: String,
    peer_mgr: State<'_, PeerManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&peer_id).map_err(|_| AppError::Connection {
        message: "Invalid peer ID format".to_string(),
        advice: "Peer IDs must be valid UUIDs.".to_string(),
    })?;
    peer_mgr.remove_saved_peer(id).await
}

/// List all saved peers.
#[tauri::command]
pub async fn peer_list_saved(
    peer_mgr: State<'_, PeerManager>,
) -> Result<Vec<SavedPeer>, AppError> {
    Ok(peer_mgr.list_saved_peers().await)
}

/// Set a custom display name for a peer.
#[tauri::command]
pub async fn peer_set_display_name(
    peer_id: String,
    name: String,
    peer_mgr: State<'_, PeerManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&peer_id).map_err(|_| AppError::Connection {
        message: "Invalid peer ID format".to_string(),
        advice: "Peer IDs must be valid UUIDs.".to_string(),
    })?;
    peer_mgr.set_peer_display_name(id, name).await
}

// ──────────────────────────────────────────────
// T-031: Manual Connection
// ──────────────────────────────────────────────

/// Manually connect to a peer by IP:port when mDNS is unavailable.
#[tauri::command]
pub async fn peer_connect_manual(
    host: String,
    port: Option<u16>,
    display_name: Option<String>,
    peer_mgr: State<'_, PeerManager>,
) -> Result<PeerDevice, AppError> {
    let config = ManualPeerConfig {
        host,
        port: port.unwrap_or(43210),
        display_name,
    };
    peer_mgr.connect_manual(config).await
}

// ──────────────────────────────────────────────
// T-031: Transfer Operations
// ──────────────────────────────────────────────

/// Request a file transfer to a peer device.
#[tauri::command]
pub async fn peer_request_transfer(
    peer_id: String,
    files: Vec<PeerFileInfo>,
    peer_mgr: State<'_, PeerManager>,
) -> Result<PeerTransferRequest, AppError> {
    let id = Uuid::parse_str(&peer_id).map_err(|_| AppError::Connection {
        message: "Invalid peer ID format".to_string(),
        advice: "Peer IDs must be valid UUIDs.".to_string(),
    })?;
    peer_mgr.request_transfer(id, files).await
}

/// List pending incoming transfer requests awaiting user response.
#[tauri::command]
pub async fn peer_list_pending_requests(
    peer_mgr: State<'_, PeerManager>,
) -> Result<Vec<PeerTransferRequest>, AppError> {
    Ok(peer_mgr.list_pending_requests().await)
}

/// Respond to an incoming transfer request (accept/deny/block).
#[tauri::command]
pub async fn peer_respond_transfer(
    request_id: String,
    response: String,
    save_path: Option<String>,
    peer_mgr: State<'_, PeerManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&request_id).map_err(|_| AppError::Transfer {
        message: "Invalid request ID format".to_string(),
        advice: "Request IDs must be valid UUIDs.".to_string(),
    })?;

    let resp = match response.as_str() {
        "accepted" => TransferResponse::Accepted,
        "denied" => TransferResponse::Denied,
        "blocked" => TransferResponse::Blocked,
        _ => TransferResponse::Denied,
    };

    peer_mgr.respond_to_transfer(id, resp, save_path).await
}

/// Get the progress of a peer transfer.
#[tauri::command]
pub async fn peer_get_transfer_progress(
    request_id: String,
    peer_mgr: State<'_, PeerManager>,
) -> Result<Option<PeerTransferProgress>, AppError> {
    let id = Uuid::parse_str(&request_id).map_err(|_| AppError::Transfer {
        message: "Invalid request ID format".to_string(),
        advice: "Request IDs must be valid UUIDs.".to_string(),
    })?;
    Ok(peer_mgr.get_transfer_progress(id).await)
}

/// List all active peer transfers.
#[tauri::command]
pub async fn peer_list_transfers(
    peer_mgr: State<'_, PeerManager>,
) -> Result<Vec<PeerTransferProgress>, AppError> {
    Ok(peer_mgr.list_transfers().await)
}

/// Cancel a peer transfer.
#[tauri::command]
pub async fn peer_cancel_transfer(
    request_id: String,
    peer_mgr: State<'_, PeerManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&request_id).map_err(|_| AppError::Transfer {
        message: "Invalid request ID format".to_string(),
        advice: "Request IDs must be valid UUIDs.".to_string(),
    })?;
    peer_mgr.cancel_transfer(id).await
}

// ──────────────────────────────────────────────
// T-033: Server-to-Server Transfer Commands
// ──────────────────────────────────────────────

/// Check what transfer method would be used between two protocols.
#[tauri::command]
pub async fn server_transfer_preview_method(
    source_protocol: String,
    dest_protocol: String,
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<DirectTransferCapability, AppError> {
    Ok(server_mgr.preview_method(&source_protocol, &dest_protocol))
}

/// Create a new server-to-server transfer job.
#[tauri::command]
pub async fn server_transfer_create(
    source: TransferEndpoint,
    destination: TransferEndpoint,
    files: Vec<ServerTransferFile>,
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<ServerTransferJob, AppError> {
    server_mgr.create_transfer(source, destination, files).await
}

/// Start a queued server-to-server transfer.
#[tauri::command]
pub async fn server_transfer_start(
    job_id: String,
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&job_id).map_err(|_| AppError::Transfer {
        message: "Invalid job ID format".to_string(),
        advice: "Job IDs must be valid UUIDs.".to_string(),
    })?;
    server_mgr.start_transfer(id).await
}

/// Pause a running server-to-server transfer.
#[tauri::command]
pub async fn server_transfer_pause(
    job_id: String,
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&job_id).map_err(|_| AppError::Transfer {
        message: "Invalid job ID format".to_string(),
        advice: "Job IDs must be valid UUIDs.".to_string(),
    })?;
    server_mgr.pause_transfer(id).await
}

/// Cancel a server-to-server transfer.
#[tauri::command]
pub async fn server_transfer_cancel(
    job_id: String,
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&job_id).map_err(|_| AppError::Transfer {
        message: "Invalid job ID format".to_string(),
        advice: "Job IDs must be valid UUIDs.".to_string(),
    })?;
    server_mgr.cancel_transfer(id).await
}

/// Retry a failed server-to-server transfer.
#[tauri::command]
pub async fn server_transfer_retry(
    job_id: String,
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<(), AppError> {
    let id = Uuid::parse_str(&job_id).map_err(|_| AppError::Transfer {
        message: "Invalid job ID format".to_string(),
        advice: "Job IDs must be valid UUIDs.".to_string(),
    })?;
    server_mgr.retry_transfer(id).await
}

/// Get details of a server-to-server transfer job.
#[tauri::command]
pub async fn server_transfer_get(
    job_id: String,
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<Option<ServerTransferJob>, AppError> {
    let id = Uuid::parse_str(&job_id).map_err(|_| AppError::Transfer {
        message: "Invalid job ID format".to_string(),
        advice: "Job IDs must be valid UUIDs.".to_string(),
    })?;
    Ok(server_mgr.get_transfer(id).await)
}

/// List all server-to-server transfer jobs.
#[tauri::command]
pub async fn server_transfer_list(
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<Vec<ServerTransferJob>, AppError> {
    Ok(server_mgr.list_transfers().await)
}

/// List only active server-to-server transfers.
#[tauri::command]
pub async fn server_transfer_list_active(
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<Vec<ServerTransferJob>, AppError> {
    Ok(server_mgr.list_active_transfers().await)
}

/// Get the full capability matrix showing which protocol pairs support direct transfer.
#[tauri::command]
pub async fn server_transfer_capability_matrix() -> Result<Vec<DirectTransferCapability>, AppError>
{
    Ok(crate::connectors::server_to_server::get_capability_matrix())
}

/// Remove completed/failed/cancelled transfer jobs.
#[tauri::command]
pub async fn server_transfer_cleanup(
    server_mgr: State<'_, ServerTransferManager>,
) -> Result<u32, AppError> {
    Ok(server_mgr.cleanup_completed().await)
}
