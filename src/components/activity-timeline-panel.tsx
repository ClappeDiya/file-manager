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
import { useActivityTimelineStore } from "@/stores/activity-timeline-store";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { useUIStore } from "@/stores/ui-store";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { cn } from "@ufop/ui-components";
import { Button, Badge, ScrollArea } from "@ufop/ui-components";
import {
  X,
  RefreshCw,
  Search,
  Activity,
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
  CheckCircle2,
  XCircle,
  Ban,
  MinusCircle,
  AlertTriangle,
  Link2,
  Undo2,
} from "lucide-react";

/**
 * Wire-format event from the Rust `ledger_recent` command.
 * Must match `LedgerEvent` in `src-tauri/src/ledger/mod.rs` exactly —
 * all `Option<T>` fields serialize as `T | null`.
 */
interface LedgerEvent {
  id: string;
  occurred_at: string;
  engine: string;
  kind: string;
  status: string;
  subject_path: string | null;
  target_path: string | null;
  bytes: number | null;
  correlation_id: string | null;
  summary: string;
  details_json: string;
  undo_token: string | null;
}

/** Centralized engine → icon map. New engines just add one entry. */
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

/** Engine display labels. Pluralization handled at call-site if needed. */
const ENGINE_LABELS: Record<string, string> = {
  fs: "File",
  transfer: "Transfer",
  sync: "Sync",
  automation: "Automation",
  mount: "Mount",
  spaces: "Space",
  ai: "AI",
  vault: "Vault",
  compat: "Compat",
  connector: "Connector",
  system: "System",
};

/** Status → (icon, color class) map. Keeps status rendering DRY. */
const STATUS_META: Record<string, { Icon: typeof Activity; className: string }> = {
  ok: { Icon: CheckCircle2, className: "text-emerald-500" },
  failed: { Icon: XCircle, className: "text-red-500" },
  cancelled: { Icon: Ban, className: "text-amber-500" },
  skipped: { Icon: MinusCircle, className: "text-slate-400" },
};

/** Format bytes into a short human string. Duplicates no existing util. */
function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`;
  if (n < 1024 * 1024 * 1024) return `${(n / (1024 * 1024)).toFixed(1)} MB`;
  return `${(n / (1024 * 1024 * 1024)).toFixed(2)} GB`;
}

/**
 * Format an ISO-ish timestamp into a short relative label ("2m ago").
 * Uses Date parsing — both chrono RFC3339 and SQLite `YYYY-MM-DD HH:MM:SS`
 * (which Date treats as local time when parsed) are handled by coercing
 * the space-separated form to `T`-separated UTC.
 */
function formatRelative(iso: string): string {
  // SQLite form: "2026-04-08 12:34:56" → treat as UTC
  const normalized = iso.includes("T") ? iso : iso.replace(" ", "T") + "Z";
  const t = new Date(normalized).getTime();
  if (Number.isNaN(t)) return iso;
  const deltaSec = Math.max(0, Math.floor((Date.now() - t) / 1000));
  if (deltaSec < 5) return "just now";
  if (deltaSec < 60) return `${deltaSec}s ago`;
  if (deltaSec < 3600) return `${Math.floor(deltaSec / 60)}m ago`;
  if (deltaSec < 86_400) return `${Math.floor(deltaSec / 3600)}h ago`;
  return `${Math.floor(deltaSec / 86_400)}d ago`;
}

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

export function ActivityTimelinePanel() {
  const panelOpen = useActivityTimelineStore((s) => s.panelOpen);
  const togglePanel = useActivityTimelineStore((s) => s.togglePanel);
  const [events, setEvents] = useState<LedgerEvent[]>([]);
  const [loading, setLoading] = useState(false);
  // Set of correlation_ids the backend currently considers reversible.
  // Fetched alongside events so each row can show a per-entry "Undo" button
  // without any extra calls per row. This is O(N) in undoable groups, with
  // N capped at 100 by `list_undoable`.
  const [undoableIds, setUndoableIds] = useState<Set<string>>(() => new Set());
  const [filter, setFilter] = useState<EngineFilter>("all");
  const [query, setQuery] = useState("");
  const [failedOnly, setFailedOnly] = useState(false);
  // Correlation-trace filter: when set, the timeline narrows to every
  // event sharing this correlation_id — exposing the cross-engine causal
  // chain the ledger already records (e.g. one sync run → N fs writes →
  // M automation fires). This surfaces hidden operation tracing with
  // literally one new predicate; zero backend changes.
  const [correlationFilter, setCorrelationFilter] = useState<string | null>(null);
  const searchInputRef = useRef<HTMLInputElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

  /**
   * "Jump-to-path" — navigate the active pane to the parent directory
   * of the subject, closing the loop from "what happened" to "take me
   * there". Pulls store methods via `.getState()` so we don't subscribe
   * the component to navigation state (which would cause re-renders on
   * every path change in either pane).
   *
   * Edge cases handled:
   * - subject_path `null` (engine events without a path target): no-op
   * - leaf-only path (no separator): navigate to the leaf itself
   * - same path as active tab: still emits navigateTab so history picks
   *   up the "jump" intent (harmless; `navigateTab` is idempotent-ish)
   */
  const navigateToEvent = useCallback((event: LedgerEvent) => {
    const target = event.subject_path;
    if (!target) return;
    const store = useFileManagerStore.getState();
    const paneIndex = store.activePaneIndex;
    const activeTab = store.getActiveTab(paneIndex);
    if (!activeTab) return;
    const { parent, leaf } = splitPath(target);
    // Navigate to the parent directory so the user can see context; if
    // there is no parent (e.g. root-level), fall back to the target.
    const destination = parent || target;
    const label = leaf || destination.split(/[/\\]/).filter(Boolean).pop() || destination;
    store.navigateTab(paneIndex, activeTab.id, destination, label);
  }, []);

  const loadEvents = useCallback(async () => {
    setLoading(true);
    // Fetch events and the current undoable groups in parallel. A single
    // round-trip per refresh; both lists already live in the ledger.
    const [rows, undoableOps] = await Promise.all([
      tauriInvokeSafe<LedgerEvent[]>("ledger_recent", { limit: 200 }, []),
      tauriInvokeSafe<Array<{ correlation_id: string }>>(
        "list_undoable",
        { limit: 100 },
        [],
      ),
    ]);
    setEvents(rows ?? []);
    setUndoableIds(new Set((undoableOps ?? []).map((o) => o.correlation_id)));
    setLoading(false);
  }, []);

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
  // most ~200 rows so a single pass with both predicates is fine — no need
  // for two array allocations. Search matches summary, kind, and subject
  // path so users can find by what happened, what kind of op, or where.
  const filteredEvents = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (filter === "all" && q === "" && !failedOnly && correlationFilter === null) {
      return events;
    }
    return events.filter((e) => {
      if (correlationFilter !== null && e.correlation_id !== correlationFilter) return false;
      if (filter !== "all" && e.engine !== filter) return false;
      if (failedOnly && e.status === "ok") return false;
      if (q === "") return true;
      if (e.summary.toLowerCase().includes(q)) return true;
      if (e.kind.toLowerCase().includes(q)) return true;
      if (e.subject_path && e.subject_path.toLowerCase().includes(q)) return true;
      return false;
    });
  }, [events, filter, query, failedOnly, correlationFilter]);

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

  // Reset scroll to top whenever the active filter changes. Without this,
  // typing in search or flipping a chip leaves the user staring at a
  // stale scroll position (sometimes past the end of the filtered list).
  // Keyed on the three filter inputs — not on `filteredEvents` identity,
  // which would also re-fire on fresh data loads. Effect is bound only
  // while the panel is open via the `panelOpen` render short-circuit.
  useEffect(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = 0;
  }, [query, filter, failedOnly, correlationFilter]);

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
          <span className="text-sm font-semibold text-[color:var(--color-text)]">
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
            title={
              filteredEvents.length !== events.length
                ? `${filteredEvents.length} of ${events.length} events match current filters`
                : `${events.length} events`
            }
          >
            {filteredEvents.length !== events.length
              ? `${filteredEvents.length} of ${events.length}`
              : events.length}
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
            title="Refresh"
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
            title="Close panel"
            aria-label="Close activity panel"
          >
            <X className="h-4 w-4" />
          </Button>
        </div>
      </div>

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
          <button
            type="button"
            onClick={() => setCorrelationFilter(null)}
            aria-label="Clear correlation trace filter"
            className="rounded p-0.5 text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text)]"
          >
            <X className="h-3 w-3" />
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
}: {
  filter: EngineFilter;
  query: string;
  failedOnly: boolean;
  correlationFilter: string | null;
}) {
  const hasQuery = query.trim() !== "";
  // Message priority: correlation trace first (most specific — user is
  // tracing a single operation), then query, then failed-only, then
  // engine filter, then the default "nothing ran yet" state. Keeps the
  // copy aligned with the most recently-set filter so users understand
  // why they're seeing an empty list.
  const message = correlationFilter !== null
    ? "No other events in this operation trace."
    : hasQuery
    ? `No activity matches "${query.trim()}".`
    : failedOnly
      ? "No failed events in the recent window — everything is healthy."
      : filter === "all"
        ? "No activity yet. Operations will appear here as engines run."
        : `No ${ENGINE_LABELS[filter] ?? filter} activity in the recent window.`;
  return (
    <div className="py-8 text-center">
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
}: {
  events: LedgerEvent[];
  onNavigate: (event: LedgerEvent) => void;
  onTrace: (id: string | null) => void;
  activeCorrelation: string | null;
  query: string;
  undoableIds: Set<string>;
  onUndoEntry: (correlationId: string) => void;
}) {
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
    const canUndo =
      ev.correlation_id !== null && undoableIds.has(ev.correlation_id);
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
}: {
  event: LedgerEvent;
  onNavigate: (event: LedgerEvent) => void;
  onTrace: (id: string | null) => void;
  activeCorrelation: string | null;
  query: string;
  canUndo: boolean;
  onUndoEntry: (correlationId: string) => void;
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
          <span>{formatRelative(event.occurred_at)}</span>
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
    </div>
  );
}
