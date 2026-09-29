/**
 * Turns a sync dry-run preview into rows for a side-by-side
 * Source | Destination comparison, grouped under their folders.
 */

export interface PreviewEntry {
  relative_path: string;
  action: string;
  source_size: number | null;
  dest_size: number | null;
  source_modified: string | null;
  dest_modified: string | null;
  reason: string;
}

export interface PreviewLike {
  additions: PreviewEntry[];
  modifications: PreviewEntry[];
  deletions: PreviewEntry[];
  skipped: PreviewEntry[];
  conflicts: PreviewEntry[];
  dirs_to_create?: string[];
  dirs_to_remove?: string[];
}

/** What will happen to a row, from the destination's point of view. */
export type RowChange = "add" | "modify" | "delete" | "conflict" | "skip" | "mkdir" | "rmdir" | "folder";

export interface CompareSide {
  size: number | null;
  modified: string | null;
}

export interface CompareRow {
  path: string;
  name: string;
  depth: number;
  isDir: boolean;
  change: RowChange;
  source: CompareSide | null;
  dest: CompareSide | null;
  reason: string;
}

const norm = (p: string) => p.replace(/\\/g, "/").replace(/\/+$/, "");

function side(size: number | null, modified: string | null): CompareSide | null {
  return size === null && modified === null ? null : { size, modified };
}

/**
 * Build comparison rows. Folders that only group changed files get a
 * "folder" row; `includeSkipped` adds files the sync will leave alone.
 */
export function buildComparisonRows(preview: PreviewLike, includeSkipped = false): CompareRow[] {
  const rows = new Map<string, CompareRow>();
  const add = (e: PreviewEntry, change: RowChange) => {
    const path = norm(e.relative_path);
    rows.set(path, {
      path,
      name: path.split("/").pop() || path,
      depth: path.split("/").length - 1,
      isDir: false,
      change,
      // Two-way additions flow dest → source; mirror that in the columns.
      source: side(e.source_size, e.source_modified),
      dest: change === "add" && e.source_size !== null ? null : side(e.dest_size, e.dest_modified),
      reason: e.reason,
    });
  };
  preview.additions.forEach((e) => add(e, "add"));
  preview.modifications.forEach((e) => add(e, "modify"));
  preview.deletions.forEach((e) => add(e, "delete"));
  preview.conflicts.forEach((e) => add(e, "conflict"));
  if (includeSkipped) preview.skipped.forEach((e) => add(e, "skip"));

  const addDir = (dir: string, change: RowChange) => {
    const path = norm(dir);
    if (!path || (rows.has(path) && rows.get(path)!.change !== "folder")) return;
    rows.set(path, {
      path,
      name: path.split("/").pop() || path,
      depth: path.split("/").length - 1,
      isDir: true,
      change,
      source: change === "rmdir" ? null : { size: null, modified: null },
      dest: change === "mkdir" ? null : { size: null, modified: null },
      reason: change === "mkdir" ? "New folder" : change === "rmdir" ? "Folder removed" : "",
    });
  };
  (preview.dirs_to_create ?? []).forEach((d) => addDir(d, "mkdir"));
  (preview.dirs_to_remove ?? []).forEach((d) => addDir(d, "rmdir"));

  // Ancestor folder rows so every file sits under its folder.
  for (const path of Array.from(rows.keys())) {
    const parts = path.split("/");
    for (let i = 1; i < parts.length; i++) {
      const dir = parts.slice(0, i).join("/");
      if (!rows.has(dir)) addDir(dir, "folder");
    }
  }

  // Folders sort before their contents; siblings: folders first, then by name.
  const key = (r: CompareRow) =>
    r.path
      .split("/")
      .map((seg, i, all) => (i === all.length - 1 && !r.isDir ? `1${seg}` : `0${seg}`))
      .join("/");
  return Array.from(rows.values()).sort((a, b) => (key(a) < key(b) ? -1 : key(a) > key(b) ? 1 : 0));
}

/** Counts shown in the comparison header. */
export function summarizeRows(rows: CompareRow[]) {
  const count = (c: RowChange) => rows.filter((r) => r.change === c).length;
  return {
    add: count("add"),
    modify: count("modify"),
    delete: count("delete") + count("rmdir"),
    conflict: count("conflict"),
    mkdir: count("mkdir"),
  };
}
