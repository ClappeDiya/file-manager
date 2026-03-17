import React from "react";
import ReactDOM from "react-dom/client";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import "./styles/globals.css";

// ---------------------------------------------------------------------------
// Error Boundary – catches render-time errors and displays them on screen
// ---------------------------------------------------------------------------
class RootErrorBoundary extends React.Component<
  { children: React.ReactNode },
  { error: Error | null }
> {
  constructor(props: { children: React.ReactNode }) {
    super(props);
    this.state = { error: null };
  }
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("[RootErrorBoundary]", error, info.componentStack);
  }
  render() {
    if (this.state.error) {
      return React.createElement(
        "div",
        {
          style: {
            padding: 32,
            fontFamily: "monospace",
            background: "#fef2f2",
            color: "#991b1b",
            height: "100%",
            overflow: "auto",
          },
        },
        React.createElement("h1", null, "⚠️  Application Error"),
        React.createElement(
          "pre",
          { style: { whiteSpace: "pre-wrap", fontSize: 13, marginTop: 12 } },
          String(this.state.error),
          "\n\n",
          this.state.error?.stack,
        ),
      );
    }
    return this.props.children;
  }
}

// Apply theme from localStorage BEFORE React renders to prevent flash
(function applyInitialTheme() {
  try {
    const stored = localStorage.getItem("ufop-ui-state");
    if (stored) {
      const parsed = JSON.parse(stored);
      const theme = parsed?.state?.theme;
      if (theme === "dark") {
        document.documentElement.setAttribute("data-theme", "dark");
        document.documentElement.style.colorScheme = "dark";
      } else if (theme === "high-contrast") {
        document.documentElement.setAttribute("data-theme", "high-contrast");
        document.documentElement.style.colorScheme = "light";
      } else if (theme === "light") {
        document.documentElement.setAttribute("data-theme", "light");
        document.documentElement.style.colorScheme = "light";
      }
      // "system" theme = no data-theme attribute, CSS handles it via media query
    }
  } catch {
    // Silently fail -- CSS defaults will apply
  }
})();

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      retry: 1,
      staleTime: 5 * 60 * 1000,
      refetchOnWindowFocus: false,
    },
  },
});

// Dynamically import the app to catch module-evaluation errors
async function bootstrap() {
  try {
    const mod = await import(/* @vite-ignore */ "./App");
    const App = mod.default;

    ReactDOM.createRoot(document.getElementById("root")!).render(
      <React.StrictMode>
        <RootErrorBoundary>
          <QueryClientProvider client={queryClient}>
            <App />
          </QueryClientProvider>
        </RootErrorBoundary>
      </React.StrictMode>,
    );
  } catch (err: any) {
    console.error("Application failed to initialize:", err);
    try {
      const root = document.getElementById("root");
      if (root) {
        root.innerHTML = `
          <div style="padding:20px;font-family:system-ui,Segoe UI,Roboto,Helvetica,Arial;">
            <h2 style="color:#b91c1c">Application failed to initialize</h2>
            <pre style="white-space:pre-wrap;color:#111;background:#fee; padding:12px;border-radius:6px;">${String(err && err.stack ? err.stack : err)}</pre>
            <p>Please check the browser console for more details.</p>
          </div>
        `;
      }
    } catch (_e) {
      // ignore
    }
  }
}

window.addEventListener("error", (e) => {
  try {
    console.error("Global error:", e.error || e.message || e);
  } catch {}
});
window.addEventListener("unhandledrejection", (e) => {
  try {
    console.error("Unhandled rejection:", e.reason || e);
  } catch {}
});

bootstrap();
