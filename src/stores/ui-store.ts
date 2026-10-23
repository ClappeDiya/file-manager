import { create } from "zustand";
import { persist } from "zustand/middleware";

/**
 * Theme options:
 * - "light": Force light theme
 * - "dark": Force dark theme
 * - "system": Follow OS preference (default)
 * - "high-contrast": High contrast mode for accessibility
 */
export type Theme = "light" | "dark" | "system" | "high-contrast";

/** Text size multiplier applied to the root font size (rem-based UI scales). */
export type FontScale = 0.875 | 1 | 1.125 | 1.25;

/** Text size choices, smallest first (Settings → Appearance and Ctrl/Cmd +/−/0). */
export const FONT_SCALES: readonly { value: FontScale; label: string }[] = [
  { value: 0.875, label: "Small" },
  { value: 1, label: "Default" },
  { value: 1.125, label: "Large" },
  { value: 1.25, label: "Larger" },
];

/** Rows per page of a sortable table; "all" scrolls one virtualized list. */
export type TablePageSize = 25 | 50 | 100 | 250 | "all";
export const TABLE_PAGE_SIZES: readonly TablePageSize[] = [25, 50, 100, 250, "all"];

/** Remembered view of one table (K122): page size and sort column. */
export interface TableView {
  pageSize: TablePageSize;
  sort: { id: string; desc: boolean } | null;
}

/**
 * Application mode:
 * - "simple": Default mode with simplified UI
 * - "advanced": Power user mode with all features
 */
export type AppMode = "simple" | "advanced";

/**
 * View mode for file listings
 */
export type ViewMode = "list" | "grid" | "details" | "columns";

/**
 * User style chosen during onboarding
 * - "personal": Personal / casual user
 * - "power": Power user / developer
 * - "work": Workplace / enterprise user
 */
export type UserStyle = "personal" | "power" | "work";

/**
 * Onboarding wizard steps
 */
export type OnboardingStep =
  | "welcome"
  | "choose-style"
  | "connect-locations"
  | "first-action"
  | "explain-compat"
  | "enter-workspace";

/**
 * Activity entry type for the activity feed (T-057)
 */
export type ActivityType =
  | "copy"
  | "move"
  | "rename"
  | "delete"
  | "create_folder"
  | "create_file"
  | "sync"
  | "transfer"
  | "connect"
  | "disconnect"
  | "error"
  | "undo";

export interface ActivityEntry {
  id: string;
  type: ActivityType;
  /** Plain-language description of what happened */
  summary: string;
  /** What the app did automatically (if anything) */
  appAction?: string;
  /** What the user can do next */
  nextStep?: string;
  /** Paths involved */
  paths: string[];
  /** ISO timestamp */
  timestamp: string;
  /** Whether this can be undone */
  undoable: boolean;
}

/**
 * Structured error that follows the content design pattern:
 * what happened, why, what the app did, what the user can do
 */
export interface StructuredError {
  id: string;
  /** What happened - plain language */
  what: string;
  /** Why it happened */
  why: string;
  /** What the app did in response */
  appDid: string;
  /** What the user can do to fix or proceed */
  userAction: string;
  /** Timestamp */
  timestamp: string;
  /** Whether it has been dismissed */
  dismissed: boolean;
}

export interface UIState {
  // Theme
  theme: Theme;
  resolvedTheme: "light" | "dark" | "high-contrast";
  setTheme: (theme: Theme) => void;

  // Text size (K121)
  fontScale: FontScale;
  setFontScale: (scale: FontScale) => void;
  /** One step larger (1), smaller (-1) or back to the default (0). */
  stepFontScale: (step: -1 | 0 | 1) => void;

  // Table views (K122), by table id
  tableViews: Record<string, TableView>;
  setTableView: (id: string, patch: Partial<TableView>) => void;

  // Sidebar
  sidebarOpen: boolean;
  sidebarWidth: number;
  toggleSidebar: () => void;
  setSidebarOpen: (open: boolean) => void;
  setSidebarWidth: (width: number) => void;

  // Application mode
  appMode: AppMode;
  setAppMode: (mode: AppMode) => void;
  /** Per-area override for the Backup screens: true/false forces advanced/simple
   *  backup UI, null follows `appMode`. Read it through `selectBackupAdvanced`. */
  backupAdvanced: boolean | null;
  setBackupAdvanced: (value: boolean | null) => void;
  /** Text editor saves on its own a moment after the last edit. */
  editorAutosave: boolean;
  setEditorAutosave: (on: boolean) => void;

  // View
  viewMode: ViewMode;
  setViewMode: (mode: ViewMode) => void;

  // Status bar
  statusBarVisible: boolean;
  setStatusBarVisible: (visible: boolean) => void;

  // Preview panel
  previewPanelOpen: boolean;
  previewPanelWidth: number;
  /** Iter 29: absolute path of the file currently being previewed.
   *  Paired with `previewPanelOpen` so the modal knows WHICH file
   *  to render. The boolean alone was a pre-existing half-wired
   *  orphan (declared, persisted, tested, but never consumed) \u2014
   *  iter 29 both consumes it and adds the path it was missing. */
  previewFilePath: string | null;
  togglePreviewPanel: () => void;
  setPreviewPanelWidth: (width: number) => void;
  /** Iter 29: open the preview panel on a specific file. Sets
   *  both the open flag AND the file path in one atomic update
   *  so consumers always see a consistent pair. */
  openPreviewFor: (path: string) => void;
  /** Iter 29: close the preview panel and clear the file path.
   *  Separate from `togglePreviewPanel` so callers that know
   *  they want the panel closed don't accidentally re-open it
   *  when it happens to be closed already. */
  closePreview: () => void;

  // ── Onboarding (T-055) ──
  onboardingComplete: boolean;
  onboardingStep: OnboardingStep;
  userStyle: UserStyle;
  setOnboardingComplete: (complete: boolean) => void;
  setOnboardingStep: (step: OnboardingStep) => void;
  setUserStyle: (style: UserStyle) => void;
  resetOnboarding: () => void;

  // ── Activity Feed (T-057) ──
  activityFeed: ActivityEntry[];
  addActivity: (entry: Omit<ActivityEntry, "id" | "timestamp">) => void;
  clearActivity: () => void;

  // ── Structured Errors (T-057) ──
  structuredErrors: StructuredError[];
  addStructuredError: (error: Omit<StructuredError, "id" | "timestamp" | "dismissed">) => void;
  dismissError: (id: string) => void;
  clearErrors: () => void;

  // ── Guided Flow State (T-056) ──
  activeGuidedFlow: string | null;
  setActiveGuidedFlow: (flowId: string | null) => void;

  // ── Toolbar Customization (#14) ──
  toolbarItems: string[];
  setToolbarItems: (items: string[]) => void;
  resetToolbar: () => void;
  toolbarCustomizerOpen: boolean;
  toggleToolbarCustomizer: () => void;

  // Internal: resolve theme based on OS preference
  _resolveTheme: () => void;
}

const MAX_ACTIVITY_ENTRIES = 100;
const MAX_ERRORS = 50;
/** Toolbar items shown by default for new installs. Exported so the toolbar
 *  customizer's "Reset to defaults" button stays in sync with the store. */
export const DEFAULT_TOOLBAR_ITEMS = ["sidebar", "view-modes", "ai", "terminal", "theme"] as const;

/**
 * Applies the resolved theme to the document.
 * This is the core of instant theme switching -- we set data-theme
 * on <html> synchronously so CSS custom properties apply immediately.
 */
function applyThemeToDocument(resolvedTheme: "light" | "dark" | "high-contrast") {
  if (typeof document === "undefined") return;

  const root = document.documentElement;

  if (resolvedTheme === "light") {
    root.setAttribute("data-theme", "light");
    root.style.colorScheme = "light";
  } else if (resolvedTheme === "dark") {
    root.setAttribute("data-theme", "dark");
    root.style.colorScheme = "dark";
  } else {
    root.setAttribute("data-theme", "high-contrast");
    root.style.colorScheme = "light";
  }
}

/** Sets the root font size so every rem-based size follows the text size. */
export function applyFontScaleToDocument(scale: FontScale) {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (scale === 1) root.style.removeProperty("--ufop-font-scale");
  else root.style.setProperty("--ufop-font-scale", String(scale));
}

/** The text size one step from `current` (clamped to the available sizes). */
export function nextFontScale(current: FontScale, step: -1 | 0 | 1): FontScale {
  if (step === 0) return 1;
  const i = FONT_SCALES.findIndex((f) => f.value === current);
  const j = Math.max(0, Math.min(FONT_SCALES.length - 1, (i < 0 ? 1 : i) + step));
  return FONT_SCALES[j].value;
}

const isFontScale = (v: unknown): v is FontScale => FONT_SCALES.some((f) => f.value === v);

const isTableView = (v: unknown): v is TableView => {
  if (!v || typeof v !== "object") return false;
  const t = v as TableView;
  const sortOk = t.sort === null || (typeof t.sort === "object" && typeof t.sort.id === "string" && typeof t.sort.desc === "boolean");
  return TABLE_PAGE_SIZES.includes(t.pageSize) && sortOk;
};

/**
 * Get the OS preferred color scheme.
 */
function getSystemTheme(): "light" | "dark" {
  if (typeof window === "undefined") return "light";
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

/**
 * Resolve a Theme setting to an actual theme.
 */
function resolveTheme(theme: Theme): "light" | "dark" | "high-contrast" {
  if (theme === "system") {
    return getSystemTheme();
  }
  return theme;
}

/** Whether the Backup screens show their advanced (expert) sections: the
 *  per-area override wins, otherwise the global app mode decides. */
export function selectBackupAdvanced(state: Pick<UIState, "appMode" | "backupAdvanced">): boolean {
  return state.backupAdvanced ?? state.appMode === "advanced";
}

/** Persisted-state schema version. v0 (no version) predates `backupAdvanced`;
 *  v1 predates the appearance settings (text size, units, table views). */
export const UI_STATE_VERSION = 2;

/**
 * Upgrades persisted UI state from an older version. Installs persisted before
 * v1 always saw every backup option, so they get `backupAdvanced: true` to keep
 * that experience; brand-new installs have no persisted state and keep the
 * default `null` (follow `appMode`).
 */
export function migrateUIState(persisted: unknown, version: number): Partial<UIState> {
  let state = (persisted && typeof persisted === "object" ? persisted : {}) as Partial<UIState>;
  if (version < 1 && !("backupAdvanced" in state)) {
    state = { ...state, backupAdvanced: true };
  }
  return sanitizeAppearance(state);
}

/** Drops unknown appearance values (hand-edited or from a newer build) so
 *  the defaults apply instead of a broken layout. Fields that are absent stay
 *  absent. */
function sanitizeAppearance(state: Partial<UIState>): Partial<UIState> {
  const out = { ...state };
  if ("fontScale" in out && !isFontScale(out.fontScale)) delete out.fontScale;
  if ("tableViews" in out) {
    const views = out.tableViews && typeof out.tableViews === "object" ? out.tableViews : {};
    out.tableViews = Object.fromEntries(Object.entries(views).filter(([, v]) => isTableView(v)));
  }
  return out;
}

let _activityIdCounter = 0;
let _errorIdCounter = 0;

export const useUIStore = create<UIState>()(
  persist(
    (set, get) => ({
      // Theme - defaults
      theme: "system",
      resolvedTheme: resolveTheme("system"),

      setTheme: (theme: Theme) => {
        const resolved = resolveTheme(theme);
        applyThemeToDocument(resolved);
        set({ theme, resolvedTheme: resolved });
      },

      // Text size
      fontScale: 1,
      setFontScale: (scale: FontScale) => {
        const next = isFontScale(scale) ? scale : 1;
        applyFontScaleToDocument(next);
        set({ fontScale: next });
      },
      stepFontScale: (step) => get().setFontScale(nextFontScale(get().fontScale, step)),

      // Table views
      tableViews: {},
      setTableView: (id, patch) =>
        set((state) => {
          const prev = state.tableViews[id] ?? { pageSize: 50, sort: null };
          return { tableViews: { ...state.tableViews, [id]: { ...prev, ...patch } } };
        }),

      // Sidebar
      sidebarOpen: true,
      sidebarWidth: 256,
      toggleSidebar: () =>
        set((state) => ({ sidebarOpen: !state.sidebarOpen })),
      setSidebarOpen: (open: boolean) => set({ sidebarOpen: open }),
      setSidebarWidth: (width: number) =>
        set({ sidebarWidth: Math.max(180, Math.min(480, width)) }),

      // Application mode - Simple is default per architecture rules
      appMode: "simple",
      setAppMode: (mode: AppMode) => set({ appMode: mode }),
      backupAdvanced: null,
      setBackupAdvanced: (value: boolean | null) => set({ backupAdvanced: value }),
      editorAutosave: false,
      setEditorAutosave: (on: boolean) => set({ editorAutosave: on }),

      // View
      viewMode: "list",
      setViewMode: (mode: ViewMode) => set({ viewMode: mode }),

      // Status bar
      statusBarVisible: true,
      setStatusBarVisible: (visible: boolean) =>
        set({ statusBarVisible: visible }),

      // Preview panel
      previewPanelOpen: false,
      previewPanelWidth: 320,
      previewFilePath: null,
      togglePreviewPanel: () =>
        set((state) => ({ previewPanelOpen: !state.previewPanelOpen })),
      setPreviewPanelWidth: (width: number) =>
        set({ previewPanelWidth: Math.max(200, Math.min(600, width)) }),
      openPreviewFor: (path: string) =>
        set({ previewPanelOpen: true, previewFilePath: path }),
      closePreview: () =>
        set({ previewPanelOpen: false, previewFilePath: null }),

      // ── Onboarding (T-055) ──
      onboardingComplete: false,
      onboardingStep: "welcome",
      userStyle: "personal",

      setOnboardingComplete: (complete: boolean) =>
        set({ onboardingComplete: complete }),

      setOnboardingStep: (step: OnboardingStep) =>
        set({ onboardingStep: step }),

      setUserStyle: (style: UserStyle) => {
        set({ userStyle: style });
        // Auto-set app mode based on style
        if (style === "power") {
          set({ appMode: "advanced" });
        } else {
          set({ appMode: "simple" });
        }
      },

      resetOnboarding: () =>
        set({
          onboardingComplete: false,
          onboardingStep: "welcome",
        }),

      // ── Activity Feed (T-057) ──
      activityFeed: [],

      addActivity: (entry) =>
        set((state) => {
          const newEntry: ActivityEntry = {
            ...entry,
            id: `activity-${Date.now()}-${++_activityIdCounter}`,
            timestamp: new Date().toISOString(),
          };
          const updated = [newEntry, ...state.activityFeed].slice(
            0,
            MAX_ACTIVITY_ENTRIES,
          );
          return { activityFeed: updated };
        }),

      clearActivity: () => set({ activityFeed: [] }),

      // ── Structured Errors (T-057) ──
      structuredErrors: [],

      addStructuredError: (error) =>
        set((state) => {
          const newError: StructuredError = {
            ...error,
            id: `error-${Date.now()}-${++_errorIdCounter}`,
            timestamp: new Date().toISOString(),
            dismissed: false,
          };
          const updated = [newError, ...state.structuredErrors].slice(
            0,
            MAX_ERRORS,
          );
          return { structuredErrors: updated };
        }),

      dismissError: (id: string) =>
        set((state) => ({
          structuredErrors: state.structuredErrors.map((e) =>
            e.id === id ? { ...e, dismissed: true } : e,
          ),
        })),

      clearErrors: () => set({ structuredErrors: [] }),

      // ── Guided Flow State (T-056) ──
      activeGuidedFlow: null,
      setActiveGuidedFlow: (flowId: string | null) =>
        set({ activeGuidedFlow: flowId }),

      // ── Toolbar Customization (#14) ──
      toolbarItems: [...DEFAULT_TOOLBAR_ITEMS],
      setToolbarItems: (items: string[]) => set({ toolbarItems: items }),
      resetToolbar: () => set({ toolbarItems: [...DEFAULT_TOOLBAR_ITEMS] }),
      toolbarCustomizerOpen: false,
      toggleToolbarCustomizer: () =>
        set((state) => ({ toolbarCustomizerOpen: !state.toolbarCustomizerOpen })),

      // Resolve theme from system preference
      _resolveTheme: () => {
        const { theme } = get();
        const resolved = resolveTheme(theme);
        applyThemeToDocument(resolved);
        set({ resolvedTheme: resolved });
      },
    }),
    {
      name: "ufop-ui-state",
      version: UI_STATE_VERSION,
      migrate: migrateUIState,
      // Only persist these specific fields to localStorage
      partialize: (state) => ({
        theme: state.theme,
        fontScale: state.fontScale,
        tableViews: state.tableViews,
        sidebarOpen: state.sidebarOpen,
        sidebarWidth: state.sidebarWidth,
        appMode: state.appMode,
        backupAdvanced: state.backupAdvanced,
        viewMode: state.viewMode,
        statusBarVisible: state.statusBarVisible,
        previewPanelOpen: state.previewPanelOpen,
        previewPanelWidth: state.previewPanelWidth,
        previewFilePath: state.previewFilePath,
        onboardingComplete: state.onboardingComplete,
        userStyle: state.userStyle,
        activityFeed: state.activityFeed,
        toolbarItems: state.toolbarItems,
        editorAutosave: state.editorAutosave,
      }),
      onRehydrateStorage: () => {
        return (state) => {
          if (state) {
            // Apply theme immediately after rehydration to prevent flash
            const resolved = resolveTheme(state.theme);
            applyThemeToDocument(resolved);
            state.resolvedTheme = resolved;
            applyFontScaleToDocument(isFontScale(state.fontScale) ? state.fontScale : 1);
          }
        };
      },
    },
  ),
);

// Listen for system theme changes when theme is set to "system"
if (typeof window !== "undefined") {
  const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
  mediaQuery.addEventListener("change", () => {
    const state = useUIStore.getState();
    if (state.theme === "system") {
      state._resolveTheme();
    }
  });
}
