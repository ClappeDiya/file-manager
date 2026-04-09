/**
 * PathJumpDialog — Instant Jump to any path the ledger has ever seen.
 *
 * The fourth feature in the ledger-axis product line, after Activity
 * Timeline, Universal Undo, and File Lineage. The first three all answer
 * retrospective questions ("what happened?" / "reverse it" / "show this
 * file's story"). This one is prospective: "take me somewhere I've been
 * before, regardless of which connector, which session, or how long ago."
 *
 * # Design principles applied here
 *
 * - **Zero new state**: backed by the existing `ledger_recent_paths`
 *   IPC command. No new tables, no cache, no background work. The
 *   debounced query fires only while the modal is open and the user
 *   has typed something.
 * - **Zero cognitive overload**: invisible until the user presses
 *   `Cmd+Shift+O`. No toolbar icon, no badge, no tutorial.
 * - **Fail-soft**: outside Tauri (pnpm dev browser preview) returns an
 *   empty list instead of crashing. No results renders a polite empty
 *   state; no fake placeholder data.
 * - **DRY**: reuses the active pane's existing `navigateTab` action
 *   from the file-manager store — we don't invent a new "open a path"
 *   code path that could drift from the standard one.
 */
import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
} from "react";
import { usePathJumpStore } from "@/stores/path-jump-store";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { tauriInvokeSafe } from "@/hooks/use-tauri";
import { cn } from "@ufop/ui-components";
import { Search, Clock, Hash, CornerDownLeft, History } from "lucide-react";

/** Wire-format mirror of `LedgerPathHit` in `src-tauri/src/ledger/mod.rs`. */
interface PathHit {
  path: string;
  last_seen: string;
  hit_count: number;
}

/** Debounce in ms. Every keystroke is cheap (single SQL scan), but
 *  batching reduces rapid-fire roundtrips during fast typing. */
const DEBOUNCE_MS = 100;

/** Max results the backend is asked for. 150 is comfortably more than a
 *  modal list can render usefully without scrolling, but small enough
 *  that the SQL scan stays trivial on any realistic ledger. */
const RESULT_LIMIT = 150;

function formatRelative(iso: string): string {
  // SQLite stores `YYYY-MM-DD HH:MM:SS` in UTC; append Z so Date parses.
  const d = new Date(iso.includes("T") ? iso : `${iso.replace(" ", "T")}Z`);
  if (Number.isNaN(d.getTime())) return iso;
  const diffMs = Date.now() - d.getTime();
  const diffSec = Math.round(diffMs / 1000);
  if (diffSec < 60) return `${diffSec}s`;
  const diffMin = Math.round(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m`;
  const diffHr = Math.round(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h`;
  const diffDay = Math.round(diffHr / 24);
  return `${diffDay}d`;
}

function deriveLabel(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/**
 * Very small highlighter — wraps exact substring matches of `query` in
 * a styled span. Case-insensitive, anchored on first match. Skipped
 * entirely when there's no query.
 */
function Highlight({ text, query }: { text: string; query: string }) {
  if (!query) return <>{text}</>;
  const idx = text.toLowerCase().indexOf(query.toLowerCase());
  if (idx < 0) return <>{text}</>;
  return (
    <>
      {text.slice(0, idx)}
      <span className="rounded bg-sky-500/20 text-sky-300">
        {text.slice(idx, idx + query.length)}
      </span>
      {text.slice(idx + query.length)}
    </>
  );
}

export function PathJumpDialog() {
  const isOpen = usePathJumpStore((s) => s.isOpen);
  const close = usePathJumpStore((s) => s.close);

  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [hits, setHits] = useState<PathHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement | null>(null);
  const listRef = useRef<HTMLDivElement | null>(null);

  // Debounce the query so fast typing doesn't thrash the IPC channel.
  useEffect(() => {
    const handle = window.setTimeout(() => setDebouncedQuery(query), DEBOUNCE_MS);
    return () => window.clearTimeout(handle);
  }, [query]);

  // Fetch results whenever the modal is open and the debounced query
  // changes. Reuses `tauriInvokeSafe` so the browser preview gets an
  // empty list fallback rather than crashing.
  useEffect(() => {
    if (!isOpen) return;
    let cancelled = false;
    setLoading(true);
    (async () => {
      const result = await tauriInvokeSafe<PathHit[]>(
        "ledger_recent_paths",
        {
          limit: RESULT_LIMIT,
          query: debouncedQuery.trim() === "" ? null : debouncedQuery,
        },
        [],
      );
      if (!cancelled) {
        setHits(result);
        setSelectedIndex(0);
        setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [isOpen, debouncedQuery]);

  // Reset query + focus the input whenever the modal opens.
  useEffect(() => {
    if (isOpen) {
      setQuery("");
      setDebouncedQuery("");
      setSelectedIndex(0);
      // Defer focus to the next tick so the modal is mounted first.
      const handle = window.setTimeout(() => inputRef.current?.focus(), 0);
      return () => window.clearTimeout(handle);
    }
    return;
  }, [isOpen]);

  // Scroll the highlighted row into view as the selection moves.
  useEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const active = list.querySelector<HTMLElement>(
      `[data-idx="${selectedIndex}"]`,
    );
    if (active) active.scrollIntoView({ block: "nearest" });
  }, [selectedIndex]);

  // The actual navigate action. Uses the active pane's current active
  // tab and navigates it to the chosen path. `navigateTab` is idempotent
  // for the same path so re-jumping to the current directory is a no-op
  // that still gets picked up by the breadcrumb-history layer.
  const navigateTo = useCallback(
    (path: string) => {
      const store = useFileManagerStore.getState();
      const paneIndex = store.activePaneIndex;
      const activeTab = store.getActiveTab(paneIndex);
      if (!activeTab) {
        close();
        return;
      }
      store.navigateTab(paneIndex, activeTab.id, path, deriveLabel(path));
      close();
    },
    [close],
  );

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Escape") {
        e.preventDefault();
        if (query !== "") {
          setQuery("");
          return;
        }
        close();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, Math.max(hits.length - 1, 0)));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const hit = hits[selectedIndex];
        if (hit) navigateTo(hit.path);
        return;
      }
    },
    [query, hits, selectedIndex, navigateTo, close],
  );

  const emptyStateMessage = useMemo(() => {
    if (loading) return "Searching the operation ledger…";
    if (debouncedQuery.trim() === "") {
      return "Start typing any fragment of a path you've ever touched. Renames and cross-connector jumps included.";
    }
    return `No paths matching "${debouncedQuery}" in the last 30 days of history.`;
  }, [loading, debouncedQuery]);

  if (!isOpen) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-start justify-center pt-[15vh]"
      role="dialog"
      aria-modal="true"
      aria-label="Instant Jump"
      onClick={(e) => {
        if (e.target === e.currentTarget) close();
      }}
    >
      {/* Backdrop */}
      <div className="absolute inset-0 bg-black/40 backdrop-blur-sm" aria-hidden="true" />

      {/* Modal card */}
      <div className="relative z-10 w-full max-w-2xl rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] shadow-2xl">
        {/* Input row */}
        <div className="flex items-center gap-2 border-b border-[var(--color-border)] px-3 py-2">
          <Search
            className="h-4 w-4 flex-shrink-0 text-[color:var(--color-text-muted)]"
            aria-hidden="true"
          />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={onKeyDown}
            placeholder="Jump to any path you've ever touched…"
            aria-label="Path to jump to"
            className="flex-1 bg-transparent text-sm text-[color:var(--color-text)] placeholder:text-[color:var(--color-text-muted)] focus:outline-none"
          />
          <div className="hidden items-center gap-1 text-[10px] text-[color:var(--color-text-muted)] sm:flex">
            <CornerDownLeft className="h-3 w-3" aria-hidden="true" />
            <span>to jump</span>
          </div>
        </div>

        {/* Result list */}
        <div
          ref={listRef}
          className="max-h-[50vh] overflow-y-auto"
          role="listbox"
          aria-label="Path results"
        >
          {hits.length === 0 ? (
            <div className="px-4 py-10 text-center">
              <History
                className="mx-auto h-6 w-6 text-[color:var(--color-text-muted)] opacity-50"
                aria-hidden="true"
              />
              <div className="mt-2 text-xs text-[color:var(--color-text-muted)]">
                {emptyStateMessage}
              </div>
            </div>
          ) : (
            hits.map((hit, idx) => (
              <button
                key={hit.path}
                type="button"
                data-idx={idx}
                role="option"
                aria-selected={idx === selectedIndex}
                onClick={() => navigateTo(hit.path)}
                onMouseEnter={() => setSelectedIndex(idx)}
                className={cn(
                  "flex w-full items-center gap-3 px-3 py-2 text-left transition-colors",
                  idx === selectedIndex
                    ? "bg-sky-500/15 text-[color:var(--color-text)]"
                    : "text-[color:var(--color-text)] hover:bg-[var(--color-bg-tertiary)]",
                )}
              >
                <div className="min-w-0 flex-1">
                  <div
                    className="truncate text-sm"
                    title={hit.path}
                  >
                    <Highlight text={hit.path} query={debouncedQuery} />
                  </div>
                  <div className="mt-0.5 flex items-center gap-3 text-[10px] text-[color:var(--color-text-muted)]">
                    <span className="flex items-center gap-1">
                      <Clock className="h-3 w-3" aria-hidden="true" />
                      {formatRelative(hit.last_seen)} ago
                    </span>
                    <span className="flex items-center gap-1">
                      <Hash className="h-3 w-3" aria-hidden="true" />
                      {hit.hit_count} {hit.hit_count === 1 ? "touch" : "touches"}
                    </span>
                  </div>
                </div>
              </button>
            ))
          )}
        </div>

        {/* Footer hint */}
        <div className="flex items-center justify-between border-t border-[var(--color-border)] px-3 py-1.5 text-[10px] text-[color:var(--color-text-muted)]">
          <span>↑↓ navigate · Enter jump · Esc close</span>
          <span>
            {hits.length} {hits.length === 1 ? "path" : "paths"}
          </span>
        </div>
      </div>
    </div>
  );
}
