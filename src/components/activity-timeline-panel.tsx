/**
 * ActivityTimelinePanel — on-demand cross-engine activity feed.
 *
 * The user-facing payoff of the unified OperationLedger: one scrollable
 * timeline showing what every engine has done recently (fs, transfer,
 * sync, automation, mount, …). Fulfills the promise made by the
 * SinceLastSeenToast's "open the timeline for details" hint.
 *
 * Design principles applied here:
 *
 * - **Zero new backend**: reads from the already-registered `ledger_recent`
 *   IPC command. No engine code is touched. The ledger's fail-open write
 *   path (Phases 1-4) keeps producing events regardless of this panel.
 * - **Pull, not push**: data is fetched on mount and on explicit refresh.
 *   No polling, no event stream, no background CPU cost — the panel is
 *   inert when closed.
 * - **Graceful outside Tauri**: uses `tauriInvokeSafe` with an empty-array
 *   fallback so the pnpm dev / browser test path stays green.
 * - **No new deps**: uses the same cn/Button/Badge/ScrollArea primitives as
 *   the AutomationPanel plus lucide icons already imported elsewhere. The
 *   visual grammar matches the existing right-side panels (AI, Automation).
 *
 * Composition: mounted inside FileManager's flex row alongside the other
 * right-side panels; self-hides via its own Zustand store when closed.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { formatBytes } from "@/lib/format-bytes";
import { useActivityTimelineStore } from "@/stores/activity-timeline-store";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { useUIStore } from "@/stores/ui-store";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { formatRelativeTime } from "@/lib/ledger-dispatch";
import {
  ENGINE_ICONS,
  ENGINE_LABELS,
  STATUS_META,
} from "@/lib/engine-ui-constants";
import { cn } from "@ufop/ui-components";
import { Button, Badge, ScrollArea } from "@ufop/ui-components";
import {
  X,
  RefreshCw,
  Search,
  Activity,
  AlertTriangle,
  Link2,
  Undo2,
  Pin,
  BookOpen,
  Folder,
  History,
} from "lucide-react";
import { useLineageStore } from "@/stores/lineage-store";
import { OperationNarrativeCard } from "./operation-narrative-card";
import { FeaturePeek } from "@/components/feature-peek";
import { useAutomationStore } from "@/stores/automation-store";
import type { LedgerEventWire as LedgerEvent } from "@/lib/ledger-tail-extract";
import {
  isPinnableEvent,
  isRetryableEvent,
  isUndoableKindEvent,
} from "@/lib/ledger-event-flags";
import { correlationCounts } from "@/lib/correlation-counts";
import { dispatchRefresh, parentDirectoriesOf } from "@/lib/refresh-affected";

/**
 * Format an ISO-ish timestamp into a short relative label ("2m ago").
 * Uses Date parsing — both chrono RFC3339 and SQLite `YYYY-MM-DD HH:MM:SS`
 * (which Date treats as local time when parsed) are handled by coercing
 * the space-separated form to `T`-separated UTC.
 */

/**
 * Group events by calendar day label ("Today", "Yesterday", "Apr 6").
 * Relies on the events already being sorted newest-first from the SQL
 * `ORDER BY occurred_at DESC` in `OperationLedger::recent`, so we only
 * need to emit a header when the day changes.
 */
function dayLabel(iso: string): string {
  const normalized = iso.includes("T") ? iso : iso.replace(" ", "T") + "Z";
  const d = new Date(normalized);
  if (Number.isNaN(d.getTime())) return "Unknown";
  const now = new Date();
  const sameDay = (a: Date, b: Date) =>
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate();
  if (sameDay(d, now)) return "Today";
  const y = new Date(now);
  y.setDate(y.getDate() - 1);
  if (sameDay(d, y)) return "Yesterday";
  return d.toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/**
 * Split a path into `{ parent, leaf }`. Pure string op — no platform
 * calls, since ledger paths come pre-normalized from the Rust side. Both
 * POSIX and Windows separators are accepted because connectors can emit
 * either. Returns the input as `leaf` with empty `parent` if un-splittable
 * (e.g. a root or bare filename) so the caller can still navigate to it.
 */
function splitPath(path: string): { parent: string; leaf: string } {
  const trimmed = path.replace(/[/\\]+$/, "");
  const lastSlash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (lastSlash <= 0) return { parent: "", leaf: trimmed };
  return { parent: trimmed.slice(0, lastSlash), leaf: trimmed.slice(lastSlash + 1) };
}

type EngineFilter = "all" | keyof typeof ENGINE_ICONS;

// Pin / Retry row eligibility predicates live in `@/lib/ledger-event-flags`
// so the React panel file stays component-only (fast-refresh clean) and
// the rules can be exhaustively unit-tested without rendering anything.

/**
 * Escape user input for safe insertion into a RegExp. Inlined (4 lines)
 * rather than pulled from a dep — this is the only place we need it.
 */
function escapeRegExp(input: string): string {
  return input.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/**
 * Render `text` with case-insensitive matches of `query` wrapped in
 * `<mark>` so the user can see *why* a row matched their search. Returns
 * the raw string when query is empty so React skips the array allocation.
 *
 * Used for the three fields the search predicate covers: summary, kind,
 * and subject_path. Keeping this DRY means a future predicate change
 * (e.g. adding `target_path`) is one render-site away from being lit up.
 */
function Highlight({ text, query }: { text: string; query: string }) {
  const q = query.trim();
  if (q === "") return <>{text}</>;
  const parts = text.split(new RegExp(`(${escapeRegExp(q)})`, "ig"));
  const lower = q.toLowerCase();
  return (
    <>
      {parts.map((part, i) =>
        part.toLowerCase() === lower ? (
          <mark
            key={i}
            className="rounded-sm bg-amber-200/70 px-0.5 text-inherit dark:bg-amber-400/30"
          >
            {part}
          </mark>
        ) : (
          <span key={i}>{part}</span>
        ),
      )}
    </>
  );
}

/** Iter 46 props — optional smart-dispatch callback. When provided,
 *  clicking a timeline row routes through the shared dispatch helper
 *  (same behaviour as Instant Jump and the Jump Ring), so archive
 *  files land in the archive browser, text files in the in-app
 *  editor, media in the preview pane, and folders in the active
 *  tab. When omitted, the panel falls back to the pre-iter-46
 *  "navigate-to-parent" behaviour so existing call sites that mount
 *  the component without props stay binary-compatible. */
export interface ActivityTimelinePanelProps {
  onDispatchPath?: (path: string) => void | Promise<void>;
}

export function ActivityTimelinePanel(
  props: ActivityTimelinePanelProps = {},
) {
  const { onDispatchPath } = props;
  const panelOpen = useActivityTimelineStore((s) => s.panelOpen);
  const togglePanel = useActivityTimelineStore((s) => s.togglePanel);
  const pendingPreset = useActivityTimelineStore((s) => s.pendingPreset);
  const consumePendingPreset = useActivityTimelineStore(
    (s) => s.consumePendingPreset,
  );
  const [events, setEvents] = useState<LedgerEvent[]>([]);
  const [loading, setLoading] = useState(false);
  // Set of correlation_ids the backend currently considers reversible.
  // Fetched alongside events so each row can show a per-entry "Undo" button
  // without any extra calls per row. This is O(N) in undoable groups, with
  // N capped at 100 by `list_undoable`.
  const [undoableIds, setUndoableIds] = useState<Set<string>>(() => new Set());
  // Mirror of `undoableIds` for the redo affordance — populated from
  // `list_redoable`, refreshed alongside the events on the same poll.
  // A correlation_id appears in at most one of the two sets at a time
  // (the backend's marker-parity logic guarantees mutual exclusion), so
  // each row renders either Undo OR Redo, never both.
  const [redoableIds, setRedoableIds] = useState<Set<string>>(() => new Set());
  const [filter, setFilter] = useState<EngineFilter>("all");
  const [query, setQuery] = useState("");
  const [failedOnly, setFailedOnly] = useState(false);
  // Correlation-trace filter: when set, the timeline narrows to every
  // event sharing this correlation_id — exposing the cross-engine causal
  // chain the ledger already records (e.g. one sync run → N fs writes →
  // M automation fires). This surfaces hidden operation tracing with
  // literally one new predicate; zero backend changes.
  const [correlationFilter, setCorrelationFilter] = useState<string | null>(null);
  // Path-prefix filter: when set, narrows the timeline to events whose
  // `subject_path` OR `target_path` starts with this prefix — answering
  // "what happened in THIS folder?". Composes with every other filter
  // (engine, status, search, correlation) and shares the same chip-row
  // UX as the correlation filter. Triggered by the "Show this folder's
  // activity" context-menu entry; dismissible via the chip. Zero new
  // backend code — just one more predicate over the existing 200-row
  // `ledger_recent` pull the panel already does.
  const [pathFilter, setPathFilter] = useState<string | null>(null);

  // Drill-in from ambient surfaces (engine-pulse error badge, future
  // overview cards): when an external caller stages a `pendingPreset`
  // via `openWithPreset`, we consume it on mount AND on every change so
  // re-clicking the same affordance after dismissal re-applies the
  // filter. `consumePendingPreset` is atomic — read-and-clear in one
  // store transaction — so the same preset never re-applies twice on
  // its own. We unset filters the caller didn't request via `undefined`
  // (i.e. we don't clobber correlation when the caller only set failed).
  useEffect(() => {
    if (pendingPreset === null) return;
    const preset = consumePendingPreset();
    if (preset === null) return;
    if (preset.failedOnly !== undefined) setFailedOnly(preset.failedOnly);
    if (preset.engineFilter !== undefined) {
      setFilter(preset.engineFilter as EngineFilter);
    }
    if (preset.correlationFilter !== undefined) {
      setCorrelationFilter(preset.correlationFilter);
    }
    if (preset.pathFilter !== undefined) {
      setPathFilter(preset.pathFilter);
    }
  }, [pendingPreset, consumePendingPreset]);
  // Whether the Operation Narrator card is open for the active
  // correlation filter. Reset automatically when the filter clears.
  const [narrativeOpen, setNarrativeOpen] = useState(false);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  /**
   * "Jump-to-path" — navigate from "what happened" to "take me there".
   * Pulls store methods via `.getState()` so we don't subscribe the
   * component to navigation state (which would cause re-renders on
   * every path change in either pane).
   *
   * Iter 46: when the parent passes `onDispatchPath`, we delegate to
   * the shared `dispatchLedgerPath` helper so archive files open the
   * archive browser, text files open the in-app editor, media opens
   * the preview pane, and folders navigate the active tab. This
   * closes the DRY loop with Instant Jump and the Jump Ring —
   * clicking a timeline row behaves identically to Enter-in-Instant-
   * Jump, and all four surfaces (FilePane double-click, Instant
   * Jump, Jump Ring, Activity Timeline) route through ONE code path.
   *
   * Falls back to the pre-iter-46 parent-navigation path when the
   * prop is absent, preserving binary-compatibility for any future
   * caller that mounts the panel without the dispatch callback.
   *
   * Edge cases handled:
   * - subject_path `null` (engine events without a path target): no-op
   * - leaf-only path (no separator): navigate to the leaf itself
   * - same path as active tab: still emits navigateTab so history picks
   *   up the "jump" intent (harmless; `navigateTab` is idempotent-ish)
   */
  const navigateToEvent = useCallback(
    (event: LedgerEvent) => {
      const target = event.subject_path;
      if (!target) return;
      if (onDispatchPath) {
        void onDispatchPath(target);
        return;
      }
      const store = useFileManagerStore.getState();
      const paneIndex = store.activePaneIndex;
      const activeTab = store.getActiveTab(paneIndex);
      if (!activeTab) return;
      const { parent, leaf } = splitPath(target);
      // Legacy fallback: navigate to the parent so the user can
      // see context; if there is no parent (e.g. root-level), fall
      // back to the target.
      const destination = parent || target;
      const label =
        leaf ||
        destination.split(/[/\\]/).filter(Boolean).pop() ||
        destination;
      store.navigateTab(paneIndex, activeTab.id, destination, label);
    },
    [onDispatchPath],
  );

  const loadEvents = useCallback(async () => {
    setLoading(true);
    // Fetch events, undoable groups, and redoable groups in parallel — one
    // round-trip per refresh; all three lists already live in the ledger
    // and share the same backing scan on the Rust side.
    const [rows, undoableOps, redoableOps] = await Promise.all([
      tauriInvokeSafe<LedgerEvent[]>("ledger_recent", { limit: 200 }, []),
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
    setEvents(rows ?? []);
    setUndoableIds(new Set((undoableOps ?? []).map((o) => o.correlation_id)));
    setRedoableIds(new Set((redoableOps ?? []).map((o) => o.correlation_id)));
    setLoading(false);
  }, []);

  // Per-entry Pin handler — promotes a single past ledger event into a
  // saved manual-trigger Quickflow via the new `pin_ledger_event` IPC.
  // The new rule is disabled-by-default; the user must explicitly enable
  // it from the automation panel before any side effects occur. Surfaces
  // the outcome through the existing structured-error toast channel.
  const pinLedgerEvent = useAutomationStore((s) => s.pinLedgerEvent);
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

  // Per-entry Retry handler — failure-side mirror of Pin. Re-attempts a
  // failed (or cancelled) fs.copy / fs.move event via the new
  // `retry_failed_event` IPC. The IPC executes the action in place, does
  // NOT persist a rule, and records the retry outcome to the ledger so
  // the activity timeline auto-refresh surfaces the attempt next to its
  // cause. Surfaces the result through the same structured-error toast
  // channel as the Pin handler for visual + cognitive parity.
  const retryFailedEvent = useAutomationStore((s) => s.retryFailedEvent);
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
          ? "Refresh the timeline to see the new success row next to the original failure"
          : "Check that the source files still exist and the destination is writable, then try again",
      });
      // On success, dispatch a path-aware refresh so any pane viewing
      // the destination directory (or a parent) repaints with the
      // restored files. The retry's ledger row carries the original
      // correlation id, so the existing tail-poll dedup correctly
      // skips emitting a second toast — but the tail-poll also won't
      // refresh on its own (kind="retry" is intentionally outside
      // MUTATION_KINDS), so the dispatch here is the single canonical
      // refresh signal for the retry flow.
      if (succeeded && log && log.files_affected.length > 0) {
        dispatchRefresh(parentDirectoriesOf(log.files_affected));
      }
      // Refresh so the new retry row shows up and the failed-only chip
      // count stays accurate.
      void loadEvents();
    },
    [retryFailedEvent, loadEvents],
  );

  // Per-entry Undo handler — reverses the single correlation group via the
  // ledger-backed backend, surfaces the result through the existing
  // structured-error toast channel, and refreshes so the "Undo" affordance
  // disappears from the row that was just reversed.
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
      } catch (err) {
        console.error("Undo by correlation failed:", err);
      }
      void loadEvents();
    },
    [loadEvents],
  );

  // Per-entry Redo handler — symmetric mirror of `handleUndoEntry`. Re-applies
  // the original operation via the ledger-backed `redo_by_correlation` IPC.
  // After it returns the panel refreshes, which flips the row's affordance
  // from "Redo" back to "Undo" (mutual exclusion of the two sets).
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
      } catch (err) {
        console.error("Redo by correlation failed:", err);
      }
      void loadEvents();
    },
    [loadEvents],
  );

  // Fetch when panel opens (not when merely mounted + closed) so the panel
  // is truly inert until the user asks for it.
  useEffect(() => {
    if (panelOpen) void loadEvents();
  }, [panelOpen, loadEvents]);

  // Keyboard shortcuts while panel is open. Bound only when open so
  // there's zero keyboard surface consumed when closed.
  //
  //   `/`     — focus search input (universal pattern: GitHub/Gmail/Discord).
  //   Escape  — close panel. Guarded so Escape inside the search input
  //             still runs its clear/blur handler (that handler doesn't
  //             preventDefault, so this listener would fire second; we
  //             explicitly skip when focus is in an input).
  //
  // Both shortcuts are ignored when any input/textarea/select/contenteditable
  // holds focus so the user can still type `/` in filenames and Escape
  // inside forms without hijacking.
  useEffect(() => {
    if (!panelOpen) return;
    const onKeyDown = (e: KeyboardEvent) => {
      if (e.metaKey || e.ctrlKey || e.altKey) return;
      const active = document.activeElement as HTMLElement | null;
      const inEditable =
        !!active &&
        (active.tagName === "INPUT" ||
          active.tagName === "TEXTAREA" ||
          active.tagName === "SELECT" ||
          active.isContentEditable);
      if (e.key === "/" && !inEditable) {
        e.preventDefault();
        searchInputRef.current?.focus();
        searchInputRef.current?.select();
        return;
      }
      if (e.key === "Escape" && !inEditable) {
        e.preventDefault();
        togglePanel();
      }
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [panelOpen, togglePanel]);

  // Refresh when the window regains focus — if the panel is open the user
  // is watching, so fresh data is the right default. When the panel is
  // closed we don't bind any listener so the tab is truly zero-cost.
  // This is "pull, not push" correctly applied: the user's attention IS
  // the pull signal. No polling, no event stream, no background CPU.
  useEffect(() => {
    if (!panelOpen) return;
    const onFocus = () => {
      if (document.visibilityState === "visible") void loadEvents();
    };
    window.addEventListener("focus", onFocus);
    document.addEventListener("visibilitychange", onFocus);
    return () => {
      window.removeEventListener("focus", onFocus);
      document.removeEventListener("visibilitychange", onFocus);
    };
  }, [panelOpen, loadEvents]);

  // Compute the filtered view once per dependency change. `events` is at
  // most ~200 rows so a single pass with all predicates is fine — no need
  // for multiple array allocations. Search matches summary, kind, and
  // subject path so users can find by what happened, what kind of op, or
  // where. The path prefix predicate uses a normalised "with trailing
  // separator" form so `/docs` does not accidentally match `/documents`.
  const filteredEvents = useMemo(() => {
    const q = query.trim().toLowerCase();
    // Strip any trailing path separator so a caller passing `/docs/`
    // matches the same set as `/docs`. Defensive — the in-app caller
    // (`onShowFolderActivity`) passes a canonical path, but the chip
    // text and any future callers should be tolerant of either form.
    const pathPrefix = pathFilter !== null
      ? pathFilter.replace(/[/\\]+$/, "")
      : null;
    // Treat exact match as a match, plus anything starting with prefix +
    // separator. This avoids the `/docs` vs `/documents` false positive.
    const startsWithPath = pathPrefix === null || pathPrefix === ""
      ? null
      : (p: string | null): boolean =>
          p !== null &&
          (p === pathPrefix ||
            p.startsWith(pathPrefix + "/") ||
            p.startsWith(pathPrefix + "\\"));
    if (
      filter === "all" &&
      q === "" &&
      !failedOnly &&
      correlationFilter === null &&
      startsWithPath === null
    ) {
      return events;
    }
    return events.filter((e) => {
      if (correlationFilter !== null && e.correlation_id !== correlationFilter) return false;
      if (filter !== "all" && e.engine !== filter) return false;
      if (failedOnly && e.status === "ok") return false;
      if (startsWithPath !== null) {
        // Match if either side of the rename/copy/move row touches the
        // scoped folder. This correctly surfaces "moved out of here" and
        // "moved into here" with the same filter.
        if (!startsWithPath(e.subject_path) && !startsWithPath(e.target_path)) {
          return false;
        }
      }
      if (q === "") return true;
      if (e.summary.toLowerCase().includes(q)) return true;
      if (e.kind.toLowerCase().includes(q)) return true;
      if (e.subject_path && e.subject_path.toLowerCase().includes(q)) return true;
      return false;
    });
  }, [events, filter, query, failedOnly, correlationFilter, pathFilter]);

  // Pre-compute engine → count for the filter chips so users can see at a
  // glance where activity is concentrated. Single pass, no repeated loops.
  const engineCounts = useMemo(() => {
    const counts: Record<string, number> = {};
    for (const e of events) counts[e.engine] = (counts[e.engine] ?? 0) + 1;
    return counts;
  }, [events]);

  const activeEngines = useMemo(
    () => Object.keys(engineCounts).sort(),
    [engineCounts],
  );

  // Sum bytes across the CURRENTLY FILTERED events — same filter
  // predicate the renderer uses, so the header byte total always
  // matches what the user actually sees. Pure derivation, single
  // pass; recomputes only when the filter set changes. Suppressed
  // (kept at 0) when every visible event omits a bytes field, so
  // metadata-only windows don't surface a noisy "0 B" label.
  const filteredBytes = useMemo(() => {
    let total = 0;
    for (const e of filteredEvents) {
      if (e.bytes !== null && e.bytes !== undefined && e.bytes > 0) {
        total += e.bytes;
      }
    }
    return total;
  }, [filteredEvents]);

  // Reset scroll to top whenever the active filter changes. Without this,
  // typing in search or flipping a chip leaves the user staring at a
  // stale scroll position (sometimes past the end of the filtered list).
  // Keyed on the three filter inputs — not on `filteredEvents` identity,
  // which would also re-fire on fresh data loads. Effect is bound only
  // while the panel is open via the `panelOpen` render short-circuit.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = 0;
  }, [query, filter, failedOnly, correlationFilter, pathFilter]);

  // How many events are non-ok (failed/cancelled/skipped)? This drives
  // the header "failed only" toggle badge — users debugging problems
  // need this count at a glance and want one-tap narrowing.
  const failedCount = useMemo(
    () => events.reduce((n, e) => (e.status !== "ok" ? n + 1 : n), 0),
    [events],
  );

  if (!panelOpen) return null;

  return (
    <aside
      className="flex flex-col border-l border-[var(--color-border)] bg-[var(--color-bg-secondary)]"
      style={{ width: 360 }}
      role="complementary"
      aria-label="Activity timeline panel"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-[var(--color-border)]">
        <div className="flex items-center gap-2">
          <Activity className="h-4 w-4 text-sky-500" aria-hidden="true" />
          <span
            className="text-sm font-semibold text-[color:var(--color-text)]"
            title="Activity Timeline (⌘⇧Y to toggle)"
          >
            Activity
          </span>
          {/*
           * Count badge shows total on an unfiltered view, and `N of M`
           * whenever any narrowing predicate is active (search, engine,
           * failed-only). This gives users instant feedback on how much
           * their filter stack narrowed the list without adding any new
           * UI elements.
           */}
          <Badge
            variant="secondary"
            className="text-[10px]"
            title={(() => {
              // Same ` · X.X MB total` suffix on both branches — extracted
              // so a future format change touches one place. Empty when
              // there's no byte volume to report.
              const bytesTitleSuffix =
                filteredBytes > 0
                  ? ` · ${formatBytes(filteredBytes)} total`
                  : "";
              return filteredEvents.length !== events.length
                ? `${filteredEvents.length} of ${events.length} events match current filters${bytesTitleSuffix}`
                : `${events.length} events${bytesTitleSuffix}`;
            })()}
            data-testid="activity-timeline-header-count"
          >
            {filteredEvents.length !== events.length
              ? `${filteredEvents.length} of ${events.length}`
              : events.length}
            {filteredBytes > 0 && (
              <span className="ml-1 opacity-70" data-testid="activity-timeline-header-bytes">
                · {formatBytes(filteredBytes)}
              </span>
            )}
          </Badge>
        </div>
        <div className="flex items-center gap-1">
          {failedCount > 0 && (
            <Button
              variant="ghost"
              size="icon"
              onClick={() => setFailedOnly((v) => !v)}
              title={failedOnly ? "Show all events" : `Show only failed (${failedCount})`}
              aria-label={failedOnly ? "Show all events" : "Show only failed events"}
              aria-pressed={failedOnly}
              className={cn(failedOnly && "text-red-500")}
            >
              <AlertTriangle className="h-4 w-4" aria-hidden="true" />
            </Button>
          )}
          <Button
            variant="ghost"
            size="icon"
            onClick={() => void loadEvents()}
            title="Refresh activity"
            aria-label="Refresh activity timeline"
            disabled={loading}
          >
            <RefreshCw
              className={cn("h-4 w-4", loading && "animate-spin")}
              aria-hidden="true"
            />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            onClick={togglePanel}
            title="Close panel (Esc or ⌘⇧Y to toggle)"
            aria-label="Close activity panel"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>

      <FeaturePeek
        id="activity-timeline-intro"
        title="Every operation is recorded here"
        shortcut="Cmd+Shift+L"
      >
        Transfers, syncs, AI runs, even safety prompts — they all land in this ledger.
        Click any row to jump back to where it happened.
      </FeaturePeek>

      {/* Search bar — client-side filter over summary, kind, and path. */}
      <div className="relative px-3 py-2 border-b border-[var(--color-border)]">
        <Search
          className="pointer-events-none absolute left-5 top-1/2 h-3 w-3 -translate-y-1/2 text-[color:var(--color-text-muted)]"
          aria-hidden="true"
        />
        <input
          ref={searchInputRef}
          type="text"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            // Escape in a non-empty search clears the query in place;
            // Escape in an empty search blurs so the user can quickly
            // hand focus back to the timeline or close the panel.
            if (e.key === "Escape") {
              if (query !== "") {
                e.preventDefault();
                setQuery("");
              } else {
                (e.currentTarget as HTMLInputElement).blur();
              }
            }
          }}
          placeholder="Search activity… (press /)"
          aria-label="Search activity timeline"
          className="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg-primary)] py-1 pl-6 pr-6 text-xs text-[color:var(--color-text)] placeholder:text-[color:var(--color-text-muted)] focus:outline-none focus:ring-2 focus:ring-sky-500/40"
        />
        {query !== "" && (
          <button
            type="button"
            onClick={() => setQuery("")}
            aria-label="Clear search"
            className="absolute right-5 top-1/2 -translate-y-1/2 rounded p-0.5 text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text)]"
          >
            <X className="h-3 w-3" />
          </button>
        )}
      </div>

      {/*
       * Active correlation-trace chip. Only rendered when the user has
       * clicked into a correlation on a row — presents the short ID with
       * a one-click clear. Placed above the engine chips so it's obvious
       * *why* the list just narrowed, and so clearing it is instant.
       */}
      {correlationFilter !== null && (
        <>
          <div className="flex items-center gap-2 px-3 py-2 border-b border-[var(--color-border)] bg-sky-500/5">
            <Link2 className="h-3 w-3 text-sky-500" aria-hidden="true" />
            <span className="text-[11px] text-[color:var(--color-text-muted)]">
              Tracing operation
            </span>
            <code
              className="flex-1 truncate rounded bg-[var(--color-bg-primary)] px-1.5 py-0.5 text-[10px] text-[color:var(--color-text)]"
              title={correlationFilter}
            >
              {correlationFilter.length > 12
                ? `${correlationFilter.slice(0, 8)}…`
                : correlationFilter}
            </code>
            {/*
             * Operation Narrator trigger. Opens an inline card with a
             * plain-language story built by a pure deterministic
             * summarizer in the Rust backend over the exact same
             * correlation-id filter. No modal, no route change, no new
             * infrastructure — just a second view of the same data.
             */}
            <button
              type="button"
              onClick={() => setNarrativeOpen((v) => !v)}
              aria-label={narrativeOpen ? "Close operation narrative" : "Explain this operation"}
              aria-pressed={narrativeOpen}
              className={cn(
                "rounded px-1.5 py-0.5 text-[10px] transition-colors",
                narrativeOpen
                  ? "bg-sky-500/20 text-sky-300"
                  : "text-[color:var(--color-text-muted)] hover:bg-[var(--color-bg-primary)] hover:text-[color:var(--color-text)]",
              )}
              title={narrativeOpen ? "Close narrative" : "Explain this operation"}
            >
              <BookOpen className="h-3 w-3" aria-hidden="true" />
            </button>
            <button
              type="button"
              onClick={() => {
                setCorrelationFilter(null);
                setNarrativeOpen(false);
              }}
              aria-label="Clear correlation trace filter"
              className="rounded p-0.5 text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text)]"
            >
              <X className="h-3 w-3" />
            </button>
          </div>
          {narrativeOpen && (
            <OperationNarrativeCard
              key={correlationFilter}
              correlationId={correlationFilter}
              onClose={() => setNarrativeOpen(false)}
            />
          )}
        </>
      )}

      {/*
       * Active path-scope chip. Symmetric mirror of the correlation chip:
       * shows the folder the timeline is narrowed to and offers a one-
       * click clear. Triggered by the "Show this folder's activity"
       * context-menu entry; composes with every other filter. The path
       * is truncated to the last two segments so a deep prefix
       * (`/Users/me/Documents/Projects/UFOP/src`) still fits the chip.
       */}
      {pathFilter !== null && (
        <div
          className="flex items-center gap-2 px-3 py-2 border-b border-[var(--color-border)] bg-emerald-500/5"
          data-testid="activity-timeline-path-chip"
        >
          <Folder className="h-3 w-3 text-emerald-500" aria-hidden="true" />
          <span className="text-[11px] text-[color:var(--color-text-muted)]">
            In folder
          </span>
          <code
            className="flex-1 truncate rounded bg-[var(--color-bg-primary)] px-1.5 py-0.5 text-[10px] text-[color:var(--color-text)]"
            title={pathFilter}
          >
            {(() => {
              const parts = pathFilter.split(/[/\\]/).filter(Boolean);
              return parts.length <= 2
                ? pathFilter
                : `…/${parts.slice(-2).join("/")}`;
            })()}
          </code>
          <button
            type="button"
            onClick={() => setPathFilter(null)}
            aria-label="Clear folder filter"
            className="rounded p-0.5 text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text)]"
          >
            <X className="h-3 w-3" />
          </button>
        </div>
      )}

      {/*
       * Clear-all-filters affordance — appears only when at least one
       * filter is active (any of: engine != all, search non-empty,
       * failed-only on, correlation trace, path filter). Lets the
       * user reset every narrowing predicate in one click rather
       * than dismissing chips and clearing the search box one by
       * one. Reuses the existing setters so no new state-flow.
       */}
      {(filter !== "all" ||
        query !== "" ||
        failedOnly ||
        correlationFilter !== null ||
        pathFilter !== null) && (
        <div className="flex items-center justify-end px-3 py-1 border-b border-[var(--color-border)]">
          <button
            type="button"
            onClick={() => {
              setFilter("all");
              setQuery("");
              setFailedOnly(false);
              setCorrelationFilter(null);
              setPathFilter(null);
              setNarrativeOpen(false);
            }}
            data-testid="activity-timeline-clear-filters"
            className="inline-flex items-center gap-1 rounded-full border border-[var(--color-border)] px-2 py-0.5 text-[10px] text-[color:var(--color-text-muted)] hover:bg-[var(--color-bg-tertiary)] hover:text-[color:var(--color-text)] transition-colors"
            title="Clear every active filter"
            aria-label="Clear every active filter"
          >
            <X className="h-3 w-3" aria-hidden="true" />
            Clear all filters
          </button>
        </div>
      )}

      {/* Engine filter chips — only show engines that actually appeared */}
      {activeEngines.length > 0 && (
        <div className="flex flex-wrap gap-1 px-3 py-2 border-b border-[var(--color-border)]">
          <FilterChip
            label="All"
            count={events.length}
            active={filter === "all"}
            onClick={() => setFilter("all")}
          />
          {activeEngines.map((engine) => (
            <FilterChip
              key={engine}
              label={ENGINE_LABELS[engine] ?? engine}
              count={engineCounts[engine] ?? 0}
              active={filter === engine}
              onClick={() => setFilter(engine as EngineFilter)}
            />
          ))}
        </div>
      )}

      {/*
       * Screen-reader-only announcement of the filtered result count.
       * `aria-live="polite"` fires after the user stops interacting, so
       * when they type in search or flip a chip they hear "Showing 5 of
       * 200 events" without any visual change. Uses `sr-only` so sighted
       * users never see it — the visible badge already covers that case.
       */}
      <div aria-live="polite" aria-atomic="true" className="sr-only">
        {filteredEvents.length === events.length
          ? `Showing ${events.length} events`
          : `Showing ${filteredEvents.length} of ${events.length} events`}
      </div>

      {/* Timeline body */}
      <ScrollArea ref={scrollRef} className="flex-1">
        <div className="px-3 py-2">
          {loading && events.length === 0 && (
            <div
              role="status"
              aria-live="polite"
              className="text-xs text-[color:var(--color-text-muted)] py-4 text-center"
            >
              Loading…
            </div>
          )}
          {!loading && filteredEvents.length === 0 && (
            <EmptyState
              filter={filter}
              query={query}
              failedOnly={failedOnly}
              correlationFilter={correlationFilter}
              pathFilter={pathFilter}
            />
          )}
          <TimelineList
            events={filteredEvents}
            onNavigate={navigateToEvent}
            onTrace={setCorrelationFilter}
            activeCorrelation={correlationFilter}
            query={query}
            undoableIds={undoableIds}
            onUndoEntry={handleUndoEntry}
            redoableIds={redoableIds}
            onRedoEntry={handleRedoEntry}
            onPinEntry={handlePinEntry}
            onRetryEntry={handleRetryEntry}
            onShowFileHistory={(path) => {
              useLineageStore.getState().beginRequest(path);
            }}
          />
        </div>
      </ScrollArea>
    </aside>
  );
}

// ── Subcomponents ──────────────────────────────────────────────────────

interface FilterChipProps {
  label: string;
  count: number;
  active: boolean;
  onClick: () => void;
}

function FilterChip({ label, count, active, onClick }: FilterChipProps) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={active}
      className={cn(
        "inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-[11px] transition-colors",
        active
          ? "border-sky-500 bg-sky-500/10 text-sky-600 dark:text-sky-300"
          : "border-[var(--color-border)] text-[color:var(--color-text-muted)] hover:bg-[var(--color-bg-tertiary)]",
      )}
    >
      {label}
      <span className="opacity-70">{count}</span>
    </button>
  );
}

function EmptyState({
  filter,
  query,
  failedOnly,
  correlationFilter,
  pathFilter,
}: {
  filter: EngineFilter;
  query: string;
  failedOnly: boolean;
  correlationFilter: string | null;
  pathFilter: string | null;
}) {
  const hasQuery = query.trim() !== "";
  // Message priority: correlation trace first (most specific — user is
  // tracing a single operation), then path-scope (also specific — they
  // asked about THIS folder), then query, then failed-only, then engine
  // filter, then the default "nothing ran yet" state. Keeps the copy
  // aligned with the most recently-set filter so users understand why
  // they're seeing an empty list.
  const message = correlationFilter !== null
    ? "No other events in this operation trace."
    : pathFilter !== null
    ? "No recent activity inside this folder. Clear the folder filter to see other engines' events."
    : hasQuery
    ? `No activity matches "${query.trim()}".`
    : failedOnly
      ? "No failed events in the recent window — everything is healthy."
      : filter === "all"
        ? "No activity yet. Operations will appear here as engines run."
        : `No ${ENGINE_LABELS[filter] ?? filter} activity in the recent window.`;
  return (
    <div className="py-8 text-center" role="status" aria-live="polite">
      <Activity
        className="mx-auto h-6 w-6 text-[color:var(--color-text-muted)] opacity-50"
        aria-hidden="true"
      />
      <div className="mt-2 text-xs text-[color:var(--color-text-muted)]">
        {message}
      </div>
    </div>
  );
}

/**
 * Render the flat event list with day separators inserted whenever the
 * calendar day changes between adjacent events. Pure function of the input
 * — groups are derived at render time, not stored in state.
 */
function TimelineList({
  events,
  onNavigate,
  onTrace,
  activeCorrelation,
  query,
  undoableIds,
  onUndoEntry,
  redoableIds,
  onRedoEntry,
  onPinEntry,
  onRetryEntry,
  onShowFileHistory,
}: {
  events: LedgerEvent[];
  onNavigate: (event: LedgerEvent) => void;
  onTrace: (id: string | null) => void;
  activeCorrelation: string | null;
  query: string;
  undoableIds: Set<string>;
  onUndoEntry: (correlationId: string) => void;
  redoableIds: Set<string>;
  onRedoEntry: (correlationId: string) => void;
  onPinEntry: (correlationId: string, kind: string) => void;
  onRetryEntry: (correlationId: string, kind: string) => void;
  /** Open the Lineage Panel for the row's subject_path. Provided only
   *  on rows that have a subject_path — symmetric mirror of the
   *  "View folder activity" button added to the Lineage Panel in
   *  iteration 15. */
  onShowFileHistory: (path: string) => void;
}) {
  // Pre-compute correlation_id → count. The hint is attached to the
  // FIRST (newest) row of each multi-row group only — track which
  // correlation_ids have already been decorated so subsequent rows in
  // the same group stay clean. Skipped when a correlation filter is
  // active because the user is already focused on one operation.
  const counts = useMemo(() => correlationCounts(events), [events]);
  const decorated = new Set<string>();

  let lastDay: string | null = null;
  const nodes: React.ReactNode[] = [];
  for (const ev of events) {
    const day = dayLabel(ev.occurred_at);
    if (day !== lastDay) {
      nodes.push(
        <div
          key={`day-${day}-${ev.id}`}
          className="sticky top-0 z-10 -mx-3 mb-1 mt-2 bg-[var(--color-bg-secondary)] px-3 py-1 text-[10px] font-semibold uppercase tracking-wide text-[color:var(--color-text-muted)]"
        >
          {day}
        </div>,
      );
      lastDay = day;
    }
    // Both Undo and Redo affordances attach only to the ORIGINAL operation
    // row (kind = copy/move/rename/etc.), never to the `fs.undone` /
    // `fs.redone` marker rows that share the same correlation_id. Without
    // the `isUndoableKindEvent` gate the marker rows would erroneously
    // surface a Redo button because they carry the same correlation_id as
    // the original op.
    const isUndoableShape = isUndoableKindEvent(ev);
    const canUndo =
      isUndoableShape && undoableIds.has(ev.correlation_id!);
    // Undo and Redo are mutually exclusive (a correlation_id can only be
    // in one of the two sets per the marker-parity logic), so the row
    // never renders both at once.
    const canRedo =
      isUndoableShape && redoableIds.has(ev.correlation_id!);
    const canPin = isPinnableEvent(ev);
    // Pin and Retry are mutually exclusive (status is either "ok" or
    // not), so the row never renders both — see `isRetryableEvent`.
    const canRetry = isRetryableEvent(ev);
    // Group-size hint: show the count chip only on the first occurrence
    // of each multi-row correlation. activeCorrelation suppresses it
    // because every visible row already shares the traced cid. Marker
    // rows (`fs.undone` / `fs.redone`) are skipped as decoration anchors
    // — they share the correlation_id with the original op but are
    // meta-events, not part of the per-file count. The chip therefore
    // attaches to the original op row (or the newest non-marker row
    // visible under the current filter), matching how the Undo/Redo
    // affordances also gate on `isUndoableKindEvent`.
    let groupSize = 0;
    const isMarkerRow =
      ev.kind === "fs.undone" || ev.kind === "fs.redone";
    if (
      ev.correlation_id !== null &&
      activeCorrelation === null &&
      !isMarkerRow &&
      !decorated.has(ev.correlation_id)
    ) {
      const c = counts.get(ev.correlation_id) ?? 0;
      if (c >= 2) {
        groupSize = c;
        decorated.add(ev.correlation_id);
      }
    }
    nodes.push(
      <TimelineRow
        key={ev.id}
        event={ev}
        onNavigate={onNavigate}
        onTrace={onTrace}
        activeCorrelation={activeCorrelation}
        query={query}
        canUndo={canUndo}
        onUndoEntry={onUndoEntry}
        canRedo={canRedo}
        onRedoEntry={onRedoEntry}
        canPin={canPin}
        onPinEntry={onPinEntry}
        canRetry={canRetry}
        onRetryEntry={onRetryEntry}
        groupSize={groupSize}
        onShowFileHistory={onShowFileHistory}
      />,
    );
  }
  return <>{nodes}</>;
}

function TimelineRow({
  event,
  onNavigate,
  onTrace,
  activeCorrelation,
  query,
  canUndo,
  onUndoEntry,
  canRedo,
  onRedoEntry,
  canPin,
  onPinEntry,
  canRetry,
  onRetryEntry,
  groupSize,
  onShowFileHistory,
}: {
  event: LedgerEvent;
  onNavigate: (event: LedgerEvent) => void;
  onTrace: (id: string | null) => void;
  activeCorrelation: string | null;
  query: string;
  canUndo: boolean;
  onUndoEntry: (correlationId: string) => void;
  canRedo: boolean;
  onRedoEntry: (correlationId: string) => void;
  canPin: boolean;
  onPinEntry: (correlationId: string, kind: string) => void;
  canRetry: boolean;
  onRetryEntry: (correlationId: string, kind: string) => void;
  /** When > 0, this row is the newest of a multi-row correlation
   *  group and should surface a "+N more in this op" chip that
   *  triggers the trace filter on click. 0 means no chip. */
  groupSize: number;
  /** Open the Lineage Panel for this row's subject_path. The button
   *  only renders when a subject_path is present. */
  onShowFileHistory: (path: string) => void;
}) {
  const EngineIcon = ENGINE_ICONS[event.engine] ?? Activity;
  const statusMeta = STATUS_META[event.status] ?? STATUS_META.ok;
  const StatusIcon = statusMeta.Icon;
  const bytesLabel =
    event.bytes !== null && event.bytes !== undefined && event.bytes > 0
      ? formatBytes(event.bytes)
      : null;
  // Rows with no subject path have nothing to navigate to — render them
  // as a plain container rather than a button so assistive tech doesn't
  // announce a dead "press to activate" affordance.
  const navigable = event.subject_path !== null && event.subject_path.length > 0;
  const hasCorrelation = event.correlation_id !== null && event.correlation_id.length > 0;
  const isTraceActive = hasCorrelation && event.correlation_id === activeCorrelation;

  // The row body holds engine icon + summary + meta line. Kept as plain
  // content so we can wrap it in either a <button> (navigable) or a <div>
  // (not navigable) without nesting interactive elements — the trace
  // affordance is rendered as a sibling, never a descendant, of the body.
  const body = (
    <>
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
            <Highlight text={event.summary} query={query} />
          </span>
        </div>
        <div className="mt-0.5 flex items-center gap-2 text-[10px] text-[color:var(--color-text-muted)]">
          <span>{formatRelativeTime(event.occurred_at, undefined, "withJustNow")}</span>
          <span aria-hidden="true">·</span>
          <span>
            <Highlight text={event.kind} query={query} />
          </span>
          {bytesLabel && (
            <>
              <span aria-hidden="true">·</span>
              <span>{bytesLabel}</span>
            </>
          )}
          {groupSize >= 2 && hasCorrelation && (
            <>
              <span aria-hidden="true">·</span>
              <span
                role="button"
                tabIndex={0}
                onClick={(e) => {
                  e.stopPropagation();
                  onTrace(event.correlation_id);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" || e.key === " ") {
                    e.preventDefault();
                    e.stopPropagation();
                    onTrace(event.correlation_id);
                  }
                }}
                title={`This operation touched ${groupSize} files — click to trace`}
                aria-label={`Trace this operation, ${groupSize} files`}
                data-testid={`group-size-${event.correlation_id}`}
                className="cursor-pointer rounded bg-sky-500/10 px-1 py-px text-[9px] font-medium text-sky-600 transition-colors hover:bg-sky-500/20 hover:text-sky-700 focus:outline-none focus:ring-1 focus:ring-sky-500/40 dark:text-sky-400 dark:hover:text-sky-300"
              >
                +{groupSize - 1} more in op
              </span>
            </>
          )}
          {navigable && (
            <>
              <span aria-hidden="true">·</span>
              <span className="truncate" title={event.subject_path ?? undefined}>
                <Highlight text={event.subject_path ?? ""} query={query} />
              </span>
            </>
          )}
        </div>
      </div>
    </>
  );

  // Trace affordance — shown only when this event carries a correlation_id.
  // Clicking it narrows the whole timeline to every event in the same causal
  // chain (sync run → fs writes → automation fires). When that same trace is
  // already active, clicking again clears it — so the button is a toggle.
  const traceButton = hasCorrelation ? (
    <button
      type="button"
      onClick={(e) => {
        e.stopPropagation();
        onTrace(isTraceActive ? null : event.correlation_id);
      }}
      title={
        isTraceActive
          ? "Clear operation trace"
          : "Trace all events in this operation"
      }
      aria-label={
        isTraceActive
          ? "Clear operation trace"
          : `Trace all events in operation ${event.correlation_id}`
      }
      aria-pressed={isTraceActive}
      className={cn(
        "flex-shrink-0 rounded p-1 transition-colors focus:outline-none focus:ring-2 focus:ring-sky-500/40",
        isTraceActive
          ? "text-sky-500"
          : "text-[color:var(--color-text-muted)] opacity-0 hover:text-sky-500 group-hover:opacity-100 focus:opacity-100",
      )}
    >
      <Link2 className="h-3 w-3" aria-hidden="true" />
    </button>
  ) : null;

  // Use a flex container with the (maybe-navigable) body as one child and
  // the trace button as a sibling. The body holds its own interactivity —
  // the two never nest, which keeps a11y semantics clean.
  return (
    <div className="group flex items-start gap-1 rounded-md pr-1 hover:bg-[var(--color-bg-tertiary)]">
      {navigable ? (
        <button
          type="button"
          onClick={() => onNavigate(event)}
          className="flex min-w-0 flex-1 items-start gap-2 rounded-md py-1.5 pl-2 pr-1 text-left focus:outline-none focus:ring-2 focus:ring-sky-500/40"
          title={`Jump to ${event.subject_path}`}
          aria-label={`Jump to ${event.subject_path}`}
        >
          {body}
        </button>
      ) : (
        <div className="flex min-w-0 flex-1 items-start gap-2 py-1.5 pl-2 pr-1">
          {body}
        </div>
      )}
      {traceButton}
      {canUndo && event.correlation_id && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onUndoEntry(event.correlation_id!);
          }}
          title={`Undo this ${event.kind}`}
          aria-label={`Undo this ${event.kind} operation`}
          className="flex-shrink-0 rounded p-1 text-[color:var(--color-text-muted)] opacity-0 transition-colors hover:text-amber-500 focus:outline-none focus:opacity-100 focus:ring-2 focus:ring-amber-500/40 group-hover:opacity-100"
        >
          <Undo2 className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
      {canRedo && event.correlation_id && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onRedoEntry(event.correlation_id!);
          }}
          title={`Redo this ${event.kind}`}
          aria-label={`Redo this ${event.kind} operation`}
          className="flex-shrink-0 rounded p-1 text-[color:var(--color-text-muted)] opacity-0 transition-colors hover:text-amber-500 focus:outline-none focus:opacity-100 focus:ring-2 focus:ring-amber-500/40 group-hover:opacity-100"
          data-testid={`redo-event-${event.correlation_id}`}
        >
          <Undo2 className="h-3 w-3 -scale-x-100" aria-hidden="true" />
        </button>
      )}
      {canPin && event.correlation_id && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onPinEntry(event.correlation_id!, event.kind);
          }}
          title={`Pin this ${event.kind} as a re-runnable Quickflow`}
          aria-label={`Pin this ${event.kind} operation as a re-runnable Quickflow`}
          className="flex-shrink-0 rounded p-1 text-[color:var(--color-text-muted)] opacity-0 transition-colors hover:text-violet-500 focus:outline-none focus:opacity-100 focus:ring-2 focus:ring-violet-500/40 group-hover:opacity-100"
          data-testid={`pin-event-${event.correlation_id}`}
        >
          <Pin className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
      {canRetry && event.correlation_id && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onRetryEntry(event.correlation_id!, event.kind);
          }}
          title={`Retry this failed ${event.kind} now`}
          aria-label={`Retry this failed ${event.kind} operation`}
          className="flex-shrink-0 rounded p-1 text-[color:var(--color-text-muted)] opacity-0 transition-colors hover:text-emerald-500 focus:outline-none focus:opacity-100 focus:ring-2 focus:ring-emerald-500/40 group-hover:opacity-100"
          data-testid={`retry-event-${event.correlation_id}`}
        >
          <RefreshCw className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
      {/* Show File History — symmetric mirror of the Lineage Panel's
       *  "View folder activity" button from iteration 15. Opens the
       *  Lineage Panel for this row's subject_path so the user can
       *  see the file's full ledger history without leaving the
       *  timeline. Only renders when subject_path is present. */}
      {event.subject_path && event.subject_path.length > 0 && (
        <button
          type="button"
          onClick={(e) => {
            e.stopPropagation();
            onShowFileHistory(event.subject_path!);
          }}
          title={`Show history for ${event.subject_path}`}
          aria-label={`Show file history for ${event.subject_path}`}
          className="flex-shrink-0 rounded p-1 text-[color:var(--color-text-muted)] opacity-0 transition-colors hover:text-sky-500 focus:outline-none focus:opacity-100 focus:ring-2 focus:ring-sky-500/40 group-hover:opacity-100"
          data-testid={`show-history-${event.id}`}
        >
          <History className="h-3 w-3" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}
