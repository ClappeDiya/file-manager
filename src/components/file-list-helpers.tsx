import { formatBytes } from "@/lib/format-bytes";


// ──────────────────────────────────────────────
// Types (maps to Rust FileEntry)
// ──────────────────────────────────────────────

export interface FileEntryData {
  name: string;
  path: string;
  is_dir: boolean;
  is_symlink: boolean;
  size: number;
  modified: string | null;
  created: string | null;
  is_hidden: boolean;
  extension: string | null;
  permissions: string | null;
}


// ──────────────────────────────────────────────
// Utility functions
// ──────────────────────────────────────────────

/** File size in the units chosen in Settings → Appearance; "--" for 0. */
export function formatFileSize(bytes: number): string {
  if (bytes === 0) return "--";
  return formatBytes(bytes);
}


export function formatDate(dateStr: string | null): string {
  if (!dateStr) return "--";
  const date = new Date(dateStr);
  return date.toLocaleDateString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}


/**
 * Render the Size column for a row, accounting for the optional
 * `folderSizes` map produced by the user-initiated "Calculate Size"
 * context-menu action.
 *
 * Files always show their byte count.
 *
 * Folders show `--` until the user explicitly asks the platform to
 * compute their recursive size. Once computed, the result is cached
 * in the FilePane's `folderSizes` state and surfaced here so the
 * value sticks for the remainder of the pane's lifetime. Before
 * this iteration the cached value was never read — see
 * `_folderSizes` in `file-manager.tsx`.
 */
export function formatEntrySize(
  entry: FileEntryData,
  folderSizes?: Record<string, number>,
): string {
  if (entry.is_dir) {
    const cached = folderSizes?.[entry.path];
    return cached !== undefined ? formatFileSize(cached) : "--";
  }
  return formatFileSize(entry.size);
}
