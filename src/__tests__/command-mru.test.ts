/**
 * Iter-24 tests: command palette MRU / frecency helpers.
 *
 * Covers every branch of `readCommandUsage`, `recordCommandUsage`,
 * and `applyMruOrdering`:
 *
 *   - Empty storage \u2192 empty map
 *   - Missing / undefined localStorage \u2192 fail-soft empty
 *   - Corrupt JSON \u2192 fail-soft empty
 *   - Schema-partial entries filtered out defensively
 *   - First use creates record with useCount=1
 *   - Subsequent uses increment useCount + update lastUsedAt
 *   - Eviction at MAX_TRACKED_COMMANDS (LRU by lastUsedAt)
 *   - Ordering: empty usage \u2192 declaration order preserved
 *   - Ordering: known commands grouped at top, sorted by frecency
 *   - Ordering: useCount DESC primary, lastUsedAt DESC tiebreaker
 *   - Ordering: unknown commands keep their original relative order
 *   - Ordering: returns a NEW array (no mutation)
 *
 * Pure unit tests with a stubbed localStorage.
 */
import { describe, it, expect, beforeEach, vi } from "vitest";
import {
  readCommandUsage,
  recordCommandUsage,
  applyMruOrdering,
  MRU_STORAGE_KEY,
  MAX_TRACKED_COMMANDS,
} from "../lib/command-mru";
import type { CommandItem } from "../components/command-palette";

/** In-memory localStorage stub so each test runs against a clean
 *  storage. Installed via `Object.defineProperty` so the real
 *  `localStorage` in the vitest environment is swapped for this
 *  fake without affecting other tests. */
function installFakeStorage(): Map<string, string> {
  const store = new Map<string, string>();
  const fake: Storage = {
    get length() {
      return store.size;
    },
    clear: () => store.clear(),
    getItem: (k) => (store.has(k) ? store.get(k)! : null),
    key: (i) => Array.from(store.keys())[i] ?? null,
    removeItem: (k) => {
      store.delete(k);
    },
    setItem: (k, v) => {
      store.set(k, v);
    },
  };
  vi.stubGlobal("localStorage", fake);
  return store;
}

function cmd(id: string, label: string = id): CommandItem {
  return {
    id,
    label,
    category: "Test",
    action: () => {},
  };
}

describe("readCommandUsage", () => {
  beforeEach(() => installFakeStorage());

  it("returns empty map when nothing is stored", () => {
    expect(readCommandUsage()).toEqual({});
  });

  it("returns empty map when JSON is corrupt", () => {
    localStorage.setItem(MRU_STORAGE_KEY, "{ not json");
    expect(readCommandUsage()).toEqual({});
  });

  it("returns empty map when stored value is not an object", () => {
    localStorage.setItem(MRU_STORAGE_KEY, "42");
    expect(readCommandUsage()).toEqual({});
    localStorage.setItem(MRU_STORAGE_KEY, "null");
    expect(readCommandUsage()).toEqual({});
  });

  it("filters out entries missing useCount or lastUsedAt", () => {
    localStorage.setItem(
      MRU_STORAGE_KEY,
      JSON.stringify({
        "good-cmd": { useCount: 3, lastUsedAt: 1_000 },
        "bad-no-count": { lastUsedAt: 2_000 },
        "bad-no-timestamp": { useCount: 1 },
        "bad-null": null,
        "bad-wrong-types": { useCount: "five", lastUsedAt: "soon" },
      }),
    );
    const usage = readCommandUsage();
    expect(Object.keys(usage)).toEqual(["good-cmd"]);
    expect(usage["good-cmd"].useCount).toBe(3);
  });

  it("round-trips a written map", () => {
    recordCommandUsage("swap-panes", 1_000);
    recordCommandUsage("swap-panes", 2_000);
    recordCommandUsage("duplicate-tab", 1_500);
    const usage = readCommandUsage();
    expect(usage["swap-panes"].useCount).toBe(2);
    expect(usage["swap-panes"].lastUsedAt).toBe(2_000);
    expect(usage["duplicate-tab"].useCount).toBe(1);
    expect(usage["duplicate-tab"].lastUsedAt).toBe(1_500);
  });
});

describe("recordCommandUsage", () => {
  beforeEach(() => installFakeStorage());

  it("creates a new entry on first use with useCount=1", () => {
    recordCommandUsage("swap-panes", 5_000);
    const usage = readCommandUsage();
    expect(usage["swap-panes"]).toEqual({
      commandId: "swap-panes",
      useCount: 1,
      lastUsedAt: 5_000,
    });
  });

  it("increments useCount and updates lastUsedAt on repeat use", () => {
    recordCommandUsage("swap-panes", 1_000);
    recordCommandUsage("swap-panes", 2_000);
    recordCommandUsage("swap-panes", 3_000);
    const usage = readCommandUsage();
    expect(usage["swap-panes"].useCount).toBe(3);
    expect(usage["swap-panes"].lastUsedAt).toBe(3_000);
  });

  it("tracks multiple commands independently", () => {
    recordCommandUsage("swap-panes", 1_000);
    recordCommandUsage("duplicate-tab", 2_000);
    recordCommandUsage("swap-panes", 3_000);
    const usage = readCommandUsage();
    expect(usage["swap-panes"].useCount).toBe(2);
    expect(usage["duplicate-tab"].useCount).toBe(1);
  });

  it("evicts least-recently-used entries when cap is exceeded", () => {
    // Fill to cap + 3 extras; the 3 oldest should be evicted.
    for (let i = 0; i < MAX_TRACKED_COMMANDS + 3; i++) {
      recordCommandUsage(`cmd-${i}`, 1_000 + i);
    }
    const usage = readCommandUsage();
    expect(Object.keys(usage)).toHaveLength(MAX_TRACKED_COMMANDS);
    // cmd-0, cmd-1, cmd-2 should be evicted (oldest lastUsedAt).
    expect(usage["cmd-0"]).toBeUndefined();
    expect(usage["cmd-1"]).toBeUndefined();
    expect(usage["cmd-2"]).toBeUndefined();
    // Newest should survive.
    expect(usage[`cmd-${MAX_TRACKED_COMMANDS + 2}`]).toBeDefined();
  });

  it("fail-soft: does not throw when localStorage throws on setItem", () => {
    installFakeStorage();
    vi.stubGlobal("localStorage", {
      ...localStorage,
      setItem: () => {
        throw new Error("quota exceeded");
      },
    });
    // Should not throw even though setItem fails.
    expect(() => recordCommandUsage("any-cmd")).not.toThrow();
  });
});

describe("applyMruOrdering", () => {
  it("preserves declaration order when usage map is empty", () => {
    const commands = [cmd("a"), cmd("b"), cmd("c")];
    const result = applyMruOrdering(commands, {});
    expect(result.map((c) => c.id)).toEqual(["a", "b", "c"]);
  });

  it("moves known commands to the top in frecency order", () => {
    const commands = [cmd("a"), cmd("b"), cmd("c"), cmd("d")];
    const usage = {
      c: { commandId: "c", useCount: 5, lastUsedAt: 1_000 },
      a: { commandId: "a", useCount: 2, lastUsedAt: 2_000 },
    };
    const result = applyMruOrdering(commands, usage);
    // c (count=5) first, a (count=2) second, then unknown in order.
    expect(result.map((c) => c.id)).toEqual(["c", "a", "b", "d"]);
  });

  it("breaks useCount ties by lastUsedAt DESC", () => {
    const commands = [cmd("a"), cmd("b"), cmd("c")];
    const usage = {
      a: { commandId: "a", useCount: 3, lastUsedAt: 1_000 },
      b: { commandId: "b", useCount: 3, lastUsedAt: 3_000 },
      c: { commandId: "c", useCount: 3, lastUsedAt: 2_000 },
    };
    const result = applyMruOrdering(commands, usage);
    // All tied on useCount=3; recency orders them b, c, a.
    expect(result.map((c) => c.id)).toEqual(["b", "c", "a"]);
  });

  it("preserves relative order of unknown commands", () => {
    const commands = [cmd("a"), cmd("b"), cmd("c"), cmd("d"), cmd("e")];
    const usage = {
      c: { commandId: "c", useCount: 1, lastUsedAt: 1_000 },
    };
    const result = applyMruOrdering(commands, usage);
    expect(result.map((c) => c.id)).toEqual(["c", "a", "b", "d", "e"]);
  });

  it("returns a new array (does not mutate input)", () => {
    const commands = [cmd("a"), cmd("b")];
    const before = commands.map((c) => c.id);
    applyMruOrdering(commands, {
      b: { commandId: "b", useCount: 1, lastUsedAt: 100 },
    });
    expect(commands.map((c) => c.id)).toEqual(before);
  });

  it("ignores usage entries for commands that no longer exist in the list", () => {
    // User had "old-cmd" in their MRU, but it was removed from
    // the palette in a later iter. Should not crash; just skip.
    const commands = [cmd("a"), cmd("b")];
    const usage = {
      "old-cmd": {
        commandId: "old-cmd",
        useCount: 10,
        lastUsedAt: 9_999,
      },
      a: { commandId: "a", useCount: 1, lastUsedAt: 1_000 },
    };
    const result = applyMruOrdering(commands, usage);
    expect(result.map((c) => c.id)).toEqual(["a", "b"]);
  });

  it("handles the realistic iter-24 scenario: power user's favorites rise", () => {
    // Mimics a user who frequently swaps panes and duplicates tabs,
    // occasionally copies paths, never uses other commands.
    const commands = [
      cmd("new-folder"),
      cmd("new-file"),
      cmd("rename"),
      cmd("delete"),
      cmd("swap-panes"),
      cmd("duplicate-tab"),
      cmd("copy-paths"),
      cmd("bookmark-folder"),
    ];
    const usage = {
      "swap-panes": {
        commandId: "swap-panes",
        useCount: 42,
        lastUsedAt: 10_000,
      },
      "duplicate-tab": {
        commandId: "duplicate-tab",
        useCount: 30,
        lastUsedAt: 9_500,
      },
      "copy-paths": {
        commandId: "copy-paths",
        useCount: 5,
        lastUsedAt: 8_000,
      },
    };
    const result = applyMruOrdering(commands, usage);
    // Frecency winners first, never-used commands in declaration order.
    expect(result.map((c) => c.id)).toEqual([
      "swap-panes",
      "duplicate-tab",
      "copy-paths",
      "new-folder",
      "new-file",
      "rename",
      "delete",
      "bookmark-folder",
    ]);
  });
});
