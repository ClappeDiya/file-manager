import { useEffect } from "react";
import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { ThemeProvider } from "@/components/theme-provider";
import { HomePage } from "@/pages/home";
import { DemoPage } from "@/pages/demo";
import { useI18n } from "@/lib/i18n";
import { initAutoUpdateCheck } from "@/lib/updater";
import { initialWindowPath, isSecondaryWindow, onSharedStateChange, sharedFields } from "@/lib/app-window";
import { FILE_MANAGER_STORAGE_KEY, paneLabel, useFileManagerStore, WINDOW_FIELDS } from "@/stores/file-manager-store";
import { useUIStore } from "@/stores/ui-store";

/** Every window: open at the folder it was opened for, and follow what the other windows change. */
function useAppWindow() {
  useEffect(() => {
    const path = initialWindowPath();
    if (path) {
      const fm = useFileManagerStore.getState();
      const tab = fm.getActiveTab(0);
      if (tab) fm.navigateTab(0, tab.id, path, paneLabel(path));
    }
    const offFiles = onSharedStateChange(FILE_MANAGER_STORAGE_KEY, (stored) => {
      const current = useFileManagerStore.getState() as unknown as Record<string, unknown>;
      const changed = Object.fromEntries(
        Object.entries(sharedFields(stored, WINDOW_FIELDS)).filter(([k, v]) => k in current && JSON.stringify(current[k]) !== JSON.stringify(v)),
      );
      if (Object.keys(changed).length > 0) useFileManagerStore.setState(changed);
    });
    const offUi = onSharedStateChange("ufop-ui-state", () => void useUIStore.persist.rehydrate());
    const offLang = onSharedStateChange("ufop-i18n", (stored) => {
      const pref = stored.preference;
      if (typeof pref === "string" && pref !== useI18n.getState().preference) void useI18n.getState().setPreference(pref as never);
    });
    return () => {
      offFiles();
      offUi();
      offLang();
    };
  }, []);
}

/**
 * Root application component.
 * Sets up routing, theme provider, i18n, auto-update, and global layout.
 */
function App() {
  const { init: initI18n, isInitialized: i18nReady } = useI18n();
  useAppWindow();

  // Initialize i18n (locale detection + translation loading)
  useEffect(() => {
    if (!i18nReady) {
      initI18n();
    }
  }, [initI18n, i18nReady]);

  // Initialize periodic auto-update checks (every 4 hours)
  // (once, in the first window)
  useEffect(() => {
    if (isSecondaryWindow()) return;
    const cleanup = initAutoUpdateCheck();
    return cleanup;
  }, []);

  return (
    <ThemeProvider>
      <BrowserRouter>
        <div className="h-full" role="application" aria-label="Unified File Operations Platform">
          <a
            href="#main-content"
            className="sr-only focus:not-sr-only focus:absolute focus:top-2 focus:left-2 focus:z-50 focus:rounded-md focus:bg-primary focus:px-3 focus:py-2 focus:text-sm focus:text-primary-foreground"
          >
            Skip to main content
          </a>
          <Routes>
            <Route path="/" element={<HomePage />} />
            <Route path="/demo" element={<DemoPage />} />
            {/* Catch-all redirect to home */}
            <Route path="*" element={<Navigate to="/" replace />} />
          </Routes>
        </div>
      </BrowserRouter>
    </ThemeProvider>
  );
}

export default App;
