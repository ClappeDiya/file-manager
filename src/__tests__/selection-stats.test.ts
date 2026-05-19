import { describe, it, expect } from "vitest";
import {
  computeListBytes,
  computeSelectionBytes,
  formatPaneStatsLabel,
  type FileEntryLike,
  type PaneStatsLike,
} from "@/lib/selection-stats";
import { formatBytes } from "@/lib/format-bytes";

function file(name: string, size: number): FileEntryLike {
  return { path: `/folder/${name}`, is_dir: false, size };
}
function dir(name: string): FileEntryLike {
  return { path: `/folder/${name}`, is_dir: true, size: 4096 };
}

describe("computeListBytes", () => {
  it("returns zeros for an empty list", () => {
    expect(computeListBytes([])).toEqual({
      count: 0,
      bytes: 0,
      hasDir: false,
    });
  });

  it("sums file bytes and ignores directory sizes", () => {
    const entries = [file("a.txt", 100), file("b.txt", 250), dir("sub")];
    const stats = computeListBytes(entries);
    expect(stats.count).toBe(3);
    expect(stats.bytes).toBe(350);
    expect(stats.hasDir).toBe(true);
  });

  it("sets hasDir=false when no directories are present", () => {
    const stats = computeListBytes([file("a.txt", 100), file("b.txt", 50)]);
    expect(stats.hasDir).toBe(false);
    expect(stats.bytes).toBe(150);
  });

  it("respects the predicate when supplied", () => {
    const entries = [file("a.txt", 100), file("b.txt", 250), file("c.txt", 50)];
    const stats = computeListBytes(entries, (e) => e.size >= 100);
    expect(stats.count).toBe(2);
    expect(stats.bytes).toBe(350);
  });
});

describe("computeSelectionBytes", () => {
  it("returns zeros when no paths are selected", () => {
    const entries = [file("a.txt", 100), file("b.txt", 50)];
    expect(computeSelectionBytes(entries, [])).toEqual({
      count: 0,
      bytes: 0,
      hasDir: false,
    });
  });

  it("sums only the matching entries", () => {
    const entries = [file("a.txt", 100), file("b.txt", 50), file("c.txt", 25)];
    const stats = computeSelectionBytes(entries, [
      "/folder/a.txt",
      "/folder/c.txt",
    ]);
    expect(stats.count).toBe(2);
    expect(stats.bytes).toBe(125);
    expect(stats.hasDir).toBe(false);
  });

  it("ignores selected paths that aren't in the list", () => {
    const entries = [file("a.txt", 100)];
    const stats = computeSelectionBytes(entries, [
      "/folder/a.txt",
      "/folder/missing.txt",
    ]);
    expect(stats.count).toBe(1);
    expect(stats.bytes).toBe(100);
  });

  it("flags hasDir when a directory is in the selection", () => {
    const entries = [file("a.txt", 100), dir("sub")];
    const stats = computeSelectionBytes(entries, ["/folder/sub"]);
    expect(stats.count).toBe(1);
    expect(stats.bytes).toBe(0);
    expect(stats.hasDir).toBe(true);
  });
});

describe("formatPaneStatsLabel", () => {
  function statsFor(part: Partial<PaneStatsLike>): PaneStatsLike {
    return {
      totalCount: 0,
      totalBytes: 0,
      totalHasDir: false,
      selectedCount: 0,
      selectedBytes: 0,
      selectedHasDir: false,
      ...part,
    };
  }
  const fmt = (b: number) => formatBytes(b);

  it("returns 'Ready' when stats are not yet published", () => {
    expect(formatPaneStatsLabel(null, fmt)).toBe("Ready");
  });

  it("returns 'Empty folder' when the folder has zero items", () => {
    expect(formatPaneStatsLabel(statsFor({}), fmt)).toBe("Empty folder");
  });

  it("formats a non-empty folder with bytes", () => {
    const label = formatPaneStatsLabel(
      statsFor({ totalCount: 3, totalBytes: 1024 * 1024 }),
      fmt,
    );
    expect(label).toMatch(/^3 items · \d+(\.\d+)? MB$/);
  });

  it("omits byte total when totalBytes is zero", () => {
    const label = formatPaneStatsLabel(
      statsFor({ totalCount: 2, totalHasDir: true }),
      fmt,
    );
    expect(label).toBe("2 items + folders");
  });

  it("singularises 'item' when count is 1", () => {
    const label = formatPaneStatsLabel(statsFor({ totalCount: 1 }), fmt);
    expect(label).toMatch(/^1 item$/);
  });

  it("formats a selection with bytes", () => {
    const label = formatPaneStatsLabel(
      statsFor({
        totalCount: 10,
        selectedCount: 3,
        selectedBytes: 512 * 1024,
      }),
      fmt,
    );
    expect(label).toMatch(/^3 of 10 selected · \d+(\.\d+)? KB$/);
  });

  it("formats a selection with only directories — no byte total but '+ folders'", () => {
    const label = formatPaneStatsLabel(
      statsFor({
        totalCount: 10,
        selectedCount: 2,
        selectedHasDir: true,
      }),
      fmt,
    );
    expect(label).toBe("2 of 10 selected + folders");
  });

  it("formats a mixed selection with both bytes AND '+ folders'", () => {
    const label = formatPaneStatsLabel(
      statsFor({
        totalCount: 10,
        selectedCount: 5,
        selectedBytes: 1024,
        selectedHasDir: true,
      }),
      fmt,
    );
    expect(label).toMatch(/^5 of 10 selected · 1(\.0)? KB \+ folders$/);
  });

  it("selection mode takes precedence over folder mode", () => {
    const label = formatPaneStatsLabel(
      statsFor({
        totalCount: 10,
        totalBytes: 9999,
        totalHasDir: true,
        selectedCount: 1,
        selectedBytes: 100,
      }),
      fmt,
    );
    expect(label).toMatch(/^1 of 10 selected · /);
    expect(label).not.toContain("items");
  });

  it("uses the injected formatter — string returned to caller is exactly what fmt produced", () => {
    const customFmt = (b: number) => `${b}b`;
    const label = formatPaneStatsLabel(
      statsFor({ totalCount: 3, totalBytes: 42 }),
      customFmt,
    );
    expect(label).toBe("3 items · 42b");
  });
});
