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

export interface FavoriteItem {
  id: string;
  name: string;
  path: string;
  icon?: string;
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

  // Actions - View
  setViewMode: (paneIndex: 0 | 1, mode: ViewMode) => void;
  setSorting: (paneIndex: 0 | 1, sortBy: string, sortAsc: boolean) => void;
  setFilterText: (paneIndex: 0 | 1, text: string) => void;
  setGroupBy: (paneIndex: 0 | 1, groupBy: string | null) => void;

  // Actions - Navigation
  addFavorite: (item: FavoriteItem) => void;
  removeFavorite: (id: string) => void;
  addRecentLocation: (path: string) => void;
  toggleTreeExpanded: (path: string) => void;

  // Actions - Selection
  setSelection: (paneId: string, paths: string[]) => void;
  addToSelection: (paneId: string, path: string) => void;
  removeFromSelection: (paneId: string, path: string) => void;
  clearSelection: (paneId: string) => void;
  selectRange: (paneId: string, paths: string[]) => void;

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

          pane.tabs = pane.tabs.filter((t) => t.id !== tabId);
          if (pane.activeTabId === tabId) {
            pane.activeTabId = pane.tabs[pane.tabs.length - 1].id;
          }
          panes[paneIndex] = pane;
          return { panes };
        }),

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
      }),
    },
  ),
);
