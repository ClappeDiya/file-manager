/**
 * Running syncs as groups in the transfer panel. Each shows its progress and
 * opens to the files it is working on and has finished: a bounded live list
 * (the newest 200 by default; the totals count everything), refreshed while
 * open.
 */
import { useCallback, useEffect, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { formatBytes } from "@/lib/format-bytes";
import { VirtualRows } from "@/components/virtual-list";
import type { OpItem } from "@/stores/operations-store";

export interface LiveFile {
  path: string;
  state: "active" | "done" | "failed" | "conflict" | string;
  action: string | null;
  bytes: number;
  error: string | null;
}

export interface LiveFiles {
  files: LiveFile[];
  active_total: number;
  finished_total: number;
  failed_total: number;
  truncated: boolean;
}

const LIST_LIMIT = 200;

const GLYPH: Record<string, { mark: string; tone: string }> = {
  active: { mark: "…", tone: "text-blue-600" },
  done: { mark: "✓", tone: "text-green-600" },
  failed: { mark: "✗", tone: "text-red-600" },
  conflict: { mark: "!", tone: "text-orange-600" },
};

/** The live file list of one running sync. */
export function LiveSyncFiles({ pairId, intervalMs = 1000 }: { pairId: string; intervalMs?: number }) {
  const [data, setData] = useState<LiveFiles | null | undefined>(undefined);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    const load = async () => {
      try {
        const r = await tauriInvoke<LiveFiles | null>("get_sync_live_files", { pairId, limit: LIST_LIMIT });
        if (live) {
          setData(r);
          setError(null);
        }
      } catch (e) {
        if (live) setError(errorText(e));
      }
    };
    void load();
    const t = setInterval(load, intervalMs);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, [pairId, intervalMs]);

  if (error) return <div className="text-xs text-red-600" role="alert">{error}</div>;
  if (data === undefined) return <div className="text-xs text-gray-500">Loading…</div>;
  if (data === null) return <div className="text-xs text-gray-500">This sync has finished; its changed files are in Sync → History.</div>;
  return (
    <div className="space-y-1" data-testid="live-sync-files">
      <div className="text-[11px] text-gray-500" aria-live="polite">
        {data.active_total} in progress · {data.finished_total} finished{data.failed_total > 0 ? ` · ${data.failed_total} failed` : ""}
      </div>
      <VirtualRows<LiveFile>
        items={data.files}
        ariaLabel="Files of this sync"
        height={Math.min(220, Math.max(48, data.files.length * 22 + 4))}
        followTail={false}
        renderRow={(f) => {
          const g = GLYPH[f.state] ?? GLYPH.done;
          return (
            <>
              <span className={`w-4 shrink-0 text-center ${g.tone}`} aria-label={f.state}>
                {g.mark}
              </span>
              <span className="truncate flex-1 text-[11px] font-mono" title={f.error ? `${f.path}: ${f.error}` : f.path}>
                {f.path}
                {f.error ? <span className="text-red-600"> — {f.error}</span> : null}
              </span>
              <span className="w-16 shrink-0 text-right text-[11px] text-gray-500">{f.state === "active" || f.state === "failed" ? "" : formatBytes(f.bytes)}</span>
            </>
          );
        }}
      />
      {data.truncated && <div className="text-[11px] text-gray-500">Showing the newest files only ({data.finished_total.toLocaleString()} finished in all).</div>}
    </div>
  );
}

/** One running sync in the transfer list. */
export function SyncGroupRow({ item, onCancel }: { item: OpItem; onCancel: (item: OpItem) => void }) {
  const [open, setOpen] = useState(false);
  const pct = item.fraction === null ? null : Math.round(item.fraction * 100);
  const toggle = useCallback(() => setOpen((o) => !o), []);
  return (
    <div className="px-3 py-2 border-b border-gray-100 dark:border-gray-800" role="listitem" data-testid="sync-group">
      <div className="flex items-center gap-2">
        <button type="button" onClick={toggle} aria-expanded={open} aria-label={`${open ? "Hide" : "Show"} files of ${item.title}`} className="text-xs w-4">
          {open ? "▾" : "▸"}
        </button>
        <span className="text-xs font-medium truncate flex-1" title={item.detail ?? undefined}>
          {item.title}
        </span>
        <span className="text-[11px] text-gray-500">Sync</span>
        <span className="text-[11px] text-gray-500 tabular-nums">
          {item.items_done}/{item.items_total || "?"} files{pct !== null ? ` · ${pct}%` : ""}
        </span>
        <button
          type="button"
          disabled={!item.cancellable}
          onClick={() => onCancel(item)}
          className="text-[11px] px-1.5 py-0.5 rounded text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30 disabled:opacity-50"
        >
          Cancel
        </button>
      </div>
      {pct !== null && (
        <div className="mt-1 h-1 rounded bg-gray-200 dark:bg-gray-700 overflow-hidden" aria-hidden="true">
          <div className="h-full bg-blue-500" style={{ width: `${pct}%` }} />
        </div>
      )}
      {item.detail && <div className="mt-0.5 text-[11px] text-gray-500 truncate">{item.detail}</div>}
      {open && (
        <div className="mt-2">
          <LiveSyncFiles pairId={item.id} />
        </div>
      )}
    </div>
  );
}
