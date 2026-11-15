


// ──────────────────────────────────────────────
// useFileFilter - hook for filtering file lists
// ──────────────────────────────────────────────

/**
 * Filter a file list by text query. Runs in <100ms for typical file lists.
 */
export function filterFiles<T extends { name: string; path: string; extension: string | null }>(
  files: T[],
  filterText: string,
): T[] {
  if (!filterText.trim()) return files;

  const query = filterText.toLowerCase().trim();
  const terms = query.split(/\s+/);

  return files.filter((file) => {
    const name = file.name.toLowerCase();
    const ext = (file.extension || "").toLowerCase();

    return terms.every(
      (term) => name.includes(term) || ext.includes(term),
    );
  });
}
