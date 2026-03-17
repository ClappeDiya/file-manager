/**
 * Peer Sidebar Component (T-031)
 *
 * Displays discovered peer devices on the LAN:
 * - Hostname, OS icon, IP address, app version
 * - Online/offline status indicator
 * - Trust level (trusted/untrusted/blocked)
 * - "Trust this device" and "Block" actions
 * - Saved peers section (persisted across sessions)
 * - Manual IP:port connection fallback
 * - Quick "Transfer to..." action per peer
 *
 * Used alongside device-sidebar.tsx for the "Peers" / "Nearby" section.
 */

import { useState, useEffect, useCallback } from "react";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { cn } from "@ufop/ui-components";
import { ScrollArea } from "@ufop/ui-components";
import {
  Monitor,
  Laptop,
  Server,
  Wifi,
  WifiOff,
  Shield,
  ShieldCheck,
  ShieldX,
  ShieldQuestion,
  Send,
  Plus,
  ChevronDown,
  ChevronRight,
  RefreshCw,
  MoreHorizontal,
  X,
  Check,
  Loader2,
  ArrowUpDown,
  XCircle,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types (matching Rust peer module)
// ──────────────────────────────────────────────

export type PeerOs = "macos" | "windows" | "linux" | "unknown";
export type PeerStatus = "online" | "offline" | "connecting" | "transferring";
export type TrustLevel = "unknown" | "untrusted" | "trusted" | "blocked";

export interface PeerDeviceData {
  id: string;
  hostname: string;
  os: PeerOs;
  addresses: string[];
  port: number;
  app_version: string;
  status: PeerStatus;
  trust: TrustLevel;
  discovered_at: string;
  last_seen: string;
  display_name: string | null;
  tls_fingerprint: string | null;
}

export interface SavedPeerData {
  id: string;
  hostname: string;
  os: PeerOs;
  last_addresses: string[];
  port: number;
  trust: TrustLevel;
  display_name: string | null;
  tls_fingerprint: string | null;
  last_seen: string;
  saved_at: string;
}

export interface PeerTransferRequestData {
  request_id: string;
  sender_id: string;
  sender_hostname: string;
  sender_os: PeerOs;
  files: { relative_path: string; size: number; is_dir: boolean }[];
  total_bytes: number;
  file_count: number;
  requested_at: string;
}

export interface PeerTransferProgress {
  request_id: string;
  peer_id: string;
  peer_hostname: string;
  direction: "outgoing" | "incoming";
  status: "pending" | "active" | "completed" | "failed" | "cancelled";
  total_bytes: number;
  transferred_bytes: number;
  file_count: number;
  files_transferred: number;
  error: string | null;
  started_at: string;
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

function formatBytes(bytes: number): string {
  const GB = 1024 * 1024 * 1024;
  const MB = 1024 * 1024;
  const KB = 1024;
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= MB) return `${(bytes / MB).toFixed(1)} MB`;
  if (bytes >= KB) return `${(bytes / KB).toFixed(1)} KB`;
  return `${bytes} B`;
}

function getOsIcon(os: PeerOs) {
  switch (os) {
    case "macos":
      return Laptop;
    case "windows":
      return Monitor;
    case "linux":
      return Server;
    default:
      return Monitor;
  }
}

function getOsLabel(os: PeerOs): string {
  switch (os) {
    case "macos":
      return "macOS";
    case "windows":
      return "Windows";
    case "linux":
      return "Linux";
    default:
      return "Unknown";
  }
}

/** Get the appropriate trust icon for a peer's trust level */
export function getTrustIcon(trust: TrustLevel) {
  switch (trust) {
    case "trusted":
      return ShieldCheck;
    case "blocked":
      return ShieldX;
    case "untrusted":
      return ShieldQuestion;
    default:
      return Shield;
  }
}

function getStatusColor(status: PeerStatus): string {
  switch (status) {
    case "online":
      return "bg-[var(--color-success)]";
    case "transferring":
      return "bg-[var(--color-primary)]";
    case "connecting":
      return "bg-[var(--color-warning)]";
    case "offline":
      return "bg-[var(--color-text-tertiary)]";
  }
}

// ──────────────────────────────────────────────
// PeerSidebar Component
// ──────────────────────────────────────────────

interface PeerSidebarProps {
  /** Start peer discovery (invoke Tauri command). */
  onStartDiscovery?: () => Promise<void>;
  /** Stop peer discovery. */
  onStopDiscovery?: () => Promise<void>;
  /** Check if discovery is running. */
  onIsDiscovering?: () => Promise<boolean>;
  /** List discovered peers. */
  onListPeers?: () => Promise<PeerDeviceData[]>;
  /** List saved peers. */
  onListSavedPeers?: () => Promise<SavedPeerData[]>;
  /** List pending incoming transfer requests. */
  onListPendingRequests?: () => Promise<PeerTransferRequestData[]>;
  /** Set trust level for a peer. */
  onSetTrust?: (peerId: string, trust: TrustLevel) => Promise<void>;
  /** Save a peer for future sessions. */
  onSavePeer?: (peerId: string) => Promise<void>;
  /** Remove a saved peer. */
  onRemoveSavedPeer?: (peerId: string) => Promise<void>;
  /** Set display name for a peer. */
  onSetDisplayName?: (peerId: string, name: string) => Promise<void>;
  /** Connect to a peer manually by IP:port. */
  onConnectManual?: (
    host: string,
    port: number,
    displayName?: string,
  ) => Promise<PeerDeviceData>;
  /** Initiate transfer to a peer (or request transfer via Tauri). */
  onTransferTo?: (peerId: string) => void;
  /** Request a file transfer to a peer. */
  onRequestTransfer?: (
    peerId: string,
    files: { relative_path: string; size: number; is_dir: boolean }[],
  ) => Promise<PeerTransferRequestData>;
  /** List active peer transfers. */
  onListTransfers?: () => Promise<PeerTransferProgress[]>;
  /** Get detailed info for a specific peer. */
  onGetPeer?: (peerId: string) => Promise<PeerDeviceData | null>;
  /** Cancel an active peer transfer. */
  onCancelTransfer?: (requestId: string) => Promise<void>;
  /** Respond to an incoming transfer request. */
  onRespondTransfer?: (
    requestId: string,
    response: "accepted" | "denied" | "blocked",
    savePath?: string,
  ) => Promise<void>;
  /** Auto-refresh interval in ms (0 = disabled). */
  refreshInterval?: number;
  className?: string;
}

export function PeerSidebar({
  onStartDiscovery: onStartDiscoveryProp,
  onStopDiscovery: onStopDiscoveryProp,
  onIsDiscovering: onIsDiscoveringProp,
  onListPeers: onListPeersProp,
  onListSavedPeers: onListSavedPeersProp,
  onListPendingRequests: onListPendingRequestsProp,
  onSetTrust: onSetTrustProp,
  onSavePeer: onSavePeerProp,
  onRemoveSavedPeer: onRemoveSavedPeerProp,
  onSetDisplayName: onSetDisplayNameProp,
  onConnectManual: onConnectManualProp,
  onTransferTo: onTransferToProp,
  onRequestTransfer: onRequestTransferProp,
  onListTransfers: onListTransfersProp,
  onGetPeer: onGetPeerProp,
  onCancelTransfer: onCancelTransferProp,
  onRespondTransfer: onRespondTransferProp,
  refreshInterval = 5000,
  className,
}: PeerSidebarProps) {
  // Wire to Tauri IPC when no callback props are provided and Tauri is available
  const onStartDiscovery = onStartDiscoveryProp ?? (isTauriAvailable()
    ? async () => { await tauriInvoke("peer_start_discovery"); }
    : undefined);

  const onStopDiscovery = onStopDiscoveryProp ?? (isTauriAvailable()
    ? async () => { await tauriInvoke("peer_stop_discovery"); }
    : undefined);

  const onIsDiscovering = onIsDiscoveringProp ?? (isTauriAvailable()
    ? async () => tauriInvoke<boolean>("peer_is_discovering", undefined, false)
    : undefined);

  const onListPeers = onListPeersProp ?? (isTauriAvailable()
    ? async () => tauriInvoke<PeerDeviceData[]>("peer_list_peers", undefined, [])
    : undefined);

  const onListSavedPeers = onListSavedPeersProp ?? (isTauriAvailable()
    ? async () => tauriInvoke<SavedPeerData[]>("peer_list_saved", undefined, [])
    : undefined);

  const onListPendingRequests = onListPendingRequestsProp ?? (isTauriAvailable()
    ? async () => tauriInvoke<PeerTransferRequestData[]>("peer_list_pending_requests", undefined, [])
    : undefined);

  const onSetTrust = onSetTrustProp ?? (isTauriAvailable()
    ? async (peerId: string, trust: TrustLevel) => { await tauriInvoke("peer_set_trust", { peerId, trust }); }
    : undefined);

  const onSavePeer = onSavePeerProp ?? (isTauriAvailable()
    ? async (peerId: string) => { await tauriInvoke("peer_save_peer", { peerId }); }
    : undefined);

  const onRemoveSavedPeer = onRemoveSavedPeerProp ?? (isTauriAvailable()
    ? async (peerId: string) => { await tauriInvoke("peer_remove_saved", { peerId }); }
    : undefined);

  const onSetDisplayName = onSetDisplayNameProp ?? (isTauriAvailable()
    ? async (peerId: string, name: string) => { await tauriInvoke("peer_set_display_name", { peerId, name }); }
    : undefined);

  const onConnectManual = onConnectManualProp ?? (isTauriAvailable()
    ? async (host: string, port: number, displayName?: string) =>
        tauriInvoke<PeerDeviceData>("peer_connect_manual", { host, port, displayName })
    : undefined);

  const onRespondTransfer = onRespondTransferProp ?? (isTauriAvailable()
    ? async (requestId: string, response: "accepted" | "denied" | "blocked", savePath?: string) => {
        await tauriInvoke("peer_respond_transfer", { requestId, response, savePath });
      }
    : undefined);

  // Wire peer_request_transfer: fallback for onTransferTo when Tauri is available
  const onRequestTransfer = onRequestTransferProp ?? (isTauriAvailable()
    ? async (peerId: string, files: { relative_path: string; size: number; is_dir: boolean }[]) =>
        tauriInvoke<PeerTransferRequestData>("peer_request_transfer", { peerId, files })
    : undefined);

  // If onTransferTo prop is not provided but Tauri is available, default to requesting
  // transfer with an empty files array (actual file selection happens in a dialog)
  const onTransferTo = onTransferToProp ?? (isTauriAvailable() && onRequestTransfer
    ? (peerId: string) => {
        onRequestTransfer(peerId, []).catch((err) => {
          console.error("peer_request_transfer failed:", err);
        });
      }
    : undefined);

  // Wire peer_list_transfers: periodically fetch active peer transfers
  const onListTransfers = onListTransfersProp ?? (isTauriAvailable()
    ? async () => tauriInvoke<PeerTransferProgress[]>("peer_list_transfers", undefined, [])
    : undefined);

  // Wire peer_get_peer: retrieve detailed info for a specific peer
  const onGetPeer = onGetPeerProp ?? (isTauriAvailable()
    ? async (peerId: string) => tauriInvoke<PeerDeviceData | null>("peer_get_peer", { peerId }, null)
    : undefined);

  // Wire peer_cancel_transfer: cancel an active peer transfer
  const onCancelTransfer = onCancelTransferProp ?? (isTauriAvailable()
    ? async (requestId: string) => { await tauriInvoke("peer_cancel_transfer", { requestId }); }
    : undefined);

  const [peers, setPeers] = useState<PeerDeviceData[]>([]);
  const [savedPeers, setSavedPeers] = useState<SavedPeerData[]>([]);
  const [pendingRequests, setPendingRequests] = useState<
    PeerTransferRequestData[]
  >([]);
  const [activeTransfers, setActiveTransfers] = useState<PeerTransferProgress[]>([]);
  const [discovering, setDiscovering] = useState(false);
  const [loading, setLoading] = useState(false);
  const [showManualConnect, setShowManualConnect] = useState(false);
  const [expandedPeerId, setExpandedPeerId] = useState<string | null>(null);
  const [expandedPeerDetail, setExpandedPeerDetail] = useState<PeerDeviceData | null>(null);

  const [expandedSections, setExpandedSections] = useState<Set<string>>(
    new Set(["online", "saved", "requests", "transfers"]),
  );

  // Load peers and active transfers
  const refresh = useCallback(async () => {
    try {
      if (onListPeers) {
        const p = await onListPeers();
        setPeers(p);
      }
      if (onListSavedPeers) {
        const s = await onListSavedPeers();
        setSavedPeers(s);
      }
      if (onListPendingRequests) {
        const r = await onListPendingRequests();
        setPendingRequests(r);
      }
      if (onListTransfers) {
        const t = await onListTransfers();
        setActiveTransfers(t);
      }
      if (onIsDiscovering) {
        const d = await onIsDiscovering();
        setDiscovering(d);
      }
    } catch {
      // Non-critical; sidebar will just show stale data
    }
  }, [onListPeers, onListSavedPeers, onListPendingRequests, onListTransfers, onIsDiscovering]);

  // Fetch peer details when expanding a peer row
  const fetchPeerDetail = useCallback(async (peerId: string) => {
    if (expandedPeerId === peerId) {
      setExpandedPeerId(null);
      setExpandedPeerDetail(null);
      return;
    }
    setExpandedPeerId(peerId);
    setExpandedPeerDetail(null);
    if (onGetPeer) {
      try {
        const detail = await onGetPeer(peerId);
        setExpandedPeerDetail(detail);
      } catch {
        // Peer detail fetch failed; leave null
      }
    }
  }, [expandedPeerId, onGetPeer]);

  // Initial load + auto-refresh
  useEffect(() => {
    refresh();
    if (refreshInterval > 0) {
      const interval = setInterval(refresh, refreshInterval);
      return () => clearInterval(interval);
    }
  }, [refresh, refreshInterval]);

  // Toggle discovery
  const toggleDiscovery = useCallback(async () => {
    setLoading(true);
    try {
      if (discovering) {
        if (onStopDiscovery) await onStopDiscovery();
        setDiscovering(false);
      } else {
        if (onStartDiscovery) await onStartDiscovery();
        setDiscovering(true);
      }
    } catch {
      // Discovery toggle failed
    } finally {
      setLoading(false);
    }
  }, [discovering, onStartDiscovery, onStopDiscovery]);

  const toggleSection = useCallback((section: string) => {
    setExpandedSections((prev) => {
      const next = new Set(prev);
      if (next.has(section)) next.delete(section);
      else next.add(section);
      return next;
    });
  }, []);

  // Group peers
  const onlinePeers = peers.filter(
    (p) => p.status === "online" || p.status === "transferring",
  );
  const offlinePeers = peers.filter(
    (p) => p.status === "offline" || p.status === "connecting",
  );

  // Saved peers that are NOT currently discovered
  const savedOnlyPeers = savedPeers.filter(
    (sp) => !peers.some((p) => p.id === sp.id),
  );

  return (
    <ScrollArea
      className={cn("h-full", className)}
      data-testid="peer-sidebar"
    >
      <div className="py-2">
        {/* Header */}
        <div className="flex items-center justify-between px-3 pb-1">
          <span className="text-[length:var(--font-size-xs)] font-semibold uppercase tracking-wider text-[color:var(--color-text-secondary)]">
            Nearby Devices
          </span>
          <div className="flex items-center gap-1">
            <button
              className={cn(
                "h-5 w-5 flex items-center justify-center rounded-sm",
                "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
                "transition-theme",
              )}
              onClick={() => setShowManualConnect(!showManualConnect)}
              aria-label="Add peer manually"
              title="Connect by IP address"
            >
              <Plus className="h-3.5 w-3.5" />
            </button>
            <button
              className={cn(
                "h-5 w-5 flex items-center justify-center rounded-sm",
                "transition-theme",
                discovering
                  ? "text-[color:var(--color-primary)]"
                  : "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
                loading && "animate-spin",
              )}
              onClick={toggleDiscovery}
              aria-label={discovering ? "Stop discovery" : "Start discovery"}
              title={discovering ? "Stop scanning" : "Scan for devices"}
            >
              {discovering ? (
                <Wifi className="h-3.5 w-3.5" />
              ) : (
                <WifiOff className="h-3.5 w-3.5" />
              )}
            </button>
            <button
              className={cn(
                "h-5 w-5 flex items-center justify-center rounded-sm",
                "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
                "transition-theme",
              )}
              onClick={refresh}
              aria-label="Refresh peers"
            >
              <RefreshCw className="h-3.5 w-3.5" />
            </button>
          </div>
        </div>

        {/* Manual connect form */}
        {showManualConnect && (
          <ManualConnectForm
            onConnect={onConnectManual}
            onClose={() => setShowManualConnect(false)}
            onRefresh={refresh}
          />
        )}

        {/* Pending incoming transfer requests */}
        {pendingRequests.length > 0 && (
          <PeerSection
            title={`Requests (${pendingRequests.length})`}
            expanded={expandedSections.has("requests")}
            onToggle={() => toggleSection("requests")}
            testId="peers-requests"
            highlight
          >
            {pendingRequests.map((req) => (
              <IncomingRequestRow
                key={req.request_id}
                request={req}
                onRespond={onRespondTransfer}
              />
            ))}
          </PeerSection>
        )}

        {/* Online peers */}
        {onlinePeers.length > 0 && (
          <PeerSection
            title={`Online (${onlinePeers.length})`}
            expanded={expandedSections.has("online")}
            onToggle={() => toggleSection("online")}
            testId="peers-online"
          >
            {onlinePeers.map((peer) => (
              <PeerRow
                key={peer.id}
                peer={peer}
                onTransferTo={onTransferTo}
                onSetTrust={onSetTrust}
                onSavePeer={onSavePeer}
                onSetDisplayName={onSetDisplayName}
                onRefresh={refresh}
                expanded={expandedPeerId === peer.id}
                expandedDetail={expandedPeerId === peer.id ? expandedPeerDetail : null}
                onToggleExpand={fetchPeerDetail}
              />
            ))}
          </PeerSection>
        )}

        {/* Saved peers (offline) */}
        {savedOnlyPeers.length > 0 && (
          <PeerSection
            title="Saved"
            expanded={expandedSections.has("saved")}
            onToggle={() => toggleSection("saved")}
            testId="peers-saved"
          >
            {savedOnlyPeers.map((sp) => (
              <SavedPeerRow
                key={sp.id}
                savedPeer={sp}
                onRemove={onRemoveSavedPeer}
                onRefresh={refresh}
              />
            ))}
          </PeerSection>
        )}

        {/* Offline peers */}
        {offlinePeers.length > 0 && (
          <PeerSection
            title="Offline"
            expanded={expandedSections.has("offline")}
            onToggle={() => toggleSection("offline")}
            testId="peers-offline"
          >
            {offlinePeers.map((peer) => (
              <PeerRow
                key={peer.id}
                peer={peer}
                onSetTrust={onSetTrust}
                onSavePeer={onSavePeer}
                onSetDisplayName={onSetDisplayName}
                onRefresh={refresh}
                expanded={expandedPeerId === peer.id}
                expandedDetail={expandedPeerId === peer.id ? expandedPeerDetail : null}
                onToggleExpand={fetchPeerDetail}
              />
            ))}
          </PeerSection>
        )}

        {/* Active Transfers */}
        {activeTransfers.length > 0 && (
          <PeerSection
            title={`Transfers (${activeTransfers.length})`}
            expanded={expandedSections.has("transfers")}
            onToggle={() => toggleSection("transfers")}
            testId="peers-transfers"
          >
            {activeTransfers.map((transfer) => (
              <ActiveTransferRow
                key={transfer.request_id}
                transfer={transfer}
                onCancel={onCancelTransfer}
                onRefresh={refresh}
              />
            ))}
          </PeerSection>
        )}

        {/* No peers message */}
        {peers.length === 0 && savedPeers.length === 0 && (
          <div className="px-4 py-6 text-center">
            <Wifi className="h-8 w-8 mx-auto mb-2 text-[color:var(--color-text-tertiary)]" />
            <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              {discovering
                ? "Scanning for nearby devices..."
                : "No devices found. Start scanning or connect manually."}
            </p>
            {!discovering && (
              <button
                onClick={toggleDiscovery}
                className={cn(
                  "mt-2 px-3 py-1.5 rounded-md text-[length:var(--font-size-xs)]",
                  "bg-[var(--color-primary)] text-white",
                  "hover:opacity-90 transition-theme",
                )}
              >
                Start Scanning
              </button>
            )}
          </div>
        )}
      </div>
    </ScrollArea>
  );
}

// ──────────────────────────────────────────────
// Peer Section (collapsible)
// ──────────────────────────────────────────────

interface PeerSectionProps {
  title: string;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
  testId?: string;
  highlight?: boolean;
}

function PeerSection({
  title,
  expanded,
  onToggle,
  children,
  testId,
  highlight,
}: PeerSectionProps) {
  return (
    <div className="mb-1" data-testid={testId}>
      <button
        className={cn(
          "flex items-center gap-1.5 w-full px-3 py-1",
          "text-[10px] font-medium uppercase tracking-wider",
          highlight
            ? "text-[color:var(--color-primary)]"
            : "text-[color:var(--color-text-tertiary)]",
          "hover:text-[color:var(--color-text-secondary)] transition-theme",
        )}
        onClick={onToggle}
        aria-expanded={expanded}
      >
        {expanded ? (
          <ChevronDown className="h-2.5 w-2.5" />
        ) : (
          <ChevronRight className="h-2.5 w-2.5" />
        )}
        {title}
      </button>
      {expanded && <div role="group">{children}</div>}
    </div>
  );
}

// ──────────────────────────────────────────────
// Peer Row
// ──────────────────────────────────────────────

interface PeerRowProps {
  peer: PeerDeviceData;
  onTransferTo?: (peerId: string) => void;
  onSetTrust?: (peerId: string, trust: TrustLevel) => Promise<void>;
  onSavePeer?: (peerId: string) => Promise<void>;
  onSetDisplayName?: (peerId: string, name: string) => Promise<void>;
  onRefresh: () => void;
  expanded?: boolean;
  expandedDetail?: PeerDeviceData | null;
  onToggleExpand?: (peerId: string) => void;
}

function PeerRow({
  peer,
  onTransferTo,
  onSetTrust,
  onSavePeer,
  onSetDisplayName: _onSetDisplayName,
  onRefresh,
  expanded,
  expandedDetail,
  onToggleExpand,
}: PeerRowProps) {
  const [showMenu, setShowMenu] = useState(false);
  const OsIcon = getOsIcon(peer.os);
  // TrustIcon available via getTrustIcon(peer.trust) for future use
  const displayName = peer.display_name || peer.hostname;
  const isOnline = peer.status === "online" || peer.status === "transferring";

  const handleTrust = useCallback(
    async (trust: TrustLevel) => {
      if (onSetTrust) {
        await onSetTrust(peer.id, trust);
        onRefresh();
      }
      setShowMenu(false);
    },
    [peer.id, onSetTrust, onRefresh],
  );

  const handleSave = useCallback(async () => {
    if (onSavePeer) {
      await onSavePeer(peer.id);
      onRefresh();
    }
    setShowMenu(false);
  }, [peer.id, onSavePeer, onRefresh]);

  return (
    <div data-testid={`peer-${peer.id}`}>
      <div
        className={cn(
          "group flex items-center gap-2 px-3 py-1.5",
          "hover:bg-[var(--color-hover-bg)] transition-theme",
          !isOnline && "opacity-50",
        )}
      >
        {/* Icon with status dot */}
        <div
          className="relative shrink-0 cursor-pointer"
          onClick={() => onToggleExpand?.(peer.id)}
          role="button"
          tabIndex={0}
          aria-label={`${expanded ? "Collapse" : "Expand"} details for ${displayName}`}
        >
          <OsIcon className="h-4 w-4 text-[color:var(--color-text-secondary)]" />
          <div
            className={cn(
              "absolute -bottom-0.5 -right-0.5 w-2 h-2 rounded-full border border-[var(--color-panel-bg)]",
              getStatusColor(peer.status),
            )}
          />
        </div>

        {/* Info */}
        <div
          className="flex-1 min-w-0 cursor-pointer"
          onClick={() => onToggleExpand?.(peer.id)}
          role="button"
          tabIndex={0}
        >
          <div className="flex items-center gap-1">
            <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text)] truncate block">
              {displayName}
            </span>
            {peer.trust === "trusted" && (
              <ShieldCheck className="h-3 w-3 text-[color:var(--color-success)] shrink-0" />
            )}
            {peer.trust === "blocked" && (
              <ShieldX className="h-3 w-3 text-[color:var(--color-error)] shrink-0" />
            )}
          </div>
          <span className="text-[9px] text-[color:var(--color-text-tertiary)] truncate block">
            {getOsLabel(peer.os)}
            {peer.addresses.length > 0 && ` - ${peer.addresses[0]}`}
          </span>
        </div>

        {/* Actions */}
        <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-theme">
          {isOnline && onTransferTo && peer.trust !== "blocked" && (
            <button
              className="h-5 w-5 flex items-center justify-center rounded-sm text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-primary)]"
              onClick={() => onTransferTo(peer.id)}
              title="Transfer files to this device"
              aria-label={`Transfer to ${displayName}`}
            >
              <Send className="h-3 w-3" />
            </button>
          )}
          <div className="relative">
            <button
              className="h-5 w-5 flex items-center justify-center rounded-sm text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]"
              onClick={() => setShowMenu(!showMenu)}
              aria-label="Peer options"
            >
              <MoreHorizontal className="h-3 w-3" />
            </button>
            {showMenu && (
              <PeerContextMenu
                peer={peer}
                onTrust={handleTrust}
                onSave={handleSave}
                onClose={() => setShowMenu(false)}
              />
            )}
          </div>
        </div>
      </div>

      {/* Expanded peer detail (via peer_get_peer) */}
      {expanded && (
        <div className="px-4 py-2 mx-2 mb-1 rounded-md bg-[var(--color-hover-bg)] border border-[var(--color-border)]">
          {expandedDetail ? (
            <div className="space-y-1">
              <div className="text-[9px] text-[color:var(--color-text-tertiary)]">
                <span className="font-medium text-[color:var(--color-text-secondary)]">ID:</span>{" "}
                <span className="font-mono">{expandedDetail.id}</span>
              </div>
              <div className="text-[9px] text-[color:var(--color-text-tertiary)]">
                <span className="font-medium text-[color:var(--color-text-secondary)]">Version:</span>{" "}
                {expandedDetail.app_version}
              </div>
              <div className="text-[9px] text-[color:var(--color-text-tertiary)]">
                <span className="font-medium text-[color:var(--color-text-secondary)]">Addresses:</span>{" "}
                {expandedDetail.addresses.join(", ") || "none"}
              </div>
              <div className="text-[9px] text-[color:var(--color-text-tertiary)]">
                <span className="font-medium text-[color:var(--color-text-secondary)]">Port:</span>{" "}
                {expandedDetail.port}
              </div>
              <div className="text-[9px] text-[color:var(--color-text-tertiary)]">
                <span className="font-medium text-[color:var(--color-text-secondary)]">Trust:</span>{" "}
                {expandedDetail.trust}
              </div>
              {expandedDetail.tls_fingerprint && (
                <div className="text-[9px] text-[color:var(--color-text-tertiary)]">
                  <span className="font-medium text-[color:var(--color-text-secondary)]">TLS:</span>{" "}
                  <span className="font-mono truncate block">{expandedDetail.tls_fingerprint}</span>
                </div>
              )}
              <div className="text-[9px] text-[color:var(--color-text-tertiary)]">
                <span className="font-medium text-[color:var(--color-text-secondary)]">Last seen:</span>{" "}
                {new Date(expandedDetail.last_seen).toLocaleString()}
              </div>
            </div>
          ) : (
            <div className="flex items-center gap-1 text-[9px] text-[color:var(--color-text-tertiary)]">
              <Loader2 className="h-3 w-3 animate-spin" />
              Loading details...
            </div>
          )}
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Peer Context Menu
// ──────────────────────────────────────────────

function PeerContextMenu({
  peer,
  onTrust,
  onSave,
  onClose,
}: {
  peer: PeerDeviceData;
  onTrust: (trust: TrustLevel) => void;
  onSave: () => void;
  onClose: () => void;
}) {
  return (
    <div
      className={cn(
        "absolute right-0 top-6 z-50 w-40",
        "bg-[var(--color-panel-bg)] border border-[var(--color-border)]",
        "rounded-lg shadow-lg py-1",
      )}
    >
      {peer.trust !== "trusted" && (
        <button
          className="w-full px-3 py-1.5 text-left text-[length:var(--font-size-xs)] hover:bg-[var(--color-hover-bg)] flex items-center gap-2"
          onClick={() => onTrust("trusted")}
        >
          <ShieldCheck className="h-3 w-3 text-[color:var(--color-success)]" />
          Trust Device
        </button>
      )}
      {peer.trust !== "blocked" && (
        <button
          className="w-full px-3 py-1.5 text-left text-[length:var(--font-size-xs)] hover:bg-[var(--color-hover-bg)] flex items-center gap-2"
          onClick={() => onTrust("blocked")}
        >
          <ShieldX className="h-3 w-3 text-[color:var(--color-error)]" />
          Block Device
        </button>
      )}
      {peer.trust === "trusted" && (
        <button
          className="w-full px-3 py-1.5 text-left text-[length:var(--font-size-xs)] hover:bg-[var(--color-hover-bg)] flex items-center gap-2"
          onClick={() => onTrust("untrusted")}
        >
          <ShieldQuestion className="h-3 w-3" />
          Untrust Device
        </button>
      )}
      {peer.trust === "blocked" && (
        <button
          className="w-full px-3 py-1.5 text-left text-[length:var(--font-size-xs)] hover:bg-[var(--color-hover-bg)] flex items-center gap-2"
          onClick={() => onTrust("untrusted")}
        >
          <Shield className="h-3 w-3" />
          Unblock Device
        </button>
      )}
      <div className="border-t border-[var(--color-border)] my-1" />
      <button
        className="w-full px-3 py-1.5 text-left text-[length:var(--font-size-xs)] hover:bg-[var(--color-hover-bg)]"
        onClick={onSave}
      >
        Save to Sidebar
      </button>
      <div className="border-t border-[var(--color-border)] my-1" />
      <button
        className="w-full px-3 py-1.5 text-left text-[length:var(--font-size-xs)] hover:bg-[var(--color-hover-bg)] text-[color:var(--color-text-tertiary)]"
        onClick={onClose}
      >
        Close
      </button>
    </div>
  );
}

// ──────────────────────────────────────────────
// Saved Peer Row (offline)
// ──────────────────────────────────────────────

function SavedPeerRow({
  savedPeer,
  onRemove,
  onRefresh,
}: {
  savedPeer: SavedPeerData;
  onRemove?: (peerId: string) => Promise<void>;
  onRefresh: () => void;
}) {
  const OsIcon = getOsIcon(savedPeer.os);
  const displayName = savedPeer.display_name || savedPeer.hostname;

  const handleRemove = useCallback(async () => {
    if (onRemove) {
      await onRemove(savedPeer.id);
      onRefresh();
    }
  }, [savedPeer.id, onRemove, onRefresh]);

  return (
    <div
      className={cn(
        "group flex items-center gap-2 px-3 py-1.5 opacity-50",
        "hover:bg-[var(--color-hover-bg)] transition-theme",
      )}
    >
      <div className="relative shrink-0">
        <OsIcon className="h-4 w-4 text-[color:var(--color-text-secondary)]" />
        <div className="absolute -bottom-0.5 -right-0.5 w-2 h-2 rounded-full border border-[var(--color-panel-bg)] bg-[var(--color-text-tertiary)]" />
      </div>
      <div className="flex-1 min-w-0">
        <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text)] truncate block">
          {displayName}
        </span>
        <span className="text-[9px] text-[color:var(--color-text-tertiary)]">
          {getOsLabel(savedPeer.os)} - Last seen:{" "}
          {new Date(savedPeer.last_seen).toLocaleDateString()}
        </span>
      </div>
      {onRemove && (
        <button
          className="opacity-0 group-hover:opacity-100 h-5 w-5 flex items-center justify-center rounded-sm text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-error)] transition-theme"
          onClick={handleRemove}
          title="Remove saved peer"
          aria-label={`Remove ${displayName}`}
        >
          <X className="h-3 w-3" />
        </button>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Incoming Transfer Request Row
// ──────────────────────────────────────────────

function IncomingRequestRow({
  request,
  onRespond,
}: {
  request: PeerTransferRequestData;
  onRespond?: (
    requestId: string,
    response: "accepted" | "denied" | "blocked",
    savePath?: string,
  ) => Promise<void>;
}) {
  const [responding, setResponding] = useState(false);
  const OsIcon = getOsIcon(request.sender_os);

  const handleRespond = useCallback(
    async (response: "accepted" | "denied" | "blocked") => {
      if (!onRespond) return;
      setResponding(true);
      try {
        await onRespond(request.request_id, response);
      } catch {
        // Response failed
      } finally {
        setResponding(false);
      }
    },
    [request.request_id, onRespond],
  );

  return (
    <div
      className={cn(
        "px-3 py-2 mx-2 my-1 rounded-lg",
        "border border-[var(--color-primary)]/30",
        "bg-[var(--color-primary)]/5",
      )}
      data-testid={`request-${request.request_id}`}
    >
      <div className="flex items-center gap-2 mb-1.5">
        <OsIcon className="h-4 w-4 text-[color:var(--color-primary)] shrink-0" />
        <span className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text)] truncate">
          {request.sender_hostname}
        </span>
      </div>

      <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] mb-2">
        wants to send you{" "}
        <span className="font-medium">
          {request.file_count} file{request.file_count !== 1 ? "s" : ""}
        </span>{" "}
        ({formatBytes(request.total_bytes)})
      </p>

      {responding ? (
        <div className="flex items-center gap-2 text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
          <Loader2 className="h-3 w-3 animate-spin" />
          Responding...
        </div>
      ) : (
        <div className="flex items-center gap-2">
          <button
            onClick={() => handleRespond("accepted")}
            className="flex-1 px-2 py-1 text-[length:var(--font-size-xs)] rounded-md bg-[var(--color-success)] text-white hover:opacity-90 transition-theme flex items-center justify-center gap-1"
          >
            <Check className="h-3 w-3" />
            Accept
          </button>
          <button
            onClick={() => handleRespond("denied")}
            className="flex-1 px-2 py-1 text-[length:var(--font-size-xs)] rounded-md border border-[var(--color-border)] text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)] transition-theme flex items-center justify-center gap-1"
          >
            <X className="h-3 w-3" />
            Deny
          </button>
          <button
            onClick={() => handleRespond("blocked")}
            className="px-2 py-1 text-[length:var(--font-size-xs)] rounded-md text-[color:var(--color-error)] hover:bg-[var(--color-error)]/10 transition-theme"
            title="Block this device"
          >
            <ShieldX className="h-3 w-3" />
          </button>
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Active Transfer Row
// ──────────────────────────────────────────────

function ActiveTransferRow({
  transfer,
  onCancel,
  onRefresh,
}: {
  transfer: PeerTransferProgress;
  onCancel?: (requestId: string) => Promise<void>;
  onRefresh: () => void;
}) {
  const [cancelling, setCancelling] = useState(false);
  const progressPct =
    transfer.total_bytes > 0
      ? Math.round((transfer.transferred_bytes / transfer.total_bytes) * 100)
      : 0;

  const isActive = transfer.status === "active" || transfer.status === "pending";

  const handleCancel = useCallback(async () => {
    if (!onCancel) return;
    setCancelling(true);
    try {
      await onCancel(transfer.request_id);
      onRefresh();
    } catch {
      // Cancel failed
    } finally {
      setCancelling(false);
    }
  }, [transfer.request_id, onCancel, onRefresh]);

  const statusColor = (() => {
    switch (transfer.status) {
      case "active": return "text-[color:var(--color-primary)]";
      case "completed": return "text-[color:var(--color-success)]";
      case "failed": return "text-[color:var(--color-error)]";
      case "cancelled": return "text-[color:var(--color-text-tertiary)]";
      default: return "text-[color:var(--color-warning)]";
    }
  })();

  return (
    <div
      className={cn(
        "px-3 py-2 mx-2 my-1 rounded-lg",
        "border border-[var(--color-border)]",
        "bg-[var(--color-hover-bg)]",
      )}
      data-testid={`transfer-${transfer.request_id}`}
    >
      <div className="flex items-center justify-between mb-1">
        <div className="flex items-center gap-1.5 min-w-0">
          <ArrowUpDown className="h-3 w-3 text-[color:var(--color-text-tertiary)] shrink-0" />
          <span className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text)] truncate">
            {transfer.peer_hostname}
          </span>
          <span className={cn("text-[9px] capitalize", statusColor)}>
            {transfer.status}
          </span>
        </div>
        <span className="text-[9px] text-[color:var(--color-text-tertiary)] capitalize shrink-0">
          {transfer.direction}
        </span>
      </div>

      {/* Progress bar */}
      <div className="w-full h-1.5 rounded-full bg-[var(--color-border)] mb-1">
        <div
          className={cn(
            "h-full rounded-full transition-all",
            transfer.status === "completed"
              ? "bg-[var(--color-success)]"
              : transfer.status === "failed"
                ? "bg-[var(--color-error)]"
                : "bg-[var(--color-primary)]",
          )}
          style={{ width: `${progressPct}%` }}
        />
      </div>

      <div className="flex items-center justify-between">
        <span className="text-[9px] text-[color:var(--color-text-tertiary)]">
          {formatBytes(transfer.transferred_bytes)} / {formatBytes(transfer.total_bytes)}
          {" "}({transfer.files_transferred}/{transfer.file_count} files)
        </span>
        {isActive && onCancel && (
          <button
            onClick={handleCancel}
            disabled={cancelling}
            className={cn(
              "flex items-center gap-0.5 px-1.5 py-0.5 rounded text-[9px]",
              "text-[color:var(--color-error)] hover:bg-[var(--color-error)]/10",
              "transition-theme disabled:opacity-50",
            )}
            title="Cancel transfer"
            aria-label={`Cancel transfer to ${transfer.peer_hostname}`}
          >
            {cancelling ? (
              <Loader2 className="h-2.5 w-2.5 animate-spin" />
            ) : (
              <XCircle className="h-2.5 w-2.5" />
            )}
            Cancel
          </button>
        )}
      </div>

      {transfer.error && (
        <p className="mt-1 text-[9px] text-[color:var(--color-error)] truncate">
          {transfer.error}
        </p>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Manual Connect Form
// ──────────────────────────────────────────────

function ManualConnectForm({
  onConnect,
  onClose,
  onRefresh,
}: {
  onConnect?: (
    host: string,
    port: number,
    displayName?: string,
  ) => Promise<PeerDeviceData>;
  onClose: () => void;
  onRefresh: () => void;
}) {
  const [host, setHost] = useState("");
  const [port, setPort] = useState("43210");
  const [name, setName] = useState("");
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSubmit = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      if (!onConnect || !host.trim()) return;

      setConnecting(true);
      setError(null);

      try {
        await onConnect(
          host.trim(),
          parseInt(port, 10) || 43210,
          name.trim() || undefined,
        );
        onRefresh();
        onClose();
      } catch (err) {
        setError(
          err instanceof Error ? err.message : "Failed to connect",
        );
      } finally {
        setConnecting(false);
      }
    },
    [host, port, name, onConnect, onRefresh, onClose],
  );

  return (
    <form
      onSubmit={handleSubmit}
      className="mx-3 my-2 p-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-hover-bg)]"
      data-testid="manual-connect-form"
    >
      <div className="flex items-center justify-between mb-2">
        <span className="text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text)]">
          Connect by IP Address
        </span>
        <button
          type="button"
          onClick={onClose}
          className="h-4 w-4 flex items-center justify-center text-[color:var(--color-text-tertiary)]"
        >
          <X className="h-3 w-3" />
        </button>
      </div>

      <div className="space-y-2">
        <div className="flex gap-2">
          <input
            type="text"
            value={host}
            onChange={(e) => setHost(e.target.value)}
            placeholder="IP Address"
            className="flex-1 px-2 py-1 text-[length:var(--font-size-xs)] rounded border border-[var(--color-border)] bg-[var(--color-panel-bg)] text-[color:var(--color-text)]"
            required
          />
          <input
            type="text"
            value={port}
            onChange={(e) => setPort(e.target.value)}
            placeholder="Port"
            className="w-16 px-2 py-1 text-[length:var(--font-size-xs)] rounded border border-[var(--color-border)] bg-[var(--color-panel-bg)] text-[color:var(--color-text)]"
          />
        </div>

        <input
          type="text"
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder="Display name (optional)"
          className="w-full px-2 py-1 text-[length:var(--font-size-xs)] rounded border border-[var(--color-border)] bg-[var(--color-panel-bg)] text-[color:var(--color-text)]"
        />

        {error && (
          <p className="text-[length:var(--font-size-xs)] text-[color:var(--color-error)]">
            {error}
          </p>
        )}

        <button
          type="submit"
          disabled={connecting || !host.trim()}
          className="w-full px-2 py-1.5 text-[length:var(--font-size-xs)] rounded-md bg-[var(--color-primary)] text-white hover:opacity-90 transition-theme disabled:opacity-50 flex items-center justify-center gap-1"
        >
          {connecting ? (
            <Loader2 className="h-3 w-3 animate-spin" />
          ) : (
            <Plus className="h-3 w-3" />
          )}
          Connect
        </button>
      </div>
    </form>
  );
}
