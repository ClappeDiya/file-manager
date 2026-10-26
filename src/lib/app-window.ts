/**
 * Several app windows (File → New Window, "Open in New Window").
 *
 * The first window is `main`; the others are opened by `open_new_window`
 * with `?window=<label>&path=<folder>`. All windows share the persisted
 * settings and lists (favorites, recent places, saved searches,
 * workspaces), but each has its own panes, tabs and layout: a second
 * window's layout is not saved over the first one's.
 *
 * `localStorage` is shared, so a change made in one window is written there;
 * the others pick it up from the `storage` event (and on focus, should a
 * webview not send it).
 */
import type { StateStorage } from "zustand/middleware";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";

const params = () => new URLSearchParams(typeof window === "undefined" ? "" : window.location.search);

/** This window's label (`main` for the first window). */
export function windowLabel(): string {
  return params().get("window") || "main";
}

export function isSecondaryWindow(): boolean {
  return windowLabel() !== "main";
}

/** The folder this window was opened at, if any. */
export function initialWindowPath(): string | null {
  return params().get("path") || null;
}

/** Open another window, at `path` when given. */
export async function openNewWindow(path?: string | null): Promise<void> {
  if (isTauriAvailable()) {
    await tauriInvoke<string>("open_new_window", { path: path ?? null });
    return;
  }
  const url = new URL(window.location.href);
  url.search = "";
  url.searchParams.set("window", `win-${Date.now().toString(36)}`);
  if (path) url.searchParams.set("path", path);
  window.open(url.toString(), "_blank", "noopener");
}

/**
 * Storage for a store whose `windowFields` belong to one window: they are
 * neither read from nor written to the shared copy, every other field is.
 */
export function windowScopedStorage(base: Storage, windowFields: readonly string[]): StateStorage {
  const strip = (state: Record<string, unknown>) => {
    const out = { ...state };
    for (const f of windowFields) delete out[f];
    return out;
  };
  return {
    getItem: (name) => {
      const raw = base.getItem(name);
      if (!raw) return raw;
      try {
        const parsed = JSON.parse(raw) as { state?: Record<string, unknown>; version?: number };
        return JSON.stringify({ ...parsed, state: strip(parsed.state ?? {}) });
      } catch {
        return null;
      }
    },
    setItem: (name, value) => {
      try {
        const next = JSON.parse(value) as { state?: Record<string, unknown>; version?: number };
        const cur = JSON.parse(base.getItem(name) ?? "{}") as { state?: Record<string, unknown> };
        const merged = { ...next, state: { ...(cur.state ?? {}), ...strip(next.state ?? {}) } };
        // Keep the shared copy's own window fields (the main window's panes).
        for (const f of windowFields) {
          if (cur.state && f in cur.state) merged.state[f] = cur.state[f];
          else delete merged.state[f];
        }
        base.setItem(name, JSON.stringify(merged));
      } catch {
        // A broken shared copy: leave it for the main window to rewrite.
      }
    },
    removeItem: () => {
      // A secondary window never removes the shared copy.
    },
  };
}

/** The fields of `state` that are not this window's own. */
export function sharedFields(state: Record<string, unknown>, windowFields: readonly string[]): Record<string, unknown> {
  const out: Record<string, unknown> = {};
  for (const [k, v] of Object.entries(state)) if (!windowFields.includes(k)) out[k] = v;
  return out;
}

/**
 * When another window changes a stored key: `apply` gets the new persisted
 * state. Also re-reads on focus. Returns the unsubscribe function.
 */
export function onSharedStateChange(key: string, apply: (state: Record<string, unknown>) => void): () => void {
  if (typeof window === "undefined") return () => {};
  const read = (raw: string | null) => {
    if (!raw) return;
    try {
      const parsed = JSON.parse(raw) as { state?: Record<string, unknown> };
      if (parsed.state) apply(parsed.state);
    } catch {
      // ignore a half-written value; the next event has the whole one
    }
  };
  const onStorage = (e: StorageEvent) => {
    if (e.key === key) read(e.newValue);
  };
  const onFocus = () => {
    try {
      read(window.localStorage.getItem(key));
    } catch {
      // storage unavailable
    }
  };
  window.addEventListener("storage", onStorage);
  window.addEventListener("focus", onFocus);
  return () => {
    window.removeEventListener("storage", onStorage);
    window.removeEventListener("focus", onFocus);
  };
}
