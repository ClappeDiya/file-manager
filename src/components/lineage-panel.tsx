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
import { useCallback, useEffect, useMemo, useState } from "react";
import {
  useLineageStore,
  filterLineageEvents,
  type LineageEvent,
} from "@/stores/lineage-store";
import { useAutomationStore } from "@/stores/automation-store";
import { useUIStore } from "@/stores/ui-store";
import { useActivityTimelineStore } from "@/stores/activity-timeline-store";
import {
  isPinnableEvent,
  isRetryableEvent,
  isUndoableKindEvent,
} from "@/lib/ledger-event-flags";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { formatRelativeTime } from "@/lib/ledger-dispatch";
import { formatBytes } from "@/lib/format-bytes";
import { ENGINE_ICONS, ENGINE_LABELS, STATUS_META } from "@/lib/engine-ui-constants";
import { cn } from "@ufop/ui-components";
import { Button, Badge, ScrollArea } from "@ufop/ui-components";
import {
  X,
  History,
  Activity,
  AlertTriangle,
  BookOpen,
  Pin,
  RefreshCw,
  Undo2,
  Folder,
} from "lucide-react";
import { OperationNarrativeCard } from "./operation-narrative-card";


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

  // Per-engine / failed-only filters. Mirror the Activity Timeline's
  // chip pattern so users only learn one filter grammar across the two
  // ledger-derived surfaces. Pure client-side over the already-fetched
  // events array — no extra IPC, no extra storage.
  //
  // `"all"` is the sentinel for "no engine filter active" — same
  // convention the timeline uses, so callers and tests can use one
  // shared mental model.
  const [engineFilter, setEngineFilter] = useState<string>("all");
  const [failedOnly, setFailedOnly] = useState(false);

  // Active "explain this operation" target: the correlation_id whose
  // narrator card is currently expanded inline beneath its row. Null
  // means no narrative is open. Matches the Activity Timeline's
  // `correlationFilter` + narrative-card UX so users learn one mental
  // model for "click to explain" across both surfaces.
  const [explainCorrelation, setExplainCorrelation] = useState<string | null>(
    null,
  );

  // Sets of correlation_ids the backend currently considers reversible
  // (Undo) or re-applyable (Redo). Same shape and same source as the
  // Activity Timeline — one `list_undoable` + one `list_redoable` IPC
  // when the lineage panel opens, cached client-side so each row's
  // per-correlation gate is a single Set membership check. Mutually
  // exclusive by construction (the backend's marker-parity logic
  // guarantees one cid is in at most one of the two sets at a time).
  const [undoableIds, setUndoableIds] = useState<Set<string>>(
    () => new Set(),
  );
  const [redoableIds, setRedoableIds] = useState<Set<string>>(
    () => new Set(),
  );

  // Reset filters AND any open narrative whenever the user opens a
  // fresh lineage. Without this, a busy file inspected after a non-busy
  // file would inherit state that may produce a surprising view (e.g.
  // a stale narrative card pointing at a correlation that has no rows
  // in the new lineage). Also clears the undoable/redoable sets so a
  // stale set from a prior lineage can't flash incorrect buttons before
  // the re-fetch lands.
  useEffect(() => {
    setEngineFilter("all");
    setFailedOnly(false);
    setExplainCorrelation(null);
    setUndoableIds(new Set());
    setRedoableIds(new Set());
  }, [pendingPath]);

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

  // Pin / Retry handlers — symmetric mirrors of the Activity Timeline's
  // handlers (see `handlePinEntry` / `handleRetryEntry` over in
  // `activity-timeline-panel.tsx`). Same automation-store actions, same
  // structured-error toast surface, same re-fetch-after-success
  // contract. The only difference is the success path here re-fetches
  // the lineage via `beginRequest(pendingPath)` instead of `loadEvents()`
  // so the panel reflects any new event (the retry's success row, the
  // pin's marker) the next time it lands in the ledger.
  const pinLedgerEvent = useAutomationStore((s) => s.pinLedgerEvent);
  const retryFailedEvent = useAutomationStore((s) => s.retryFailedEvent);
  const beginRequest = useLineageStore((s) => s.beginRequest);
  const refreshLineage = useCallback(() => {
    if (pendingPath !== null) beginRequest(pendingPath);
  }, [pendingPath, beginRequest]);

  const handlePinEntry = useCallback(
    async (correlationId: string, kind: string) => {
      const rule = await pinLedgerEvent(correlationId);
      useUIStore.getState().addStructuredError({
        what: rule ? `Pinned ${kind}` : "Pin failed",
        why: rule
          ? `Saved as "${rule.name}" (disabled — enable from the Automation panel to run it)`
          : "Backend rejected the pin request",
        appDid: rule
          ? "Created a re-runnable manual Quickflow capturing the original sources and destination"
          : "Could not build a replay rule from this ledger event",
        userAction: rule
          ? "Open the Automation panel to enable and run this Quickflow on demand"
          : "Confirm the original copy/move event is still in the ledger and try again",
      });
    },
    [pinLedgerEvent],
  );

  const handleRetryEntry = useCallback(
    async (correlationId: string, kind: string) => {
      const log = await retryFailedEvent(correlationId);
      const succeeded = log?.status === "success";
      const count = log?.files_affected.length ?? 0;
      useUIStore.getState().addStructuredError({
        what: succeeded
          ? `Retried ${kind}`
          : log
            ? `Retry of ${kind} failed`
            : "Retry rejected",
        why: succeeded
          ? `Re-attempted ${count} file${count === 1 ? "" : "s"} successfully`
          : log?.error_message ??
            "Backend rejected the retry — the original event may no longer be eligible",
        appDid: succeeded
          ? "Re-executed the original copy/move and recorded a fresh ledger row for the retry"
          : "Replayed the action through the executor; nothing changed on disk",
        userAction: succeeded
          ? "Reopen this file's history to see the retry row alongside the original failure"
          : "Check that the source files still exist and the destination is writable, then try again",
      });
      if (succeeded) refreshLineage();
    },
    [retryFailedEvent, refreshLineage],
  );

  // Per-entry Undo / Redo handlers — symmetric mirrors of
  // `handleUndoEntry` / `handleRedoEntry` in the Activity Timeline. Same
  // `undo_by_correlation` / `redo_by_correlation` IPCs, same structured
  // toast surface. After success, re-fetch the lineage so the undo/redo
  // marker row (`fs.undone` / `fs.redone`) appears in the panel and the
  // membership sets refresh below.
  const handleUndoEntry = useCallback(
    async (correlationId: string) => {
      try {
        const outcome = await tauriInvoke<{
          success: boolean;
          correlation_id: string;
          kind: string;
          summary: string;
          item_count: number;
        }>("undo_by_correlation", { correlationId }, {
          success: false,
          correlation_id: correlationId,
          kind: "",
          summary: "",
          item_count: 0,
        });
        useUIStore.getState().addStructuredError({
          what: outcome.success ? `Undid ${outcome.kind}` : "Undo failed",
          why: outcome.summary,
          appDid: outcome.success
            ? `Reversed ${outcome.item_count} item(s) via operation ledger`
            : "Backend rejected the undo request",
          userAction: outcome.success
            ? "The file has been restored to its previous state"
            : "Check that the files still exist at their current paths",
        });
        if (outcome.success) refreshLineage();
      } catch (err) {
        console.error("Undo by correlation failed:", err);
      }
    },
    [refreshLineage],
  );

  const handleRedoEntry = useCallback(
    async (correlationId: string) => {
      try {
        const outcome = await tauriInvoke<{
          success: boolean;
          correlation_id: string;
          kind: string;
          summary: string;
          item_count: number;
        }>("redo_by_correlation", { correlationId }, {
          success: false,
          correlation_id: correlationId,
          kind: "",
          summary: "",
          item_count: 0,
        });
        useUIStore.getState().addStructuredError({
          what: outcome.success ? `Redid ${outcome.kind}` : "Redo failed",
          why: outcome.summary,
          appDid: outcome.success
            ? `Re-applied ${outcome.item_count} item(s) via operation ledger`
            : "Backend rejected the redo request",
          userAction: outcome.success
            ? "The operation has been re-applied"
            : "Check that the original source files still exist at their expected paths",
        });
        if (outcome.success) refreshLineage();
      } catch (err) {
        console.error("Redo by correlation failed:", err);
      }
    },
    [refreshLineage],
  );

  // Fetch undoable/redoable sets whenever a fresh lineage result lands.
  // Cheap (one `ledger_recent` scan per IPC) and runs only when there
  // are events to potentially decorate — never when the panel is closed
  // or in the loading state. Falls back to empty sets outside Tauri so
  // the browser preview stays green.
  useEffect(() => {
    if (request === null) return;
    if (request.events.length === 0) return;
    let cancelled = false;
    (async () => {
      const [undoable, redoable] = await Promise.all([
        tauriInvokeSafe<Array<{ correlation_id: string }>>(
          "list_undoable",
          { limit: 100 },
          [],
        ),
        tauriInvokeSafe<Array<{ correlation_id: string }>>(
          "list_redoable",
          { limit: 100 },
          [],
        ),
      ]);
      if (cancelled) return;
      setUndoableIds(new Set((undoable ?? []).map((o) => o.correlation_id)));
      setRedoableIds(new Set((redoable ?? []).map((o) => o.correlation_id)));
    })();
    return () => {
      cancelled = true;
    };
  }, [request]);

  // Per-engine counts derived from the full event list. Used to render
  // a chip per engine that actually appeared in this file's lineage —
  // we never render a "Sync (0)" chip for an engine that never touched
  // the file, mirroring the Activity Timeline's `activeEngines` logic.
  const engineCounts = useMemo<Record<string, number>>(() => {
    const counts: Record<string, number> = {};
    if (!request) return counts;
    for (const ev of request.events) {
      counts[ev.engine] = (counts[ev.engine] ?? 0) + 1;
    }
    return counts;
  }, [request]);
  const availableEngines = useMemo(
    () => Object.keys(engineCounts).sort(),
    [engineCounts],
  );
  const failedCount = useMemo<number>(() => {
    if (!request) return 0;
    return request.events.reduce(
      (n, ev) => n + (ev.status !== "ok" ? 1 : 0),
      0,
    );
  }, [request]);

  // Apply both filters via the shared pure helper so the filter
  // semantics are unit-testable without rendering the panel. Returns
  // the same array reference when neither filter is active.
  const filteredEvents = useMemo<LineageEvent[]>(() => {
    if (!request) return [];
    return filterLineageEvents(request.events, engineFilter, failedOnly);
  }, [request, engineFilter, failedOnly]);

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
            <div
              className="text-sm font-semibold text-[color:var(--color-text)]"
              title="File History (⌘⇧G to toggle)"
            >
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
        <div className="flex items-center gap-1">
          {/*
           * Refresh — re-fetches the lineage for the current path.
           * Useful when the user performs an operation in another
           * panel (timeline, file list) while lineage is open and
           * wants the panel to reflect the new event without closing
           * and reopening. Reuses the existing `beginRequest` action
           * which sets loading=true and re-fetches in place.
           */}
          {pendingPath && (
            <Button
              variant="ghost"
              size="icon"
              onClick={() => beginRequest(pendingPath)}
              title="Refresh history"
              aria-label="Refresh file history"
              data-testid="lineage-refresh"
              disabled={loading}
            >
              <RefreshCw
                className={cn("h-4 w-4", loading && "animate-spin")}
                aria-hidden="true"
              />
            </Button>
          )}
          {/*
           * "View folder activity" — bridges the per-file Lineage Panel
           * to the folder-scoped Activity Timeline (introduced in
           * iteration 2). Computes the parent directory of the current
           * pendingPath, then opens the timeline pre-filtered to it
           * via the existing `openWithPreset({pathFilter})` primitive.
           * Cross-surface composition with zero new infrastructure:
           * lineage answers "what happened to THIS file?", timeline
           * answers "what happened in this file's folder?" — both
           * derived from the same operation_ledger.
           */}
          {pendingPath && (() => {
            // Pre-compute the parent path AND its short leaf so both
            // the click handler and the hover/aria text see the same
            // result. Keeps the click and the label aligned even if a
            // future refactor changes the parent-extraction rule.
            const parent = pendingPath.includes("/")
              ? pendingPath.slice(0, pendingPath.lastIndexOf("/"))
              : pendingPath.includes("\\")
                ? pendingPath.slice(0, pendingPath.lastIndexOf("\\"))
                : "";
            const parentLeaf = parent.length === 0
              ? null
              : shortPath(parent);
            return (
              <Button
                variant="ghost"
                size="icon"
                onClick={() => {
                  if (parent.length === 0) return;
                  useActivityTimelineStore.getState().openWithPreset({
                    pathFilter: parent,
                    engineFilter: "all",
                    failedOnly: false,
                    correlationFilter: null,
                  });
                }}
                title={
                  parentLeaf
                    ? `View activity in ${parentLeaf}`
                    : "View this folder's activity in the Timeline"
                }
                aria-label={
                  parentLeaf
                    ? `View activity in ${parentLeaf} in the Activity Timeline`
                    : "View this folder's activity in the Activity Timeline"
                }
                data-testid="lineage-view-folder-activity"
              >
                <Folder className="h-4 w-4" />
              </Button>
            );
          })()}
          <Button
            variant="ghost"
            size="icon"
            onClick={onClose}
            title="Close history panel (Esc or ⌘⇧G to toggle)"
            aria-label="Close history panel"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>

      <ScrollArea className="flex-1">
        <div className="flex flex-col gap-2 px-3 py-3">
          {loading && (
            <div
              className="py-8 text-center text-xs text-[color:var(--color-text-muted)]"
              role="status"
              aria-live="polite"
            >
              Walking the operation ledger…
            </div>
          )}

          {!loading && error && (
            <div
              className="flex items-start gap-2 rounded-md border border-red-500/30 bg-red-500/5 px-3 py-2 text-xs text-red-500"
              role="alert"
            >
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
                <div
                  className="flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-[11px] text-amber-600 dark:text-amber-400"
                  role="status"
                >
                  <AlertTriangle className="mt-0.5 h-3.5 w-3.5 flex-shrink-0" aria-hidden="true" />
                  <div>
                    History walk stopped at the depth cap — older events may exist beyond this window.
                  </div>
                </div>
              )}

              {/* Events list */}
              {request.events.length === 0 ? (
                <div className="py-6 text-center" role="status" aria-live="polite">
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
                <>
                  {/* Filter chip row — only render when there's enough
                       material to filter (more than one event AND more
                       than one engine OR at least one failure). Below
                       that threshold the panel stays minimal. */}
                  {request.events.length > 1 &&
                    (availableEngines.length > 1 || failedCount > 0) && (
                      <div
                        className="flex flex-wrap gap-1 px-1 pb-1 pt-1"
                        data-testid="lineage-filter-chips"
                      >
                        <LineageFilterChip
                          label="All"
                          count={request.events.length}
                          active={engineFilter === "all" && !failedOnly}
                          onClick={() => {
                            setEngineFilter("all");
                            setFailedOnly(false);
                          }}
                        />
                        {availableEngines.map((engine) => (
                          <LineageFilterChip
                            key={engine}
                            label={ENGINE_LABELS[engine] ?? engine}
                            count={engineCounts[engine] ?? 0}
                            active={engineFilter === engine && !failedOnly}
                            onClick={() => {
                              setEngineFilter(engine);
                              setFailedOnly(false);
                            }}
                          />
                        ))}
                        {failedCount > 0 && (
                          <LineageFilterChip
                            label="Failed"
                            count={failedCount}
                            active={failedOnly}
                            onClick={() => {
                              setFailedOnly((v) => !v);
                              setEngineFilter("all");
                            }}
                            danger
                          />
                        )}
                      </div>
                    )}

                  <div className="flex items-center gap-2 px-1 pb-1 pt-2 text-[10px] text-[color:var(--color-text-muted)]">
                    <Badge variant="secondary" className="text-[10px]">
                      {filteredEvents.length}
                    </Badge>
                    <span>
                      {filteredEvents.length === 1 ? "event" : "events"}
                      {filteredEvents.length !== request.events.length &&
                        ` of ${request.events.length}`}
                      {request.correlation_ids.length > 0 &&
                        ` · ${request.correlation_ids.length} ${request.correlation_ids.length === 1 ? "operation" : "operations"}`}
                    </span>
                  </div>
                </>
              )}

              {filteredEvents.length === 0 && request.events.length > 0 && (
                <div
                  className="py-4 text-center text-[11px] text-[color:var(--color-text-muted)]"
                  role="status"
                  aria-live="polite"
                >
                  No events match the current filter.
                </div>
              )}

              {filteredEvents.map((ev) => {
                // Pin and Retry are mutually exclusive (status is either
                // "ok" or not) — same predicate the Activity Timeline
                // uses, so both surfaces light up the same rows for the
                // same reasons.
                const canPin = isPinnableEvent(ev);
                const canRetry = isRetryableEvent(ev);
                // Undo and Redo are mutually exclusive via the backend's
                // marker-parity logic: a correlation_id appears in at
                // most one of the two sets at a time. The kind gate
                // (`isUndoableKindEvent`) ensures marker rows
                // (`fs.undone` / `fs.redone`) never surface the buttons
                // — only original copy/move/rename/etc. rows do, even
                // though they share the correlation_id with the marker.
                const isUndoableShape = isUndoableKindEvent(ev);
                const canUndo =
                  isUndoableShape && undoableIds.has(ev.correlation_id!);
                const canRedo =
                  isUndoableShape && redoableIds.has(ev.correlation_id!);
                return (
                  <LineageRow
                    key={ev.id}
                    event={ev}
                    isExplainActive={
                      ev.correlation_id !== null &&
                      explainCorrelation === ev.correlation_id
                    }
                    onExplain={
                      ev.correlation_id !== null
                        ? () => {
                            // Toggle off if already open for this row;
                            // otherwise open. Matches the trace-button
                            // toggle UX in the Activity Timeline.
                            setExplainCorrelation((current) =>
                              current === ev.correlation_id
                                ? null
                                : ev.correlation_id,
                            );
                          }
                        : undefined
                    }
                    canPin={canPin}
                    onPinEntry={
                      canPin && ev.correlation_id !== null
                        ? () =>
                            handlePinEntry(ev.correlation_id!, ev.kind)
                        : undefined
                    }
                    canRetry={canRetry}
                    onRetryEntry={
                      canRetry && ev.correlation_id !== null
                        ? () =>
                            handleRetryEntry(ev.correlation_id!, ev.kind)
                        : undefined
                    }
                    canUndo={canUndo}
                    onUndoEntry={
                      canUndo && ev.correlation_id !== null
                        ? () => handleUndoEntry(ev.correlation_id!)
                        : undefined
                    }
                    canRedo={canRedo}
                    onRedoEntry={
                      canRedo && ev.correlation_id !== null
                        ? () => handleRedoEntry(ev.correlation_id!)
                        : undefined
                    }
                  />
                );
              })}

              {explainCorrelation && (
                <OperationNarrativeCard
                  key={explainCorrelation}
                  correlationId={explainCorrelation}
                  onClose={() => setExplainCorrelation(null)}
                />
              )}
            </>
          )}
        </div>
      </ScrollArea>
    </aside>
  );
}

/**
 * LineageFilterChip — visual mirror of the Activity Timeline's
 * `FilterChip`, kept local to this file rather than extracted into a
 * shared component because both copies are ~20 LOC and the second one
 * exists primarily to avoid cross-file coupling and the react-refresh
 * "fast refresh only works when a file only exports components"
 * warning that exporting a helper from `activity-timeline-panel.tsx`
 * would trigger. The visual grammar is intentionally identical so
 * users learn one filter mental model across both surfaces.
 */
function LineageFilterChip({
  label,
  count,
  active,
  onClick,
  danger = false,
}: {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
  /** When true, render with red tint instead of sky-blue. Used by the
   *  "Failed" chip so failure narrowing reads as urgent at a glance. */
  danger?: boolean;
}) {
  const activeRing = danger
    ? "border-red-500 bg-red-500/10 text-red-600 dark:text-red-300"
    : "border-sky-500 bg-sky-500/10 text-sky-600 dark:text-sky-300";
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] transition-colors",
        active
          ? activeRing
          : "border-[var(--color-border)] text-[color:var(--color-text-muted)] hover:bg-[var(--color-bg-tertiary)]",
      )}
    >
      {label}
      <span className="opacity-70">{count}</span>
    </button>
  );
}

function LineageRow({
  event,
  isExplainActive,
  onExplain,
  canPin,
  onPinEntry,
  canRetry,
  onRetryEntry,
  canUndo,
  onUndoEntry,
  canRedo,
  onRedoEntry,
}: {
  event: LineageEvent;
  /** True when the narrator card is currently open for this row's
   *  correlation_id — used to highlight the explain button as
   *  pressed. */
  isExplainActive: boolean;
  /** Click handler for the inline "explain this operation" button.
   *  Provided only for rows that carry a correlation_id; absent rows
   *  render no button. */
  onExplain?: () => void;
  /** True when the event qualifies for Pin (successful fs.copy/fs.move
   *  with a correlation_id). Mirrors the timeline's per-row Pin gate. */
  canPin: boolean;
  /** Click handler for the inline Pin button. Provided only when
   *  `canPin` is true so absent rows render no button. */
  onPinEntry?: () => void;
  /** True when the event qualifies for Retry (failed/cancelled
   *  fs.copy/fs.move with a correlation_id). Mutually exclusive with
   *  `canPin`. */
  canRetry: boolean;
  /** Click handler for the inline Retry button. */
  onRetryEntry?: () => void;
  /** True when the event's correlation_id is in the backend's
   *  currently-undoable set AND the kind is an original undoable op
   *  (not a marker). Mutually exclusive with `canRedo`. */
  canUndo: boolean;
  /** Click handler for the inline Undo button. */
  onUndoEntry?: () => void;
  /** True when the event's correlation_id is in the backend's
   *  currently-redoable set. Mutually exclusive with `canUndo`. */
  canRedo: boolean;
  /** Click handler for the inline Redo button. */
  onRedoEntry?: () => void;
}) {
  const EngineIcon = ENGINE_ICONS[event.engine] ?? Activity;
  const statusMeta = STATUS_META[event.status] ?? STATUS_META.ok;
  const StatusIcon = statusMeta.Icon;
  // Surface byte size in the meta row when present and non-zero —
  // same shape and same suppression semantic the Activity Timeline
  // TimelineRow uses, so a user comparing the two surfaces sees
  // identical event data presented identically. Renames and folder
  // creates carry no bytes (the field is null on the wire) so the
  // suppression keeps the meta row terse for those events.
  const bytesLabel =
    event.bytes !== null && event.bytes !== undefined && event.bytes > 0
      ? formatBytes(event.bytes)
      : null;
  return (
    <div className="group flex items-start gap-2 rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] px-2 py-1.5 hover:bg-[var(--color-bg-tertiary)]">
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
          <span>{formatRelativeTime(event.occurred_at, undefined, "withSuffix")}</span>
          <span aria-hidden="true">·</span>
          <span>{event.engine}</span>
          <span aria-hidden="true">·</span>
          <span>{event.kind}</span>
          {bytesLabel && (
            <>
              <span aria-hidden="true">·</span>
              <span data-testid={`lineage-bytes-${event.id}`}>
                {bytesLabel}
              </span>
            </>
          )}
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
      {/*
       * Explain affordance — only when the row has a correlation_id.
       * Renders the BookOpen icon used by the Activity Timeline's
       * narrator trigger so users learn one icon across both surfaces.
       * Always visible when active; hover-reveal otherwise to keep
       * inactive rows quiet.
       */}
      {onExplain && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onExplain();
          }}
          aria-pressed={isExplainActive}
          aria-label={
            isExplainActive
              ? "Close operation narrative"
              : "Explain this operation"
          }
          title={
            isExplainActive ? "Close narrative" : "Explain this operation"
          }
          className={cn(
            "flex-shrink-0 rounded p-1 transition-colors focus:outline-none focus:ring-2 focus:ring-sky-500/40",
            isExplainActive
              ? "text-sky-500"
              : "text-[color:var(--color-text-muted)] opacity-0 hover:text-sky-500 group-hover:opacity-100 focus:opacity-100",
          )}
          data-testid={`lineage-explain-${event.correlation_id}`}
        >
          <BookOpen className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
      {/* Pin and Retry affordances — same icons, same hover/focus
       *  grammar, same per-row gating predicates as the Activity
       *  Timeline. Mutually exclusive on a given row because Pin is
       *  ok-only and Retry is failed/cancelled-only. */}
      {canPin && onPinEntry && event.correlation_id && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onPinEntry();
          }}
          title={`Pin this ${event.kind} as a re-runnable Quickflow`}
          aria-label={`Pin this ${event.kind} operation as a re-runnable Quickflow`}
          className="flex-shrink-0 rounded p-1 text-[color:var(--color-text-muted)] opacity-0 transition-colors hover:text-violet-500 focus:outline-none focus:opacity-100 focus:ring-2 focus:ring-violet-500/40 group-hover:opacity-100"
          data-testid={`lineage-pin-${event.correlation_id}`}
        >
          <Pin className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
      {canRetry && onRetryEntry && event.correlation_id && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onRetryEntry();
          }}
          title={`Retry this failed ${event.kind} now`}
          aria-label={`Retry this failed ${event.kind} operation`}
          className="flex-shrink-0 rounded p-1 text-[color:var(--color-text-muted)] opacity-0 transition-colors hover:text-emerald-500 focus:outline-none focus:opacity-100 focus:ring-2 focus:ring-emerald-500/40 group-hover:opacity-100"
          data-testid={`lineage-retry-${event.correlation_id}`}
        >
          <RefreshCw className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
      {/* Undo and Redo affordances — same icons (Undo2 + mirrored
       *  Undo2), same amber hover/focus grammar as the Activity
       *  Timeline. Mutually exclusive on a given row via the
       *  marker-parity invariant. */}
      {canUndo && onUndoEntry && event.correlation_id && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onUndoEntry();
          }}
          title={`Undo this ${event.kind}`}
          aria-label={`Undo this ${event.kind} operation`}
          className="flex-shrink-0 rounded p-1 text-[color:var(--color-text-muted)] opacity-0 transition-colors hover:text-amber-500 focus:outline-none focus:opacity-100 focus:ring-2 focus:ring-amber-500/40 group-hover:opacity-100"
          data-testid={`lineage-undo-${event.correlation_id}`}
        >
          <Undo2 className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
      {canRedo && onRedoEntry && event.correlation_id && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onRedoEntry();
          }}
          title={`Redo this ${event.kind}`}
          aria-label={`Redo this ${event.kind} operation`}
          className="flex-shrink-0 rounded p-1 text-[color:var(--color-text-muted)] opacity-0 transition-colors hover:text-amber-500 focus:outline-none focus:opacity-100 focus:ring-2 focus:ring-amber-500/40 group-hover:opacity-100"
          data-testid={`lineage-redo-${event.correlation_id}`}
        >
          <Undo2 className="h-3 w-3 -scale-x-100" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}
