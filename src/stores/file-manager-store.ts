import { create } from "zustand";
import { persist } from "zustand/middleware";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

export type ViewMode = "list" | "detail" | "grid" | "compact";

export interface PaneTab {
  id: string;
  path: string;
  label: string;
  pinned: boolean;
  pathHistory: string[];
  historyIndex: number;
}

export interface PaneState {
  id: string;
  tabs: PaneTab[];
  activeTabId: string;
  viewMode: ViewMode;
  sortBy: string;
  sortAsc: boolean;
  filterText: string;
  groupBy: string | null;
}

/** A frozen snapshot of a working selection — typically the user's
 *  multi-file marquee or click-pick across one or more directories. Saved
 *  via `Cmd+Shift+M` so the user can navigate away, do something else,
 *  and come back to the *exact* same selection without rebuilding it.
 *
 *  Why a stash and not just "let the persisted selection hang around"?
 *  Because the live selection is per-pane and gets clobbered the moment
 *  the user clicks anywhere else. The stash is an explicit, named, non-
 *  destructive **memory slot** that survives navigation, tab switches,
 *  pane changes, and full app restarts (it's in the persist allowlist).
 *  This pairs with the closed-tab ring buffer (iter 8): together they
 *  let the user resume *both* the where-I-was and the what-I-was-working-
 *  on context with two keystrokes.
 *
 *  The stash deliberately captures `sourceDir` so recall can navigate
 *  the active pane back home before re-applying the selection — the
 *  selection paths only make sense once you can see them. */
export interface SelectionStash {
  id: string;
  /** Snapshot of selected absolute paths at stash time. Frozen — never
   *  mutated, always replaced. */
  paths: string[];
  /** The directory the active pane was viewing when the stash was made.
   *  Used as the recall navigation target so the selection has somewhere
   *  to land. Falls back to `/` if the pane was somehow pathless. */
  sourceDir: string;
  /** Wall-clock at stash time — drives the LIFO ordering and the
   *  human-readable "stashed 5m ago" hint in the palette description. */
  stashedAt: number;
  /** Optional user-supplied label. Empty string when unlabeled — the UI
   *  derives a default from `sourceDir` + count. */
  label: string;
}

/** Snapshot of a tab the user closed, retained in a small per-pane ring
 *  buffer so `Cmd+Shift+T` can revive it with full back/forward history
 *  intact. Stored separately from the live `tabs` array to keep the
 *  active-pane shape unchanged for every other consumer.
 *
 *  We persist this in the partialize allowlist so closing the app and
 *  re-opening it still lets the user reach for the same muscle-memory
 *  shortcut after a relaunch — the same way a browser preserves "reopen
 *  closed tab" across sessions. The bound is small enough that the
 *  localStorage cost is negligible. */
export interface ClosedTabSnapshot {
  path: string;
  label: string;
  pathHistory: string[];
  historyIndex: number;
  /** Wall-clock at the moment of close. Used purely as a tie-break /
   *  diagnostic field — the ring is otherwise pure LIFO. */
  closedAt: number;
}

export interface FavoriteItem {
  id: string;
  name: string;
  path: string;
  icon?: string;
}

/** Aggregate stats a FilePane publishes for the global status bar.
 *  Two parallel tallies — one for the whole folder listing and one
 *  for the current selection — so the status bar can switch
 *  presentation when the user selects items without re-walking the
 *  list. `hasDir` is the "byte total excludes folders" footnote;
 *  the UI appends "+ folders" when true so the size never lies. */
export interface PaneStats {
  totalCount: number;
  totalBytes: number;
  totalHasDir: boolean;
  selectedCount: number;
  selectedBytes: number;
  selectedHasDir: boolean;
}

export interface RecentLocation {
  path: string;
  timestamp: number;
}

export interface SavedSearch {
  id: string;
  name: string;
  query: string;
  path: string;
  recursive: boolean;
}

export interface UndoEntry {
  id: string;
  type: "copy" | "move" | "rename" | "delete" | "create_folder" | "create_file";
  sourcePaths: string[];
  destPaths: string[];
  timestamp: number;
}

export interface SavedWorkspace {
  id: string;
  name: string;
  panes: [PaneState, PaneState];
  singlePaneMode: boolean;
  paneSplitPercent: number;
  paneOrientation: "horizontal" | "vertical";
  syncBrowsing: boolean;
  savedAt: number;
}

export type DoubleClickBehavior = "open" | "edit" | "preview" | "transfer";

export interface EditorMapping {
  id: string;
  extension: string;
  appName: string;
  appPath: string;
  args: string[];
}

export interface PrivacySettings {
  autoUpdateCheck: boolean;
  crashReportOptIn: boolean;
}

export interface FileManagerState {
  // Dual-pane layout
  panes: [PaneState, PaneState];
  activePaneIndex: 0 | 1;
  singlePaneMode: boolean;
  paneSplitPercent: number; // 20-80 range
  paneOrientation: "horizontal" | "vertical";

  // Sync browsing
  syncBrowsing: boolean;

  // Navigation
  favorites: FavoriteItem[];
  recentLocations: RecentLocation[];
  expandedTreePaths: string[];

  // Selection
  selectedPaths: Record<string, string[]>; // paneId -> selected paths

  // Per-pane status-bar stats — published by each FilePane whenever its
  // file list or selection changes. Lets the global status bar render
  // "N items · X.X MB" (folder total) or "K of N selected · Y.Y MB"
  // (selection) without lifting the entire `files` array into the store.
  // `null` when the pane has not yet published (initial mount).
  paneStats: Record<string, PaneStats | null>;

  // Search
  savedSearches: SavedSearch[];

  // Undo stack
  undoStack: UndoEntry[];

  // Command palette
  commandPaletteOpen: boolean;

  // Hidden files
  showHiddenFiles: boolean;

  // Per-folder view defaults
  folderViewDefaults: Record<string, { viewMode: ViewMode; sortBy: string; sortAsc: boolean }>;

  // Saved workspaces
  workspaces: SavedWorkspace[];

  // Double-click behavior
  doubleClickBehavior: DoubleClickBehavior;

  // Editor mappings
  editorMappings: EditorMapping[];

  // Privacy settings
  privacySettings: PrivacySettings;

  // Recently closed tabs — small per-pane LIFO ring buffer that powers the
  // browser-style "reopen closed tab" shortcut. Each entry preserves the
  // tab's path, label, and full back/forward stack so revival is bit-exact.
  recentlyClosedTabs: { 0: ClosedTabSnapshot[]; 1: ClosedTabSnapshot[] };

  // Selection Stash — global LIFO ring of explicit selection snapshots.
  // Persisted across sessions so a multi-file working set built on Friday
  // is still recallable on Monday. Single ring (not per-pane) because a
  // stash represents a *working set*, not a pane state — recalling it
  // into either pane is a deliberate user choice, not a property of the
  // stash itself.
  selectionStashes: SelectionStash[];

  // Actions - Double-click
  setDoubleClickBehavior: (behavior: DoubleClickBehavior) => void;

  // Actions - Editor mappings
  addEditorMapping: (mapping: EditorMapping) => void;
  updateEditorMapping: (mapping: EditorMapping) => void;
  removeEditorMapping: (id: string) => void;

  // Actions - Privacy
  setPrivacySettings: (settings: Partial<PrivacySettings>) => void;

  // Actions - Workspace management
  saveWorkspace: (name: string) => SavedWorkspace;
  loadWorkspace: (id: string) => void;
  deleteWorkspace: (id: string) => void;
  listWorkspaces: () => SavedWorkspace[];

  // Actions - Pane management
  setActivePaneIndex: (index: 0 | 1) => void;
  toggleSinglePaneMode: () => void;
  setPaneSplitPercent: (percent: number) => void;
  togglePaneOrientation: () => void;
  toggleSyncBrowsing: () => void;

  // Actions - Tab management
  addTab: (paneIndex: 0 | 1, path: string, label: string) => void;
  closeTab: (paneIndex: 0 | 1, tabId: string) => void;
  setActiveTab: (paneIndex: 0 | 1, tabId: string) => void;
  pinTab: (paneIndex: 0 | 1, tabId: string) => void;
  unpinTab: (paneIndex: 0 | 1, tabId: string) => void;
  reorderTabs: (paneIndex: 0 | 1, fromIndex: number, toIndex: number) => void;
  navigateTab: (paneIndex: 0 | 1, tabId: string, path: string, label: string) => void;
  /** Iter 20 — tab context menu. Close every non-pinned tab in this
   *  pane except `keepTabId`. Pinned tabs survive regardless of
   *  position. Each close pushes a snapshot onto the closed-tab ring
   *  (one per closed tab) so Cmd+Shift+T can revive them in LIFO
   *  order. No-op when `keepTabId` doesn't exist in this pane. */
  closeOtherTabs: (paneIndex: 0 | 1, keepTabId: string) => void;
  /** Iter 20 — tab context menu. Close every non-pinned tab whose
   *  visual position is to the right of `fromTabId`. Visual position
   *  follows the tab-bar render order (pinned first, then original
   *  insertion order), so "to the right" matches what the user sees.
   *  No-op when `fromTabId` is the rightmost tab. Each closed tab is
   *  ring-buffered exactly the same way `closeTab` ring-buffers a
   *  single close. */
  closeTabsToRight: (paneIndex: 0 | 1, fromTabId: string) => void;
  /** Iter 20 — tab context menu. Duplicate a specific tab (not just
   *  the active one). Returns the same shape as `duplicateActiveTab`
   *  so toast surfaces can render consistent post-action copy. */
  duplicateTab: (paneIndex: 0 | 1, tabId: string) => { path: string; label: string } | null;
  /** Iter 20 — tab context menu. Move a tab from this pane to the
   *  other pane. The receiving pane becomes active. No-op when the
   *  source pane has only one tab (moving would leave it empty).
   *  Reveals the second pane if the app is in single-pane mode. */
  moveTabToOtherPane: (paneIndex: 0 | 1, tabId: string) => { otherIndex: 0 | 1; path: string; label: string } | null;
  /** Pop the most recent entry from the active pane's closed-tab ring and
   *  re-open it as a new tab, preserving the original back/forward
   *  history. Returns the revived tab's id, or `null` when the ring is
   *  empty (no-op so the keyboard shortcut and palette entry stay safe). */
  reopenClosedTab: (paneIndex: 0 | 1) => string | null;

  // Actions - Selection Stash
  /** Capture the active pane's current selection (and its source dir)
   *  into the global stash ring. Returns the freshly created stash, or
   *  `null` when the active pane has nothing selected — the no-op return
   *  keeps the shortcut safe to bind. Optional `label` lets the caller
   *  supply a human-readable name; the empty default makes the palette
   *  fall back to a derived "{count} files in {basename}" hint. */
  stashCurrentSelection: (label?: string) => SelectionStash | null;
  /** Replay a stash into the given pane: navigate the pane's active tab
   *  to `sourceDir`, then overwrite the pane's selection with the stash
   *  paths. The stash itself stays in the ring so the same selection can
   *  be re-recalled into the other pane or after further navigation. */
  recallStash: (id: string, paneIndex: 0 | 1) => void;
  /** Forget a single stash by id. */
  discardStash: (id: string) => void;
  /** Wipe every stash. Used by the palette's "clear all" entry and by
   *  test setup. */
  clearStashes: () => void;

  // Actions - View
  setViewMode: (paneIndex: 0 | 1, mode: ViewMode) => void;
  setSorting: (paneIndex: 0 | 1, sortBy: string, sortAsc: boolean) => void;
  setFilterText: (paneIndex: 0 | 1, text: string) => void;
  setGroupBy: (paneIndex: 0 | 1, groupBy: string | null) => void;

  // Actions - Navigation
  addFavorite: (item: FavoriteItem) => void;
  removeFavorite: (id: string) => void;
  /** Add the active pane's current directory to favorites if it isn't
   *  already bookmarked, or remove it if it is. Reuses the same
   *  `favorites` ring the sidebar drag-drop populates — same surface,
   *  new accelerator. Returns a descriptor so the caller can render a
   *  user-facing toast (added vs removed + the resolved label). Returns
   *  `null` only when the active pane has no active tab, which should
   *  never happen in practice but keeps the keyboard handler safe. */
  toggleFavoriteForCurrentDir: () =>
    | { added: boolean; item: FavoriteItem }
    | null;
  /** Jump the active pane's active tab to the favorite at the given
   *  zero-based index. No-op (returns false) when the index is out of
   *  range, so binding `Cmd+1`..`Cmd+9` to a sparse list is always safe.
   *  Routes through `navigateTab` so the back/forward stack and ledger
   *  bookkeeping match every other navigation in the app — DRY. */
  jumpToFavoriteByIndex: (paneIndex: 0 | 1, index: number) => boolean;
  /** Open the given pane's active path as a fresh tab in the OTHER pane,
   *  automatically exiting single-pane mode if it's on (the gesture is
   *  "show me this folder side-by-side", so revealing the second pane is
   *  part of the action). Focuses the other pane so the user immediately
   *  operates over there. Returns a descriptor for the toast, or `null`
   *  when the source pane has no active tab (defensive — should never
   *  happen in practice but keeps the keyboard handler safe). */
  openInOtherPane: (
    paneIndex: 0 | 1,
  ) => { path: string; label: string; otherIndex: 0 | 1 } | null;
  /** Swap the contents of the two panes — what was on the left is now on
   *  the right and vice versa. Selections, tabs, scroll/sort state, and
   *  per-pane history all move with their pane (selections live in
   *  `selectedPaths` keyed by the embedded `pane.id`, not by array index,
   *  so they follow the content automatically). `activePaneIndex` is
   *  intentionally **not** flipped — the user's focus stays on the
   *  physical side they were on, and they see the other pane's content
   *  arrive there. This matches the convention in Total Commander /
   *  Krusader / Double Commander where Tab moves focus between panes and
   *  swap is purely a visual flip. Auto-exits single-pane mode (the
   *  gesture only makes sense when both sides are visible). Returns the
   *  new active pane's path so the keyboard handler can surface a toast.
   *  Returns `null` only in the defensive "no active tab" case. */
  swapPanes: () => { activePath: string; activeLabel: string } | null;
  /** Copy the given pane's `filterText` onto the OTHER pane so both
   *  sides filter for the same query. Auto-exits single-pane mode so
   *  the echoed filter is visible. Reuses the existing per-pane
   *  `filterText` field — no new state, no parallel filter pipeline,
   *  no new IPC. Returns a descriptor for the toast (the echoed text
   *  + which side received it), or `null` when the source filter is
   *  empty (echoing nothing is a no-op the user shouldn't be told
   *  about, since it would just confuse them). */
  echoFilterToOtherPane: (
    paneIndex: 0 | 1,
  ) => { text: string; otherIndex: 0 | 1 } | null;
  /** Duplicate the given pane's active tab as a fresh tab in the
   *  SAME pane (different from `openInOtherPane`, which targets the
   *  other side). The clone gets its own `pathHistory: [path]` and
   *  `historyIndex: 0` because it routes through the canonical
   *  `addTab` action — exactly the same way every other tab is
   *  born. The new tab is automatically focused (matching the
   *  browser convention where Cmd+T immediately switches to the
   *  new tab). Returns a descriptor for the toast, or `null` only
   *  in the defensive "no active tab" case. */
  duplicateActiveTab: (
    paneIndex: 0 | 1,
  ) => { path: string; label: string } | null;
  /** Collect the paths the user would reasonably want on their
   *  system clipboard right now: the active pane's full multi-file
   *  selection when there IS one, or the active pane's current
   *  directory path when nothing is selected. The graceful fallback
   *  means the gesture is always useful — never a no-op that leaves
   *  the user wondering what happened. Pure derived read: zero
   *  mutations, zero IPC, zero new state. The keyboard handler is
   *  responsible for the actual `navigator.clipboard.writeText`
   *  side-effect so the store stays side-effect-free and testable.
   *  Returns `{ paths, source }` where `source` is "selection" or
   *  "directory" so the toast can describe what was copied. */
  collectClipboardPaths: () => {
    paths: string[];
    source: "selection" | "directory";
  };
  /** Copy the given pane's full view configuration (viewMode, sortBy,
   *  sortAsc, groupBy) onto the OTHER pane so both sides render with
   *  identical layout, sort, and grouping. The keystroke replaces the
   *  3-4 menu clicks needed to align the two panes manually before a
   *  side-by-side comparison. Auto-exits single-pane mode (the gesture
   *  only makes sense with both sides visible). Reuses the existing
   *  `setViewMode` / `setSorting` / `setGroupBy` action set so the
   *  echo path has zero parallel write logic — DRY by construction.
   *  Returns a descriptor for the toast (which side received it +
   *  the echoed view label), or `null` only in the defensive
   *  no-op-needed case where the two panes already match. */
  mirrorViewToOtherPane: (
    paneIndex: 0 | 1,
  ) => {
    otherIndex: 0 | 1;
    viewMode: ViewMode;
    sortBy: string;
    sortAsc: boolean;
    groupBy: string | null;
  } | null;
  addRecentLocation: (path: string) => void;
  toggleTreeExpanded: (path: string) => void;

  // Actions - Selection
  setSelection: (paneId: string, paths: string[]) => void;
  addToSelection: (paneId: string, path: string) => void;
  removeFromSelection: (paneId: string, path: string) => void;
  clearSelection: (paneId: string) => void;
  selectRange: (paneId: string, paths: string[]) => void;

  // Actions - Status-bar stats. FilePane is the source of truth for its
  // own `files` array, so it publishes the aggregate (folder + selection)
  // here whenever the underlying data changes. The setter is no-op when
  // the incoming stats are equal to the cached ones — keeps the
  // subscriber graph quiet between irrelevant renders.
  setPaneStats: (paneId: string, stats: PaneStats) => void;

  // Actions - Search
  addSavedSearch: (search: SavedSearch) => void;
  removeSavedSearch: (id: string) => void;

  // Actions - Undo
  pushUndo: (entry: UndoEntry) => void;
  popUndo: () => UndoEntry | undefined;

  // Actions - Command palette
  setCommandPaletteOpen: (open: boolean) => void;
  toggleCommandPalette: () => void;

  // Actions - Hidden files
  toggleHiddenFiles: () => void;

  // Actions - Per-folder view defaults
  saveFolderViewDefault: (path: string, settings: { viewMode: ViewMode; sortBy: string; sortAsc: boolean }) => void;
  getFolderViewDefault: (path: string) => { viewMode: ViewMode; sortBy: string; sortAsc: boolean } | undefined;

  // Actions - Navigation history
  navigateBack: (paneIndex: 0 | 1) => void;
  navigateForward: (paneIndex: 0 | 1) => void;
  canNavigateBack: (paneIndex: 0 | 1) => boolean;
  canNavigateForward: (paneIndex: 0 | 1) => boolean;

  // Helpers
  getActivePane: () => PaneState;
  getActiveTab: (paneIndex: 0 | 1) => PaneTab | undefined;
  getActivePath: (paneIndex: 0 | 1) => string;
}

// ──────────────────────────────────────────────
// Helpers
// ──────────────────────────────────────────────

let _idCounter = 0;
function genId(): string {
  return `id-${Date.now()}-${++_idCounter}`;
}

function getHomePath(): string {
  // In Tauri, we get this from the backend. Default to "/" for now.
  return "/";
}

const MAX_HISTORY_ENTRIES = 50;

function createDefaultTab(path?: string): PaneTab {
  const p = path || getHomePath();
  const label = p === "/" ? "Root" : p.split("/").filter(Boolean).pop() || "Root";
  return {
    id: genId(),
    path: p,
    label,
    pinned: false,
    pathHistory: [p],
    historyIndex: 0,
  };
}

function createDefaultPane(path?: string): PaneState {
  const tab = createDefaultTab(path);
  return {
    id: genId(),
    tabs: [tab],
    activeTabId: tab.id,
    viewMode: "detail",
    sortBy: "name",
    sortAsc: true,
    filterText: "",
    groupBy: null,
  };
}

const MAX_UNDO_LEVELS = 10;
const MAX_RECENT_LOCATIONS = 20;
/** Per-pane cap on the closed-tab ring buffer. Browsers typically retain
 *  10–20 closed tabs; 10 is plenty for muscle-memory recovery without
 *  growing the persisted state appreciably. */
const MAX_CLOSED_TABS_PER_PANE = 10;
/** Cap on the global selection-stash ring. Larger than the closed-tab
 *  ring because users build complex selections more deliberately and
 *  may want to keep several around (one per project, one per cleanup
 *  session, etc.). 20 is well within localStorage budget even with
 *  long path lists per stash. */
const MAX_SELECTION_STASHES = 20;

// ──────────────────────────────────────────────
// Store
// ──────────────────────────────────────────────

export const useFileManagerStore = create<FileManagerState>()(
  persist(
    (set, get) => ({
      // Initial state
      panes: [createDefaultPane(), createDefaultPane()],
      activePaneIndex: 0,
      singlePaneMode: true, // Simple mode default
      paneSplitPercent: 50,
      paneOrientation: "horizontal",

      syncBrowsing: false,

      favorites: [],
      recentLocations: [],
      expandedTreePaths: [],

      selectedPaths: {},
      paneStats: {},

      savedSearches: [],

      undoStack: [],

      commandPaletteOpen: false,

      showHiddenFiles: false,

      folderViewDefaults: {},

      workspaces: [],

      doubleClickBehavior: "open",
      editorMappings: [],
      privacySettings: {
        autoUpdateCheck: true,
        crashReportOptIn: false,
      },

      recentlyClosedTabs: { 0: [], 1: [] },

      selectionStashes: [],

      setDoubleClickBehavior: (behavior) => set({ doubleClickBehavior: behavior }),

      addEditorMapping: (mapping) =>
        set((s) => ({ editorMappings: [...s.editorMappings, mapping] })),

      updateEditorMapping: (mapping) =>
        set((s) => ({
          editorMappings: s.editorMappings.map((m) =>
            m.id === mapping.id ? mapping : m,
          ),
        })),

      removeEditorMapping: (id) =>
        set((s) => ({
          editorMappings: s.editorMappings.filter((m) => m.id !== id),
        })),

      setPrivacySettings: (settings) =>
        set((s) => ({
          privacySettings: { ...s.privacySettings, ...settings },
        })),

      // ── Pane management ──

      setActivePaneIndex: (index) => set({ activePaneIndex: index }),

      toggleSinglePaneMode: () =>
        set((s) => ({ singlePaneMode: !s.singlePaneMode })),

      setPaneSplitPercent: (percent) =>
        set({ paneSplitPercent: Math.max(20, Math.min(80, percent)) }),

      togglePaneOrientation: () =>
        set((s) => ({
          paneOrientation: s.paneOrientation === "horizontal" ? "vertical" : "horizontal",
        })),

      toggleSyncBrowsing: () =>
        set((s) => ({ syncBrowsing: !s.syncBrowsing })),

      // ── Tab management ──

      addTab: (paneIndex, path, label) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          const newTab: PaneTab = { id: genId(), path, label, pinned: false, pathHistory: [path], historyIndex: 0 };
          pane.tabs = [...pane.tabs, newTab];
          pane.activeTabId = newTab.id;
          panes[paneIndex] = pane;
          return { panes };
        }),

      closeTab: (paneIndex, tabId) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          // Don't close pinned tabs
          const tab = pane.tabs.find((t) => t.id === tabId);
          if (tab?.pinned) return s;
          // Don't close last tab
          if (pane.tabs.length <= 1) return s;

          // Capture a snapshot BEFORE removing the tab so reopenClosedTab
          // can revive it bit-exact (path + label + back/forward stack).
          // The ring is bounded at MAX_CLOSED_TABS_PER_PANE: oldest entry
          // falls off the end so we never grow without bound.
          const snapshot: ClosedTabSnapshot | null = tab
            ? {
                path: tab.path,
                label: tab.label,
                pathHistory: [...(tab.pathHistory ?? [tab.path])],
                historyIndex: tab.historyIndex ?? 0,
                closedAt: Date.now(),
              }
            : null;

          pane.tabs = pane.tabs.filter((t) => t.id !== tabId);
          if (pane.activeTabId === tabId) {
            pane.activeTabId = pane.tabs[pane.tabs.length - 1].id;
          }
          panes[paneIndex] = pane;

          if (!snapshot) return { panes };

          // Defensive: if a pre-iteration persisted state hydrates without
          // the new field, the merged shape could be missing/non-object.
          // Rehydrate with the empty default rather than crashing on the
          // first close after upgrade.
          const closed = s.recentlyClosedTabs ?? { 0: [], 1: [] };
          const ring = closed[paneIndex] ?? [];
          const nextRing = [snapshot, ...ring].slice(0, MAX_CLOSED_TABS_PER_PANE);
          return {
            panes,
            recentlyClosedTabs: {
              ...closed,
              [paneIndex]: nextRing,
            },
          };
        }),

      reopenClosedTab: (paneIndex) => {
        const snapshot = get().recentlyClosedTabs?.[paneIndex]?.[0];
        if (!snapshot) return null;
        let revivedId: string | null = null;
        set((s) => {
          // Re-pop from the freshest state inside `set` to avoid a TOCTOU
          // race against a parallel close — get() above is just the cheap
          // pre-check that lets the action stay a no-op when the ring is
          // empty without paying for a state read on the hot path.
          const closed = s.recentlyClosedTabs ?? { 0: [], 1: [] };
          const ring = closed[paneIndex] ?? [];
          if (ring.length === 0) return s;
          const [head, ...tail] = ring;

          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          const newTab: PaneTab = {
            id: genId(),
            path: head.path,
            label: head.label,
            pinned: false,
            // Defensive copies — the snapshot stays in the ring until we
            // overwrite it, and a downstream mutation of the live tab
            // shouldn't bleed back into the persisted ring entry.
            pathHistory: [...head.pathHistory],
            historyIndex: head.historyIndex,
          };
          revivedId = newTab.id;
          pane.tabs = [...pane.tabs, newTab];
          pane.activeTabId = newTab.id;
          panes[paneIndex] = pane;

          return {
            panes,
            recentlyClosedTabs: {
              ...closed,
              [paneIndex]: tail,
            },
          };
        });
        return revivedId;
      },

      // ── Selection Stash ──

      stashCurrentSelection: (label) => {
        const state = get();
        const pane = state.panes[state.activePaneIndex];
        const paths = state.selectedPaths[pane.id] ?? [];
        if (paths.length === 0) return null;
        const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
        const sourceDir = tab?.path || "/";
        const stash: SelectionStash = {
          id: genId(),
          // Defensive copy — the live selection array can be replaced at
          // any time, so the stash must own its own snapshot.
          paths: [...paths],
          sourceDir,
          stashedAt: Date.now(),
          label: label?.trim() ?? "",
        };
        set((s) => {
          // Defensive against pre-iteration hydration: the persisted
          // blob from before this iteration won't have the field. The
          // shallow zustand merge keeps `currentState`'s default in
          // that case, but a hand-crafted persistedState with `null`
          // would still slip past — guard explicitly.
          const existing = s.selectionStashes ?? [];
          return {
            selectionStashes: [stash, ...existing].slice(0, MAX_SELECTION_STASHES),
          };
        });
        return stash;
      },

      recallStash: (id, paneIndex) => {
        const state = get();
        const stash = (state.selectionStashes ?? []).find((s) => s.id === id);
        if (!stash) return;
        // Reuse the existing navigateTab + setSelection actions rather
        // than poking the panes array directly, so the recall path goes
        // through the same back/forward-history bookkeeping every other
        // navigation does. DRY: zero new code paths for the file list to
        // ignore.
        const pane = state.panes[paneIndex];
        const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
        if (!tab) return;
        const label =
          stash.sourceDir === "/"
            ? "Root"
            : stash.sourceDir.split("/").filter(Boolean).pop() || "Root";
        get().navigateTab(paneIndex, tab.id, stash.sourceDir, label);
        get().setSelection(pane.id, [...stash.paths]);
      },

      discardStash: (id) =>
        set((s) => ({
          selectionStashes: (s.selectionStashes ?? []).filter((entry) => entry.id !== id),
        })),

      clearStashes: () => set({ selectionStashes: [] }),

      setActiveTab: (paneIndex, tabId) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          pane.activeTabId = tabId;
          panes[paneIndex] = pane;
          return { panes };
        }),

      pinTab: (paneIndex, tabId) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          pane.tabs = pane.tabs.map((t) =>
            t.id === tabId ? { ...t, pinned: true } : t,
          );
          panes[paneIndex] = pane;
          return { panes };
        }),

      unpinTab: (paneIndex, tabId) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          pane.tabs = pane.tabs.map((t) =>
            t.id === tabId ? { ...t, pinned: false } : t,
          );
          panes[paneIndex] = pane;
          return { panes };
        }),

      reorderTabs: (paneIndex, fromIndex, toIndex) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          const tabs = [...pane.tabs];
          const [moved] = tabs.splice(fromIndex, 1);
          tabs.splice(toIndex, 0, moved);
          pane.tabs = tabs;
          panes[paneIndex] = pane;
          return { panes };
        }),

      // Iter 20 — tab context menu. Close every non-pinned tab in this
      // pane except `keepTabId`. The implementation defers to the
      // canonical `closeTab` action one-by-one so the closed-tab ring
      // gets one snapshot per closed tab (matching what the user would
      // get from clicking the × on each tab individually). This also
      // means the pinned-tab and last-tab safety checks in `closeTab`
      // apply identically — no duplicated invariants here.
      closeOtherTabs: (paneIndex, keepTabId) => {
        const targetIds = get()
          .panes[paneIndex].tabs.filter(
            (t) => t.id !== keepTabId && !t.pinned,
          )
          .map((t) => t.id);
        // Focus the surviving tab BEFORE the closes so the active-tab
        // fallback inside `closeTab` doesn't pick a tab we're about to
        // close. Without this, the rapid-fire close loop would walk
        // through tab activations the user never sees.
        if (targetIds.length > 0) {
          get().setActiveTab(paneIndex, keepTabId);
        }
        for (const id of targetIds) {
          get().closeTab(paneIndex, id);
        }
      },

      // Iter 20 — tab context menu. Close every non-pinned tab visually
      // to the right of `fromTabId`. Visual order = pinned first, then
      // insertion order, matching the tab-bar render in
      // tab-bar.tsx:127 — sorting here keeps the gesture's "right"
      // direction aligned with what the user sees.
      closeTabsToRight: (paneIndex, fromTabId) => {
        const tabs = get().panes[paneIndex].tabs;
        // Snapshot the visual order at decision time.
        const sorted = [...tabs].sort((a, b) => {
          if (a.pinned && !b.pinned) return -1;
          if (!a.pinned && b.pinned) return 1;
          return 0;
        });
        const fromIdx = sorted.findIndex((t) => t.id === fromTabId);
        if (fromIdx < 0) return;
        const targetIds = sorted
          .slice(fromIdx + 1)
          .filter((t) => !t.pinned)
          .map((t) => t.id);
        // Focus the anchor tab so the active-tab fallback doesn't
        // briefly land on one of the doomed tabs.
        if (targetIds.length > 0) {
          get().setActiveTab(paneIndex, fromTabId);
        }
        for (const id of targetIds) {
          get().closeTab(paneIndex, id);
        }
      },

      // Iter 20 — tab context menu. Duplicate a specific tab. Mirrors
      // `duplicateActiveTab` (which only handles the active tab) but
      // accepts an explicit `tabId` so the right-click target — which
      // may not be the active tab — is what gets duplicated. Reuses
      // `addTab` so the new tab gets its own fresh `pathHistory` /
      // `historyIndex`, identical to every other tab creation path.
      duplicateTab: (paneIndex, tabId) => {
        const sourcePane = get().panes[paneIndex];
        const sourceTab = sourcePane.tabs.find((t) => t.id === tabId);
        if (!sourceTab) return null;
        get().addTab(paneIndex, sourceTab.path, sourceTab.label);
        return { path: sourceTab.path, label: sourceTab.label };
      },

      // Iter 20 — tab context menu. Move a specific tab to the other
      // pane. Pre-checks: the source pane must have at least 2 tabs
      // (otherwise the move would leave it empty and the active-tab
      // fallback would have nothing to land on). If the app is in
      // single-pane mode we reveal the second pane first — same
      // pattern `openInOtherPane` uses for the active-tab equivalent.
      // The receiving pane becomes active so the user immediately
      // operates where the tab landed.
      moveTabToOtherPane: (paneIndex, tabId) => {
        const state = get();
        const sourcePane = state.panes[paneIndex];
        if (sourcePane.tabs.length <= 1) return null;
        const sourceTab = sourcePane.tabs.find((t) => t.id === tabId);
        if (!sourceTab) return null;
        // Pinned tabs survive in the source pane; moving them out would
        // contradict the pin contract. Treat as a no-op so the menu
        // entry can be disabled at the UI layer based on the same
        // condition.
        if (sourceTab.pinned) return null;
        const otherIndex: 0 | 1 = paneIndex === 0 ? 1 : 0;
        if (state.singlePaneMode) {
          set({ singlePaneMode: false });
        }
        // Open in the other pane first (the addTab path focuses the
        // new tab), then close in the source pane. Order matters:
        // closing first would activate a different tab in the source
        // and the user would see one extra navigation flash before the
        // move completes.
        get().addTab(otherIndex, sourceTab.path, sourceTab.label);
        get().closeTab(paneIndex, tabId);
        set({ activePaneIndex: otherIndex });
        return { otherIndex, path: sourceTab.path, label: sourceTab.label };
      },

      navigateTab: (paneIndex, tabId, path, label) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          pane.tabs = pane.tabs.map((t) => {
            if (t.id !== tabId) return t;
            // Push onto history stack, truncating forward history
            const history = t.pathHistory?.length ? t.pathHistory : [t.path];
            const idx = t.historyIndex ?? history.length - 1;
            const newHistory = [...history.slice(0, idx + 1), path].slice(-MAX_HISTORY_ENTRIES);
            return { ...t, path, label, pathHistory: newHistory, historyIndex: newHistory.length - 1 };
          });
          panes[paneIndex] = pane;

          // Sync browsing: also navigate the other pane's active tab
          if (s.syncBrowsing && !s.singlePaneMode) {
            const otherIndex = paneIndex === 0 ? 1 : 0;
            const otherPane = { ...panes[otherIndex] };
            const otherActiveTab = otherPane.tabs.find((t) => t.id === otherPane.activeTabId);
            if (otherActiveTab) {
              otherPane.tabs = otherPane.tabs.map((t) => {
                if (t.id !== otherActiveTab.id) return t;
                const history = t.pathHistory?.length ? t.pathHistory : [t.path];
                const idx = t.historyIndex ?? history.length - 1;
                const newHistory = [...history.slice(0, idx + 1), path].slice(-MAX_HISTORY_ENTRIES);
                return { ...t, path, label, pathHistory: newHistory, historyIndex: newHistory.length - 1 };
              });
              panes[otherIndex] = otherPane;
            }
          }

          return { panes };
        }),

      // ── View ──

      setViewMode: (paneIndex, mode) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          pane.viewMode = mode;
          panes[paneIndex] = pane;
          return { panes };
        }),

      setSorting: (paneIndex, sortBy, sortAsc) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          pane.sortBy = sortBy;
          pane.sortAsc = sortAsc;
          panes[paneIndex] = pane;
          return { panes };
        }),

      setFilterText: (paneIndex, text) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          pane.filterText = text;
          panes[paneIndex] = pane;
          return { panes };
        }),

      setGroupBy: (paneIndex, groupBy) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          pane.groupBy = groupBy;
          panes[paneIndex] = pane;
          return { panes };
        }),

      // ── Navigation ──

      addFavorite: (item) =>
        set((s) => ({ favorites: [...s.favorites, item] })),

      removeFavorite: (id) =>
        set((s) => ({ favorites: s.favorites.filter((f) => f.id !== id) })),

      toggleFavoriteForCurrentDir: () => {
        const state = get();
        const pane = state.panes[state.activePaneIndex];
        const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
        if (!tab) return null;
        const path = tab.path;
        const existing = state.favorites.find((f) => f.path === path);
        if (existing) {
          set((s) => ({
            favorites: s.favorites.filter((f) => f.id !== existing.id),
          }));
          return { added: false, item: existing };
        }
        const name =
          path === "/" ? "Root" : path.split("/").filter(Boolean).pop() || path;
        const item: FavoriteItem = { id: `fav-${genId()}`, name, path };
        set((s) => ({ favorites: [...s.favorites, item] }));
        return { added: true, item };
      },

      jumpToFavoriteByIndex: (paneIndex, index) => {
        const state = get();
        const fav = state.favorites[index];
        if (!fav) return false;
        const pane = state.panes[paneIndex];
        const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
        if (!tab) return false;
        get().navigateTab(paneIndex, tab.id, fav.path, fav.name);
        return true;
      },

      collectClipboardPaths: () => {
        const state = get();
        const pane = state.panes[state.activePaneIndex];
        const selection = state.selectedPaths[pane.id] ?? [];
        if (selection.length > 0) {
          return { paths: [...selection], source: "selection" };
        }
        // Graceful fallback: when nothing is selected, the user is
        // almost always asking for "the path of the folder I'm
        // looking at". Return it as a one-element array so the
        // caller can join with "\n" uniformly in both branches.
        const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
        const dirPath = tab?.path ?? "";
        return { paths: dirPath ? [dirPath] : [], source: "directory" };
      },

      duplicateActiveTab: (paneIndex) => {
        const state = get();
        const sourcePane = state.panes[paneIndex];
        const sourceTab = sourcePane.tabs.find(
          (t) => t.id === sourcePane.activeTabId,
        );
        if (!sourceTab) return null;
        // Reuse the canonical `addTab` write site so the duplicate
        // gets its own fresh `pathHistory` / `historyIndex` exactly
        // the way every other tab is born — single source of truth,
        // no parallel history bookkeeping. `addTab` already focuses
        // the newly-created tab, so we don't need to touch
        // `activeTabId` ourselves.
        get().addTab(paneIndex, sourceTab.path, sourceTab.label);
        return {
          path: sourceTab.path,
          label: sourceTab.label,
        };
      },

      mirrorViewToOtherPane: (paneIndex) => {
        const state = get();
        const sourcePane = state.panes[paneIndex];
        const otherIndex: 0 | 1 = paneIndex === 0 ? 1 : 0;
        const targetPane = state.panes[otherIndex];
        // Defensive equality check: when both panes already match,
        // there is nothing to do AND nothing to tell the user about.
        // Returning null here prevents a useless toast on a wasted
        // keystroke and keeps the action idempotent.
        if (
          sourcePane.viewMode === targetPane.viewMode &&
          sourcePane.sortBy === targetPane.sortBy &&
          sourcePane.sortAsc === targetPane.sortAsc &&
          sourcePane.groupBy === targetPane.groupBy
        ) {
          return null;
        }
        if (state.singlePaneMode) {
          set({ singlePaneMode: false });
        }
        // Reuse the same per-field setters every other code path
        // uses (sort menu, view toggle, group selector). Each setter
        // is the single write site for its field, so going through
        // them keeps the view pipeline DRY and any future side
        // effects (e.g. analytics, persist, derived caches) fire
        // exactly the same way as a manual user click would.
        get().setViewMode(otherIndex, sourcePane.viewMode);
        get().setSorting(otherIndex, sourcePane.sortBy, sourcePane.sortAsc);
        get().setGroupBy(otherIndex, sourcePane.groupBy);
        return {
          otherIndex,
          viewMode: sourcePane.viewMode,
          sortBy: sourcePane.sortBy,
          sortAsc: sourcePane.sortAsc,
          groupBy: sourcePane.groupBy,
        };
      },

      echoFilterToOtherPane: (paneIndex) => {
        const state = get();
        const sourcePane = state.panes[paneIndex];
        const text = sourcePane.filterText;
        // Echoing an empty filter is meaningless and would silently
        // wipe whatever the other pane is currently filtering for.
        // Bail out so the keyboard handler can swallow the gesture
        // without surprising the user.
        if (!text) return null;
        const otherIndex: 0 | 1 = paneIndex === 0 ? 1 : 0;
        if (state.singlePaneMode) {
          set({ singlePaneMode: false });
        }
        // Reuse the same `setFilterText` action every other code
        // path uses (typing, the X button, content-search clear) so
        // the filter pipeline has exactly one write site — DRY.
        get().setFilterText(otherIndex, text);
        return { text, otherIndex };
      },

      swapPanes: () => {
        const before = get();
        if (before.singlePaneMode) {
          set({ singlePaneMode: false });
        }
        // The swap itself is a single immutable replacement of the
        // tuple — every per-pane field (tabs, history, sort, filter,
        // viewMode, groupBy, the embedded `id`) travels with its pane
        // because PaneState is the unit of identity here. Selections
        // live one level up in `selectedPaths` keyed by `pane.id`, so
        // they follow the content for free without any extra fixup.
        // No new state, no parallel bookkeeping, no derived caches to
        // invalidate — just a tuple flip.
        set((s) => ({
          panes: [s.panes[1], s.panes[0]],
        }));
        const after = get();
        const activePane = after.panes[after.activePaneIndex];
        const activeTab = activePane.tabs.find(
          (t) => t.id === activePane.activeTabId,
        );
        if (!activeTab) return null;
        return {
          activePath: activeTab.path,
          activeLabel: activeTab.label,
        };
      },

      openInOtherPane: (paneIndex) => {
        const state = get();
        const sourcePane = state.panes[paneIndex];
        const sourceTab = sourcePane.tabs.find(
          (t) => t.id === sourcePane.activeTabId,
        );
        if (!sourceTab) return null;
        const otherIndex: 0 | 1 = paneIndex === 0 ? 1 : 0;
        // Coming out of single-pane mode is part of the gesture: the
        // user is explicitly asking for a side-by-side view of the
        // current location, so revealing the second pane is required
        // for the new tab to be visible at all.
        if (state.singlePaneMode) {
          set({ singlePaneMode: false });
        }
        // Reuse `addTab` so the new tab gets a fresh `pathHistory` and
        // `historyIndex` initialised exactly the way every other tab
        // creation path does — single source of truth, no parallel
        // history bookkeeping to drift over time.
        get().addTab(otherIndex, sourceTab.path, sourceTab.label);
        // Focus the inactive pane so the user immediately operates in
        // the freshly-opened side. Matches the muscle memory of "I want
        // to do something *over there* with this folder".
        set({ activePaneIndex: otherIndex });
        return {
          path: sourceTab.path,
          label: sourceTab.label,
          otherIndex,
        };
      },

      addRecentLocation: (path) =>
        set((s) => {
          const filtered = s.recentLocations.filter((r) => r.path !== path);
          const updated = [{ path, timestamp: Date.now() }, ...filtered];
          return { recentLocations: updated.slice(0, MAX_RECENT_LOCATIONS) };
        }),

      toggleTreeExpanded: (path) =>
        set((s) => {
          const expanded = s.expandedTreePaths.includes(path)
            ? s.expandedTreePaths.filter((p) => p !== path)
            : [...s.expandedTreePaths, path];
          return { expandedTreePaths: expanded };
        }),

      // ── Selection ──

      setSelection: (paneId, paths) =>
        set((s) => ({
          selectedPaths: { ...s.selectedPaths, [paneId]: paths },
        })),

      addToSelection: (paneId, path) =>
        set((s) => {
          const current = s.selectedPaths[paneId] || [];
          if (current.includes(path)) return s;
          return {
            selectedPaths: {
              ...s.selectedPaths,
              [paneId]: [...current, path],
            },
          };
        }),

      removeFromSelection: (paneId, path) =>
        set((s) => {
          const current = s.selectedPaths[paneId] || [];
          return {
            selectedPaths: {
              ...s.selectedPaths,
              [paneId]: current.filter((p) => p !== path),
            },
          };
        }),

      clearSelection: (paneId) =>
        set((s) => ({
          selectedPaths: { ...s.selectedPaths, [paneId]: [] },
        })),

      selectRange: (paneId, paths) =>
        set((s) => ({
          selectedPaths: { ...s.selectedPaths, [paneId]: paths },
        })),

      setPaneStats: (paneId, stats) =>
        set((s) => {
          // Equality short-circuit — six scalar fields, cheap to
          // compare, and lets FilePane re-publish on every render
          // without thrashing the status-bar subscriber.
          const prev = s.paneStats[paneId];
          if (
            prev &&
            prev.totalCount === stats.totalCount &&
            prev.totalBytes === stats.totalBytes &&
            prev.totalHasDir === stats.totalHasDir &&
            prev.selectedCount === stats.selectedCount &&
            prev.selectedBytes === stats.selectedBytes &&
            prev.selectedHasDir === stats.selectedHasDir
          ) {
            return s;
          }
          return { paneStats: { ...s.paneStats, [paneId]: stats } };
        }),

      // ── Search ──

      addSavedSearch: (search) =>
        set((s) => ({ savedSearches: [...s.savedSearches, search] })),

      removeSavedSearch: (id) =>
        set((s) => ({
          savedSearches: s.savedSearches.filter((ss) => ss.id !== id),
        })),

      // ── Undo ──

      pushUndo: (entry) =>
        set((s) => ({
          undoStack: [entry, ...s.undoStack].slice(0, MAX_UNDO_LEVELS),
        })),

      popUndo: () => {
        const state = get();
        if (state.undoStack.length === 0) return undefined;
        const [entry, ...rest] = state.undoStack;
        set({ undoStack: rest });
        return entry;
      },

      // ── Command palette ──

      setCommandPaletteOpen: (open) => set({ commandPaletteOpen: open }),

      toggleCommandPalette: () =>
        set((s) => ({ commandPaletteOpen: !s.commandPaletteOpen })),

      // ── Hidden files ──

      toggleHiddenFiles: () =>
        set((s) => ({ showHiddenFiles: !s.showHiddenFiles })),

      // ── Per-folder view defaults ──

      saveFolderViewDefault: (path, settings) =>
        set((s) => ({
          folderViewDefaults: { ...s.folderViewDefaults, [path]: settings },
        })),

      getFolderViewDefault: (path) => {
        return get().folderViewDefaults[path];
      },

      // ── Workspace management ──

      saveWorkspace: (name) => {
        const state = get();
        const workspace: SavedWorkspace = {
          id: genId(),
          name,
          panes: JSON.parse(JSON.stringify(state.panes)),
          singlePaneMode: state.singlePaneMode,
          paneSplitPercent: state.paneSplitPercent,
          paneOrientation: state.paneOrientation,
          syncBrowsing: state.syncBrowsing,
          savedAt: Date.now(),
        };
        set((s) => ({ workspaces: [...s.workspaces, workspace] }));
        return workspace;
      },

      loadWorkspace: (id) => {
        const state = get();
        const workspace = state.workspaces.find((w) => w.id === id);
        if (!workspace) return;
        set({
          panes: JSON.parse(JSON.stringify(workspace.panes)),
          singlePaneMode: workspace.singlePaneMode,
          paneSplitPercent: workspace.paneSplitPercent,
          paneOrientation: workspace.paneOrientation,
          syncBrowsing: workspace.syncBrowsing,
        });
      },

      deleteWorkspace: (id) => {
        set((s) => ({
          workspaces: s.workspaces.filter((w) => w.id !== id),
        }));
      },

      listWorkspaces: () => {
        return get().workspaces;
      },

      // ── Navigation history ──

      navigateBack: (paneIndex) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
          if (!tab) return s;
          const history = tab.pathHistory?.length ? tab.pathHistory : [tab.path];
          const idx = tab.historyIndex ?? history.length - 1;
          if (idx <= 0) return s;
          const newIdx = idx - 1;
          const newPath = history[newIdx];
          const newLabel = newPath === "/" ? "Root" : newPath.split("/").filter(Boolean).pop() || "Root";
          pane.tabs = pane.tabs.map((t) =>
            t.id === tab.id ? { ...t, path: newPath, label: newLabel, historyIndex: newIdx } : t,
          );
          panes[paneIndex] = pane;
          return { panes };
        }),

      navigateForward: (paneIndex) =>
        set((s) => {
          const panes = [...s.panes] as [PaneState, PaneState];
          const pane = { ...panes[paneIndex] };
          const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
          if (!tab) return s;
          const history = tab.pathHistory?.length ? tab.pathHistory : [tab.path];
          const idx = tab.historyIndex ?? history.length - 1;
          if (idx >= history.length - 1) return s;
          const newIdx = idx + 1;
          const newPath = history[newIdx];
          const newLabel = newPath === "/" ? "Root" : newPath.split("/").filter(Boolean).pop() || "Root";
          pane.tabs = pane.tabs.map((t) =>
            t.id === tab.id ? { ...t, path: newPath, label: newLabel, historyIndex: newIdx } : t,
          );
          panes[paneIndex] = pane;
          return { panes };
        }),

      canNavigateBack: (paneIndex) => {
        const state = get();
        const pane = state.panes[paneIndex];
        const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
        if (!tab) return false;
        const idx = tab.historyIndex ?? 0;
        return idx > 0;
      },

      canNavigateForward: (paneIndex) => {
        const state = get();
        const pane = state.panes[paneIndex];
        const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
        if (!tab) return false;
        const history = tab.pathHistory?.length ? tab.pathHistory : [tab.path];
        const idx = tab.historyIndex ?? history.length - 1;
        return idx < history.length - 1;
      },

      // ── Helpers ──

      getActivePane: () => {
        const state = get();
        return state.panes[state.activePaneIndex];
      },

      getActiveTab: (paneIndex) => {
        const state = get();
        const pane = state.panes[paneIndex];
        return pane.tabs.find((t) => t.id === pane.activeTabId);
      },

      getActivePath: (paneIndex) => {
        const state = get();
        const pane = state.panes[paneIndex];
        const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
        return tab?.path || "/";
      },
    }),
    {
      name: "ufop-file-manager-state",
      partialize: (state) => ({
        panes: state.panes,
        singlePaneMode: state.singlePaneMode,
        paneSplitPercent: state.paneSplitPercent,
        paneOrientation: state.paneOrientation,
        syncBrowsing: state.syncBrowsing,
        favorites: state.favorites,
        recentLocations: state.recentLocations,
        savedSearches: state.savedSearches,
        showHiddenFiles: state.showHiddenFiles,
        folderViewDefaults: state.folderViewDefaults,
        workspaces: state.workspaces,
        doubleClickBehavior: state.doubleClickBehavior,
        editorMappings: state.editorMappings,
        privacySettings: state.privacySettings,
        recentlyClosedTabs: state.recentlyClosedTabs,
        selectionStashes: state.selectionStashes,
      }),
    },
  ),
);
