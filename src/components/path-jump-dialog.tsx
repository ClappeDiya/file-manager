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
import {
  dispatchLedgerPath,
  deriveLabel,
  labelForKind,
  formatRelativeTime,
} from "@/lib/ledger-dispatch";
import { cn } from "@ufop/ui-components";
import { Search, Clock, Hash, CornerDownLeft, History } from "lucide-react";

/** Wire-format mirror of `LedgerPathHit` in `src-tauri/src/ledger/mod.rs`.
 *  Iter 40: `last_kind` added so each hit row can render a
 *  contextual badge ("edit_text", "copy", "move", …) that tells
 *  the user WHY the path is ranked where it is. Optional because
 *  the Rust side serde-skips None, and because sibling queries
 *  like `directory_activity` / `frecent_paths` leave it unset. */
interface PathHit {
  path: string;
  last_seen: string;
  hit_count: number;
  last_kind?: string;
}

/** Iter 37 props — optional callback that lifts the file-vs-dir
 *  decision up to the host. When omitted the dialog falls back to
 *  the pre-iter-37 behaviour of unconditionally navigating the
 *  active pane's tab, so existing call sites that pass no props
 *  stay binary-compatible. The host is expected to decide whether
 *  to open the file in the in-app text editor, the archive
 *  browser, or reveal it in its parent directory — matching the
 *  iter-31/32 double-click routing in file-manager.tsx. */
export interface PathJumpDialogProps {
  onOpenFile?: (path: string, size: number) => void;
  onOpenArchive?: (path: string) => void;
  /** Iter 38: open a media/PDF path in the inline preview pane.
   *  Called only when `isPreviewablePath` returns true for the
   *  hit AND the host decided preview routing is appropriate
   *  (the host is responsible for any contextual gating — e.g.
   *  the FilePane handleOpen only routes previewable files
   *  when the pane is already visible; the Instant Jump gate
   *  is different because a successful jump usually implies
   *  the user wants the content surfaced immediately). */
  onOpenPreview?: (path: string) => void;
  /** Classification predicate so the host can share its
   *  extension whitelist for text files. If omitted, the
   *  dialog treats every non-directory as a "reveal in parent"
   *  candidate. */
  isPlainTextPath?: (path: string) => boolean;
  /** Classification predicate for archive files. See above. */
  isArchivePath?: (path: string) => boolean;
  /** Iter 38: classification predicate for previewable files. */
  isPreviewablePath?: (path: string) => boolean;
}

/** Debounce in ms. Every keystroke is cheap (single SQL scan), but
 *  batching reduces rapid-fire roundtrips during fast typing. */
const DEBOUNCE_MS = 100;

/** Max results the backend is asked for. 150 is comfortably more than a
 *  modal list can render usefully without scrolling, but small enough
 *  that the SQL scan stays trivial on any realistic ledger. */
const RESULT_LIMIT = 150;

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

export function PathJumpDialog({
  onOpenFile,
  onOpenArchive,
  onOpenPreview,
  isPlainTextPath,
  isArchivePath,
  isPreviewablePath,
}: PathJumpDialogProps = {}) {
  const isOpen = usePathJumpStore((s) => s.isOpen);
  const close = usePathJumpStore((s) => s.close);

  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [hits, setHits] = useState<PathHit[]>([]);
  const [loading, setLoading] = useState(false);
  const [selectedIndex, setSelectedIndex] = useState(0);
  // Iter 41: client-side kind filter. When non-null, the result
  // list is narrowed to hits whose `last_kind` exactly matches
  // the filter. Set by clicking a badge on any row; cleared by
  // pressing Escape once (before the Escape-to-close path fires)
  // or by clicking the filter pill's close button. Pure client
  // filter — no extra IPC — because `ledger_recent_paths` already
  // returned every kind in the top-100 window.
  const [kindFilter, setKindFilter] = useState<string | null>(null);
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
      // Iter 41: clear any stale filter from a previous open, so
      // every modal session starts with the full ledger view.
      setKindFilter(null);
      // Defer focus to the next tick so the modal is mounted first.
      const handle = window.setTimeout(() => inputRef.current?.focus(), 0);
      return () => window.clearTimeout(handle);
    }
    return;
  }, [isOpen]);

  // Iter 41: derived visible hits. When a kind filter is active
  // the list narrows to hits whose `last_kind` matches exactly.
  // When the filter is null (default), visibleHits === hits so
  // the pre-iter-41 behaviour is a zero-cost pass-through.
  const visibleHits = useMemo(
    () =>
      kindFilter === null
        ? hits
        : hits.filter((h) => h.last_kind === kindFilter),
    [hits, kindFilter],
  );

  // Iter 41: clamp the selected index whenever `visibleHits`
  // shrinks beneath it (e.g. user clicks a filter and the list
  // collapses from 40 rows to 5). Prevents a stale index from
  // pointing past the end of the list.
  useEffect(() => {
    if (selectedIndex >= visibleHits.length && visibleHits.length > 0) {
      setSelectedIndex(0);
    }
  }, [visibleHits.length, selectedIndex]);

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
  //
  // Iter 37: file-aware routing. Because iter 35 put every in-app
  // text-editor save into the ledger under kind `edit_text`, the
  // ledger_recent_paths response now includes FILES as well as
  // directories. The pre-iter-37 code unconditionally called
  // `navigateTab` on the hit path, which worked for directories
  // but failed silently for files (list_directory on a file
  // errors). This callback now fetches metadata, then routes:
  //   - is_dir=true              → navigateTab (unchanged behaviour)
  //   - archive file             → onOpenArchive(path)
  //   - plain-text file ≤1MB     → onOpenFile(path, size)
  //   - everything else (files)  → navigate the pane to the file's
  //                                parent directory so the file is
  //                                at least visible in the list
  // Metadata IPC failure falls through to the legacy path so an
  // offline/broken state never locks the dialog.
  const doNavigateTab = useCallback(
    (path: string) => {
      const store = useFileManagerStore.getState();
      const paneIndex = store.activePaneIndex;
      const activeTab = store.getActiveTab(paneIndex);
      if (!activeTab) return;
      store.navigateTab(paneIndex, activeTab.id, path, deriveLabel(path));
    },
    [],
  );

  // Iter 42: routing core lifted into `src/lib/ledger-dispatch.ts`.
  // `dispatchLedgerPath` is shared with the file-manager's new
  // Cmd+Shift+L "Jump to Last Touched" shortcut, so Instant Jump
  // (Enter) and the last-touched teleport now route through one
  // codepath — zero divergence risk.
  const navigateTo = useCallback(
    async (path: string) => {
      await dispatchLedgerPath(path, {
        isArchivePath,
        isPlainTextPath,
        isPreviewablePath,
        onOpenArchive,
        onOpenFile,
        onOpenPreview,
        onNavigateDir: doNavigateTab,
      });
      close();
    },
    [
      close,
      doNavigateTab,
      isArchivePath,
      isPlainTextPath,
      isPreviewablePath,
      onOpenArchive,
      onOpenFile,
      onOpenPreview,
    ],
  );

  const onKeyDown = useCallback(
    (e: KeyboardEvent<HTMLInputElement>) => {
      if (e.key === "Escape") {
        e.preventDefault();
        // Iter 41: Escape precedence — active kind filter clears
        // first, then query text, then closes the modal. This
        // gives the user a consistent "back up one level" gesture
        // without stealing the close shortcut.
        if (kindFilter !== null) {
          setKindFilter(null);
          return;
        }
        if (query !== "") {
          setQuery("");
          return;
        }
        close();
        return;
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) =>
          Math.min(i + 1, Math.max(visibleHits.length - 1, 0)),
        );
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter") {
        e.preventDefault();
        const hit = visibleHits[selectedIndex];
        if (hit) navigateTo(hit.path);
        return;
      }
    },
    [query, visibleHits, selectedIndex, navigateTo, close, kindFilter],
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

        {/* Iter 41: active kind-filter pill. Appears just below
            the input row whenever the user has clicked a kind
            badge on any hit. Clears via its own ✕ button or via
            Escape (the iter-41 precedence layer). */}
        {kindFilter !== null && (() => {
          const { label, tone } = labelForKind(kindFilter);
          return (
            <div
              className="flex items-center gap-2 border-b border-[var(--color-border)] px-3 py-1.5"
              role="status"
              aria-live="polite"
            >
              <span className="text-[10px] text-[color:var(--color-text-muted)]">
                Filtering by
              </span>
              <span
                className={cn(
                  "rounded px-1.5 py-px text-[10px] font-medium",
                  tone,
                )}
              >
                {label}
              </span>
              <button
                type="button"
                onClick={() => setKindFilter(null)}
                className="ml-auto text-[10px] text-[color:var(--color-text-muted)] hover:text-[color:var(--color-text)]"
                aria-label="Clear kind filter"
              >
                ✕ clear
              </button>
            </div>
          );
        })()}

        {/* Result list */}
        <div
          ref={listRef}
          className="max-h-[50vh] overflow-y-auto"
          role="listbox"
          aria-label="Path results"
        >
          {visibleHits.length === 0 ? (
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
            visibleHits.map((hit, idx) => (
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
                      {formatRelativeTime(hit.last_seen, undefined, "withSuffix")}
                    </span>
                    <span className="flex items-center gap-1">
                      <Hash className="h-3 w-3" aria-hidden="true" />
                      {hit.hit_count} {hit.hit_count === 1 ? "touch" : "touches"}
                    </span>
                    {hit.last_kind && (() => {
                      const kindKey = hit.last_kind;
                      const { label, tone } = labelForKind(kindKey);
                      const isActive = kindFilter === kindKey;
                      return (
                        <button
                          type="button"
                          onClick={(e) => {
                            // Iter 41: click on a kind badge toggles
                            // the client-side filter WITHOUT jumping
                            // to the path (stopPropagation + the
                            // parent button's onClick). Clicking the
                            // same kind a second time clears the
                            // filter so the gesture round-trips in
                            // one fingertip.
                            e.stopPropagation();
                            e.preventDefault();
                            setKindFilter((prev) =>
                              prev === kindKey ? null : kindKey,
                            );
                          }}
                          className={cn(
                            "rounded px-1.5 py-px text-[10px] font-medium transition-all",
                            tone,
                            isActive && "ring-1 ring-sky-400",
                          )}
                          aria-label={
                            isActive
                              ? `Clear ${label} filter`
                              : `Filter to ${label} only`
                          }
                          aria-pressed={isActive}
                        >
                          {label}
                        </button>
                      );
                    })()}
                  </div>
                </div>
              </button>
            ))
          )}
        </div>

        {/* Footer hint */}
        <div className="flex items-center justify-between border-t border-[var(--color-border)] px-3 py-1.5 text-[10px] text-[color:var(--color-text-muted)]">
          <span>↑↓ navigate · Enter jump · click badge to filter · Esc back</span>
          <span>
            {visibleHits.length} of {hits.length}{" "}
            {hits.length === 1 ? "path" : "paths"}
          </span>
        </div>
      </div>
    </div>
  );
}
