/**
 * Transfer Panel Component (T-015 Frontend)
 *
 * Two display modes:
 * - Mini mode: Compact progress bar showing overall transfer status
 * - Full mode: Per-item detail with progress, ETA, throughput, priority, retry status
 *
 * Provides controls for:
 * - Pause/resume/cancel individual transfers
 * - Set priority (high/normal/low)
 * - Drag to reorder
 * - Retry failed transfers
 * - Bandwidth throttle configuration
 */
import { useState, useCallback, useEffect, useRef } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";

// ── Types (mirrors Rust types) ──

interface TransferJob {
  id: string;
  source_path: string;
  dest_path: string;
  total_bytes: number;
  transferred_bytes: number;
  status: string;
  priority: string;
  sort_order: number;
  created_at: string;
  updated_at: string;
  error_message: string | null;
  retry_count: number;
  speed_bps: number;
  eta_seconds: number | null;
  verify_checksum: boolean;
}

interface ThrottleConfig {
  global_limit_bps: number;
  per_connection_limit_bps: number;
}

interface TransferConfig {
  verify_tier: string;
  verify_checksums: boolean;
  checksum_algorithm: string;
  conflict_policy: string;
  max_retries: number;
  backoff_seconds: number[];
}

interface TransferHistoryRecord {
  id: string;
  source_path: string;
  dest_path: string;
  total_bytes: number;
  status: string;
  started_at: string;
  completed_at: string | null;
  checksum_verified: boolean;
}

interface TransferHistoryFilter {
  query?: string;
  status?: string;
  limit?: number;
}

interface ConflictCheckResult {
  has_conflict: boolean;
  source_path: string;
  dest_path: string;
  conflict_type: string | null;
}

type TransferPanelMode = "mini" | "full";

// ── Utility functions ──

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

function formatSpeed(bps: number): string {
  if (bps === 0) return "--";
  return `${formatBytes(bps)}/s`;
}

function formatEta(seconds: number | null): string {
  if (seconds === null || seconds === 0) return "--";
  if (seconds < 60) return `${seconds}s`;
  if (seconds < 3600) return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
  const hours = Math.floor(seconds / 3600);
  const mins = Math.floor((seconds % 3600) / 60);
  return `${hours}h ${mins}m`;
}

function progressPercent(job: TransferJob): number {
  if (job.total_bytes === 0) return 0;
  return Math.round((job.transferred_bytes / job.total_bytes) * 100);
}

function statusLabel(status: string): string {
  const labels: Record<string, string> = {
    queued: "Queued",
    active: "Transferring",
    paused: "Paused",
    completed: "Completed",
    failed: "Failed",
    cancelled: "Cancelled",
    conflict: "Conflict",
  };
  return labels[status] || status;
}

function statusColor(status: string): string {
  const colors: Record<string, string> = {
    queued: "text-gray-500 dark:text-gray-400",
    active: "text-blue-600 dark:text-blue-400",
    paused: "text-yellow-600 dark:text-yellow-400",
    completed: "text-green-600 dark:text-green-400",
    failed: "text-red-600 dark:text-red-400",
    cancelled: "text-gray-400 dark:text-gray-500",
    conflict: "text-orange-600 dark:text-orange-400",
  };
  return colors[status] || "text-gray-500";
}

function progressBarColor(status: string): string {
  const colors: Record<string, string> = {
    queued: "bg-gray-400",
    active: "bg-blue-500",
    paused: "bg-yellow-500",
    completed: "bg-green-500",
    failed: "bg-red-500",
    cancelled: "bg-gray-400",
    conflict: "bg-orange-500",
  };
  return colors[status] || "bg-gray-400";
}

function fileName(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

// ── Mini Mode Component ──

function TransferPanelMini({
  jobs,
  onExpand,
}: {
  jobs: TransferJob[];
  onExpand: () => void;
}) {
  const activeJobs = jobs.filter(
    (j) => j.status === "active" || j.status === "queued"
  );
  const totalBytes = activeJobs.reduce((sum, j) => sum + j.total_bytes, 0);
  const transferredBytes = activeJobs.reduce(
    (sum, j) => sum + j.transferred_bytes,
    0
  );
  const totalSpeed = activeJobs.reduce((sum, j) => sum + j.speed_bps, 0);
  const pct = totalBytes > 0 ? (transferredBytes / totalBytes) * 100 : 0;

  if (activeJobs.length === 0) return null;

  return (
    <button
      onClick={onExpand}
      className="flex items-center gap-2 px-3 py-1.5 rounded-md bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 transition-colors w-full text-left"
      aria-label={`${activeJobs.length} active transfers, click to expand`}
      type="button"
    >
      <div className="flex-1 min-w-0">
        <div className="flex items-center justify-between text-xs mb-1">
          <span className="font-medium truncate">
            {activeJobs.length} transfer{activeJobs.length !== 1 ? "s" : ""} in
            progress
          </span>
          <span className="text-gray-500 dark:text-gray-400 ml-2">
            {formatSpeed(totalSpeed)}
          </span>
        </div>
        <div className="w-full h-1.5 bg-gray-300 dark:bg-gray-600 rounded-full overflow-hidden">
          <div
            className="h-full bg-blue-500 rounded-full transition-all duration-300"
            style={{ width: `${Math.min(pct, 100)}%` }}
          />
        </div>
      </div>
      <span className="text-xs text-gray-500 dark:text-gray-400 whitespace-nowrap">
        {Math.round(pct)}%
      </span>
    </button>
  );
}

// ── Transfer Item Component ──

function TransferItem({
  job,
  onPause,
  onResume,
  onCancel,
  onRetry,
  onSetPriority,
  onSetConflictPolicy,
  onReorder,
  onResolveConflict,
  totalJobs,
}: {
  job: TransferJob;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
  onRetry: (id: string) => void;
  onSetPriority: (id: string, priority: string) => void;
  onSetConflictPolicy: (jobId: string, policy: string) => void;
  onReorder: (jobId: string, newSortOrder: number) => void;
  onResolveConflict: (job: TransferJob) => void;
  totalJobs: number;
}) {
  const pct = progressPercent(job);
  const isActive = job.status === "active" || job.status === "queued";
  const isPaused = job.status === "paused";
  const isFailed = job.status === "failed";
  const hasConflict = job.status === "conflict";

  return (
    <div
      className="px-3 py-2 border-b border-gray-200 dark:border-gray-700 last:border-b-0"
      role="listitem"
      aria-label={`Transfer: ${fileName(job.source_path)}`}
    >
      <div className="flex items-center justify-between mb-1">
        <div className="flex items-center gap-2 min-w-0 flex-1">
          {/* Reorder buttons */}
          <div className="flex flex-col gap-0.5 flex-shrink-0">
            <button
              onClick={() => onReorder(job.id, Math.max(0, job.sort_order - 1))}
              disabled={job.sort_order <= 0}
              className="text-[10px] leading-none px-1 py-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-700 text-gray-500 dark:text-gray-400 disabled:opacity-30"
              title="Move up"
              type="button"
              aria-label="Move transfer up"
            >
              ^
            </button>
            <button
              onClick={() => onReorder(job.id, job.sort_order + 1)}
              disabled={job.sort_order >= totalJobs - 1}
              className="text-[10px] leading-none px-1 py-0.5 rounded hover:bg-gray-200 dark:hover:bg-gray-700 text-gray-500 dark:text-gray-400 disabled:opacity-30"
              title="Move down"
              type="button"
              aria-label="Move transfer down"
            >
              v
            </button>
          </div>
          <span className="text-sm font-medium truncate">
            {fileName(job.source_path)}
          </span>
          {job.priority !== "normal" && (
            <span
              className={`text-xs px-1.5 py-0.5 rounded-full ${
                job.priority === "high"
                  ? "bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400"
                  : "bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400"
              }`}
            >
              {job.priority}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1 ml-2">
          {/* Priority selector */}
          <select
            value={job.priority}
            onChange={(e) => onSetPriority(job.id, e.target.value)}
            className="text-xs bg-transparent border border-gray-300 dark:border-gray-600 rounded px-1 py-0.5"
            aria-label="Set priority"
          >
            <option value="high">High</option>
            <option value="normal">Normal</option>
            <option value="low">Low</option>
          </select>

          {/* Per-item conflict policy (only for queued/active) */}
          {isActive && (
            <select
              value={job.verify_checksum ? "overwrite" : "skip"}
              onChange={(e) => onSetConflictPolicy(job.id, e.target.value)}
              className="text-xs bg-transparent border border-gray-300 dark:border-gray-600 rounded px-1 py-0.5"
              aria-label="Conflict policy for this transfer"
              title="Per-item conflict policy"
            >
              <option value="skip">Skip</option>
              <option value="overwrite">Overwrite</option>
              <option value="rename">Rename</option>
              <option value="ask">Ask</option>
            </select>
          )}

          {/* Action buttons */}
          {isActive && (
            <button
              onClick={() => onPause(job.id)}
              className="p-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700 text-gray-600 dark:text-gray-400"
              aria-label="Pause transfer"
              title="Pause"
              type="button"
            >
              ||
            </button>
          )}
          {isPaused && (
            <button
              onClick={() => onResume(job.id)}
              className="p-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700 text-blue-600 dark:text-blue-400"
              aria-label="Resume transfer"
              title="Resume"
              type="button"
            >
              &gt;
            </button>
          )}
          {isFailed && (
            <button
              onClick={() => onRetry(job.id)}
              className="p-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700 text-orange-600 dark:text-orange-400"
              aria-label="Retry transfer"
              title={`Retry (attempt ${job.retry_count + 1})`}
              type="button"
            >
              R
            </button>
          )}
          {hasConflict && (
            <button
              onClick={() => onResolveConflict(job)}
              className="p-1 rounded hover:bg-orange-100 dark:hover:bg-orange-900/30 text-orange-600 dark:text-orange-400"
              aria-label="Resolve conflict"
              title="Resolve conflict"
              type="button"
            >
              Fix
            </button>
          )}
          {(isActive || isPaused) && (
            <button
              onClick={() => onCancel(job.id)}
              className="p-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700 text-red-600 dark:text-red-400"
              aria-label="Cancel transfer"
              title="Cancel"
              type="button"
            >
              X
            </button>
          )}
        </div>
      </div>

      {/* Destination */}
      <div className="text-xs text-gray-500 dark:text-gray-400 truncate mb-1">
        &rarr; {fileName(job.dest_path)}
      </div>

      {/* Progress bar */}
      <div className="w-full h-1.5 bg-gray-200 dark:bg-gray-700 rounded-full overflow-hidden mb-1">
        <div
          className={`h-full rounded-full transition-all duration-300 ${progressBarColor(job.status)}`}
          style={{ width: `${Math.min(pct, 100)}%` }}
          role="progressbar"
          aria-valuenow={pct}
          aria-valuemin={0}
          aria-valuemax={100}
        />
      </div>

      {/* Status line */}
      <div className="flex items-center justify-between text-xs">
        <span className={statusColor(job.status)}>
          {statusLabel(job.status)}
          {isFailed && job.error_message && `: ${job.error_message}`}
          {isFailed && ` (${job.retry_count} retries)`}
        </span>
        <div className="flex items-center gap-3 text-gray-500 dark:text-gray-400">
          <span>
            {formatBytes(job.transferred_bytes)} / {formatBytes(job.total_bytes)}
          </span>
          {job.speed_bps > 0 && <span>{formatSpeed(job.speed_bps)}</span>}
          {job.eta_seconds !== null && job.eta_seconds > 0 && (
            <span>ETA: {formatEta(job.eta_seconds)}</span>
          )}
          <span>{pct}%</span>
        </div>
      </div>
    </div>
  );
}

// ── Transfer Settings Component ──

function TransferSettings({
  config,
  onSetVerifyTier,
  onSetVerifyChecksums,
  onSetChecksumAlgorithm,
  onSetRetryPolicy,
  onSetConflictPolicy,
  onSearchHistory,
  onExportHistory,
  onCleanupHistory,
  onSaveAllSettings,
}: {
  config: TransferConfig;
  onSetVerifyTier: (tier: string) => void;
  onSetVerifyChecksums: (verify: boolean) => void;
  onSetChecksumAlgorithm: (algorithm: string) => void;
  onSetRetryPolicy: (maxRetries: number, backoffSeconds: number[]) => void;
  onSetConflictPolicy: (policy: string) => void;
  onSearchHistory: (query: string) => void;
  onExportHistory: (format: string) => void;
  onCleanupHistory: (days: number) => void;
  onSaveAllSettings: (config: TransferConfig) => void;
}) {
  const [maxRetries, setMaxRetries] = useState(config.max_retries);
  const [historySearch, setHistorySearch] = useState("");
  const [cleanupDays, setCleanupDays] = useState(30);

  // Completion hooks state – loaded from and saved to backend
  const [showNotification, setShowNotification] = useState(true);
  const [playSound, setPlaySound] = useState(false);
  const [openDestination, setOpenDestination] = useState(false);
  const [completionCommand, setCompletionCommand] = useState("");

  // Load completion hooks from backend on mount
  useEffect(() => {
    (async () => {
      const hooks = await tauriInvokeSafe<{
        show_notification: boolean;
        play_sound: boolean;
        open_destination: boolean;
        completion_command: string;
      } | null>("get_completion_hooks", undefined, null);
      if (hooks) {
        setShowNotification(hooks.show_notification);
        setPlaySound(hooks.play_sound);
        setOpenDestination(hooks.open_destination);
        setCompletionCommand(hooks.completion_command);
      }
    })();
  }, []);

  // Persist completion hooks to backend whenever they change
  const saveCompletionHooks = useCallback(
    async (overrides?: {
      showNotification?: boolean;
      playSound?: boolean;
      openDestination?: boolean;
      completionCommand?: string;
    }) => {
      try {
        await tauriInvoke("set_completion_hooks", {
          hooks: {
            show_notification: overrides?.showNotification ?? showNotification,
            play_sound: overrides?.playSound ?? playSound,
            open_destination: overrides?.openDestination ?? openDestination,
            completion_command: overrides?.completionCommand ?? completionCommand,
          },
        });
      } catch {
        // Silently fail – backend may not be available in dev mode
      }
    },
    [showNotification, playSound, openDestination, completionCommand],
  );

  // Sync maxRetries when config changes externally
  useEffect(() => {
    setMaxRetries(config.max_retries);
  }, [config.max_retries]);

  return (
    <div className="border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/50">
      {/* Verification Settings */}
      <div className="px-3 py-2 border-b border-gray-100 dark:border-gray-700/50">
        <div className="text-xs font-semibold text-gray-700 dark:text-gray-300 mb-1.5">
          Verification
        </div>
        <div className="flex flex-wrap items-center gap-3">
          <div className="flex items-center gap-1.5">
            <label className="text-xs text-gray-600 dark:text-gray-400">
              Tier:
            </label>
            <select
              value={config.verify_tier}
              onChange={(e) => onSetVerifyTier(e.target.value)}
              className="text-xs px-1.5 py-0.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              aria-label="Verification tier"
            >
              <option value="quick">Quick</option>
              <option value="standard">Standard</option>
              <option value="paranoid">Paranoid</option>
            </select>
          </div>
          <div className="flex items-center gap-1.5">
            <label className="text-xs text-gray-600 dark:text-gray-400">
              Verify checksums:
            </label>
            <input
              type="checkbox"
              checked={config.verify_checksums}
              onChange={(e) => onSetVerifyChecksums(e.target.checked)}
              className="h-3.5 w-3.5 rounded border-gray-300 dark:border-gray-600"
              aria-label="Enable checksum verification"
            />
          </div>
          <div className="flex items-center gap-1.5">
            <label className="text-xs text-gray-600 dark:text-gray-400">
              Algorithm:
            </label>
            <select
              value={config.checksum_algorithm}
              onChange={(e) => onSetChecksumAlgorithm(e.target.value)}
              className="text-xs px-1.5 py-0.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              aria-label="Checksum algorithm"
            >
              <option value="md5">MD5</option>
              <option value="sha256">SHA-256</option>
              <option value="sha512">SHA-512</option>
              <option value="xxhash">xxHash</option>
            </select>
          </div>
        </div>
      </div>

      {/* Retry Policy */}
      <div className="px-3 py-2 border-b border-gray-100 dark:border-gray-700/50">
        <div className="text-xs font-semibold text-gray-700 dark:text-gray-300 mb-1.5">
          Retry Policy
        </div>
        <div className="flex items-center gap-2">
          <label className="text-xs text-gray-600 dark:text-gray-400">
            Max retries:
          </label>
          <input
            type="number"
            min={1}
            max={10}
            value={maxRetries}
            onChange={(e) => setMaxRetries(Math.min(10, Math.max(1, parseInt(e.target.value) || 1)))}
            className="w-14 text-xs px-2 py-0.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            aria-label="Maximum retries"
          />
          <button
            onClick={() => {
              // Generate exponential backoff: 1s, 2s, 4s, ... up to maxRetries entries
              const backoff = Array.from({ length: maxRetries }, (_, i) => Math.pow(2, i));
              onSetRetryPolicy(maxRetries, backoff);
            }}
            className="text-xs px-2 py-0.5 rounded bg-blue-500 text-white hover:bg-blue-600"
            type="button"
          >
            Apply
          </button>
        </div>
      </div>

      {/* Conflict Resolution */}
      <div className="px-3 py-2 border-b border-gray-100 dark:border-gray-700/50">
        <div className="text-xs font-semibold text-gray-700 dark:text-gray-300 mb-1.5">
          Conflict Resolution
        </div>
        <div className="flex items-center gap-1.5">
          <label className="text-xs text-gray-600 dark:text-gray-400">
            Policy:
          </label>
          <select
            value={config.conflict_policy}
            onChange={(e) => onSetConflictPolicy(e.target.value)}
            className="text-xs px-1.5 py-0.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            aria-label="Conflict resolution policy"
          >
            <option value="skip">Skip</option>
            <option value="overwrite">Overwrite</option>
            <option value="rename">Rename</option>
            <option value="ask">Ask</option>
          </select>
        </div>
      </div>

      {/* Completion Actions */}
      <div className="px-3 py-2 border-b border-gray-100 dark:border-gray-700/50">
        <div className="text-xs font-semibold text-gray-700 dark:text-gray-300 mb-1.5">
          After Transfer Completes
        </div>
        <div className="space-y-2">
          <label className="flex items-center gap-2 text-xs text-gray-600 dark:text-gray-400 cursor-pointer">
            <input
              type="checkbox"
              checked={showNotification}
              onChange={(e) => {
                setShowNotification(e.target.checked);
                saveCompletionHooks({ showNotification: e.target.checked });
              }}
              className="h-3.5 w-3.5 rounded border-gray-300 dark:border-gray-600"
            />
            Show notification
          </label>
          <label className="flex items-center gap-2 text-xs text-gray-600 dark:text-gray-400 cursor-pointer">
            <input
              type="checkbox"
              checked={playSound}
              onChange={(e) => {
                setPlaySound(e.target.checked);
                saveCompletionHooks({ playSound: e.target.checked });
              }}
              className="h-3.5 w-3.5 rounded border-gray-300 dark:border-gray-600"
            />
            Play sound
          </label>
          <label className="flex items-center gap-2 text-xs text-gray-600 dark:text-gray-400 cursor-pointer">
            <input
              type="checkbox"
              checked={openDestination}
              onChange={(e) => {
                setOpenDestination(e.target.checked);
                saveCompletionHooks({ openDestination: e.target.checked });
              }}
              className="h-3.5 w-3.5 rounded border-gray-300 dark:border-gray-600"
            />
            Open destination folder
          </label>
          <div>
            <label className="block text-xs text-gray-500 dark:text-gray-400 mb-0.5">
              Run command (optional)
            </label>
            <input
              type="text"
              value={completionCommand}
              onChange={(e) => setCompletionCommand(e.target.value)}
              onBlur={(e) => saveCompletionHooks({ completionCommand: e.target.value })}
              placeholder="e.g., say 'Transfer complete'"
              className="w-full text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              aria-label="Post-transfer command"
            />
          </div>
        </div>
      </div>

      {/* Save All Settings Button */}
      <div className="px-3 py-2 border-b border-gray-100 dark:border-gray-700/50">
        <button
          onClick={() => onSaveAllSettings(config)}
          className="text-xs px-3 py-1.5 rounded bg-green-500 text-white hover:bg-green-600 w-full"
          type="button"
        >
          Save All Settings
        </button>
      </div>

      {/* Transfer History */}
      <div className="px-3 py-2">
        <div className="text-xs font-semibold text-gray-700 dark:text-gray-300 mb-1.5">
          Transfer History
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div className="flex items-center gap-1.5">
            <input
              type="text"
              value={historySearch}
              onChange={(e) => setHistorySearch(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") onSearchHistory(historySearch);
              }}
              placeholder="Search history..."
              className="w-36 text-xs px-2 py-0.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              aria-label="Search transfer history"
            />
            <button
              onClick={() => onSearchHistory(historySearch)}
              className="text-xs px-2 py-0.5 rounded bg-blue-500 text-white hover:bg-blue-600"
              type="button"
            >
              Search
            </button>
          </div>
          <button
            onClick={() => onExportHistory("json")}
            className="text-xs px-2 py-0.5 rounded bg-gray-100 hover:bg-gray-200 dark:bg-gray-700 dark:hover:bg-gray-600 text-gray-700 dark:text-gray-300"
            type="button"
            title="Export transfer history as JSON"
          >
            Export
          </button>
          <div className="flex items-center gap-1.5">
            <input
              type="number"
              min={1}
              value={cleanupDays}
              onChange={(e) => setCleanupDays(Math.max(1, parseInt(e.target.value) || 1))}
              className="w-14 text-xs px-2 py-0.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              aria-label="Cleanup older than days"
            />
            <span className="text-xs text-gray-500 dark:text-gray-400">days</span>
            <button
              onClick={() => onCleanupHistory(cleanupDays)}
              className="text-xs px-2 py-0.5 rounded text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30"
              type="button"
              title="Delete history records older than specified days"
            >
              Cleanup
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Full Mode Component ──

function TransferPanelFull({
  jobs,
  onCollapse,
  onPause,
  onResume,
  onCancel,
  onRetry,
  onSetPriority,
  onRetryAllFailed,
  throttleConfig,
  onSetThrottle,
  onSetConnectionThrottle,
  transferConfig,
  onSetVerifyTier,
  onSetVerifyChecksums,
  onSetChecksumAlgorithm,
  onSetRetryPolicy,
  onSetConflictPolicy,
  onSetItemConflictPolicy,
  onReorder,
  onResolveConflict,
  onSearchHistory,
  onExportHistory,
  onCleanupHistory,
  onSaveAllSettings,
  showFailedOnly,
  onToggleShowFailed,
}: {
  jobs: TransferJob[];
  onCollapse: () => void;
  onPause: (id: string) => void;
  onResume: (id: string) => void;
  onCancel: (id: string) => void;
  onRetry: (id: string) => void;
  onSetPriority: (id: string, priority: string) => void;
  onRetryAllFailed: () => void;
  throttleConfig: ThrottleConfig;
  onSetThrottle: (bps: number) => void;
  onSetConnectionThrottle: (connectionId: string, bytesPerSec: number) => void;
  transferConfig: TransferConfig;
  onSetVerifyTier: (tier: string) => void;
  onSetVerifyChecksums: (verify: boolean) => void;
  onSetChecksumAlgorithm: (algorithm: string) => void;
  onSetRetryPolicy: (maxRetries: number, backoffSeconds: number[]) => void;
  onSetConflictPolicy: (policy: string) => void;
  onSetItemConflictPolicy: (jobId: string, policy: string) => void;
  onReorder: (jobId: string, newSortOrder: number) => void;
  onResolveConflict: (job: TransferJob) => void;
  onSearchHistory: (query: string) => void;
  onExportHistory: (format: string) => void;
  onCleanupHistory: (days: number) => void;
  onSaveAllSettings: (config: TransferConfig) => void;
  showFailedOnly: boolean;
  onToggleShowFailed: () => void;
}) {
  const activeJobs = jobs.filter(
    (j) => j.status === "active" || j.status === "queued"
  );
  const pausedJobs = jobs.filter((j) => j.status === "paused");
  const failedJobs = jobs.filter((j) => j.status === "failed");
  const completedJobs = jobs.filter((j) => j.status === "completed");
  const cancelledJobs = jobs.filter((j) => j.status === "cancelled");

  const [showThrottle, setShowThrottle] = useState(false);
  const [showSettings, setShowSettings] = useState(false);
  const [throttleInput, setThrottleInput] = useState(
    throttleConfig.global_limit_bps > 0
      ? (throttleConfig.global_limit_bps / (1024 * 1024)).toString()
      : ""
  );
  const [connThrottleId, setConnThrottleId] = useState("");
  const [connThrottleValue, setConnThrottleValue] = useState("");

  return (
    <div
      className="flex flex-col h-full bg-white dark:bg-gray-900 border-t border-gray-200 dark:border-gray-700"
      role="region"
      aria-label="Transfer queue"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-semibold">Transfers</h2>
          <span className="text-xs text-gray-500 dark:text-gray-400">
            {activeJobs.length} active, {pausedJobs.length} paused,{" "}
            {failedJobs.length} failed
          </span>
        </div>
        <div className="flex items-center gap-2">
          {/* Show Failed Only toggle */}
          <button
            onClick={onToggleShowFailed}
            className={`text-xs px-2 py-1 rounded ${
              showFailedOnly
                ? "bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400"
                : "bg-gray-100 hover:bg-gray-200 dark:bg-gray-800 dark:hover:bg-gray-700"
            }`}
            type="button"
            title="Show only failed transfers"
          >
            {showFailedOnly ? "Show All" : "Show Failed Only"}
          </button>
          {failedJobs.length > 0 && (
            <button
              onClick={onRetryAllFailed}
              className="text-xs px-2 py-1 rounded bg-orange-100 text-orange-700 hover:bg-orange-200 dark:bg-orange-900/30 dark:text-orange-400 dark:hover:bg-orange-900/50"
              type="button"
            >
              Retry All Failed ({failedJobs.length})
            </button>
          )}
          <button
            onClick={() => setShowThrottle(!showThrottle)}
            className="text-xs px-2 py-1 rounded bg-gray-100 hover:bg-gray-200 dark:bg-gray-800 dark:hover:bg-gray-700"
            type="button"
            title="Bandwidth throttle settings"
          >
            Throttle
            {throttleConfig.global_limit_bps > 0 &&
              ` (${formatBytes(throttleConfig.global_limit_bps)}/s)`}
          </button>
          <button
            onClick={() => setShowSettings(!showSettings)}
            className={`text-xs px-2 py-1 rounded ${
              showSettings
                ? "bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400"
                : "bg-gray-100 hover:bg-gray-200 dark:bg-gray-800 dark:hover:bg-gray-700"
            }`}
            type="button"
            title="Transfer settings"
          >
            Settings
          </button>
          <button
            onClick={onCollapse}
            className="text-xs px-2 py-1 rounded bg-gray-100 hover:bg-gray-200 dark:bg-gray-800 dark:hover:bg-gray-700"
            aria-label="Minimize transfer panel"
            type="button"
          >
            Minimize
          </button>
        </div>
      </div>

      {/* Throttle settings */}
      {showThrottle && (
        <div className="px-3 py-2 border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/50 space-y-2">
          {/* Global throttle */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-gray-600 dark:text-gray-400">
              Global limit:
            </label>
            <input
              type="number"
              value={throttleInput}
              onChange={(e) => setThrottleInput(e.target.value)}
              placeholder="Unlimited"
              className="w-20 text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              aria-label="Bandwidth limit in MB/s"
            />
            <span className="text-xs text-gray-500">MB/s</span>
            <button
              onClick={() => {
                const mbps = parseFloat(throttleInput) || 0;
                onSetThrottle(mbps * 1024 * 1024);
              }}
              className="text-xs px-2 py-1 rounded bg-blue-500 text-white hover:bg-blue-600"
              type="button"
            >
              Apply
            </button>
            {throttleConfig.global_limit_bps > 0 && (
              <button
                onClick={() => {
                  setThrottleInput("");
                  onSetThrottle(0);
                }}
                className="text-xs px-2 py-1 rounded text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30"
                type="button"
              >
                Remove Limit
              </button>
            )}
          </div>

          {/* Per-connection throttle */}
          <div className="flex items-center gap-2">
            <label className="text-xs text-gray-600 dark:text-gray-400">
              Per-connection:
            </label>
            <input
              type="text"
              value={connThrottleId}
              onChange={(e) => setConnThrottleId(e.target.value)}
              placeholder="Connection ID"
              className="w-28 text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              aria-label="Connection ID for throttle"
            />
            <input
              type="number"
              value={connThrottleValue}
              onChange={(e) => setConnThrottleValue(e.target.value)}
              placeholder="MB/s"
              className="w-16 text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              aria-label="Per-connection bandwidth limit in MB/s"
            />
            <span className="text-xs text-gray-500">MB/s</span>
            <button
              onClick={() => {
                if (!connThrottleId) return;
                const mbps = parseFloat(connThrottleValue) || 0;
                onSetConnectionThrottle(connThrottleId, mbps * 1024 * 1024);
                setConnThrottleId("");
                setConnThrottleValue("");
              }}
              disabled={!connThrottleId}
              className="text-xs px-2 py-1 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50"
              type="button"
            >
              Set
            </button>
          </div>
        </div>
      )}

      {/* Transfer settings */}
      {showSettings && (
        <TransferSettings
          config={transferConfig}
          onSetVerifyTier={onSetVerifyTier}
          onSetVerifyChecksums={onSetVerifyChecksums}
          onSetChecksumAlgorithm={onSetChecksumAlgorithm}
          onSetRetryPolicy={onSetRetryPolicy}
          onSetConflictPolicy={onSetConflictPolicy}
          onSearchHistory={onSearchHistory}
          onExportHistory={onExportHistory}
          onCleanupHistory={onCleanupHistory}
          onSaveAllSettings={onSaveAllSettings}
        />
      )}

      {/* Job list */}
      <div className="flex-1 overflow-y-auto" role="list">
        {jobs.length === 0 ? (
          <div className="flex items-center justify-center h-24 text-sm text-gray-400 dark:text-gray-500">
            {showFailedOnly ? "No failed transfers" : "No transfers"}
          </div>
        ) : (
          jobs.map((job) => (
            <TransferItem
              key={job.id}
              job={job}
              onPause={onPause}
              onResume={onResume}
              onCancel={onCancel}
              onRetry={onRetry}
              onSetPriority={onSetPriority}
              onSetConflictPolicy={onSetItemConflictPolicy}
              onReorder={onReorder}
              onResolveConflict={onResolveConflict}
              totalJobs={jobs.length}
            />
          ))
        )}
      </div>

      {/* Summary footer */}
      {completedJobs.length + cancelledJobs.length > 0 && (
        <div className="px-3 py-1.5 border-t border-gray-200 dark:border-gray-700 text-xs text-gray-500 dark:text-gray-400">
          {completedJobs.length > 0 && (
            <span className="mr-3">{completedJobs.length} completed</span>
          )}
          {cancelledJobs.length > 0 && (
            <span>{cancelledJobs.length} cancelled</span>
          )}
        </div>
      )}
    </div>
  );
}

// ── Main Transfer Panel ──

export function TransferPanel() {
  const [mode, setMode] = useState<TransferPanelMode>("mini");
  const [jobs, setJobs] = useState<TransferJob[]>([]);
  const [showFailedOnly, setShowFailedOnly] = useState(false);
  const [throttleConfig, setThrottleConfig] = useState<ThrottleConfig>({
    global_limit_bps: 0,
    per_connection_limit_bps: 0,
  });
  const [transferConfig, setTransferConfig] = useState<TransferConfig>({
    verify_tier: "standard",
    verify_checksums: false,
    checksum_algorithm: "sha256",
    conflict_policy: "ask",
    max_retries: 3,
    backoff_seconds: [1, 2, 4],
  });

  const pollRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Poll for transfer updates
  const refreshJobs = useCallback(async () => {
    try {
      if (showFailedOnly) {
        const result = await tauriInvoke<TransferJob[]>("list_failed_transfers", undefined, []);
        setJobs(result);
      } else {
        const result = await tauriInvoke<TransferJob[]>("list_transfers", undefined, []);
        setJobs(result);
      }
    } catch (err) {
      console.error("Failed to list transfers:", err);
    }
  }, [showFailedOnly]);

  const refreshThrottle = useCallback(async () => {
    try {
      const result = await tauriInvoke<ThrottleConfig>("get_throttle_config", undefined, {
        global_limit_bps: 0,
        per_connection_limit_bps: 0,
      });
      setThrottleConfig(result);
    } catch (err) {
      console.error("Failed to get throttle config:", err);
    }
  }, []);

  const refreshTransferConfig = useCallback(async () => {
    try {
      const result = await tauriInvoke<TransferConfig>("get_transfer_config", undefined, {
        verify_tier: "standard",
        verify_checksums: false,
        checksum_algorithm: "sha256",
        conflict_policy: "ask",
        max_retries: 3,
        backoff_seconds: [1, 2, 4],
      });
      setTransferConfig(result);
    } catch (err) {
      console.error("Failed to get transfer config:", err);
    }
  }, []);

  // Fetch verify tier on mount and use it to initialize the dropdown
  const refreshVerifyTier = useCallback(async () => {
    try {
      const tier = await tauriInvoke<string>("get_verify_tier", undefined, "standard");
      setTransferConfig((prev) => ({ ...prev, verify_tier: tier }));
    } catch (err) {
      console.error("Failed to get verify tier:", err);
    }
  }, []);

  useEffect(() => {
    refreshJobs();
    refreshThrottle();
    refreshTransferConfig();
    refreshVerifyTier();

    // Poll every 500ms when in full mode, 2s in mini mode
    const interval = mode === "full" ? 500 : 2000;
    pollRef.current = setInterval(refreshJobs, interval);

    return () => {
      if (pollRef.current) clearInterval(pollRef.current);
    };
  }, [mode, refreshJobs, refreshThrottle, refreshTransferConfig, refreshVerifyTier]);

  // ── Action handlers ──

  const handlePause = useCallback(
    async (jobId: string) => {
      try {
        await tauriInvoke("pause_transfer", { jobId });
        refreshJobs();
      } catch (err) {
        console.error("Failed to pause transfer:", err);
      }
    },
    [refreshJobs]
  );

  const handleResume = useCallback(
    async (jobId: string) => {
      try {
        await tauriInvoke("resume_transfer", { jobId });
        refreshJobs();
      } catch (err) {
        console.error("Failed to resume transfer:", err);
      }
    },
    [refreshJobs]
  );

  const handleCancel = useCallback(
    async (jobId: string) => {
      try {
        await tauriInvoke("cancel_transfer", { jobId });
        refreshJobs();
      } catch (err) {
        console.error("Failed to cancel transfer:", err);
      }
    },
    [refreshJobs]
  );

  const handleRetry = useCallback(
    async (jobId: string) => {
      try {
        await tauriInvoke("retry_transfer", { jobId });
        refreshJobs();
      } catch (err) {
        console.error("Failed to retry transfer:", err);
      }
    },
    [refreshJobs]
  );

  const handleSetPriority = useCallback(
    async (jobId: string, priority: string) => {
      try {
        await tauriInvoke("set_transfer_priority", { jobId, priority });
        refreshJobs();
      } catch (err) {
        console.error("Failed to set priority:", err);
      }
    },
    [refreshJobs]
  );

  const handleRetryAllFailed = useCallback(async () => {
    try {
      await tauriInvoke("retry_all_failed");
      refreshJobs();
    } catch (err) {
      console.error("Failed to retry all failed:", err);
    }
  }, [refreshJobs]);

  const handleSetThrottle = useCallback(
    async (bps: number) => {
      try {
        await tauriInvoke("set_global_throttle", { bytesPerSec: bps });
        refreshThrottle();
      } catch (err) {
        console.error("Failed to set throttle:", err);
      }
    },
    [refreshThrottle]
  );

  // (4) set_connection_throttle - per-connection throttle
  const handleSetConnectionThrottle = useCallback(
    async (connectionId: string, bytesPerSec: number) => {
      try {
        await tauriInvoke("set_connection_throttle", { connectionId, bytesPerSec });
        refreshThrottle();
      } catch (err) {
        console.error("Failed to set connection throttle:", err);
      }
    },
    [refreshThrottle]
  );

  // (3) set_transfer_conflict_policy - per-item conflict policy
  const handleSetItemConflictPolicy = useCallback(
    async (jobId: string, policy: string) => {
      try {
        await tauriInvoke("set_transfer_conflict_policy", { jobId, policy });
        refreshJobs();
      } catch (err) {
        console.error("Failed to set item conflict policy:", err);
      }
    },
    [refreshJobs]
  );

  // (5) reorder_transfer - reorder items
  const handleReorder = useCallback(
    async (jobId: string, newSortOrder: number) => {
      try {
        await tauriInvoke("reorder_transfer", { jobId, newSortOrder });
        refreshJobs();
      } catch (err) {
        console.error("Failed to reorder transfer:", err);
      }
    },
    [refreshJobs]
  );

  // (7) resolve_conflict - check and resolve conflicts
  const handleResolveConflict = useCallback(
    async (job: TransferJob) => {
      try {
        // First check the conflict
        const conflict = await tauriInvoke<ConflictCheckResult>(
          "check_conflict",
          { sourcePath: job.source_path, destPath: job.dest_path }
        );
        if (conflict && conflict.has_conflict) {
          // Resolve the conflict using the current global conflict policy
          const action = await tauriInvoke<string>("resolve_conflict", {
            jobId: job.id,
            sourcePath: job.source_path,
            destPath: job.dest_path,
            policy: transferConfig.conflict_policy,
          });
          console.log(`Conflict resolved for ${job.id}: ${action}`);
        }
        refreshJobs();
      } catch (err) {
        console.error("Failed to resolve conflict:", err);
      }
    },
    [refreshJobs, transferConfig.conflict_policy]
  );

  // (6) list_failed_transfers - toggle
  const handleToggleShowFailed = useCallback(() => {
    setShowFailedOnly((prev) => !prev);
  }, []);

  // ── Transfer config handlers ──

  const handleSetVerifyTier = useCallback(
    async (tier: string) => {
      try {
        await tauriInvoke("set_verify_tier", { tier });
        setTransferConfig((prev) => ({ ...prev, verify_tier: tier }));
      } catch (err) {
        console.error("Failed to set verify tier:", err);
      }
    },
    []
  );

  const handleSetVerifyChecksums = useCallback(
    async (verify: boolean) => {
      try {
        await tauriInvoke("set_verify_checksums", { verify });
        setTransferConfig((prev) => ({ ...prev, verify_checksums: verify }));
      } catch (err) {
        console.error("Failed to set verify checksums:", err);
      }
    },
    []
  );

  const handleSetChecksumAlgorithm = useCallback(
    async (algorithm: string) => {
      try {
        await tauriInvoke("set_checksum_algorithm", { algorithm });
        setTransferConfig((prev) => ({ ...prev, checksum_algorithm: algorithm }));
      } catch (err) {
        console.error("Failed to set checksum algorithm:", err);
      }
    },
    []
  );

  const handleSetRetryPolicy = useCallback(
    async (maxRetries: number, backoffSeconds: number[]) => {
      try {
        await tauriInvoke("set_retry_policy", { maxRetries, backoffSeconds });
        setTransferConfig((prev) => ({
          ...prev,
          max_retries: maxRetries,
          backoff_seconds: backoffSeconds,
        }));
      } catch (err) {
        console.error("Failed to set retry policy:", err);
      }
    },
    []
  );

  const handleSetConflictPolicy = useCallback(
    async (policy: string) => {
      try {
        await tauriInvoke("set_conflict_policy", { policy });
        setTransferConfig((prev) => ({ ...prev, conflict_policy: policy }));
      } catch (err) {
        console.error("Failed to set conflict policy:", err);
      }
    },
    []
  );

  // (2) set_transfer_config - save entire config at once
  const handleSaveAllSettings = useCallback(
    async (config: TransferConfig) => {
      try {
        await tauriInvoke("set_transfer_config", { config });
        setTransferConfig(config);
        console.log("All transfer settings saved.");
      } catch (err) {
        console.error("Failed to save all settings:", err);
      }
    },
    []
  );

  const handleSearchHistory = useCallback(async (query: string) => {
    try {
      const filter: TransferHistoryFilter = { query: query || undefined, limit: 100 };
      const results = await tauriInvoke<TransferHistoryRecord[]>(
        "search_transfer_history",
        { filter },
        []
      );
      console.log("Transfer history search results:", results);
    } catch (err) {
      console.error("Failed to search transfer history:", err);
    }
  }, []);

  const handleExportHistory = useCallback(async (format: string) => {
    try {
      const data = await tauriInvoke<string>("export_transfer_history", { format }, "");
      // Trigger a file download in the browser
      const blob = new Blob([data], { type: format === "json" ? "application/json" : "text/csv" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `transfer-history.${format}`;
      document.body.appendChild(a);
      a.click();
      document.body.removeChild(a);
      URL.revokeObjectURL(url);
    } catch (err) {
      console.error("Failed to export transfer history:", err);
    }
  }, []);

  const handleCleanupHistory = useCallback(async (days: number) => {
    try {
      const removed = await tauriInvoke<number>("cleanup_transfer_history", { olderThanDays: days }, 0);
      console.log(`Cleaned up ${removed} transfer history records`);
    } catch (err) {
      console.error("Failed to cleanup transfer history:", err);
    }
  }, []);

  if (mode === "mini") {
    return (
      <TransferPanelMini jobs={jobs} onExpand={() => setMode("full")} />
    );
  }

  return (
    <TransferPanelFull
      jobs={jobs}
      onCollapse={() => setMode("mini")}
      onPause={handlePause}
      onResume={handleResume}
      onCancel={handleCancel}
      onRetry={handleRetry}
      onSetPriority={handleSetPriority}
      onRetryAllFailed={handleRetryAllFailed}
      throttleConfig={throttleConfig}
      onSetThrottle={handleSetThrottle}
      onSetConnectionThrottle={handleSetConnectionThrottle}
      transferConfig={transferConfig}
      onSetVerifyTier={handleSetVerifyTier}
      onSetVerifyChecksums={handleSetVerifyChecksums}
      onSetChecksumAlgorithm={handleSetChecksumAlgorithm}
      onSetRetryPolicy={handleSetRetryPolicy}
      onSetConflictPolicy={handleSetConflictPolicy}
      onSetItemConflictPolicy={handleSetItemConflictPolicy}
      onReorder={handleReorder}
      onResolveConflict={handleResolveConflict}
      onSearchHistory={handleSearchHistory}
      onExportHistory={handleExportHistory}
      onCleanupHistory={handleCleanupHistory}
      onSaveAllSettings={handleSaveAllSettings}
      showFailedOnly={showFailedOnly}
      onToggleShowFailed={handleToggleShowFailed}
    />
  );
}

export default TransferPanel;
