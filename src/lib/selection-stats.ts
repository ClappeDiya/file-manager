/**
 * Selection-stats helper — computes the classic file-manager status-bar
 * line ("N items, X MB"). Pure function so it's trivial to unit-test and
 * works equally well for "the whole folder" stats and "just the
 * selection" stats — the caller passes a predicate for selection mode.
 *
 * Directories are intentionally NOT summed into `bytes` because
 * `FileEntryData.size` for a directory is just the inode size, not the
 * recursive content size (those are loaded lazily by iteration 7's
 * `folderSizes` map). When any directory is present in the counted set,
 * we set `hasDir` so the caller can append a "+ folders" suffix —
 * matching Finder's behavior of refusing to lie about folder size.
 */

export interface FileEntryLike {
  path: string;
  is_dir: boolean;
  size: number;
}

export interface ListStats {
  /** Number of entries matched (files + dirs). */
  count: number;
  /** Sum of `size` for non-directory entries only. */
  bytes: number;
  /** True if at least one directory was counted. Lets the UI append
   *  "+ folders" so the byte total isn't misleading. */
  hasDir: boolean;
}

/**
 * Tally `count`, `bytes`, and `hasDir` over `entries`. When `predicate`
 * is supplied, only entries that satisfy it are counted; otherwise the
 * whole list is counted.
 */
export function computeListBytes<T extends FileEntryLike>(
  entries: readonly T[],
  predicate?: (entry: T) => boolean,
): ListStats {
  let count = 0;
  let bytes = 0;
  let hasDir = false;
  for (const e of entries) {
    if (predicate && !predicate(e)) continue;
    count++;
    if (e.is_dir) {
      hasDir = true;
    } else {
      bytes += e.size;
    }
  }
  return { count, bytes, hasDir };
}

/**
 * Convenience wrapper that builds the selection predicate from a set of
 * paths. Caller passes the raw `selectedPaths` array; we materialise the
 * Set once so per-entry lookups are O(1).
 */
export function computeSelectionBytes<T extends FileEntryLike>(
  entries: readonly T[],
  selectedPaths: readonly string[],
): ListStats {
  if (selectedPaths.length === 0) {
    return { count: 0, bytes: 0, hasDir: false };
  }
  const set = new Set(selectedPaths);
  return computeListBytes(entries, (e) => set.has(e.path));
}

/** Tally a pane publishes for the global status bar. Matches the
 *  `PaneStats` interface in the file-manager store byte-for-byte but
 *  duplicated here as a structural type so this lib stays a pure
 *  zero-dependency helper (no store import → no risk of circular
 *  imports from a unit test importing the helper). */
export interface PaneStatsLike {
  totalCount: number;
  totalBytes: number;
  totalHasDir: boolean;
  selectedCount: number;
  selectedBytes: number;
  selectedHasDir: boolean;
}

/** Render the classic Finder-style status-bar summary from a pane's
 *  published stats. Four modes:
 *    - no stats yet  → "Ready" (initial / non-Tauri demo path)
 *    - empty folder  → "Empty folder"
 *    - selection > 0 → "K of N selected · Y MB [+ folders]"
 *    - selection = 0 → "N items · X MB [+ folders]"
 *
 *  The "+ folders" suffix appears whenever a directory is in the
 *  counted set — `FileEntryData.size` for a directory is just the
 *  inode size (not the recursive content size), so summing those
 *  would lie. Iteration 7's `folderSizes` map already does recursive
 *  sizing inline in the file list; the status bar deliberately stays
 *  out of that work to remain a zero-cost summary. */
export function formatPaneStatsLabel(
  stats: PaneStatsLike | null,
  formatBytes: (bytes: number) => string,
): string {
  if (!stats) return "Ready";
  if (stats.selectedCount > 0) {
    let label = `${stats.selectedCount} of ${stats.totalCount} selected`;
    if (stats.selectedBytes > 0) label += ` · ${formatBytes(stats.selectedBytes)}`;
    if (stats.selectedHasDir) label += ` + folders`;
    return label;
  }
  if (stats.totalCount === 0) return "Empty folder";
  let label = `${stats.totalCount} item${stats.totalCount === 1 ? "" : "s"}`;
  if (stats.totalBytes > 0) label += ` · ${formatBytes(stats.totalBytes)}`;
  if (stats.totalHasDir) label += ` + folders`;
  return label;
}
