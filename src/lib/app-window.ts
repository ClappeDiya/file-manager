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
 * Storage shared by the windows. `windowFields` belong to one window: the
 * first window stores its own (they reopen next start), another window keeps
 * them to itself (`secondary`). Every other field is shared, and a window
 * writes only the shared fields it changed since it last read them, so a
 * window that has not heard of another's change yet cannot write an old
 * value over it.
 */
export function sharedWindowStorage(base: Storage, windowFields: readonly string[], secondary: boolean): StateStorage {
  // Per stored key: each field as this window last read or wrote it.
  const seen = new Map<string, Map<string, string>>();
  const parse = (raw: string | null) => {
    try {
      return raw ? (JSON.parse(raw) as { state?: Record<string, unknown>; version?: number }) : {};
    } catch {
      return {};
    }
  };
  const remember = (name: string, state: Record<string, unknown>) => {
    seen.set(name, new Map(Object.entries(state).map(([k, v]) => [k, JSON.stringify(v)])));
  };
  return {
    getItem: (name) => {
      const raw = base.getItem(name);
      if (!raw) return raw;
      const parsed = parse(raw);
      const state = { ...(parsed.state ?? {}) };
      if (secondary) for (const f of windowFields) delete state[f];
      remember(name, state);
      return JSON.stringify({ ...parsed, state });
    },
    setItem: (name, value) => {
      const next = parse(value);
      const cur = parse(base.getItem(name)).state ?? {};
      const last = seen.get(name) ?? new Map<string, string>();
      const merged: Record<string, unknown> = { ...cur };
      for (const [k, v] of Object.entries(next.state ?? {})) {
        const own = windowFields.includes(k);
        if (own && secondary) continue; // the first window's stay
        const changedHere = !last.has(k) || last.get(k) !== JSON.stringify(v);
        if (own || changedHere || !(k in cur)) merged[k] = v;
      }
      const mine = { ...merged };
      if (secondary) for (const f of windowFields) delete mine[f];
      remember(name, mine);
      base.setItem(name, JSON.stringify({ ...next, state: merged }));
    },
    removeItem: (name) => {
      if (!secondary) base.removeItem(name);
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
