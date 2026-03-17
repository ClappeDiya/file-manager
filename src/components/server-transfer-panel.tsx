/**
 * Server Transfer Panel
 *
 * Wires all 11 server-to-server transfer Tauri IPC commands:
 * - server_transfer_preview_method
 * - server_transfer_create
 * - server_transfer_start / pause / cancel / retry
 * - server_transfer_get
 * - server_transfer_list / list_active
 * - server_transfer_cleanup
 * - server_transfer_capability_matrix
 *
 * Provides a UI for creating, monitoring, and managing server-to-server transfers.
 */
import { useState, useCallback, useEffect } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types (mirrors Rust types) ──

interface TransferEndpoint {
  protocol: string;
  host: string;
  port: number | null;
  path: string;
}

interface ServerTransferFile {
  path: string;
  size: number;
}

interface ServerTransferJob {
  id: string;
  source: TransferEndpoint;
  destination: TransferEndpoint;
  files: ServerTransferFile[];
  status: string; // "queued" | "active" | "paused" | "completed" | "failed" | "cancelled"
  total_bytes: number;
  transferred_bytes: number;
  created_at: string;
  error: string | null;
}

interface TransferMethodPreview {
  method: string;
  direct_possible: boolean;
  fallback_method: string | null;
  estimated_overhead: string | null;
}

interface DirectTransferCapability {
  source_protocol: string;
  dest_protocol: string;
  method: string;
  direct_possible: boolean;
  fallback_method: string | null;
  estimated_overhead: string | null;
}

// ── Helpers ──

function formatFileSize(bytes: number): string {
  if (bytes === 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(1024));
  const size = bytes / Math.pow(1024, i);
  return `${size.toFixed(i === 0 ? 0 : 1)} ${units[i]}`;
}

function formatTimestamp(ts: string): string {
  try {
    return new Date(ts).toLocaleString();
  } catch {
    return ts;
  }
}

function getStatusColor(status: string): string {
  switch (status) {
    case "active":
      return "text-blue-500 dark:text-blue-400";
    case "completed":
      return "text-green-500 dark:text-green-400";
    case "failed":
      return "text-red-500 dark:text-red-400";
    case "cancelled":
      return "text-gray-400 dark:text-gray-500";
    case "paused":
      return "text-yellow-500 dark:text-yellow-400";
    case "queued":
      return "text-gray-500 dark:text-gray-400";
    default:
      return "text-gray-500";
  }
}

const PROTOCOLS = ["sftp", "ftp", "ftps", "webdav", "smb", "nfs", "s3"];

// ── Create Transfer Form ──

function CreateTransferForm({
  onCreated,
}: {
  onCreated: () => void;
}) {
  const [srcProtocol, setSrcProtocol] = useState("sftp");
  const [srcHost, setSrcHost] = useState("");
  const [srcPort, setSrcPort] = useState("");
  const [srcPath, setSrcPath] = useState("/");
  const [destProtocol, setDestProtocol] = useState("sftp");
  const [destHost, setDestHost] = useState("");
  const [destPort, setDestPort] = useState("");
  const [destPath, setDestPath] = useState("/");
  const [filesInput, setFilesInput] = useState("");
  const [preview, setPreview] = useState<TransferMethodPreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);
  const [creating, setCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handlePreview = useCallback(async () => {
    setPreviewLoading(true);
    setPreview(null);
    try {
      const result = await tauriInvoke<TransferMethodPreview>(
        "server_transfer_preview_method",
        { sourceProtocol: srcProtocol, destProtocol: destProtocol },
      );
      setPreview(result);
    } catch (err) {
      console.error("Preview failed:", err);
      setError(err instanceof Error ? err.message : "Preview failed");
    } finally {
      setPreviewLoading(false);
    }
  }, [srcProtocol, destProtocol]);

  const handleCreate = useCallback(
    async (e: React.FormEvent) => {
      e.preventDefault();
      setCreating(true);
      setError(null);

      // Parse files from text input (one per line: path,size)
      const files: ServerTransferFile[] = filesInput
        .split("\n")
        .map((line) => line.trim())
        .filter(Boolean)
        .map((line) => {
          const [path, sizeStr] = line.split(",").map((s) => s.trim());
          return { path: path || "", size: parseInt(sizeStr, 10) || 0 };
        })
        .filter((f) => f.path);

      try {
        const job = await tauriInvoke<ServerTransferJob>(
          "server_transfer_create",
          {
            source: {
              protocol: srcProtocol,
              host: srcHost,
              port: srcPort ? parseInt(srcPort, 10) : null,
              path: srcPath,
            },
            destination: {
              protocol: destProtocol,
              host: destHost,
              port: destPort ? parseInt(destPort, 10) : null,
              path: destPath,
            },
            files,
          },
        );
        console.log("Transfer created:", job.id);
        onCreated();
      } catch (err) {
        console.error("Create failed:", err);
        setError(err instanceof Error ? err.message : "Create failed");
      } finally {
        setCreating(false);
      }
    },
    [srcProtocol, srcHost, srcPort, srcPath, destProtocol, destHost, destPort, destPath, filesInput, onCreated],
  );

  return (
    <form onSubmit={handleCreate} className="space-y-3 p-3 border-b border-gray-200 dark:border-gray-700">
      {/* Source endpoint */}
      <div>
        <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">
          Source
        </label>
        <div className="grid grid-cols-4 gap-1.5">
          <select
            value={srcProtocol}
            onChange={(e) => setSrcProtocol(e.target.value)}
            className="text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            {PROTOCOLS.map((p) => (
              <option key={p} value={p}>
                {p.toUpperCase()}
              </option>
            ))}
          </select>
          <input
            type="text"
            value={srcHost}
            onChange={(e) => setSrcHost(e.target.value)}
            placeholder="Host"
            required
            className="col-span-2 text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
          <input
            type="text"
            value={srcPort}
            onChange={(e) => setSrcPort(e.target.value)}
            placeholder="Port"
            className="text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        </div>
        <input
          type="text"
          value={srcPath}
          onChange={(e) => setSrcPath(e.target.value)}
          placeholder="Remote path"
          className="w-full mt-1.5 text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>

      {/* Destination endpoint */}
      <div>
        <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">
          Destination
        </label>
        <div className="grid grid-cols-4 gap-1.5">
          <select
            value={destProtocol}
            onChange={(e) => setDestProtocol(e.target.value)}
            className="text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          >
            {PROTOCOLS.map((p) => (
              <option key={p} value={p}>
                {p.toUpperCase()}
              </option>
            ))}
          </select>
          <input
            type="text"
            value={destHost}
            onChange={(e) => setDestHost(e.target.value)}
            placeholder="Host"
            required
            className="col-span-2 text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
          <input
            type="text"
            value={destPort}
            onChange={(e) => setDestPort(e.target.value)}
            placeholder="Port"
            className="text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          />
        </div>
        <input
          type="text"
          value={destPath}
          onChange={(e) => setDestPath(e.target.value)}
          placeholder="Remote path"
          className="w-full mt-1.5 text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
        />
      </div>

      {/* Preview method */}
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={handlePreview}
          disabled={previewLoading}
          className="text-xs px-2.5 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50"
        >
          {previewLoading ? "Checking..." : "Preview Method"}
        </button>
        {preview && (
          <span className="text-xs text-gray-500 dark:text-gray-400">
            {preview.method}
            {preview.direct_possible ? " (direct)" : ""}
            {preview.fallback_method ? ` / fallback: ${preview.fallback_method}` : ""}
            {preview.estimated_overhead ? ` / overhead: ${preview.estimated_overhead}` : ""}
          </span>
        )}
      </div>

      {/* Files input */}
      <div>
        <label className="block text-xs font-medium text-gray-600 dark:text-gray-400 mb-1">
          Files (one per line: path, size)
        </label>
        <textarea
          value={filesInput}
          onChange={(e) => setFilesInput(e.target.value)}
          placeholder={"/data/file.txt, 1024\n/data/image.png, 204800"}
          rows={3}
          className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 font-mono resize-y"
        />
      </div>

      {/* Error */}
      {error && (
        <p className="text-xs text-red-500 dark:text-red-400">{error}</p>
      )}

      {/* Submit */}
      <button
        type="submit"
        disabled={creating || !srcHost || !destHost}
        className="w-full text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
      >
        {creating ? "Creating..." : "Create Transfer"}
      </button>
    </form>
  );
}

// ── Transfer Job Item ──

function TransferJobItem({
  job,
  onStart,
  onPause,
  onCancel,
  onRetry,
}: {
  job: ServerTransferJob;
  onStart: (jobId: string) => void;
  onPause: (jobId: string) => void;
  onCancel: (jobId: string) => void;
  onRetry: (jobId: string) => void;
}) {
  const progressPct =
    job.total_bytes > 0
      ? Math.round((job.transferred_bytes / job.total_bytes) * 100)
      : 0;

  const isQueued = job.status === "queued";
  const isActive = job.status === "active";
  const isPaused = job.status === "paused";
  const isFailed = job.status === "failed";
  const isTerminal = job.status === "completed" || job.status === "cancelled";

  return (
    <div className="px-3 py-2.5 border-b border-gray-200 dark:border-gray-700 last:border-b-0 hover:bg-gray-50 dark:hover:bg-gray-800/50">
      {/* Source -> Dest */}
      <div className="flex items-center gap-1 mb-1 text-sm">
        <span className="font-medium truncate">
          {job.source.protocol.toUpperCase()}://{job.source.host}
          {job.source.port ? `:${job.source.port}` : ""}
        </span>
        <span className="text-gray-400 dark:text-gray-500 flex-shrink-0">-&gt;</span>
        <span className="font-medium truncate">
          {job.destination.protocol.toUpperCase()}://{job.destination.host}
          {job.destination.port ? `:${job.destination.port}` : ""}
        </span>
      </div>

      {/* Status line */}
      <div className="flex items-center justify-between text-xs mb-1">
        <span className={getStatusColor(job.status)}>
          {job.status.charAt(0).toUpperCase() + job.status.slice(1)}
        </span>
        <span className="text-gray-400 dark:text-gray-500">
          {formatTimestamp(job.created_at)}
        </span>
      </div>

      {/* Progress bar */}
      {!isTerminal && (
        <div className="w-full h-1.5 rounded-full bg-gray-200 dark:bg-gray-700 mb-1.5">
          <div
            className={`h-full rounded-full transition-all ${
              isFailed
                ? "bg-red-500"
                : "bg-blue-500"
            }`}
            style={{ width: `${progressPct}%` }}
          />
        </div>
      )}

      {/* Bytes / Files */}
      <div className="flex items-center justify-between text-xs text-gray-500 dark:text-gray-400 mb-1.5">
        <span>
          {formatFileSize(job.transferred_bytes)} / {formatFileSize(job.total_bytes)} ({progressPct}%)
        </span>
        <span>
          {job.files.length} file{job.files.length !== 1 ? "s" : ""}
        </span>
      </div>

      {/* Error */}
      {job.error && (
        <p className="text-xs text-red-500 dark:text-red-400 mb-1.5 truncate" title={job.error}>
          {job.error}
        </p>
      )}

      {/* Actions */}
      {!isTerminal && (
        <div className="flex items-center gap-1">
          {isQueued && (
            <button
              onClick={() => onStart(job.id)}
              className="text-xs px-2 py-1 rounded bg-blue-500 text-white hover:bg-blue-600"
              type="button"
            >
              Start
            </button>
          )}
          {isActive && (
            <button
              onClick={() => onPause(job.id)}
              className="text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
              type="button"
            >
              Pause
            </button>
          )}
          {isPaused && (
            <button
              onClick={() => onStart(job.id)}
              className="text-xs px-2 py-1 rounded bg-blue-500 text-white hover:bg-blue-600"
              type="button"
            >
              Resume
            </button>
          )}
          {isFailed && (
            <button
              onClick={() => onRetry(job.id)}
              className="text-xs px-2 py-1 rounded bg-yellow-500 text-white hover:bg-yellow-600"
              type="button"
            >
              Retry
            </button>
          )}
          {(isQueued || isActive || isPaused || isFailed) && (
            <button
              onClick={() => onCancel(job.id)}
              className="text-xs px-2 py-1 rounded hover:bg-red-100 dark:hover:bg-red-900/30 text-red-600 dark:text-red-400"
              type="button"
            >
              Cancel
            </button>
          )}
        </div>
      )}
    </div>
  );
}

// ── Capability Matrix ──

function CapabilityMatrix({
  capabilities,
}: {
  capabilities: DirectTransferCapability[];
}) {
  if (capabilities.length === 0) {
    return (
      <p className="text-xs text-gray-400 dark:text-gray-500 px-3 py-2">
        No capability data available.
      </p>
    );
  }

  return (
    <div className="overflow-x-auto">
      <table className="w-full text-xs">
        <thead>
          <tr className="border-b border-gray-200 dark:border-gray-700">
            <th className="text-left px-3 py-1.5 font-medium text-gray-500 dark:text-gray-400">Source</th>
            <th className="text-left px-3 py-1.5 font-medium text-gray-500 dark:text-gray-400">Dest</th>
            <th className="text-left px-3 py-1.5 font-medium text-gray-500 dark:text-gray-400">Method</th>
            <th className="text-left px-3 py-1.5 font-medium text-gray-500 dark:text-gray-400">Direct</th>
            <th className="text-left px-3 py-1.5 font-medium text-gray-500 dark:text-gray-400">Fallback</th>
          </tr>
        </thead>
        <tbody>
          {capabilities.map((cap, idx) => (
            <tr
              key={`${cap.source_protocol}-${cap.dest_protocol}-${idx}`}
              className="border-b border-gray-100 dark:border-gray-800 last:border-b-0 hover:bg-gray-50 dark:hover:bg-gray-800/30"
            >
              <td className="px-3 py-1.5 uppercase">{cap.source_protocol}</td>
              <td className="px-3 py-1.5 uppercase">{cap.dest_protocol}</td>
              <td className="px-3 py-1.5">{cap.method}</td>
              <td className="px-3 py-1.5">
                <span className={cap.direct_possible ? "text-green-500" : "text-red-500"}>
                  {cap.direct_possible ? "Yes" : "No"}
                </span>
              </td>
              <td className="px-3 py-1.5 text-gray-400 dark:text-gray-500">
                {cap.fallback_method || "--"}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

// ── Main Server Transfer Panel ──

export function ServerTransferPanel() {
  const [jobs, setJobs] = useState<ServerTransferJob[]>([]);
  const [activeJobs, setActiveJobs] = useState<ServerTransferJob[]>([]);
  const [capabilities, setCapabilities] = useState<DirectTransferCapability[]>([]);
  const [showCreateForm, setShowCreateForm] = useState(false);
  const [showCapabilities, setShowCapabilities] = useState(false);
  const [showAllJobs, setShowAllJobs] = useState(false);
  const [cleanupCount, setCleanupCount] = useState<number | null>(null);

  // Fetch all jobs
  const refreshJobs = useCallback(async () => {
    try {
      const [all, active] = await Promise.all([
        tauriInvoke<ServerTransferJob[]>("server_transfer_list", undefined, []),
        tauriInvoke<ServerTransferJob[]>("server_transfer_list_active", undefined, []),
      ]);
      setJobs(all);
      setActiveJobs(active);
    } catch (err) {
      console.error("Failed to load transfer jobs:", err);
    }
  }, []);

  // Fetch a single job by ID (server_transfer_get)
  const getJob = useCallback(async (jobId: string): Promise<ServerTransferJob | null> => {
    try {
      return await tauriInvoke<ServerTransferJob | null>(
        "server_transfer_get",
        { jobId },
        null,
      );
    } catch (err) {
      console.error("Failed to get transfer job:", err);
      return null;
    }
  }, []);

  // Auto-refresh active transfers every 3 seconds
  useEffect(() => {
    refreshJobs();
    const interval = setInterval(refreshJobs, 3000);
    return () => clearInterval(interval);
  }, [refreshJobs]);

  // Load capability matrix
  const loadCapabilities = useCallback(async () => {
    try {
      const caps = await tauriInvoke<DirectTransferCapability[]>(
        "server_transfer_capability_matrix",
        undefined,
        [],
      );
      setCapabilities(caps);
      setShowCapabilities(true);
    } catch (err) {
      console.error("Failed to load capabilities:", err);
    }
  }, []);

  // Job actions: after each action, refresh single job then full list
  const handleStart = useCallback(
    async (jobId: string) => {
      try {
        await tauriInvoke("server_transfer_start", { jobId });
        // Optionally fetch updated job details immediately
        const updated = await getJob(jobId);
        if (updated) {
          setJobs((prev) => prev.map((j) => (j.id === jobId ? updated : j)));
        }
        refreshJobs();
      } catch (err) {
        console.error("Start failed:", err);
      }
    },
    [refreshJobs, getJob],
  );

  const handlePause = useCallback(
    async (jobId: string) => {
      try {
        await tauriInvoke("server_transfer_pause", { jobId });
        refreshJobs();
      } catch (err) {
        console.error("Pause failed:", err);
      }
    },
    [refreshJobs],
  );

  const handleCancel = useCallback(
    async (jobId: string) => {
      try {
        await tauriInvoke("server_transfer_cancel", { jobId });
        refreshJobs();
      } catch (err) {
        console.error("Cancel failed:", err);
      }
    },
    [refreshJobs],
  );

  const handleRetry = useCallback(
    async (jobId: string) => {
      try {
        await tauriInvoke("server_transfer_retry", { jobId });
        refreshJobs();
      } catch (err) {
        console.error("Retry failed:", err);
      }
    },
    [refreshJobs],
  );

  const handleCleanup = useCallback(async () => {
    try {
      const count = await tauriInvoke<number>(
        "server_transfer_cleanup",
        undefined,
        0,
      );
      setCleanupCount(count);
      refreshJobs();
      // Clear the count message after 3 seconds
      setTimeout(() => setCleanupCount(null), 3000);
    } catch (err) {
      console.error("Cleanup failed:", err);
    }
  }, [refreshJobs]);

  // Which list to show
  const displayJobs = showAllJobs ? jobs : activeJobs;

  return (
    <div
      className="flex flex-col h-full"
      role="region"
      aria-label="Server transfer manager"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-semibold">Server Transfers</h2>
          {activeJobs.length > 0 && (
            <span className="text-xs px-1.5 py-0.5 rounded-full bg-blue-100 dark:bg-blue-900/30 text-blue-600 dark:text-blue-400">
              {activeJobs.length} active
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          <button
            onClick={() => setShowCreateForm(!showCreateForm)}
            className="text-xs px-2 py-1 rounded bg-blue-500 text-white hover:bg-blue-600"
            type="button"
          >
            + New
          </button>
          <button
            onClick={handleCleanup}
            className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
            type="button"
            title="Clean up completed/cancelled transfers"
          >
            Cleanup
          </button>
        </div>
      </div>

      {/* Cleanup message */}
      {cleanupCount !== null && (
        <div className="px-3 py-1.5 text-xs text-green-600 dark:text-green-400 bg-green-50 dark:bg-green-900/20 border-b border-gray-200 dark:border-gray-700">
          Cleaned up {cleanupCount} transfer{cleanupCount !== 1 ? "s" : ""}.
        </div>
      )}

      {/* Create form */}
      {showCreateForm && (
        <CreateTransferForm
          onCreated={() => {
            setShowCreateForm(false);
            refreshJobs();
          }}
        />
      )}

      {/* Toggle between all / active */}
      <div className="flex items-center gap-2 px-3 py-1.5 border-b border-gray-200 dark:border-gray-700">
        <button
          onClick={() => setShowAllJobs(false)}
          className={`text-xs px-2 py-1 rounded ${
            !showAllJobs
              ? "bg-gray-200 dark:bg-gray-700 font-medium"
              : "hover:bg-gray-100 dark:hover:bg-gray-800"
          }`}
          type="button"
        >
          Active ({activeJobs.length})
        </button>
        <button
          onClick={() => setShowAllJobs(true)}
          className={`text-xs px-2 py-1 rounded ${
            showAllJobs
              ? "bg-gray-200 dark:bg-gray-700 font-medium"
              : "hover:bg-gray-100 dark:hover:bg-gray-800"
          }`}
          type="button"
        >
          All ({jobs.length})
        </button>
        <div className="flex-1" />
        <button
          onClick={showCapabilities ? () => setShowCapabilities(false) : loadCapabilities}
          className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
          type="button"
        >
          {showCapabilities ? "Hide Matrix" : "Capabilities"}
        </button>
      </div>

      {/* Capability matrix (collapsible) */}
      {showCapabilities && (
        <div className="border-b border-gray-200 dark:border-gray-700">
          <div className="px-3 py-1.5 text-xs font-semibold text-gray-500 dark:text-gray-400 bg-gray-50 dark:bg-gray-800/50 uppercase tracking-wider">
            Transfer Capability Matrix
          </div>
          <CapabilityMatrix capabilities={capabilities} />
        </div>
      )}

      {/* Transfer list */}
      <div className="flex-1 overflow-y-auto">
        {displayJobs.length === 0 && !showCreateForm && (
          <div className="flex flex-col items-center justify-center h-32 text-sm text-gray-400 dark:text-gray-500">
            <p>{showAllJobs ? "No transfers yet" : "No active transfers"}</p>
            <button
              onClick={() => setShowCreateForm(true)}
              className="mt-2 text-blue-500 hover:text-blue-600"
              type="button"
            >
              Create a new transfer
            </button>
          </div>
        )}
        {displayJobs.map((job) => (
          <TransferJobItem
            key={job.id}
            job={job}
            onStart={handleStart}
            onPause={handlePause}
            onCancel={handleCancel}
            onRetry={handleRetry}
          />
        ))}
      </div>
    </div>
  );
}

export default ServerTransferPanel;
