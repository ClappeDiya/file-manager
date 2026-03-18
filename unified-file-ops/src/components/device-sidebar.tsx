/**
 * Device Sidebar Component (T-025)
 *
 * Displays all connected drives with metadata:
 * - Drive name, icon, total/free space, filesystem type
 * - Usage bar with color coding (green/yellow/red)
 * - Eject button for removable drives
 * - Network section for SMB/NFS mounts
 *
 * Used alongside sidebar-nav.tsx for the "Devices" section.
 */

import { useState, useEffect, useCallback } from "react";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { cn } from "@ufop/ui-components";
import { ScrollArea } from "@ufop/ui-components";
import {
  HardDrive,
  Usb,
  Wifi,
  Disc,
  Server,
  Power,
  RefreshCw,
  ChevronDown,
  ChevronRight,
  AlertTriangle,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types (matching Rust DriveInfo)
// ──────────────────────────────────────────────

export interface DriveInfoData {
  name: string;
  mount_point: string;
  device: string;
  total_bytes: number;
  free_bytes: number;
  used_bytes: number;
  filesystem: string | { unknown: string };
  drive_type: string;
  removable: boolean;
  read_only: boolean;
  label: string | null;
  uuid: string | null;
}

export interface PreflightResultData {
  can_proceed: boolean;
  warnings: PreflightWarningData[];
  errors: PreflightErrorData[];
  source_drive: DriveInfoData | null;
  dest_drive: DriveInfoData | null;
}

export interface PreflightWarningData {
  code: string;
  message: string;
  detail: string | null;
}

export interface PreflightErrorData {
  code: string;
  message: string;
  detail: string | null;
}

export interface SmbShareData {
  host: string;
  share_name: string;
  share_type: string;
  comment: string | null;
  is_nas: boolean;
  workgroup: string | null;
}

export interface SmbDiscoveryData {
  shares: SmbShareData[];
  scan_duration_ms: number;
  hosts_scanned: number;
  errors: string[];
}

export interface NfsExportData {
  path: string;
  allowed_clients: string[];
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

/** Format bytes to human-readable string (e.g., "256.0 GB"). */
function formatBytes(bytes: number): string {
  const TB = 1024 * 1024 * 1024 * 1024;
  const GB = 1024 * 1024 * 1024;
  const MB = 1024 * 1024;
  const KB = 1024;

  if (bytes >= TB) return `${(bytes / TB).toFixed(1)} TB`;
  if (bytes >= GB) return `${(bytes / GB).toFixed(1)} GB`;
  if (bytes >= MB) return `${(bytes / MB).toFixed(1)} MB`;
  if (bytes >= KB) return `${(bytes / KB).toFixed(1)} KB`;
  return `${bytes} B`;
}

/** Get the filesystem type display string. */
function getFilesystemDisplay(fs: string | { unknown: string }): string {
  if (typeof fs === "string") {
    const map: Record<string, string> = {
      apfs: "APFS",
      hfs: "HFS+",
      ntfs: "NTFS",
      ext4: "ext4",
      ext3: "ext3",
      btrfs: "Btrfs",
      xfs: "XFS",
      zfs: "ZFS",
      fat32: "FAT32",
      exfat: "exFAT",
      ufs: "UFS",
    };
    return map[fs] || fs.toUpperCase();
  }
  return (fs as { unknown: string }).unknown || "Unknown";
}

/** Get the icon component for a drive type. */
function getDriveIcon(driveType: string, removable: boolean) {
  if (removable) return Usb;
  switch (driveType) {
    case "external":
      return Usb;
    case "network":
      return Wifi;
    case "optical":
      return Disc;
    case "virtual":
      return Server;
    default:
      return HardDrive;
  }
}

/** Get usage percentage color class. */
function getUsageColor(percent: number): string {
  if (percent > 90) return "bg-[var(--color-error)]";
  if (percent > 70) return "bg-[var(--color-warning)]";
  return "bg-[var(--color-primary)]";
}

// ──────────────────────────────────────────────
// DeviceSidebar Component
// ──────────────────────────────────────────────

interface DeviceSidebarProps {
  /** Callback when a drive is clicked to navigate to it */
  onNavigate: (path: string) => void;
  /** Callback to detect drives (invoke Tauri command) */
  onDetectDrives?: () => Promise<DriveInfoData[]>;
  /** Callback to eject a drive (invoke Tauri command) */
  onEjectDrive?: (mountPoint: string) => Promise<void>;
  /** Callback to discover SMB shares (invoke Tauri command) */
  onDiscoverSmb?: () => Promise<SmbDiscoveryData>;
  /** Callback to list NFS exports (invoke Tauri command) */
  onListNfsExports?: (host: string) => Promise<NfsExportData[]>;
  /** Callback when an SMB share is selected */
  onConnectSmb?: (host: string, shareName: string) => void;
  /** Callback when an NFS export is selected */
  onConnectNfs?: (host: string, exportPath: string) => void;
  /** Auto-refresh interval in ms (0 = disabled) */
  refreshInterval?: number;
  className?: string;
}

export function DeviceSidebar({
  onNavigate,
  onDetectDrives: onDetectDrivesProp,
  onEjectDrive: onEjectDriveProp,
  onDiscoverSmb: onDiscoverSmbProp,
  onListNfsExports: onListNfsExportsProp,
  onConnectSmb,
  onConnectNfs: onConnectNfsProp,
  refreshInterval = 30000,
  className,
}: DeviceSidebarProps) {
  // Wire to Tauri IPC when no callback props are provided and Tauri is available
  const onDetectDrives = onDetectDrivesProp ?? (isTauriAvailable()
    ? async () => tauriInvoke<DriveInfoData[]>("detect_drives", undefined, [])
    : undefined);

  const onEjectDrive = onEjectDriveProp ?? (isTauriAvailable()
    ? async (mountPoint: string) => { await tauriInvoke("eject_drive", { mountPoint }); }
    : undefined);

  const onDiscoverSmb = onDiscoverSmbProp ?? (isTauriAvailable()
    ? async () => tauriInvoke<SmbDiscoveryData>("smb_discover_shares", undefined, {
        shares: [],
        scan_duration_ms: 0,
        hosts_scanned: 0,
        errors: [],
      })
    : undefined);

  const onListNfsExports = onListNfsExportsProp ?? (isTauriAvailable()
    ? async (host: string) => tauriInvoke<NfsExportData[]>("nfs_list_exports", { host }, [])
    : undefined);

  const onConnectNfs = onConnectNfsProp ?? undefined;

  const [drives, setDrives] = useState<DriveInfoData[]>([]);
  const [smbShares, setSmbShares] = useState<SmbShareData[]>([]);
  const [loading, setLoading] = useState(false);
  const [smbLoading, setSmbLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [nfsHost, setNfsHost] = useState("");
  const [nfsExports, setNfsExports] = useState<NfsExportData[]>([]);
  const [nfsLoading, setNfsLoading] = useState(false);
  const [preflightResult, setPreflightResult] = useState<PreflightResultData | null>(null);
  const [preflightDrive, setPreflightDrive] = useState<string | null>(null);

  const [expandedSections, setExpandedSections] = useState<Set<string>>(
    new Set(["internal", "external"]),
  );

  // Load drives
  const loadDrives = useCallback(async () => {
    if (!onDetectDrives) return;
    setLoading(true);
    setError(null);
    try {
      const result = await onDetectDrives();
      setDrives(result);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to detect drives");
    } finally {
      setLoading(false);
    }
  }, [onDetectDrives]);

  // Initial load + auto-refresh
  useEffect(() => {
    loadDrives();

    if (refreshInterval > 0) {
      const interval = setInterval(loadDrives, refreshInterval);
      return () => clearInterval(interval);
    }
  }, [loadDrives, refreshInterval]);

  // Discover SMB shares
  const discoverSmb = useCallback(async () => {
    if (!onDiscoverSmb) return;
    setSmbLoading(true);
    try {
      const result = await onDiscoverSmb();
      setSmbShares(result.shares);
    } catch {
      // SMB discovery is best-effort
    } finally {
      setSmbLoading(false);
    }
  }, [onDiscoverSmb]);

  // Discover NFS exports on a host
  const discoverNfs = useCallback(async () => {
    if (!onListNfsExports || !nfsHost.trim()) return;
    setNfsLoading(true);
    try {
      const exports = await onListNfsExports(nfsHost.trim());
      setNfsExports(exports);
    } catch {
      /* NFS discovery is best-effort */
    } finally {
      setNfsLoading(false);
    }
  }, [onListNfsExports, nfsHost]);

  // List SMB shares on a specific host
  const listSmbShares = useCallback(async (host: string) => {
    if (!isTauriAvailable()) return [];
    try {
      return await tauriInvoke<SmbShareData[]>("smb_list_shares", { host }, []);
    } catch {
      return [];
    }
  }, []);

  // Refresh shares for a specific SMB host
  const refreshSmbHostShares = useCallback(async (host: string) => {
    const freshShares = await listSmbShares(host);
    if (freshShares.length > 0) {
      setSmbShares((prev) => {
        const filtered = prev.filter((s) => s.host !== host);
        return [...filtered, ...freshShares];
      });
    }
  }, [listSmbShares]);

  // Transfer preflight check
  const handlePreflight = useCallback(async (sourcePath: string, destPath: string) => {
    if (!isTauriAvailable()) return null;
    try {
      return await tauriInvoke<PreflightResultData>("transfer_preflight", { sourcePath, destPath });
    } catch (err) {
      console.error("Preflight check failed:", err);
      return null;
    }
  }, []);

  // Run preflight for a drive (source = current first internal, dest = selected drive)
  const runPreflight = useCallback(async (drive: DriveInfoData) => {
    const source = drives.find((d) => d.drive_type === "internal" && !d.removable);
    if (!source) return;
    setPreflightDrive(drive.device);
    const result = await handlePreflight(source.mount_point, drive.mount_point);
    setPreflightResult(result);
  }, [drives, handlePreflight]);

  const toggleSection = useCallback((section: string) => {
    setExpandedSections((prev) => {
      const next = new Set(prev);
      if (next.has(section)) {
        next.delete(section);
      } else {
        next.add(section);
      }
      return next;
    });
  }, []);

  // Handle eject
  const handleEject = useCallback(
    async (mountPoint: string) => {
      if (!onEjectDrive) return;
      try {
        await onEjectDrive(mountPoint);
        // Refresh after eject
        loadDrives();
      } catch {
        // Error handling done by caller
      }
    },
    [onEjectDrive, loadDrives],
  );

  // Group drives by type
  const internalDrives = drives.filter(
    (d) => d.drive_type === "internal" && !d.removable,
  );
  const externalDrives = drives.filter(
    (d) => d.drive_type === "external" || d.removable,
  );
  const networkDrives = drives.filter((d) => d.drive_type === "network");

  return (
    <ScrollArea
      className={cn("h-full", className)}
      data-testid="device-sidebar"
    >
      <div className="py-2">
        {/* Header with refresh button */}
        <div className="flex items-center justify-between px-3 pb-1">
          <span className="text-[length:var(--font-size-xs)] font-semibold uppercase tracking-wider text-[color:var(--color-text-secondary)]">
            Devices
          </span>
          <button
            className={cn(
              "h-5 w-5 flex items-center justify-center rounded-sm",
              "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
              "transition-theme",
              loading && "animate-spin",
            )}
            onClick={loadDrives}
            aria-label="Refresh drives"
            title="Refresh drives"
          >
            <RefreshCw className="h-3.5 w-3.5" aria-hidden="true" />
          </button>
        </div>

        {error && (
          <div className="mx-3 mb-2 px-2 py-1 rounded bg-[var(--color-error)]/10 text-[color:var(--color-error)] text-[length:var(--font-size-xs)]">
            <AlertTriangle className="inline h-3 w-3 mr-1" aria-hidden="true" />
            {error}
          </div>
        )}

        {/* Internal Drives */}
        {internalDrives.length > 0 && (
          <DriveSection
            title="Internal"
            expanded={expandedSections.has("internal")}
            onToggle={() => toggleSection("internal")}
            testId="drives-internal"
          >
            {internalDrives.map((drive) => (
              <DriveRow
                key={drive.device}
                drive={drive}
                onNavigate={onNavigate}
                onEject={
                  drive.removable ? () => handleEject(drive.mount_point) : undefined
                }
              />
            ))}
          </DriveSection>
        )}

        {/* External/Removable Drives */}
        {externalDrives.length > 0 && (
          <DriveSection
            title="External"
            expanded={expandedSections.has("external")}
            onToggle={() => toggleSection("external")}
            testId="drives-external"
          >
            {externalDrives.map((drive) => (
              <DriveRow
                key={drive.device}
                drive={drive}
                onNavigate={onNavigate}
                onEject={
                  drive.removable
                    ? () => handleEject(drive.mount_point)
                    : undefined
                }
                onPreflight={() => runPreflight(drive)}
                preflightResult={preflightDrive === drive.device ? preflightResult : null}
                onDismissPreflight={() => {
                  setPreflightDrive(null);
                  setPreflightResult(null);
                }}
              />
            ))}
          </DriveSection>
        )}

        {/* Network Drives */}
        {(networkDrives.length > 0 || smbShares.length > 0 || nfsExports.length > 0) && (
          <DriveSection
            title="Network"
            expanded={expandedSections.has("network")}
            onToggle={() => toggleSection("network")}
            testId="drives-network"
          >
            {networkDrives.map((drive) => (
              <DriveRow
                key={drive.device}
                drive={drive}
                onNavigate={onNavigate}
              />
            ))}
            {smbShares.map((share) => (
              <SmbShareRow
                key={`${share.host}-${share.share_name}`}
                share={share}
                onConnect={onConnectSmb}
                onRefreshHost={() => refreshSmbHostShares(share.host)}
              />
            ))}
            {nfsExports.map((exp) => (
              <NfsExportRow
                key={`${nfsHost}-${exp.path}`}
                export_data={exp}
                host={nfsHost}
                onConnect={onConnectNfs}
              />
            ))}
          </DriveSection>
        )}

        {/* SMB Discovery Button */}
        {onDiscoverSmb && (
          <div className="px-3 pt-2">
            <button
              className={cn(
                "w-full px-2 py-1.5 rounded-md text-[length:var(--font-size-xs)]",
                "bg-[var(--color-hover-bg)] text-[color:var(--color-text-secondary)]",
                "hover:bg-[var(--color-active-bg)] transition-theme",
                "flex items-center justify-center gap-1",
              )}
              onClick={discoverSmb}
              disabled={smbLoading}
              aria-label="Scan for network shares"
            >
              <Wifi className="h-3.5 w-3.5" aria-hidden="true" />
              {smbLoading ? "Scanning..." : "Scan Network"}
            </button>
          </div>
        )}

        {/* NFS Discovery */}
        {onListNfsExports && (
          <div className="px-3 pt-2">
            <div className="flex items-center gap-1">
              <input
                className={cn(
                  "flex-1 px-2 py-1 rounded-md text-[length:var(--font-size-xs)]",
                  "bg-[var(--color-hover-bg)] text-[color:var(--color-text)]",
                  "border border-[var(--color-border)]",
                  "placeholder:text-[color:var(--color-text-tertiary)]",
                  "focus:outline-none focus:ring-1 focus:ring-[var(--color-focus-ring)]",
                )}
                type="text"
                placeholder="NFS host IP/name"
                value={nfsHost}
                onChange={(e) => setNfsHost(e.target.value)}
                onKeyDown={(e) => {
                  if (e.key === "Enter") discoverNfs();
                }}
                aria-label="NFS host address"
              />
              <button
                className={cn(
                  "px-2 py-1 rounded-md text-[length:var(--font-size-xs)]",
                  "bg-[var(--color-hover-bg)] text-[color:var(--color-text-secondary)]",
                  "hover:bg-[var(--color-active-bg)] transition-theme",
                  "flex items-center gap-1 shrink-0",
                )}
                onClick={discoverNfs}
                disabled={nfsLoading || !nfsHost.trim()}
                aria-label="List NFS exports"
              >
                <Server className="h-3.5 w-3.5" aria-hidden="true" />
                {nfsLoading ? "Listing..." : "NFS Exports"}
              </button>
            </div>
          </div>
        )}

        {/* Fallback when no drives */}
        {drives.length === 0 && !loading && !error && (
          <p className="px-4 py-3 text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] italic text-center">
            No drives detected
          </p>
        )}
      </div>
    </ScrollArea>
  );
}

// ──────────────────────────────────────────────
// Drive Section (collapsible)
// ──────────────────────────────────────────────

interface DriveSectionProps {
  title: string;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
  testId?: string;
}

function DriveSection({
  title,
  expanded,
  onToggle,
  children,
  testId,
}: DriveSectionProps) {
  return (
    <div className="mb-1" data-testid={testId}>
      <button
        className={cn(
          "flex items-center gap-1.5 w-full px-3 py-1",
          "text-[10px] font-medium uppercase tracking-wider",
          "text-[color:var(--color-text-tertiary)]",
          "hover:text-[color:var(--color-text-secondary)] transition-theme",
        )}
        onClick={onToggle}
        aria-expanded={expanded}
        aria-label={`${title} drives`}
      >
        {expanded ? (
          <ChevronDown className="h-2.5 w-2.5" aria-hidden="true" />
        ) : (
          <ChevronRight className="h-2.5 w-2.5" aria-hidden="true" />
        )}
        {title}
      </button>
      {expanded && <div role="group" aria-label={`${title} drives`}>{children}</div>}
    </div>
  );
}

// ──────────────────────────────────────────────
// Drive Row
// ──────────────────────────────────────────────

interface DriveRowProps {
  drive: DriveInfoData;
  onNavigate: (path: string) => void;
  onEject?: () => void;
  onPreflight?: () => void;
  preflightResult?: PreflightResultData | null;
  onDismissPreflight?: () => void;
}

function DriveRow({ drive, onNavigate, onEject, onPreflight, preflightResult, onDismissPreflight }: DriveRowProps) {
  const usedPercent =
    drive.total_bytes > 0
      ? (drive.used_bytes / drive.total_bytes) * 100
      : 0;

  const Icon = getDriveIcon(drive.drive_type, drive.removable);
  const fsDisplay = getFilesystemDisplay(drive.filesystem);

  return (
    <div
      className={cn(
        "group relative flex items-center gap-2 px-3 py-1.5",
        "hover:bg-[var(--color-hover-bg)] transition-theme",
      )}
      data-testid={`drive-${drive.device}`}
    >
      <button
        className={cn(
          "flex items-start gap-2 flex-1 min-w-0 text-left",
          "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
        )}
        onClick={() => onNavigate(drive.mount_point)}
        title={`${drive.name}\n${drive.mount_point}\n${fsDisplay} - ${formatBytes(drive.free_bytes)} free of ${formatBytes(drive.total_bytes)}`}
      >
        <Icon
          className="h-4 w-4 mt-0.5 shrink-0 text-[color:var(--color-text-secondary)]"
          aria-hidden="true"
        />
        <div className="flex-1 min-w-0">
          <div className="flex items-baseline gap-1">
            <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text)] truncate block">
              {drive.name}
            </span>
            {drive.read_only && (
              <span className="text-[9px] text-[color:var(--color-text-tertiary)] shrink-0">
                RO
              </span>
            )}
          </div>
          {/* Space bar */}
          {drive.total_bytes > 0 && (
            <>
              <div className="mt-0.5 h-1 w-full bg-[var(--color-border)] rounded-full overflow-hidden">
                <div
                  className={cn("h-full rounded-full", getUsageColor(usedPercent))}
                  style={{ width: `${Math.min(usedPercent, 100)}%` }}
                />
              </div>
              <div className="flex justify-between mt-0.5">
                <span className="text-[9px] text-[color:var(--color-text-tertiary)]">
                  {formatBytes(drive.free_bytes)} free
                </span>
                <span className="text-[9px] text-[color:var(--color-text-tertiary)]">
                  {fsDisplay}
                </span>
              </div>
            </>
          )}
        </div>
      </button>

      {/* Preflight button */}
      {onPreflight && (
        <button
          className={cn(
            "opacity-0 group-hover:opacity-100 transition-theme",
            "h-5 w-5 flex items-center justify-center rounded-sm shrink-0",
            "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={(e) => {
            e.stopPropagation();
            onPreflight();
          }}
          aria-label={`Preflight check for ${drive.name}`}
          title={`Preflight check for ${drive.name}`}
        >
          <AlertTriangle className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      )}

      {/* Eject button */}
      {onEject && (
        <button
          className={cn(
            "opacity-0 group-hover:opacity-100 transition-theme",
            "h-5 w-5 flex items-center justify-center rounded-sm shrink-0",
            "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={(e) => {
            e.stopPropagation();
            onEject();
          }}
          aria-label={`Eject ${drive.name}`}
          title={`Eject ${drive.name}`}
        >
          <Power className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      )}

      {/* Preflight result display */}
      {preflightResult && (
        <div className="absolute left-0 right-0 top-full z-10 mx-2 mt-1">
          <PreflightDisplay
            result={preflightResult}
            onCancel={onDismissPreflight}
          />
        </div>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// SMB Share Row (discovered)
// ──────────────────────────────────────────────

interface SmbShareRowProps {
  share: SmbShareData;
  onConnect?: (host: string, shareName: string) => void;
  onRefreshHost?: () => void;
}

function SmbShareRow({ share, onConnect, onRefreshHost }: SmbShareRowProps) {
  return (
    <div className="group flex items-center gap-0 hover:bg-[var(--color-hover-bg)] transition-theme">
      <button
        className={cn(
          "flex items-center gap-2 flex-1 min-w-0 px-3 py-1.5 text-left",
          "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
        )}
        onClick={() => onConnect?.(share.host, share.share_name)}
        title={`${share.host}/${share.share_name}${share.comment ? ` - ${share.comment}` : ""}`}
      >
        <Server
          className="h-4 w-4 shrink-0 text-[color:var(--color-text-secondary)]"
          aria-hidden="true"
        />
        <div className="flex-1 min-w-0">
          <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text)] truncate block">
            {share.share_name || share.host}
          </span>
          <span className="text-[9px] text-[color:var(--color-text-tertiary)] truncate block">
            {share.host}
            {share.is_nas && " (NAS)"}
          </span>
        </div>
      </button>
      {onRefreshHost && (
        <button
          className={cn(
            "opacity-0 group-hover:opacity-100 transition-theme",
            "h-5 w-5 flex items-center justify-center rounded-sm shrink-0 mr-2",
            "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={(e) => {
            e.stopPropagation();
            onRefreshHost();
          }}
          aria-label={`Refresh shares on ${share.host}`}
          title={`Refresh shares on ${share.host}`}
        >
          <RefreshCw className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// NFS Export Row (discovered)
// ──────────────────────────────────────────────

interface NfsExportRowProps {
  export_data: NfsExportData;
  host: string;
  onConnect?: (host: string, exportPath: string) => void;
}

function NfsExportRow({ export_data, host, onConnect }: NfsExportRowProps) {
  return (
    <button
      className={cn(
        "flex items-center gap-2 w-full px-3 py-1.5 text-left",
        "hover:bg-[var(--color-hover-bg)] transition-theme",
        "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
      )}
      onClick={() => onConnect?.(host, export_data.path)}
      disabled={!onConnect}
      title={`${host}:${export_data.path} (${export_data.allowed_clients.join(", ") || "all clients"})`}
    >
      <Server
        className="h-4 w-4 shrink-0 text-[color:var(--color-text-secondary)]"
        aria-hidden="true"
      />
      <div className="flex-1 min-w-0">
        <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text)] truncate block">
          {export_data.path}
        </span>
        <span className="text-[9px] text-[color:var(--color-text-tertiary)] truncate block">
          {host}
          {export_data.allowed_clients.length > 0
            ? ` - ${export_data.allowed_clients.join(", ")}`
            : " - all clients"}
        </span>
      </div>
    </button>
  );
}

// ──────────────────────────────────────────────
// Preflight Result Display
// ──────────────────────────────────────────────

interface PreflightDisplayProps {
  result: PreflightResultData;
  onProceed?: () => void;
  onCancel?: () => void;
}

export function PreflightDisplay({
  result,
  onProceed,
  onCancel,
}: PreflightDisplayProps) {
  return (
    <div
      className="p-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-panel-bg)]"
      data-testid="preflight-result"
    >
      <h3
        className={cn(
          "text-[length:var(--font-size-sm)] font-semibold mb-2",
          result.can_proceed
            ? "text-[color:var(--color-success)]"
            : "text-[color:var(--color-error)]",
        )}
      >
        {result.can_proceed ? "Transfer Ready" : "Transfer Blocked"}
      </h3>

      {/* Errors */}
      {result.errors.length > 0 && (
        <div className="mb-2 space-y-1">
          {result.errors.map((err, i) => (
            <div
              key={`${err.code}-${i}`}
              className="flex items-start gap-1.5 text-[length:var(--font-size-xs)] text-[color:var(--color-error)]"
            >
              <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-0.5" aria-hidden="true" />
              <div>
                <p className="font-medium">{err.message}</p>
                {err.detail && (
                  <p className="text-[color:var(--color-text-tertiary)]">{err.detail}</p>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Warnings */}
      {result.warnings.length > 0 && (
        <div className="mb-2 space-y-1">
          {result.warnings.map((warn, i) => (
            <div
              key={`${warn.code}-${i}`}
              className="flex items-start gap-1.5 text-[length:var(--font-size-xs)] text-[color:var(--color-warning)]"
            >
              <AlertTriangle className="h-3.5 w-3.5 shrink-0 mt-0.5" aria-hidden="true" />
              <div>
                <p>{warn.message}</p>
                {warn.detail && (
                  <p className="text-[color:var(--color-text-tertiary)]">{warn.detail}</p>
                )}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Drive info */}
      {(result.source_drive || result.dest_drive) && (
        <div className="mt-2 pt-2 border-t border-[var(--color-border)] text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] space-y-0.5">
          {result.source_drive && (
            <p>
              Source: {result.source_drive.name} (
              {getFilesystemDisplay(result.source_drive.filesystem)})
            </p>
          )}
          {result.dest_drive && (
            <p>
              Destination: {result.dest_drive.name} (
              {getFilesystemDisplay(result.dest_drive.filesystem)},{" "}
              {formatBytes(result.dest_drive.free_bytes)} free)
            </p>
          )}
        </div>
      )}

      {/* Action buttons */}
      {(onProceed || onCancel) && (
        <div className="mt-3 flex gap-2 justify-end">
          {onCancel && (
            <button
              className="px-3 py-1 text-[length:var(--font-size-xs)] rounded-md border border-[var(--color-border)] text-[color:var(--color-text-secondary)] hover:bg-[var(--color-hover-bg)]"
              onClick={onCancel}
            >
              Cancel
            </button>
          )}
          {onProceed && result.can_proceed && (
            <button
              className="px-3 py-1 text-[length:var(--font-size-xs)] rounded-md bg-[var(--color-primary)] text-white hover:opacity-90"
              onClick={onProceed}
            >
              Proceed
            </button>
          )}
        </div>
      )}
    </div>
  );
}
