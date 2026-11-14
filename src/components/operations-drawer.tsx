/**
 * Operations drawer: everything running or queued (transfers, syncs,
 * backups, search indexing) with progress and cancel, and what finished
 * recently across all engines (file operations, archives, automation…).
 * Polls `operations_snapshot` once a second while open.
 */
import { useCallback, useEffect, useState } from "react";
import { Activity, Loader2, X, XCircle } from "lucide-react";
import { Button } from "@ufop/ui-components";
import { isTauriAvailable } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { formatBytes } from "@/lib/format-bytes";
import { formatEta, operations, useOperationsStore, type OpItem, type OpsSnapshot } from "@/stores/operations-store";

const POLL_MS = 1000;
const INDICATOR_POLL_MS = 3000;
const ENGINE_LABEL: Record<string, string> = { transfer: "Transfer", sync: "Sync", backup: "Backup", index: "Search index", fs: "Files", automation: "Automation", connector: "Connection" };
const STATUS_TONE: Record<string, string> = { ok: "text-green-600", failed: "text-red-500", cancelled: "text-amber-600", partial: "text-amber-600" };

function OpRow({ op, onCancel }: { op: OpItem; onCancel: () => void }) {
  const pct = op.fraction === null ? null : Math.round(op.fraction * 100);
  const eta = formatEta(op.eta_secs);
  return (
    <li className="rounded border border-[var(--color-border)] p-2 space-y-1" aria-label={op.title}>
      <div className="flex items-center gap-2">
        {op.state === "running" ? <Loader2 className="h-3.5 w-3.5 animate-spin shrink-0" aria-hidden="true" /> : <Activity className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />}
        <span className="font-medium truncate flex-1">{op.title}</span>
        <span className="text-[color:var(--color-text-tertiary)] shrink-0">{ENGINE_LABEL[op.engine] ?? op.engine}</span>
        {op.cancellable && (
          <button type="button" onClick={onCancel} aria-label={`Cancel ${op.title}`} className="text-[color:var(--color-text-tertiary)] hover:text-red-500">
            <XCircle className="h-3.5 w-3.5" />
          </button>
        )}
      </div>
      {op.detail && <div className="text-[color:var(--color-text-tertiary)] truncate" title={op.detail}>{op.detail}</div>}
      <div className="h-1.5 rounded bg-[var(--color-bg)] overflow-hidden" role="progressbar" aria-valuenow={pct ?? undefined} aria-valuemin={0} aria-valuemax={100} aria-label={`${op.title} progress`}>
        <div className={`h-full bg-[var(--color-primary)] ${pct === null && op.state === "running" ? "w-1/3 animate-pulse" : ""}`} style={pct === null ? undefined : { width: `${pct}%` }} />
      </div>
      <div className="flex gap-2 text-[color:var(--color-text-tertiary)]">
        <span>{op.state === "running" ? (pct === null ? "Working" : `${pct}%`) : op.state === "queued" ? "Waiting" : "Paused"}</span>
        {op.items_total > 1 && <span>{op.items_done.toLocaleString()} / {op.items_total.toLocaleString()} items</span>}
        {op.bytes_total > 0 && <span>{formatBytes(op.bytes_done)} / {formatBytes(op.bytes_total)}</span>}
        {op.bytes_per_sec > 0 && <span>{formatBytes(op.bytes_per_sec)}/s</span>}
        {eta && <span>{eta} left</span>}
      </div>
    </li>
  );
}

export function OperationsDrawer() {
  const open = useOperationsStore((s) => s.open);
  const setOpen = useOperationsStore((s) => s.setOpen);
  const [snap, setSnap] = useState<OpsSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setSnap(await operations.snapshot(30));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);
  useEffect(() => {
    if (!open || !isTauriAvailable()) return;
    void refresh();
    const t = setInterval(refresh, POLL_MS);
    return () => clearInterval(t);
  }, [open, refresh]);

  if (!open) return null;
  return (
    <aside className="fixed right-0 top-0 bottom-0 z-50 w-96 max-w-full flex flex-col border-l border-[var(--color-border)] bg-[var(--color-bg-secondary)] shadow-xl text-xs" aria-label="Operations">
      <div className="flex items-center justify-between px-3 py-2 border-b border-[var(--color-border)]">
        <h2 className="text-sm font-semibold">Operations</h2>
        <Button variant="ghost" size="icon" onClick={() => setOpen(false)} aria-label="Close operations">
          <X className="h-4 w-4" />
        </Button>
      </div>
      <div className="flex-1 overflow-y-auto p-3 space-y-4">
        {error && <p className="text-red-500" role="alert">{error}</p>}
        <section aria-labelledby="ops-active">
          <h3 id="ops-active" className="font-semibold mb-2">In progress</h3>
          {!snap ? (
            <p className="text-[color:var(--color-text-tertiary)]">Loading…</p>
          ) : snap.active.length === 0 ? (
            <p className="text-[color:var(--color-text-tertiary)]">Nothing running.</p>
          ) : (
            <ul className="space-y-2">
              {snap.active.map((op) => (
                <OpRow key={`${op.engine}:${op.id}`} op={op} onCancel={() => void operations.cancel(op.engine, op.id).then(refresh, (e) => setError(errorText(e)))} />
              ))}
            </ul>
          )}
        </section>
        <section aria-labelledby="ops-recent">
          <h3 id="ops-recent" className="font-semibold mb-2">Recently</h3>
          {snap && snap.recent.length === 0 && <p className="text-[color:var(--color-text-tertiary)]">No recent activity.</p>}
          <ul className="space-y-1">
            {snap?.recent.map((ev) => (
              <li key={ev.id} className="flex gap-2">
                <span className="text-[color:var(--color-text-tertiary)] shrink-0 w-14">{new Date(ev.occurred_at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</span>
                <span className="text-[color:var(--color-text-tertiary)] shrink-0 w-20 truncate">{ENGINE_LABEL[ev.engine] ?? ev.engine}</span>
                <span className="flex-1 truncate" title={ev.summary}>{ev.summary}</span>
                <span className={`shrink-0 ${STATUS_TONE[ev.status] ?? ""}`}>{ev.status}</span>
              </li>
            ))}
          </ul>
        </section>
      </div>
    </aside>
  );
}

/** Status-bar button: count of running operations; opens the drawer. */
export function OperationsIndicator() {
  const toggle = useOperationsStore((s) => s.toggle);
  const open = useOperationsStore((s) => s.open);
  const [count, setCount] = useState(0);
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let live = true;
    const tick = () =>
      operations
        .snapshot(0)
        .then((s) => live && setCount(s.active.filter((o) => o.state === "running").length))
        .catch(() => {});
    void tick();
    const t = setInterval(tick, INDICATOR_POLL_MS);
    return () => {
      live = false;
      clearInterval(t);
    };
  }, []);
  return (
    <button
      type="button"
      onClick={toggle}
      aria-pressed={open}
      aria-label={count ? `${count} operation${count === 1 ? "" : "s"} running — show operations` : "Show operations"}
      className="flex items-center gap-1 px-1.5 rounded hover:bg-[var(--color-hover-bg)]"
      data-testid="operations-indicator"
    >
      {count > 0 ? <Loader2 className="h-3 w-3 animate-spin" aria-hidden="true" /> : <Activity className="h-3 w-3" aria-hidden="true" />}
      {count > 0 ? `${count} running` : "Operations"}
    </button>
  );
}
