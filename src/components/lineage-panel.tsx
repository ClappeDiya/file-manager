/**
 * LineagePanel — "Show File History" provenance viewer.
 *
 * Answers the question no mainstream file manager can: "what is the
 * complete life story of this file?". Every rename, move, copy, sync,
 * transfer, and automation fire that ever touched this file — or any
 * path it used to live at — rendered as a single vertical timeline.
 *
 * Design principles applied here:
 *
 * - **Zero new state**: reads `FileLineage` from the backend via
 *   `get_file_lineage`, which is a pure query over the existing unified
 *   OperationLedger. No polling, no caching, no daemon.
 * - **Opt-in**: the panel is entirely inert until the user explicitly
 *   right-clicks a file → "Show History". Zero cognitive overhead for
 *   users who don't care.
 * - **DRY**: reuses the same `LedgerEvent` shape and row styling the
 *   Activity Timeline already uses. A future refactor could extract a
 *   shared TimelineRow, but today's small duplication is deliberate:
 *   this panel is read-only, has no correlation trace, and no Undo —
 *   extracting a shared row would require configurable props that add
 *   surface area the Activity Timeline doesn't need yet.
 * - **Fail-soft**: an unknown path renders an empty-state card ("No
 *   history recorded for this file yet"), not an error toast.
 */
import { useCallback, useEffect } from "react";
import { useLineageStore, type LineageEvent } from "@/stores/lineage-store";
import { tauriInvoke } from "@/hooks/use-tauri";
import { cn } from "@ufop/ui-components";
import { Button, Badge, ScrollArea } from "@ufop/ui-components";
import {
  X,
  History,
  FileText,
  ArrowRightLeft,
  RefreshCcw,
  Zap,
  HardDrive,
  Layers,
  Bot,
  ShieldCheck,
  Wrench,
  Plug,
  Settings,
  Activity,
  AlertTriangle,
  CheckCircle2,
  XCircle,
  Ban,
  MinusCircle,
} from "lucide-react";

/** Same engine → icon map shape as activity-timeline-panel, kept local
 *  so this component stays self-contained. */
const ENGINE_ICONS: Record<string, typeof Activity> = {
  fs: FileText,
  transfer: ArrowRightLeft,
  sync: RefreshCcw,
  automation: Zap,
  mount: HardDrive,
  spaces: Layers,
  ai: Bot,
  vault: ShieldCheck,
  compat: Wrench,
  connector: Plug,
  system: Settings,
};

const STATUS_META: Record<string, { Icon: typeof CheckCircle2; className: string }> = {
  ok: { Icon: CheckCircle2, className: "text-emerald-500" },
  failed: { Icon: XCircle, className: "text-red-500" },
  cancelled: { Icon: Ban, className: "text-amber-500" },
  skipped: { Icon: MinusCircle, className: "text-[color:var(--color-text-muted)]" },
};

function formatRelative(iso: string): string {
  // SQLite stores `YYYY-MM-DD HH:MM:SS` in UTC; append Z so Date parses it.
  const d = new Date(iso.includes("T") ? iso : `${iso.replace(" ", "T")}Z`);
  if (Number.isNaN(d.getTime())) return iso;
  const diffMs = Date.now() - d.getTime();
  const diffSec = Math.round(diffMs / 1000);
  if (diffSec < 60) return `${diffSec}s ago`;
  const diffMin = Math.round(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m ago`;
  const diffHr = Math.round(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h ago`;
  const diffDay = Math.round(diffHr / 24);
  return `${diffDay}d ago`;
}

function shortPath(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  if (parts.length <= 2) return path;
  return `…/${parts.slice(-2).join("/")}`;
}

export function LineagePanel() {
  const request = useLineageStore((s) => s.request);
  const loading = useLineageStore((s) => s.loading);
  const pendingPath = useLineageStore((s) => s.pendingPath);
  const error = useLineageStore((s) => s.error);
  const setResult = useLineageStore((s) => s.setResult);
  const setError = useLineageStore((s) => s.setError);
  const close = useLineageStore((s) => s.close);

  // When a new path is requested, fetch its lineage. Keyed on
  // `pendingPath` so re-asking for the same path is a no-op (the store
  // already holds the result).
  useEffect(() => {
    if (!pendingPath) return;
    let cancelled = false;
    (async () => {
      try {
        const lineage = await tauriInvoke<{
          root_path: string;
          aliases: string[];
          events: LineageEvent[];
          correlation_ids: string[];
          truncated: boolean;
        }>(
          "get_file_lineage",
          { path: pendingPath, maxDepth: 16 },
          {
            root_path: pendingPath,
            aliases: [],
            events: [],
            correlation_ids: [],
            truncated: false,
          },
        );
        if (!cancelled) setResult(lineage);
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [pendingPath, setResult, setError]);

  // Escape closes. Only bound while the panel is open so there's zero
  // keyboard surface consumed otherwise.
  const panelOpen = pendingPath !== null;
  useEffect(() => {
    if (!panelOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        close();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [panelOpen, close]);

  const onClose = useCallback(() => close(), [close]);

  if (!panelOpen) return null;

  return (
    <aside
      className="flex flex-col border-l border-[var(--color-border)] bg-[var(--color-bg-secondary)]"
      style={{ width: 360 }}
      role="complementary"
      aria-label="File history panel"
    >
      {/* Header */}
      <div className="flex items-center justify-between border-b border-[var(--color-border)] px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <History className="h-4 w-4 flex-shrink-0 text-sky-500" aria-hidden="true" />
          <div className="min-w-0">
            <div className="text-sm font-semibold text-[color:var(--color-text)]">
              File History
            </div>
            <div
              className="truncate text-[10px] text-[color:var(--color-text-muted)]"
              title={pendingPath ?? undefined}
            >
              {pendingPath ? shortPath(pendingPath) : ""}
            </div>
          </div>
        </div>
        <Button
          variant="ghost"
          size="icon"
          onClick={onClose}
          title="Close history panel (Esc)"
          aria-label="Close history panel"
        >
          <X className="h-4 w-4" />
        </Button>
      </div>

      <ScrollArea className="flex-1">
        <div className="flex flex-col gap-2 px-3 py-3">
          {loading && (
            <div className="py-8 text-center text-xs text-[color:var(--color-text-muted)]">
              Walking the operation ledger…
            </div>
          )}

          {!loading && error && (
            <div className="flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/5 px-3 py-2 text-xs text-red-500">
              <AlertTriangle className="mt-0.5 h-3.5 w-3.5 flex-shrink-0" aria-hidden="true" />
              <div>{error}</div>
            </div>
          )}

          {!loading && !error && request && (
            <>
              {/* Aliases list — "also known as" */}
              {request.aliases.length > 0 && (
                <div className="rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] p-2">
                  <div className="mb-1 text-[10px] font-semibold uppercase tracking-wide text-[color:var(--color-text-muted)]">
                    Also known as
                  </div>
                  <ul className="flex flex-col gap-0.5">
                    {request.aliases.map((alias) => (
                      <li
                        key={alias}
                        className="truncate text-[11px] text-[color:var(--color-text)]"
                        title={alias}
                      >
                        {alias}
                      </li>
                    ))}
                  </ul>
                </div>
              )}

              {/* Truncation warning */}
              {request.truncated && (
                <div className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] text-amber-600 dark:text-amber-400">
                  <AlertTriangle className="mt-0.5 h-3.5 w-3.5 flex-shrink-0" aria-hidden="true" />
                  <div>
                    History walk stopped at the depth cap — older events may exist beyond this window.
                  </div>
                </div>
              )}

              {/* Events list */}
              {request.events.length === 0 ? (
                <div className="py-6 text-center">
                  <History
                    className="mx-auto h-6 w-6 text-[color:var(--color-text-muted)] opacity-50"
                    aria-hidden="true"
                  />
                  <div className="mt-2 text-xs text-[color:var(--color-text-muted)]">
                    No history recorded for this file yet.
                  </div>
                  <div className="mt-1 text-[10px] text-[color:var(--color-text-muted)]">
                    Operations are recorded as engines run. Rename, move, or sync this file to start its history.
                  </div>
                </div>
              ) : (
                <div className="flex items-center gap-2 px-1 pb-1 pt-2 text-[10px] text-[color:var(--color-text-muted)]">
                  <Badge variant="secondary" className="text-[10px]">
                    {request.events.length}
                  </Badge>
                  <span>
                    {request.events.length === 1 ? "event" : "events"}
                    {request.correlation_ids.length > 0 &&
                      ` · ${request.correlation_ids.length} ${request.correlation_ids.length === 1 ? "operation" : "operations"}`}
                  </span>
                </div>
              )}

              {request.events.map((ev) => (
                <LineageRow key={ev.id} event={ev} />
              ))}
            </>
          )}
        </div>
      </ScrollArea>
    </aside>
  );
}

function LineageRow({ event }: { event: LineageEvent }) {
  const EngineIcon = ENGINE_ICONS[event.engine] ?? Activity;
  const statusMeta = STATUS_META[event.status] ?? STATUS_META.ok;
  const StatusIcon = statusMeta.Icon;
  return (
    <div className="flex items-start gap-2 rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-2 py-1.5 hover:bg-[var(--color-bg-tertiary)]">
      <EngineIcon
        className="mt-0.5 h-3.5 w-3.5 flex-shrink-0 text-[color:var(--color-text-muted)]"
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-1.5">
          <StatusIcon
            className={cn("h-3 w-3 flex-shrink-0", statusMeta.className)}
            aria-hidden="true"
          />
          <span
            className="truncate text-xs text-[color:var(--color-text)]"
            title={event.summary}
          >
            {event.summary || event.kind}
          </span>
        </div>
        <div className="mt-0.5 flex items-center gap-2 text-[10px] text-[color:var(--color-text-muted)]">
          <span>{formatRelative(event.occurred_at)}</span>
          <span aria-hidden="true">·</span>
          <span>{event.engine}</span>
          <span aria-hidden="true">·</span>
          <span>{event.kind}</span>
        </div>
        {(event.subject_path || event.target_path) && (
          <div
            className="mt-0.5 truncate text-[10px] text-[color:var(--color-text-muted)]"
            title={`${event.subject_path ?? ""} → ${event.target_path ?? ""}`}
          >
            {event.subject_path && event.target_path
              ? `${shortPath(event.subject_path)} → ${shortPath(event.target_path)}`
              : shortPath(event.subject_path ?? event.target_path ?? "")}
          </div>
        )}
      </div>
    </div>
  );
}
