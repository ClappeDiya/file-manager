/**
 * The files one sync run changed: what it copied, updated, deleted, moved or
 * could not handle, loaded when the run row is opened. At most 10 000 are
 * stored per run; when the run touched more, the list says so (failures and
 * conflicts are always kept).
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { formatBytes } from "@/lib/format-bytes";
import { VirtualRows } from "@/components/virtual-list";

export type SyncItemAction = "copied" | "updated" | "deleted" | "moved" | "renamed" | "failed" | "conflict";

export interface SyncRunItem {
  rel_path: string;
  action: SyncItemAction;
  bytes: number;
  error: string | null;
}

interface RunItemsPage {
  items: SyncRunItem[];
  total: number;
  truncated: boolean;
}

const PAGE = 5000;

const TONE: Record<SyncItemAction, string> = {
  copied: "text-green-600",
  updated: "text-blue-600",
  deleted: "text-amber-600",
  moved: "text-violet-600",
  renamed: "text-violet-600",
  failed: "text-red-600",
  conflict: "text-orange-600",
};

export function SyncRunItems({ runId }: { runId: string }) {
  const [items, setItems] = useState<SyncRunItem[]>([]);
  const [total, setTotal] = useState(0);
  const [truncated, setTruncated] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [only, setOnly] = useState<SyncItemAction | null>(null);

  const load = useCallback(
    async (offset: number) => {
      setLoading(true);
      try {
        const page = await tauriInvoke<RunItemsPage>("get_sync_run_items", { runId, offset, limit: PAGE });
        setItems((prev) => (offset === 0 ? page.items : [...prev, ...page.items]));
        setTotal(page.total);
        setTruncated(page.truncated);
        setError(null);
      } catch (e) {
        setError(errorText(e));
      } finally {
        setLoading(false);
      }
    },
    [runId],
  );

  useEffect(() => {
    void load(0);
  }, [load]);

  const counts = useMemo(() => {
    const c = new Map<SyncItemAction, number>();
    for (const i of items) c.set(i.action, (c.get(i.action) ?? 0) + 1);
    return [...c.entries()];
  }, [items]);
  const shown = useMemo(() => (only ? items.filter((i) => i.action === only) : items), [items, only]);

  if (error) return <div className="text-xs text-red-600" role="alert">{error}</div>;
  if (!loading && items.length === 0) return <div className="text-xs text-zinc-500">This run changed no files.</div>;

  return (
    <div className="space-y-1" data-testid="sync-run-items">
      <div className="flex items-center gap-1 flex-wrap text-[11px]">
        <button type="button" className={`px-1.5 rounded border ${only === null ? "bg-blue-500 border-blue-500 text-white" : "border-zinc-300 dark:border-zinc-600"}`} aria-pressed={only === null} onClick={() => setOnly(null)}>
          All {items.length}
        </button>
        {counts.map(([action, n]) => (
          <button
            key={action}
            type="button"
            className={`px-1.5 rounded border ${only === action ? "bg-blue-500 border-blue-500 text-white" : "border-zinc-300 dark:border-zinc-600"}`}
            aria-pressed={only === action}
            onClick={() => setOnly(only === action ? null : action)}
          >
            {action} {n}
          </button>
        ))}
        {loading && <span className="text-zinc-500">Loading…</span>}
      </div>
      <VirtualRows<SyncRunItem>
        items={shown}
        ariaLabel="Changed files"
        height={Math.min(260, Math.max(48, shown.length * 22 + 4))}
        renderRow={(i) => (
          <>
            <span className={`w-16 shrink-0 text-[11px] ${TONE[i.action] ?? ""}`}>{i.action}</span>
            <span className="truncate flex-1 text-[11px] font-mono" title={i.error ? `${i.rel_path}: ${i.error}` : i.rel_path}>
              {i.rel_path}
              {i.error ? <span className="text-red-600"> — {i.error}</span> : null}
            </span>
            <span className="w-16 shrink-0 text-right text-[11px] text-zinc-500">{i.action === "failed" ? "" : formatBytes(i.bytes)}</span>
          </>
        )}
      />
      {items.length < total && (
        <button type="button" className="text-[11px] underline" onClick={() => void load(items.length)} disabled={loading}>
          Load {Math.min(PAGE, total - items.length)} more ({total - items.length} not shown)
        </button>
      )}
      {truncated && (
        <div className="text-[11px] text-amber-700 dark:text-amber-400">
          This run changed more files than are kept ({total.toLocaleString()} listed); failures and conflicts are always listed.
        </div>
      )}
    </div>
  );
}
