import { useEffect } from "react";
import { BrowserRouter, Routes, Route, Navigate } from "react-router-dom";
import { ThemeProvider } from "@/components/theme-provider";
import { HomePage } from "@/pages/home";
import { DemoPage } from "@/pages/demo";
import { useI18n } from "@/lib/i18n";
import { initAutoUpdateCheck } from "@/lib/updater";

/**
 * Root application component.
 * Sets up routing, theme provider, i18n, auto-update, and global layout.
 */
function App() {
  const { init: initI18n, isInitialized: i18nReady } = useI18n();

  // Initialize i18n (locale detection + translation loading)
  useEffect(() => {
    if (!i18nReady) {
      initI18n();
    }
  }, [initI18n, i18nReady]);

  // Initialize periodic auto-update checks (every 4 hours)
  useEffect(() => {
    const cleanup = initAutoUpdateCheck();
    return cleanup;
  }, []);

  return (
    <ThemeProvider>
      <BrowserRouter>
        <div className="h-full" role="application" aria-label="Unified File Operations Platform">
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
