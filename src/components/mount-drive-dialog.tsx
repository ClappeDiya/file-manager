/**
 * Mount Drive Dialog - mount a remote/cloud connection as a local folder (or,
 * on Windows, a drive letter), or edit a saved mount.
 *
 * Reached from the Connection Manager ("Mount as drive"), the Mounts list
 * ("Mount drive…", "Edit"), the toolbar and the command palette (see
 * `mounts-host.tsx`). The list of active and saved mounts, with their
 * actions, lives in `mounts-panel.tsx`.
 *
 * Fields: connection, mount point, an optional subfolder of the connection,
 * cache options for cloud drives (locked where the organization's mount
 * profile forces them), auto-mount, "open after mounting", and, in Advanced
 * mode, the cloud drive's advanced options.
 */
import { useState, useEffect } from "react";
import { useFocusTrap } from "@/hooks/use-focus-trap";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { MountAdvancedFields, MountCacheFields } from "@/components/mount-options";
import {
  DEFAULT_MOUNT_OPTIONS,
  describePolicy,
  isCloudMount,
  mountPayloadOptions,
  useMountPolicy,
  type MountInfo,
  type MountOptions,
  type MountResult,
} from "@/lib/mount-options";
import { useUIStore } from "@/stores/ui-store";
import { errorText } from "@/lib/error-text";
import { pickFolder } from "@/lib/folder-picker";
import { HardDrive, FolderOpen, Plug } from "lucide-react";

// ── Types (mirrors Rust mount_engine types) ──

/** Mirrors `mount_engine::MountCapability`. */
type MountCapability =
  | { native_mount: { command: string } }
  | { requires_install: { tool: string; install_hint: string } }
  | { not_supported: { reason: string } };

function CapabilityBadge({ capability }: { capability: MountCapability }) {
  if ("native_mount" in capability) {
    return (
      <p className="text-xs text-green-600" data-testid="mount-capability">
        Can be mounted (uses {capability.native_mount.command})
      </p>
    );
  }
  if ("requires_install" in capability) {
    return (
      <p className="text-xs text-amber-600" data-testid="mount-capability">
        Needs {capability.requires_install.tool}: <code>{capability.requires_install.install_hint}</code>
      </p>
    );
  }
  return (
    <p className="text-xs text-gray-500" data-testid="mount-capability">
      Can't be mounted: {capability.not_supported.reason}
    </p>
  );
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

// ── Mount / Edit Form ──

function MountForm({
  connections,
  initialConnectionId,
  editing,
  onDone,
  onClose,
}: {
  connections: ConnectionProfile[];
  initialConnectionId: string | null;
  editing: MountInfo | null;
  onDone: () => void;
  onClose: () => void;
}) {
  const [selectedConnectionId, setSelectedConnectionId] = useState(editing?.connection_id ?? initialConnectionId ?? "");
  const [mountPoint, setMountPoint] = useState(editing?.mount_point ?? "");
  const [autoMount, setAutoMount] = useState(editing?.auto_mount ?? false);
  const [working, setWorking] = useState(false);
  const [error, setError] = useState("");
  const [successMessage, setSuccessMessage] = useState("");
  const [done, setDone] = useState(false);
  const [capability, setCapability] = useState<MountCapability | null>(null);
  const [options, setOptions] = useState<MountOptions>({ ...DEFAULT_MOUNT_OPTIONS, ...editing?.options });
  const advanced = useUIStore((s) => s.appMode === "advanced");
  const policy = useMountPolicy();
  const selected = connections.find((c) => c.id === selectedConnectionId);
  const cloud = !!selected && isCloudMount(selected.protocol);
  const usesDriveLetter = cloud && !!options.drive_letter;

  // What mounting this connection needs (rclone, FUSE, sshfs…), shown as a badge.
  useEffect(() => {
    setCapability(null);
    const conn = connections.find((c) => c.id === selectedConnectionId);
    if (!conn) return;
    let live = true;
    tauriInvoke<MountCapability>("check_mount_capability", { protocol: conn.protocol })
      .then((c) => live && setCapability(c))
      .catch(() => live && setCapability(null));
    return () => {
      live = false;
    };
  }, [selectedConnectionId, connections]);

  // The result stays on screen for a moment, then the dialog closes.
  useEffect(() => {
    if (!done) return;
    const t = setTimeout(onClose, 2000);
    return () => clearTimeout(t);
  }, [done, onClose]);

  // Suggest a mount point when a connection is picked for a new mount.
  useEffect(() => {
    if (editing || !selectedConnectionId) return;
    const conn = connections.find((c) => c.id === selectedConnectionId);
    if (conn) {
      const safeName = conn.name.replace(/[^a-zA-Z0-9_-]/g, "_");
      setMountPoint(`~/UFOP-Mounts/${safeName}`);
    }
  }, [selectedConnectionId, connections, editing]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setSuccessMessage("");

    if (!selectedConnectionId) {
      setError("Please select a connection.");
      return;
    }
    // A drive letter stands in for the mount point (Windows).
    const point = usesDriveLetter ? `${options.drive_letter}:` : mountPoint.trim();
    if (!point) {
      setError("Please specify a mount point.");
      return;
    }
    const sent = mountPayloadOptions(cloud, options);

    setWorking(true);
    try {
      if (editing) {
        await tauriInvoke("update_mount_config", {
          mountId: editing.id,
          connectionId: selectedConnectionId,
          mountPoint: point,
          autoMount,
          options: sent,
        });
        setSuccessMessage("Saved. The new settings apply the next time the drive is mounted.");
        setDone(true);
        onDone();
        return;
      }
      const result = await tauriInvoke<MountResult>("mount_remote", {
        connectionId: selectedConnectionId,
        mountPoint: point,
        options: sent,
      });

      if (result.status === "mounted") {
        setSuccessMessage(result.message);
        // Save config if auto-mount is enabled
        if (autoMount) {
          await tauriInvoke("save_mount_config", {
            connectionId: selectedConnectionId,
            mountPoint: point,
            autoMount: true,
            options: sent,
          });
        }
        setDone(true);
        onDone();
      } else {
        setError(`Mount returned status: ${result.status}`);
      }
    } catch (err) {
      setError(errorText(err));
    } finally {
      setWorking(false);
    }
  };

  return (
    <form onSubmit={handleSubmit} className="space-y-3">
      <div>
        <label className="block text-xs font-medium mb-1" htmlFor="mount-connection">Connection</label>
        <select
          id="mount-connection"
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

      {capability && <CapabilityBadge capability={capability} />}

      {usesDriveLetter && !advanced && (
        // The drive-letter picker is an Advanced option; without it a saved drive letter
        // could not be changed back to a folder.
        <div data-testid="mount-drive-letter-note">
          <span className="block text-xs font-medium mb-1">Mount Point</span>
          <div className="flex items-center gap-2 text-sm">
            <span>Drive {options.drive_letter?.toUpperCase()}:</span>
            <button
              type="button"
              onClick={() => {
                setOptions({ ...options, drive_letter: null });
                setMountPoint("");
              }}
              className="text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
            >
              Use a folder instead
            </button>
          </div>
        </div>
      )}

      {!usesDriveLetter && (
        <div>
          <label className="block text-xs font-medium mb-1" htmlFor="mount-point">Mount Point</label>
          <div className="flex gap-2">
            <input
              id="mount-point"
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
              aria-label="Browse for mount point"
            >
              <FolderOpen size={16} />
            </button>
          </div>
          <p className="text-xs text-gray-400 mt-1">
            Default: ~/UFOP-Mounts/&#123;connection-name&#125;
          </p>
        </div>
      )}

      <div>
        <label className="block text-xs font-medium mb-1" htmlFor="mount-subfolder">Subfolder (optional)</label>
        <input
          id="mount-subfolder"
          type="text"
          value={options.subfolder ?? ""}
          onChange={(e) => setOptions({ ...options, subfolder: e.target.value === "" ? null : e.target.value })}
          placeholder="e.g. photos/2024"
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
        <p className="text-xs text-gray-400 mt-1">
          Mount only this folder of the connection{selected?.remote_path && selected.remote_path !== "/" ? ` (inside ${selected.remote_path})` : ""}.
        </p>
      </div>

      {cloud && <MountCacheFields value={options} onChange={setOptions} policy={policy} />}
      {cloud && advanced && <MountAdvancedFields value={options} onChange={setOptions} policy={policy} />}
      {policy && describePolicy(policy) && (
        <p className="text-xs text-gray-500" data-testid="mount-policy">
          Your organization's mount profile: {describePolicy(policy)}
        </p>
      )}

      <label className="flex items-center gap-2 text-sm cursor-pointer">
        <input
          type="checkbox"
          checked={autoMount}
          onChange={(e) => setAutoMount(e.target.checked)}
          className="rounded"
        />
        Auto-mount on startup
      </label>
      <label className="flex items-center gap-2 text-sm cursor-pointer">
        <input
          type="checkbox"
          checked={!!options.open_after_mount}
          onChange={(e) => setOptions({ ...options, open_after_mount: e.target.checked })}
          className="rounded"
        />
        Open the drive after mounting
      </label>

      {error && <p className="text-xs text-red-500" role="alert">{error}</p>}
      {successMessage && (
        <p className="text-xs text-green-600 dark:text-green-400" role="status">
          {successMessage}
        </p>
      )}

      <div className="flex justify-end">
        <button
          type="submit"
          disabled={working || done || !selectedConnectionId || (!editing && capability !== null && !("native_mount" in capability))}
          className="inline-flex items-center gap-1.5 text-sm px-4 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50 disabled:cursor-not-allowed"
        >
          <Plug size={14} />
          {editing ? (working ? "Saving..." : "Save changes") : working ? "Mounting..." : "Mount"}
        </button>
      </div>
    </form>
  );
}

// ── Main Dialog Component ──

export default function MountDriveDialog({
  open,
  onClose,
  connectionId = null,
  editing = null,
  onChanged,
}: {
  open: boolean;
  onClose: () => void;
  /** Connection to preselect for a new mount. */
  connectionId?: string | null;
  /** A saved mount to edit (null = mount a new drive). */
  editing?: MountInfo | null;
  /** Called after a mount or an edit changed something (reload lists). */
  onChanged?: () => void;
}) {
  const [connections, setConnections] = useState<ConnectionProfile[] | null>(null);
  const containerRef = useFocusTrap<HTMLDivElement>({ active: open, onEscape: onClose });

  useEffect(() => {
    if (!open) return;
    let live = true;
    void tauriInvokeSafe<ConnectionProfile[]>("list_connections", undefined, []).then((c) => live && setConnections(c));
    return () => {
      live = false;
    };
  }, [open]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center" role="dialog" aria-modal="true" aria-labelledby="mount-drive-title">
      {/* Backdrop */}
      <div className="absolute inset-0 bg-black/50" onClick={onClose} aria-hidden="true" />

      {/* Dialog */}
      <div ref={containerRef} className="relative bg-white dark:bg-gray-900 rounded-lg shadow-xl w-full max-w-lg max-h-[85vh] overflow-hidden flex flex-col">
        <div className="flex items-center justify-between px-4 py-3 border-b border-gray-200 dark:border-gray-700">
          <div className="flex items-center gap-2">
            <HardDrive size={18} className="text-blue-500" aria-hidden="true" />
            <h2 id="mount-drive-title" className="text-base font-semibold">
              {editing ? "Edit Mount" : "Mount Remote Drive"}
            </h2>
          </div>
          <button
            onClick={onClose}
            className="text-sm text-gray-400 hover:text-gray-600 dark:hover:text-gray-300 px-2 py-0.5"
          >
            Close
          </button>
        </div>

        <div className="flex-1 overflow-y-auto p-4">
          {connections === null ? (
            <p className="text-sm text-gray-500 text-center py-4">Loading connections…</p>
          ) : connections.length === 0 ? (
            <p className="text-sm text-gray-500 text-center py-4">
              No saved connections found. Add a connection in the Connection Manager first.
            </p>
          ) : (
            <MountForm
              connections={connections}
              initialConnectionId={connectionId}
              editing={editing}
              onDone={() => onChanged?.()}
              onClose={onClose}
            />
          )}
        </div>
      </div>
    </div>
  );
}
