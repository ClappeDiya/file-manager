import { describe, expect, it } from "vitest";
import {
  buildTree,
  largestDifference,
  matchesFilter,
  mergeOutcome,
  nextDifference,
  nextDifferencePath,
  rangeBetween,
  selectionRoots,
  treeOrder,
  visibleRows,
  type ApplyOutcome,
  type CompareEntry,
  type CompareResult,
  type CompareStatus,
} from "@/lib/folder-compare";

function entry(path: string, status: CompareStatus, over: Partial<CompareEntry> = {}): CompareEntry {
  const isDir = over.is_dir ?? false;
  return {
    path,
    name: path.split("/").pop() ?? path,
    is_dir: isDir,
    status,
    in_left: status !== "right_only",
    in_right: status !== "left_only",
    left_size: isDir || status === "right_only" ? null : 10,
    right_size: isDir || status === "left_only" ? null : 10,
    left_modified: null,
    right_modified: null,
    reason: null,
    error: null,
    children_skipped: false,
    ...over,
  };
}

const dir = (path: string, status: CompareStatus) => entry(path, status, { is_dir: true });

/**
 * docs/ (different): a.txt same, b.txt different (10 vs 400), new.txt left only
 * media/ (different): big/ (different): x.bin different (1 vs 5000), y.bin right only (20)
 * same/ (same): s.txt same
 * zzz.txt right only (30)
 */
const ENTRIES: CompareEntry[] = [
  dir("docs", "different"),
  entry("docs/a.txt", "same"),
  entry("docs/b.txt", "different", { right_size: 400 }),
  entry("docs/new.txt", "left_only", { left_size: 5 }),
  dir("media", "different"),
  dir("media/big", "different"),
  entry("media/big/x.bin", "different", { left_size: 1, right_size: 5000 }),
  entry("media/big/y.bin", "right_only", { right_size: 20 }),
  dir("same", "same"),
  entry("same/s.txt", "same"),
  entry("zzz.txt", "right_only", { right_size: 30 }),
];

const none = new Set<CompareStatus>();
const noCollapse = new Set<string>();

describe("folder compare tree", () => {
  it("rolls each file's status and size difference up into every folder above it", () => {
    const tree = buildTree(ENTRIES);
    const docs = tree.nodes.get("docs")!;
    expect(docs.counts).toEqual({ left_only: 1, right_only: 0, same: 1, different: 1, error: 0 });
    expect(docs.diffFiles).toBe(2);
    expect(docs.diffBytes).toBe(390 + 5);
    const media = tree.nodes.get("media")!;
    expect(media.counts.different).toBe(1);
    expect(media.counts.right_only).toBe(1);
    expect(media.diffBytes).toBe(4999 + 20);
    expect(tree.nodes.get("media/big/x.bin")!.depth).toBe(2);
    expect(tree.roots).toEqual(["docs", "media", "same", "zzz.txt"]);
  });

  it("status filters are OR-combined and folders show when anything below matches", () => {
    const tree = buildTree(ENTRIES);
    const paths = (statuses: CompareStatus[]) =>
      visibleRows(tree, { statuses: new Set(statuses), collapsed: noCollapse, sort: "name" }).map((r) => r.path);
    expect(paths(["left_only"])).toEqual(["docs", "docs/new.txt"]);
    expect(paths(["left_only", "right_only"])).toEqual(["docs", "docs/new.txt", "media", "media/big", "media/big/y.bin", "zzz.txt"]);
    expect(paths([])).toHaveLength(ENTRIES.length);
    expect(matchesFilter(tree.nodes.get("same")!, new Set(["different"]))).toBe(false);
    expect(matchesFilter(tree.nodes.get("same")!, new Set(["same"]))).toBe(true);
  });

  it("collapsed folders hide their contents; folders sort first", () => {
    const tree = buildTree([entry("a.txt", "same"), ...ENTRIES]);
    const rows = visibleRows(tree, { statuses: none, collapsed: new Set(["docs", "media"]), sort: "name" }).map((r) => r.path);
    expect(rows).toEqual(["docs", "media", "same", "same/s.txt", "a.txt", "zzz.txt"]);
  });

  it("sorts siblings by size difference", () => {
    const tree = buildTree(ENTRIES);
    const rows = visibleRows(tree, { statuses: none, collapsed: new Set(["docs", "media", "same"]), sort: "size_delta" }).map((r) => r.path);
    expect(rows).toEqual(["media", "docs", "zzz.txt", "same"]);
  });

  it("finds the next difference, stepping into folders and wrapping", () => {
    const tree = buildTree(ENTRIES);
    const rows = visibleRows(tree, { statuses: none, collapsed: noCollapse, sort: "name" });
    const at = (i: number) => rows[i].path;
    let i = nextDifference(rows, -1);
    expect(at(i)).toBe("docs/b.txt");
    i = nextDifference(rows, i);
    expect(at(i)).toBe("docs/new.txt");
    i = nextDifference(rows, i);
    expect(at(i)).toBe("media/big/x.bin");
    expect(at(nextDifference(rows, rows.findIndex((r) => r.path === "zzz.txt")))).toBe("docs/b.txt");
    // Inside collapsed folders too.
    expect(nextDifferencePath(tree, { statuses: none, sort: "name" }, "docs/new.txt")).toBe("media/big/x.bin");
    expect(nextDifference(visibleRows(buildTree([entry("s", "same")]), { statuses: none, collapsed: noCollapse, sort: "name" }), -1)).toBe(-1);
  });

  it("finds the largest difference by size and by file count", () => {
    const tree = buildTree(ENTRIES);
    // media/big holds all of media's bytes: the deeper folder wins the tie.
    expect(largestDifference(tree, "size")).toBe("media/big");
    expect(largestDifference(tree, "count")).toBe("media/big");
    const more = buildTree([...ENTRIES, entry("docs/c.txt", "left_only"), entry("docs/d.txt", "right_only")]);
    expect(largestDifference(more, "count")).toBe("docs");
    expect(largestDifference(buildTree([entry("x", "same")]), "size")).toBeNull();
    expect(largestDifference(buildTree([entry("x", "left_only", { left_size: 9 }), entry("y", "same")]), "size")).toBe("x");
  });

  it("selects shift ranges in row order and sends only the topmost selected paths", () => {
    const tree = buildTree(ENTRIES);
    const rows = visibleRows(tree, { statuses: none, collapsed: noCollapse, sort: "name" });
    expect(rangeBetween(rows, "docs/new.txt", "docs/a.txt")).toEqual(["docs/a.txt", "docs/b.txt", "docs/new.txt"]);
    expect(rangeBetween(rows, null, "zzz.txt")).toEqual(["zzz.txt"]);
    expect(selectionRoots(new Set(["docs/a.txt", "docs", "media/big/x.bin", "docs-old"]))).toEqual(["docs", "docs-old", "media/big/x.bin"]);
    expect(["a-b", "a/b", "a"].sort(treeOrder)).toEqual(["a", "a/b", "a-b"]);
  });

  it("folds re-compared entries into the results", () => {
    const result = { entries: ENTRIES, summary: {} } as unknown as CompareResult;
    const outcome = {
      changed: [entry("docs/new.txt", "same"), entry("docs/b.txt", "same")],
      removed: ["zzz.txt"],
      summary: { duration_ms: 1 },
    } as unknown as ApplyOutcome;
    const merged = mergeOutcome(result, outcome);
    expect(merged.entries.find((e) => e.path === "docs/new.txt")!.status).toBe("same");
    expect(merged.entries.some((e) => e.path === "zzz.txt")).toBe(false);
    expect(merged.entries.map((e) => e.path).indexOf("docs/new.txt")).toBe(3);
    expect(merged.summary).toBe(outcome.summary);
  });
});
