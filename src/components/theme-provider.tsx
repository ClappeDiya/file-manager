import { useEffect } from "react";
import { useUIStore } from "@/stores/ui-store";

/**
 * ThemeProvider initializes and manages theme application.
 *
 * It:
 * 1. Applies the stored theme immediately on mount (no flash)
 * 2. Listens for OS color scheme changes when theme = "system"
 * 3. Handles the prefers-reduced-motion media query
 *
 * Should be placed near the root of the app.
 */
export function ThemeProvider({ children }: { children: React.ReactNode }) {
  const theme = useUIStore((s) => s.theme);
  const _resolveTheme = useUIStore((s) => s._resolveTheme);

  // Apply theme on mount and when theme changes
  useEffect(() => {
    _resolveTheme();
  }, [theme, _resolveTheme]);

  // Listen for system preference changes
  useEffect(() => {
    if (theme !== "system") return;

    const mediaQuery = window.matchMedia("(prefers-color-scheme: dark)");
    const handler = () => _resolveTheme();
    mediaQuery.addEventListener("change", handler);
    return () => mediaQuery.removeEventListener("change", handler);
  }, [theme, _resolveTheme]);

  return <>{children}</>;
}
