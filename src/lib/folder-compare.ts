/**
 * Standalone folder compare: wire types (mirroring `ufop_core::sync::compare`
 * and `src-tauri/src/compare_engine`) and the pure helpers behind the compare
 * window — the tree with each folder's roll-up of what is inside it, the
 * OR-combined status filter, sorting, "next difference" / "largest
 * difference" navigation, shift-range selection, and folding an action's
 * re-compared entries back into the results.
 */
import type { CompareMode } from "@/lib/copy-options";
import type { SyncFilter } from "@/stores/sync-store";
import type { OperationIntent } from "@/lib/safety";

export type CompareStatus = "left_only" | "right_only" | "same" | "different" | "error";

export const COMPARE_STATUSES: CompareStatus[] = ["left_only", "right_only", "different", "same", "error"];

/** One side of a comparison (`CompareSide` in compare_engine). */
export type CompareSide =
  | { type: "local"; path: string }
  | { type: "remote"; connection_id: string | null; protocol: string | null; path: string };

export interface CompareOptions {
  filter: SyncFilter;
  compare_mode: CompareMode;
  /** Folder levels below each root; 0 = all. */
  max_depth: number;
  /** Times up to 2 s apart count as equal. */
  time_tolerance: boolean;
}

export const EMPTY_FILTER: SyncFilter = {
  include_patterns: [],
  exclude_patterns: [],
  include_regex: [],
  exclude_regex: [],
  min_size: 0,
  max_size: 0,
  file_types: [],
  exclude_file_types: [],
  rules: [],
};

export const DEFAULT_COMPARE_OPTIONS: CompareOptions = {
  filter: EMPTY_FILTER,
  compare_mode: "size_mtime",
  max_depth: 0,
  time_tolerance: false,
};

export interface CompareEntry {
  path: string;
  name: string;
  is_dir: boolean;
  status: CompareStatus;
  in_left: boolean;
  in_right: boolean;
  left_size: number | null;
  right_size: number | null;
  left_modified: string | null;
  right_modified: string | null;
  reason: string | null;
  error: string | null;
  children_skipped: boolean;
}

export interface StatusTotals {
  files: number;
  folders: number;
  left_bytes: number;
  right_bytes: number;
}

export interface CompareSummary {
  left_only: StatusTotals;
  right_only: StatusTotals;
  same: StatusTotals;
  different: StatusTotals;
  error: StatusTotals;
  duration_ms: number;
}

export interface CompareResult {
  task_id: string;
  left: CompareSide;
  right: CompareSide;
  left_label: string;
  right_label: string;
  options: CompareOptions;
  entries: CompareEntry[];
  summary: CompareSummary;
  notes: string[];
  computed_at: string;
}

export type ComparePhase = "listing" | "comparing" | "applying" | "refreshing";

export interface CompareProgress {
  task_id: string;
  phase: ComparePhase;
  folders_listed: number;
  entries_found: number;
  done: number;
  total: number;
  current: string | null;
}

export const COMPARE_PROGRESS_EVENT = "compare-progress";

export type CompareAction = "copy_to_right" | "copy_to_left" | "delete_left" | "delete_right";
export type CompareSideId = "left" | "right";

export type ActionStep =
  | { type: "make_dir"; side: CompareSideId; path: string }
  | { type: "copy"; from: CompareSideId; to: CompareSideId; path: string; size: number; modified: string | null; replace: boolean }
  | { type: "delete"; side: CompareSideId; path: string; is_dir: boolean; size: number };

export interface SkippedEntry {
  path: string;
  reason: string;
}

export interface ActionPlan {
  action: CompareAction;
  steps: ActionStep[];
  skipped: SkippedEntry[];
  files: number;
  folders: number;
  bytes: number;
  replaced: number;
}

export interface CompareActionPlan {
  plan: ActionPlan;
  intent: OperationIntent;
}

export interface ApplyOutcome {
  task_id: string;
  action: CompareAction;
  done: string[];
  failed: { path: string; error: string }[];
  skipped: SkippedEntry[];
  kept: string[];
  notes: string[];
  cancelled: boolean;
  undo_id: string | null;
  /** Everything "Undo copies" reverses, in order (the copies, then the folders made for them). */
  undo_ids: string[];
  changed: CompareEntry[];
  removed: string[];
  summary: CompareSummary;
}

export const STATUS_LABEL: Record<CompareStatus, string> = {
  left_only: "Left only",
  right_only: "Right only",
  same: "Same",
  different: "Different",
  error: "Error",
};

/** A side as the UI names it (short form for buttons and headers). */
export function sideLabel(side: CompareSide): string {
  return side.type === "local" ? side.path : `${side.protocol ?? "remote"}:${side.path}`;
}

/** Order paths as a tree (`a`, `a/b`, `a-b`) — same rule as the backend. */
export function treeOrder(a: string, b: string): number {
  const x = a.split("/");
  const y = b.split("/");
  for (let i = 0; i < Math.min(x.length, y.length); i++) {
    if (x[i] !== y[i]) return x[i] < y[i] ? -1 : 1;
  }
  return x.length - y.length;
}

export function parentPath(path: string): string | null {
  const i = path.lastIndexOf("/");
  return i < 0 ? null : path.slice(0, i);
}

/** Per-status counts of the files (and folders) below a folder. */
export type StatusCounts = Record<CompareStatus, number>;

const zeroCounts = (): StatusCounts => ({ left_only: 0, right_only: 0, same: 0, different: 0, error: 0 });

export interface TreeNode {
  entry: CompareEntry;
  depth: number;
  parent: string | null;
  /** Folders: the files below it, by status (folders that could not be read count as errors). */
  counts: StatusCounts;
  /** Folders: files that differ in any way below it. */
  diffFiles: number;
  /** Bytes the two sides differ by: |left − right| for this file, summed for a folder. */
  diffBytes: number;
  /** Children in tree order. */
  children: string[];
}

export interface CompareTree {
  nodes: Map<string, TreeNode>;
  roots: string[];
}

const isDifference = (s: CompareStatus) => s === "left_only" || s === "right_only" || s === "different";

function sizeDelta(e: CompareEntry): number {
  if (e.is_dir || e.status === "same") return 0;
  return Math.abs((e.left_size ?? 0) - (e.right_size ?? 0));
}

/** Build the tree, rolling each file's status and size delta up into every folder above it. */
export function buildTree(entries: CompareEntry[]): CompareTree {
  const sorted = [...entries].sort((a, b) => treeOrder(a.path, b.path));
  const nodes = new Map<string, TreeNode>();
  const roots: string[] = [];
  for (const entry of sorted) {
    let parent = parentPath(entry.path);
    while (parent !== null && !nodes.has(parent)) parent = parentPath(parent);
    const depth = parent === null ? 0 : (nodes.get(parent)?.depth ?? -1) + 1;
    nodes.set(entry.path, { entry, depth, parent, counts: zeroCounts(), diffFiles: 0, diffBytes: 0, children: [] });
    if (parent === null) roots.push(entry.path);
    else nodes.get(parent)?.children.push(entry.path);
  }
  for (const node of nodes.values()) {
    const e = node.entry;
    const counted = !e.is_dir || e.status === "error";
    if (!counted) continue;
    const delta = sizeDelta(e);
    if (!e.is_dir) node.diffBytes = delta;
    let up = node.parent;
    while (up !== null) {
      const p = nodes.get(up);
      if (!p) break;
      p.counts[e.status] += 1;
      if (isDifference(e.status)) p.diffFiles += 1;
      p.diffBytes += delta;
      up = p.parent;
    }
  }
  return { nodes, roots };
}

/** Whether a node passes the status filter: its own status, or (folders) anything below it. OR-combined; empty = all. */
export function matchesFilter(node: TreeNode, statuses: ReadonlySet<CompareStatus>): boolean {
  if (statuses.size === 0) return true;
  if (statuses.has(node.entry.status)) return true;
  if (!node.entry.is_dir) return false;
  for (const s of statuses) if (node.counts[s] > 0) return true;
  return false;
}

export type CompareSort = "name" | "size_delta";

export interface VisibleRow {
  node: TreeNode;
  path: string;
}

/**
 * The rows to show: tree order (folders first among siblings when sorting by
 * name; largest size difference first when sorting by delta), filtered, with
 * collapsed folders' contents hidden.
 */
export function visibleRows(
  tree: CompareTree,
  opts: { statuses: ReadonlySet<CompareStatus>; collapsed: ReadonlySet<string>; sort: CompareSort },
): VisibleRow[] {
  const out: VisibleRow[] = [];
  const order = (paths: string[]) => {
    const list = paths.map((p) => tree.nodes.get(p)).filter((n): n is TreeNode => !!n);
    if (opts.sort === "size_delta") {
      list.sort((a, b) => b.diffBytes - a.diffBytes || treeOrder(a.entry.path, b.entry.path));
    } else {
      list.sort((a, b) => Number(b.entry.is_dir) - Number(a.entry.is_dir) || a.entry.name.localeCompare(b.entry.name));
    }
    return list;
  };
  const walk = (paths: string[]) => {
    for (const node of order(paths)) {
      if (!matchesFilter(node, opts.statuses)) continue;
      out.push({ node, path: node.entry.path });
      if (node.entry.is_dir && !opts.collapsed.has(node.entry.path)) walk(node.children);
    }
  };
  walk(tree.roots);
  return out;
}

/**
 * Index of the next row (after `from`, wrapping) that is a difference: a file
 * that differs or is on one side only, or a folder on one side only (a
 * folder that merely contains differences is stepped into, not stopped at).
 * -1 when none.
 */
export function nextDifference(rows: VisibleRow[], from: number): number {
  for (let k = 1; k <= rows.length; k++) {
    const i = (((from + k) % rows.length) + rows.length) % rows.length;
    const e = rows[i].node.entry;
    const target = e.is_dir ? e.status === "left_only" || e.status === "right_only" : isDifference(e.status);
    if (target) return i;
  }
  return -1;
}

/**
 * The folder (or, with no folder differing, the file) with the largest
 * difference: by bytes (`size`) or by number of differing files (`count`).
 * Null when nothing differs.
 */
export function largestDifference(tree: CompareTree, by: "size" | "count"): string | null {
  let best: TreeNode | null = null;
  const score = (n: TreeNode) => (by === "size" ? n.diffBytes : n.diffFiles);
  for (const n of tree.nodes.values()) {
    if (!n.entry.is_dir || score(n) === 0) continue;
    // Prefer the deepest folder holding the most: a folder whose single
    // child folder accounts for all of it is not the interesting one.
    if (!best || score(n) > score(best) || (score(n) === score(best) && n.depth > best.depth)) best = n;
  }
  if (best) return best.entry.path;
  for (const n of tree.nodes.values()) {
    if (n.entry.is_dir || !isDifference(n.entry.status)) continue;
    const s = by === "size" ? n.diffBytes : 1;
    if (!best || s > (by === "size" ? best.diffBytes : 1)) best = n;
  }
  return best?.entry.path ?? null;
}

/** Folders to expand so `path` is visible. */
export function ancestorsOf(path: string): string[] {
  const out: string[] = [];
  let p = parentPath(path);
  while (p !== null) {
    out.push(p);
    p = parentPath(p);
  }
  return out;
}

/** Paths from `anchor` to `target` in row order (both included); just `target` when the anchor is not shown. */
export function rangeBetween(rows: VisibleRow[], anchor: string | null, target: string): string[] {
  const b = rows.findIndex((r) => r.path === target);
  const a = anchor === null ? -1 : rows.findIndex((r) => r.path === anchor);
  if (a < 0 || b < 0) return [target];
  const [lo, hi] = a <= b ? [a, b] : [b, a];
  return rows.slice(lo, hi + 1).map((r) => r.path);
}

/**
 * The selection to send: drop paths below a selected folder (the folder
 * already stands for them).
 */
export function selectionRoots(selected: ReadonlySet<string>): string[] {
  const all = [...selected].sort(treeOrder);
  const out: string[] = [];
  for (const p of all) {
    if (out.some((root) => p.startsWith(`${root}/`))) continue;
    out.push(p);
  }
  return out;
}

/** Fold an action's re-compared entries into a result. */
export function mergeOutcome(result: CompareResult, outcome: ApplyOutcome): CompareResult {
  const removed = new Set(outcome.removed);
  const changed = new Map(outcome.changed.map((e) => [e.path, e]));
  const entries = result.entries.filter((e) => !removed.has(e.path) && !changed.has(e.path));
  entries.push(...changed.values());
  entries.sort((a, b) => treeOrder(a.path, b.path));
  return { ...result, entries, summary: outcome.summary };
}

/** "3 files, 1 folder" style count for chips. */
export function totalsCount(t: StatusTotals): number {
  return t.files + t.folders;
}

/** Comma/newline separated patterns → list. */
export function parsePatterns(text: string): string[] {
  return text
    .split(/[,\n]/)
    .map((s) => s.trim())
    .filter(Boolean);
}

/**
 * The next difference after `current` in the fully expanded tree (so a
 * difference inside a collapsed folder is found too); null when none.
 */
export function nextDifferencePath(
  tree: CompareTree,
  opts: { statuses: ReadonlySet<CompareStatus>; sort: CompareSort },
  current: string | null,
): string | null {
  const rows = visibleRows(tree, { ...opts, collapsed: new Set() });
  const from = current === null ? -1 : rows.findIndex((r) => r.path === current);
  const i = nextDifference(rows, from);
  return i < 0 ? null : rows[i].path;
}
