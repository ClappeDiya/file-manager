/**
 * Live progress for a running sync: bar, current file, time left, Cancel.
 * Polls `get_sync_progress` while mounted (no event plumbing needed, and it
 * degrades to nothing outside Tauri).
 */
import { useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";
import {
  estimateSecondsLeft,
  formatSecondsLeft,
  phaseLabel,
  progressPercent,
  type SyncProgress,
} from "@/lib/sync-progress";

const POLL_MS = 400;

export function SyncProgressCard({ pairId, pairName }: { pairId: string; pairName: string }) {
  const [progress, setProgress] = useState<SyncProgress | null>(null);
  const [cancelling, setCancelling] = useState(false);

  useEffect(() => {
    let alive = true;
    const tick = async () => {
      const p = await tauriInvokeSafe<SyncProgress | null>("get_sync_progress", { pairId }, null);
      if (alive) setProgress(p);
    };
    tick();
    const id = setInterval(tick, POLL_MS);
    return () => {
      alive = false;
      clearInterval(id);
    };
  }, [pairId]);

  const handleCancel = async () => {
    setCancelling(true);
    try {
      await tauriInvoke<boolean>("cancel_sync", { pairId });
    } catch {
      setCancelling(false);
    }
  };

  const pct = progress ? progressPercent(progress) : 0;
  const stopping = cancelling || progress?.cancel_requested;

  return (
    <div
      className="mb-2 rounded border border-blue-200 dark:border-blue-800 bg-blue-50 dark:bg-blue-900/20 p-2 space-y-1.5"
      role="status"
      aria-live="polite"
      data-testid="sync-progress"
    >
      <div className="flex items-center justify-between gap-2 text-xs">
        <span className="font-medium text-zinc-800 dark:text-zinc-200 truncate">
          Syncing {pairName}
        </span>
        <button
          type="button"
          onClick={handleCancel}
          disabled={!!stopping}
          className="px-2 py-0.5 text-[11px] rounded border border-zinc-300 dark:border-zinc-600 hover:bg-white dark:hover:bg-zinc-800 disabled:opacity-50"
        >
          {stopping ? "Stopping…" : "Cancel"}
        </button>
      </div>
      <div
        className="h-1.5 w-full rounded bg-blue-100 dark:bg-blue-950 overflow-hidden"
        role="progressbar"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={pct}
        aria-label={`Sync progress for ${pairName}`}
      >
        <div className="h-full bg-blue-500 transition-[width] duration-300" style={{ width: `${pct}%` }} />
      </div>
      <div className="flex flex-wrap justify-between gap-x-2 text-[11px] text-zinc-600 dark:text-zinc-400 tabular-nums">
        <span>{progress ? phaseLabel(progress) : "Starting…"}</span>
        {progress && progress.bytes_total > 0 && (
          <span>
            {formatBytes(progress.bytes_done)} / {formatBytes(progress.bytes_total)}
            {" · "}
            {pct}%
          </span>
        )}
      </div>
      {progress?.current_file && (
        <div className="truncate font-mono text-[10px] text-zinc-500" title={progress.current_file}>
          {progress.current_file}
        </div>
      )}
      {progress && estimateSecondsLeft(progress) !== null && (
        <div className="text-[10px] text-zinc-500">
          {formatSecondsLeft(estimateSecondsLeft(progress))}
        </div>
      )}
    </div>
  );
}
