/**
 * Mount Drive Dialog - Finder-mounted remote storage management
 *
 * Allows users to mount remote/cloud connections as local filesystem directories,
 * similar to FileZilla's RemoteDrive feature.
 *
 * Features:
 * - Mount new drive: pick connection, set mount point, optional auto-mount
 * - Active mounts: list with status badges, unmount button
 * - Saved configs: collapsible section with auto-mount toggle, delete
 *
 * Integration notes for device-sidebar.tsx (DO NOT modify device-sidebar.tsx directly):
 * - Add a "Mount Remote Drive" button that opens this dialog (e.g. setShowMountDialog(true))
 * - Add a "Remote Drives" section listing mounted drives from list_mounts()
 * - Mounted drives should show connection name, mount path, and status badge
 * - Clicking a mounted drive could navigate the file list to that mount point
 */
import { useState, useCallback, useEffect } from "react";
import { useFocusTrap } from "@/hooks/use-focus-trap";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { pickFolder } from "@/lib/folder-picker";
import {
  HardDrive,
  FolderOpen,
  Plug,
  PlugZap,
  Trash2,
  ChevronDown,
  ChevronRight,
  RefreshCw,
  AlertCircle,
  CheckCircle2,
  Circle,
} from "lucide-react";

// ── Types (mirrors Rust mount_engine types) ──

interface MountInfo {
  id: string;
  connection_id: string;
  connection_name: string;
  protocol: string;
  mount_point: string;
  status: "mounted" | "unmounted" | "error";
  auto_mount: boolean;
  created_at: string;
  error_message: string | null;
}

interface MountResult {
  mount_id: string;
  mount_point: string;
  status: "mounted" | "unmounted" | "error";
  message: string;
}

interface ConnectionProfile {
  id: string;
  name: string;
  protocol: string;
  host: string;
  port: number | null;
  username: string | null;
  remote_path: string;
}

// ── Status Badge ──

function StatusBadge({ status }: { status: MountInfo["status"] }) {
  switch (status) {
    case "mounted":
      return (
        <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400">
          <CheckCircle2 size={12} />
          Mounted
        </span>
      );
    case "unmounted":
      return (
        <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400">
          <Circle size={12} />
          Unmounted
        </span>
      );
    case "error":
      return (
        <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400">
          <AlertCircle size={12} />
          Error
        </span>
      );
  }
}

// ── Mount New Drive Form ──

function MountNewDriveForm({
  connections,
  onMounted,
}: {
  connections: ConnectionProfile[];
  onMounted: () => void;
}) {
  const [selectedConnectionId, setSelectedConnectionId] = useState("");
  const [mountPoint, setMountPoint] = useState("");
  const [autoMount, setAutoMount] = useState(false);
  const [mounting, setMounting] = useState(false);
  const [error, setError] = useState("");
  const [successMessage, setSuccessMessage] = useState("");

  // Auto-generate mount point based on selected connection
  useEffect(() => {
    if (selectedConnectionId) {
      const conn = connections.find((c) => c.id === selectedConnectionId);
      if (conn) {
        const safeName = conn.name.replace(/[^a-zA-Z0-9_-]/g, "_");
        setMountPoint(`~/UFOP-Mounts/${safeName}`);
      }
    }
  }, [selectedConnectionId, connections]);

  const handleMount = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setSuccessMessage("");

    if (!selectedConnectionId) {
      setError("Please select a connection.");
      return;
    }
    if (!mountPoint.trim()) {
      setError("Please specify a mount point.");
      return;
    }

    setMounting(true);
    try {
      const result = await tauriInvoke<MountResult>("mount_remote", {
        connectionId: selectedConnectionId,
        mountPoint: mountPoint.trim(),
      });

      if (result.status === "mounted") {
        setSuccessMessage(result.message);
        // Save config if auto-mount is enabled
        if (autoMount) {
          await tauriInvoke("save_mount_config", {
            connectionId: selectedConnectionId,
            mountPoint: mountPoint.trim(),
            autoMount: true,
          });
        }
        onMounted();
        // Reset form after short delay
        setTimeout(() => {
          setSelectedConnectionId("");
          setMountPoint("");
          setAutoMount(false);
          setSuccessMessage("");
        }, 2000);
      } else {
        setError(`Mount returned status: ${result.status}`);
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setMounting(false);
    }
  };

  return (
    <form onSubmit={handleMount} className="space-y-3">
      <div>
        <label className="block text-xs font-medium mb-1">Connection</label>
        <select
          value={selectedConnectionId}
          onChange={(e) => setSelectedConnectionId(e.target.value)}
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        >
          <option value="">Select a saved connection...</option>
          {connections.map((conn) => (
            <option key={conn.id} value={conn.id}>
              {conn.name} ({conn.protocol.toUpperCase()}
              {conn.host ? ` - ${conn.host}` : ""})
            </option>
          ))}
        </select>
      </div>

      <div>
        <label className="block text-xs font-medium mb-1">Mount Point</label>
        <div className="flex gap-2">
          <input
            type="text"
            value={mountPoint}
            onChange={(e) => setMountPoint(e.target.value)}
            placeholder="~/UFOP-Mounts/my-server"
            className="flex-1 text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
          <button
            type="button"
            onClick={async () => {
              const path = await pickFolder("Select mount point");
              if (path) setMountPoint(path);
            }}
            className="text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
            title="Browse for folder"
          >
            <FolderOpen size={16} />
          </button>
        </div>
        <p className="text-xs text-gray-400 mt-1">
          Default: ~/UFOP-Mounts/&#123;connection-name&#125;
        </p>
      </div>

      <label className="flex items-center gap-2 text-sm cursor-pointer">
        <input
          type="checkbox"
          checked={autoMount}
          onChange={(e) => setAutoMount(e.target.checked)}
          className="rounded"
        />
        Auto-mount on startup
      </label>

      {error && <p className="text-xs text-red-500">{error}</p>}
      {successMessage && (
        <p className="text-xs text-green-600 dark:text-green-400">
          {successMessage}
        </p>
      )}

      <div className="flex justify-end">
        <button
          type="submit"
          disabled={mounting || !selectedConnectionId}
          className="inline-flex items-center gap-1.5 text-sm px-4 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Plug size={14} />
          {mounting ? "Mounting..." : "Mount"}
        </button>
      </div>
    </form>
  );
}

// ── Active Mount Item ──

function ActiveMountItem({
  mount,
  onUnmount,
  unmounting,
}: {
  mount: MountInfo;
  onUnmount: (mountId: string) => void;
  unmounting: boolean;
}) {
  return (
    <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700 last:border-b-0 hover:bg-gray-50 dark:hover:bg-gray-800/50">
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-2">
          <HardDrive size={14} className="text-gray-500 flex-shrink-0" />
          <span className="text-sm font-medium truncate">
            {mount.connection_name}
          </span>
          <StatusBadge status={mount.status} />
        </div>
        <p className="text-xs text-gray-500 mt-0.5 truncate pl-5">
          {mount.mount_point}
        </p>
        <p className="text-xs text-gray-400 pl-5">
          {mount.protocol.toUpperCase()}
          {mount.auto_mount ? " | Auto-mount" : ""}
        </p>
        {mount.error_message && (
          <p className="text-xs text-red-500 pl-5 mt-0.5">
            {mount.error_message}
          </p>
        )}
      </div>
      {mount.status === "mounted" && (
        <button
          onClick={() => onUnmount(mount.id)}
          disabled={unmounting}
          className="flex-shrink-0 ml-2 p-1.5 rounded text-gray-500 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 disabled:opacity-50"
          title="Unmount"
        >
          <PlugZap size={14} />
        </button>
      )}
    </div>
  );
}

// ── Saved Config Item ──

function SavedConfigItem({
  config,
  connectionName,
  onDelete,
  deleting,
}: {
  config: MountInfo;
  connectionName: string;
  onDelete: (mountId: string) => void;
  deleting: boolean;
}) {
  return (
    <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700 last:border-b-0">
      <div className="flex-1 min-w-0">
        <span className="text-sm truncate">{connectionName}</span>
        <p className="text-xs text-gray-400 truncate">{config.mount_point}</p>
      </div>
      <div className="flex items-center gap-2 flex-shrink-0">
        <span
          className={`text-xs px-1.5 py-0.5 rounded ${
            config.auto_mount
              ? "bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400"
              : "bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400"
          }`}
        >
          {config.auto_mount ? "Auto" : "Manual"}
        </span>
        <button
          onClick={() => onDelete(config.id)}
          disabled={deleting}
          className="p-1 rounded text-gray-400 hover:text-red-500 hover:bg-red-50 dark:hover:bg-red-900/20 disabled:opacity-50"
          title="Delete saved config"
        >
          <Trash2 size={12} />
        </button>
      </div>
    </div>
  );
}

// ── Main Dialog Component ──

export default function MountDriveDialog({
  open,
  onClose,
}: {
  open: boolean;
  onClose: () => void;
}) {
  const [connections, setConnections] = useState<ConnectionProfile[]>([]);
  const [mounts, setMounts] = useState<MountInfo[]>([]);
  const [loading, setLoading] = useState(false);
  const [unmountingId, setUnmountingId] = useState<string | null>(null);
  const [deletingId, setDeletingId] = useState<string | null>(null);
  const [savedConfigsExpanded, setSavedConfigsExpanded] = useState(false);
  const containerRef = useFocusTrap<HTMLDivElement>({ active: open, onEscape: onClose });

  // Load connections and mounts
  const loadData = useCallback(async () => {
    setLoading(true);
    try {
      const [conns, mountList] = await Promise.all([
        tauriInvokeSafe<ConnectionProfile[]>(
          "list_connections",
          undefined,
          []
        ),
        tauriInvokeSafe<MountInfo[]>("list_mounts", undefined, []),
      ]);
      setConnections(conns);
      setMounts(mountList);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    if (open) {
      loadData();
    }
  }, [open, loadData]);

  const handleUnmount = async (mountId: string) => {
    setUnmountingId(mountId);
    try {
      await tauriInvoke("unmount_remote", { mountId });
      await loadData();
    } catch (err) {
      console.error("Unmount failed:", err);
    } finally {
      setUnmountingId(null);
    }
  };

  const handleDeleteConfig = async (mountId: string) => {
    setDeletingId(mountId);
    try {
      await tauriInvoke("delete_mount_config", { mountId });
      await loadData();
    } catch (err) {
      console.error("Delete config failed:", err);
    } finally {
      setDeletingId(null);
    }
  };

  // Separate active (mounted) from saved-only (unmounted) configs
  const activeMounts = mounts.filter((m) => m.status === "mounted");
  const savedConfigs = mounts.filter((m) => m.status !== "mounted");

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center" role="dialog" aria-modal="true" aria-labelledby="mount-drive-title">
      {/* Backdrop */}
      <div
        className="absolute inset-0 bg-black/50"
        onClick={onClose}
        aria-hidden="true"
      />

      {/* Dialog */}
      <div ref={containerRef} className="relative bg-white dark:bg-gray-900 rounded-lg shadow-xl w-full max-w-lg max-h-[85vh] overflow-hidden flex flex-col">
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <div className="flex items-center gap-2">
            <HardDrive size={18} className="text-blue-500" aria-hidden="true" />
            <h2 id="mount-drive-title" className="text-base font-semibold">Mount Remote Drive</h2>
          </div>
          <div className="flex items-center gap-2">
            <button
              onClick={loadData}
              disabled={loading}
              className="p-1 rounded text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
              title="Refresh"
            >
              <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
            </button>
            <button
              onClick={onClose}
              className="text-sm text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 px-2 py-0.5"
            >
              Close
            </button>
          </div>
        </div>

        {/* Scrollable Content */}
        <div className="flex-1 overflow-y-auto">
          {/* Mount New Drive Section */}
          <div className="p-4 border-b border-gray-200 dark:border-gray-700">
            <h3 className="text-sm font-medium mb-3 flex items-center gap-1.5">
              <Plug size={14} />
              Mount New Drive
            </h3>
            {connections.length === 0 ? (
              <p className="text-sm text-gray-500 text-center py-4">
                No saved connections found. Add a connection in the Connection
                Manager first.
              </p>
            ) : (
              <MountNewDriveForm
                connections={connections}
                onMounted={loadData}
              />
            )}
          </div>

          {/* Active Mounts Section */}
          <div className="border-b border-gray-200 dark:border-gray-700">
            <h3 className="text-sm font-medium px-4 py-2 flex items-center gap-1.5 bg-gray-50 dark:bg-gray-800/50">
              <PlugZap size={14} />
              Active Mounts
              {activeMounts.length > 0 && (
                <span className="text-xs text-gray-400 ml-1">
                  ({activeMounts.length})
                </span>
              )}
            </h3>
            {activeMounts.length === 0 ? (
              <p className="text-sm text-gray-500 text-center py-6 px-4">
                No drives mounted. Mount a connection to access it as a local
                folder.
              </p>
            ) : (
              <div>
                {activeMounts.map((mount) => (
                  <ActiveMountItem
                    key={mount.id}
                    mount={mount}
                    onUnmount={handleUnmount}
                    unmounting={unmountingId === mount.id}
                  />
                ))}
              </div>
            )}
          </div>

          {/* Saved Configs Section (collapsed by default) */}
          <div>
            <button
              onClick={() => setSavedConfigsExpanded(!savedConfigsExpanded)}
              className="w-full text-sm font-medium px-4 py-2 flex items-center gap-1.5 bg-gray-50 dark:bg-gray-800/50 hover:bg-gray-100 dark:hover:bg-gray-800 text-left"
            >
              {savedConfigsExpanded ? (
                <ChevronDown size={14} />
              ) : (
                <ChevronRight size={14} />
              )}
              Saved Configurations
              {savedConfigs.length > 0 && (
                <span className="text-xs text-gray-400 ml-1">
                  ({savedConfigs.length})
                </span>
              )}
            </button>
            {savedConfigsExpanded && (
              <div>
                {savedConfigs.length === 0 ? (
                  <p className="text-sm text-gray-500 text-center py-4 px-4">
                    No saved mount configurations.
                  </p>
                ) : (
                  savedConfigs.map((config) => (
                    <SavedConfigItem
                      key={config.id}
                      config={config}
                      connectionName={config.connection_name}
                      onDelete={handleDeleteConfig}
                      deleting={deletingId === config.id}
                    />
                  ))
                )}
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
