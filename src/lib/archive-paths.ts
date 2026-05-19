/**
 * Pure helpers for the "Extract Here" right-click action (iter 15).
 *
 * Right-clicking an archive and choosing "Extract Here" should drop
 * the archive's contents into a same-name subfolder next to the
 * archive — e.g., `/path/to/foo.zip` → `/path/to/foo/`. This file
 * houses the pure path math so it stays unit-testable without
 * dragging in any Tauri / store dependencies, and is reusable from
 * any future caller that needs the same destination convention
 * (e.g., automation rules that auto-extract incoming downloads).
 *
 * Compound extensions like `.tar.gz` and `.tgz` are stripped
 * holistically so the resulting stem doesn't end with a stray
 * `.tar`. Extensions are matched case-insensitively because users
 * routinely have `Foo.ZIP` and `Bar.Tar.Gz` from heterogeneous
 * archive tools.
 */

const COMPOUND_ARCHIVE_EXTENSIONS = [
  ".tar.gz",
  ".tar.bz2",
  ".tar.xz",
] as const;

const SINGLE_ARCHIVE_EXTENSIONS = [
  ".tar",
  ".tgz",
  ".tbz2",
  ".txz",
  ".zip",
  ".7z",
  ".rar",
  ".gz",
  ".bz2",
  ".xz",
] as const;

/**
 * Strip a known archive extension from a file name (case-insensitive),
 * preferring compound matches (`.tar.gz` over `.gz`). Unrecognised
 * extensions return the input unchanged.
 */
export function stripArchiveExtension(name: string): string {
  const lower = name.toLowerCase();
  for (const ext of COMPOUND_ARCHIVE_EXTENSIONS) {
    if (lower.endsWith(ext)) return name.slice(0, name.length - ext.length);
  }
  for (const ext of SINGLE_ARCHIVE_EXTENSIONS) {
    if (lower.endsWith(ext)) return name.slice(0, name.length - ext.length);
  }
  return name;
}

/**
 * Compute the "Extract Here" destination directory for an archive at
 * `archivePath`. Returns `${parent}/${stem}` — the same convention
 * macOS Archive Utility uses when you double-click a `.zip`.
 *
 * If the archive's basename has no recognised extension, we still
 * compute a sensible same-name destination so the action never
 * silently falls back to writing into the parent directory.
 */
export function archiveExtractDest(archivePath: string): string {
  const lastSep = archivePath.lastIndexOf("/");
  if (lastSep < 0) {
    // Relative path with no separators ("foo.zip") — stay relative.
    return stripArchiveExtension(archivePath);
  }
  const basename = archivePath.slice(lastSep + 1);
  const stem = stripArchiveExtension(basename);
  // Root-level archives ("/foo.zip" → "/foo") keep the leading slash.
  const parent = lastSep === 0 ? "/" : archivePath.slice(0, lastSep);
  return parent === "/" ? `/${stem}` : `${parent}/${stem}`;
}
