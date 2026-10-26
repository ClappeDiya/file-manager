import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import {
  initialWindowPath,
  isSecondaryWindow,
  onSharedStateChange,
  openNewWindow,
  sharedFields,
  windowLabel,
  sharedWindowStorage,
} from "@/lib/app-window";
import { resolveNewWindowKey } from "@/lib/file-action-keys";
import { WINDOW_FIELDS } from "@/stores/file-manager-store";

const KEY = "ufop-file-manager-state";
const k = (over: Partial<{ key: string; metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean }>) => ({ key: "n", metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...over });

function setUrl(search: string) {
  window.history.replaceState(null, "", `/${search}`);
}

beforeEach(() => {
  localStorage.clear();
  mockInvoke.mockReset().mockResolvedValue("win-abc");
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
});

afterEach(() => setUrl(""));

describe("several windows", () => {
  it("knows which window it is and where it was opened", () => {
    setUrl("");
    expect(windowLabel()).toBe("main");
    expect(isSecondaryWindow()).toBe(false);
    expect(initialWindowPath()).toBeNull();
    setUrl("?window=win-1&path=%2Fhome%2Fme%2FA%20%26%20B");
    expect(windowLabel()).toBe("win-1");
    expect(isSecondaryWindow()).toBe(true);
    expect(initialWindowPath()).toBe("/home/me/A & B");
  });

  it("a second window shares favorites but not the first window's panes", () => {
    const mainState = { panes: ["main-left", "main-right"], singlePaneMode: false, favorites: ["/a"], recentLocations: [] };
    localStorage.setItem(KEY, JSON.stringify({ state: mainState, version: 0 }));
    const storage = sharedWindowStorage(localStorage, WINDOW_FIELDS, true);
    // Reading: no panes, so the window starts with its own.
    expect(JSON.parse(storage.getItem(KEY) as string)).toEqual({ state: { favorites: ["/a"], recentLocations: [] }, version: 0 });
    // Writing its own panes and a new favorite keeps the first window's panes.
    storage.setItem(KEY, JSON.stringify({ state: { panes: ["second"], singlePaneMode: true, favorites: ["/a", "/b"], recentLocations: ["/b"] }, version: 0 }));
    expect(JSON.parse(localStorage.getItem(KEY)!)).toEqual({
      state: { panes: ["main-left", "main-right"], singlePaneMode: false, favorites: ["/a", "/b"], recentLocations: ["/b"] },
      version: 0,
    });
    storage.removeItem(KEY);
    expect(localStorage.getItem(KEY)).not.toBeNull();
  });

  // Review finding: every write carried all shared fields from that window's
  // memory, so a window that had not heard of a new favorite yet removed it.
  it("a window that has not heard of another's change does not write over it", () => {
    localStorage.setItem(KEY, JSON.stringify({ state: { panes: ["main"], favorites: ["/a"] }, version: 0 }));
    const main = sharedWindowStorage(localStorage, WINDOW_FIELDS, false);
    const second = sharedWindowStorage(localStorage, WINDOW_FIELDS, true);
    main.getItem(KEY);
    second.getItem(KEY);
    // The second window adds a favorite.
    second.setItem(KEY, JSON.stringify({ state: { panes: ["second"], favorites: ["/a", "/b"] }, version: 0 }));
    // The first window navigates; its memory still has the old favorites.
    main.setItem(KEY, JSON.stringify({ state: { panes: ["main", "moved"], favorites: ["/a"] }, version: 0 }));
    expect(JSON.parse(localStorage.getItem(KEY)!).state).toEqual({ panes: ["main", "moved"], favorites: ["/a", "/b"] });
    // A favorite the first window itself removes is removed.
    main.setItem(KEY, JSON.stringify({ state: { panes: ["main", "moved"], favorites: [] }, version: 0 }));
    expect(JSON.parse(localStorage.getItem(KEY)!).state.favorites).toEqual([]);
  });

  it("writes only shared fields when no window has saved panes yet", () => {
    const storage = sharedWindowStorage(localStorage, WINDOW_FIELDS, true);
    storage.setItem(KEY, JSON.stringify({ state: { panes: ["second"], favorites: ["/x"] }, version: 0 }));
    expect(JSON.parse(localStorage.getItem(KEY)!).state).toEqual({ favorites: ["/x"] });
    expect(sharedFields({ panes: 1, favorites: 2, activePaneIndex: 0 }, WINDOW_FIELDS)).toEqual({ favorites: 2 });
  });

  it("hears what another window stored, and re-reads on focus", () => {
    const seen: unknown[] = [];
    const off = onSharedStateChange(KEY, (s) => seen.push(s));
    window.dispatchEvent(new StorageEvent("storage", { key: KEY, newValue: JSON.stringify({ state: { favorites: ["/z"] } }) }));
    window.dispatchEvent(new StorageEvent("storage", { key: "other", newValue: "{}" }));
    window.dispatchEvent(new StorageEvent("storage", { key: KEY, newValue: "{half" }));
    localStorage.setItem(KEY, JSON.stringify({ state: { favorites: ["/f"] } }));
    window.dispatchEvent(new Event("focus"));
    off();
    window.dispatchEvent(new Event("focus"));
    expect(seen).toEqual([{ favorites: ["/z"] }, { favorites: ["/f"] }]);
  });

  it("opens a window through the desktop shell, at a folder", async () => {
    await openNewWindow("/home/me/Docs");
    expect(mockInvoke).toHaveBeenCalledWith("open_new_window", { path: "/home/me/Docs" });
    await openNewWindow();
    expect(mockInvoke).toHaveBeenLastCalledWith("open_new_window", { path: null });
  });

  it("in a browser, opens a tab with the window and folder in the address", async () => {
    delete (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__;
    const open = vi.spyOn(window, "open").mockReturnValue(null);
    await openNewWindow("conn://x/Docs");
    const url = new URL(open.mock.calls[0][0] as string);
    expect(url.searchParams.get("window")).toMatch(/^win-/);
    expect(url.searchParams.get("path")).toBe("conn://x/Docs");
    open.mockRestore();
  });

  it("Ctrl/⌘+N opens a window; ⌘⇧N stays New Folder; typing is left alone", () => {
    expect(resolveNewWindowKey(k({ ctrlKey: true }), false)).toBe(true);
    expect(resolveNewWindowKey(k({ metaKey: true, key: "N" }), false)).toBe(true);
    expect(resolveNewWindowKey(k({ metaKey: true, shiftKey: true }), false)).toBe(false);
    expect(resolveNewWindowKey(k({ ctrlKey: true }), true)).toBe(false);
    expect(resolveNewWindowKey(k({}), false)).toBe(false);
  });
});
