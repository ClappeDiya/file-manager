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

  // Sidebar
  sidebarOpen: boolean;
  sidebarWidth: number;
  toggleSidebar: () => void;
  setSidebarOpen: (open: boolean) => void;
  setSidebarWidth: (width: number) => void;

  // Application mode
  appMode: AppMode;
  setAppMode: (mode: AppMode) => void;

  // View
  viewMode: ViewMode;
  setViewMode: (mode: ViewMode) => void;

  // Status bar
  statusBarVisible: boolean;
  setStatusBarVisible: (visible: boolean) => void;

  // Preview panel
  previewPanelOpen: boolean;
  previewPanelWidth: number;
  togglePreviewPanel: () => void;
  setPreviewPanelWidth: (width: number) => void;

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
  toolbarCustomizerOpen: boolean;
  toggleToolbarCustomizer: () => void;

  // Internal: resolve theme based on OS preference
  _resolveTheme: () => void;
}

const MAX_ACTIVITY_ENTRIES = 100;
const MAX_ERRORS = 50;

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
      togglePreviewPanel: () =>
        set((state) => ({ previewPanelOpen: !state.previewPanelOpen })),
      setPreviewPanelWidth: (width: number) =>
        set({ previewPanelWidth: Math.max(200, Math.min(600, width)) }),

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
      toolbarItems: ["sidebar", "view-modes", "ai", "terminal", "theme"],
      setToolbarItems: (items: string[]) => set({ toolbarItems: items }),
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
      // Only persist these specific fields to localStorage
      partialize: (state) => ({
        theme: state.theme,
        sidebarOpen: state.sidebarOpen,
        sidebarWidth: state.sidebarWidth,
        appMode: state.appMode,
        viewMode: state.viewMode,
        statusBarVisible: state.statusBarVisible,
        previewPanelOpen: state.previewPanelOpen,
        previewPanelWidth: state.previewPanelWidth,
        onboardingComplete: state.onboardingComplete,
        userStyle: state.userStyle,
        activityFeed: state.activityFeed,
        toolbarItems: state.toolbarItems,
      }),
      onRehydrateStorage: () => {
        return (state) => {
          if (state) {
            // Apply theme immediately after rehydration to prevent flash
            const resolved = resolveTheme(state.theme);
            applyThemeToDocument(resolved);
            state.resolvedTheme = resolved;
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
