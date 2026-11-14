/**
 * Two-way sync history (the pair's baseline): how many files both sides
 * agreed on after the last successful run, and a reset. Advanced view only.
 *
 * The baseline is what lets a two-way pair tell "deleted here" from "new
 * there"; resetting it makes the next run combine both sides again and
 * delete nothing.
 */
import { useCallback, useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { useUIStore } from "@/stores/ui-store";

/** Mirrors `BaselineInfo` in src-tauri sync_engine/mod.rs. */
export interface BaselineInfo {
  files: number;
  recorded_at: string | null;
  run_id: string | null;
}

export function TwoWayHistory({ pairId, refreshKey }: { pairId: string; refreshKey?: unknown }) {
  const advanced = useUIStore((s) => s.appMode === "advanced");
  const [info, setInfo] = useState<BaselineInfo | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    setInfo(await tauriInvokeSafe<BaselineInfo | null>("get_sync_baseline_info", { pairId }, null));
  }, [pairId]);

  useEffect(() => {
    if (advanced) void load();
  }, [advanced, load, refreshKey]);

  if (!advanced) return null;

  const reset = async () => {
    if (!confirm("Reset the sync history? The next run combines both folders again and deletes nothing.")) return;
    setError(null);
    try {
      await tauriInvoke("reset_sync_baseline", { pairId });
      await load();
    } catch (e: unknown) {
      setError((e as Error)?.message || String(e));
    }
  };

  return (
    <div className="mt-1 flex flex-wrap items-center gap-2 text-[11px] text-zinc-500" data-testid="two-way-history">
      {info ? (
        <span>
          Sync history: {info.files} file{info.files === 1 ? "" : "s"}
          {info.recorded_at && ` (recorded ${new Date(info.recorded_at).toLocaleString()})`}
        </span>
      ) : (
        <span>No sync history yet: the first run combines both folders and deletes nothing.</span>
      )}
      {info && (
        <button type="button" onClick={() => void reset()} className="text-blue-600 dark:text-blue-400 hover:underline">
          Reset history
        </button>
      )}
      {error && <span className="text-red-600">{error}</span>}
    </div>
  );
}
