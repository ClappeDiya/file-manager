/**
 * Path-aware cross-pane refresh (iter 18).
 *
 * Before iter 18 the codebase had TWO inconsistent refresh patterns:
 *
 *   1. `handleCopy` fired a global `ufop:refresh-directory` CustomEvent
 *      with no payload. Every FilePane listener reloaded its own
 *      `currentPath` unconditionally — both panes refreshed even when
 *      one was viewing a completely unrelated directory.
 *
 *   2. `handleCut` called `loadDirectory(currentPath)` directly on the
 *      SOURCE pane only. The DESTINATION pane was left showing stale
 *      state after a cross-pane move — a genuinely annoying UX bug
 *      that every power-user of the dual-pane workflow hit eventually.
 *
 * This module centralises a single path-aware refresh pattern that
 * both fixes the bug and optimises the common case:
 *
 *   - Dispatchers call `dispatchRefresh(affectedPaths)` with the list
 *     of paths the operation touched (source dir + dest dir, or the
 *     set of files that changed).
 *   - Listeners (each FilePane) call `shouldRefreshPane(currentPath,
 *     detail.affectedPaths)` to decide whether to reload. The pane
 *     refreshes iff at least one affected path is equal to or a
 *     descendant of the viewed path (or the affected path IS the
 *     viewed path's parent and thus the viewed path's contents may
 *     have changed \u2014 we treat an exact match as sufficient since the
 *     dispatcher is expected to pass the destination directory itself).
 *
 * Backwards compatibility: the palette "Refresh" action and any
 * unknown caller still dispatch the plain `ufop:refresh-directory`
 * event without a detail payload. `shouldRefreshPane` treats an empty
 * or missing `affectedPaths` list as "I don't know what changed \u2014
 * refresh to be safe", so the existing call sites keep working.
 */

/** Global CustomEvent name used for cross-pane refresh coordination. */
export const REFRESH_EVENT = "ufop:refresh-directory";

/** Shape carried in the `detail` field of a REFRESH_EVENT CustomEvent. */
export interface RefreshEventDetail {
  /** Absolute paths the triggering operation affected. Can include
   *  directories (where contents changed) and/or individual files.
   *  An empty array means "unknown \u2014 every listener should refresh",
   *  which matches the pre-iter-18 behaviour for backwards compat. */
  affectedPaths: string[];
}

/**
 * Dispatch a path-aware refresh event. Pass every path the operation
 * touched so panes viewing those locations can decide whether to
 * reload. For a cross-pane copy/move, pass `[destDir, ...sourcePaths]`
 * so both the source pane (which lost files in the move case) and
 * the destination pane (which gained files) refresh.
 */
export function dispatchRefresh(affectedPaths: string[]): void {
  window.dispatchEvent(
    new CustomEvent<RefreshEventDetail>(REFRESH_EVENT, {
      detail: { affectedPaths },
    }),
  );
}

/**
 * Iter 25: derive the distinct parent directories from a list of
 * absolute file paths. Used by the Batch Rename onComplete callback
 * to dispatch a single refresh covering every pane that might be
 * viewing a parent of one of the renamed files.
 *
 * Pure function: no side effects, returns a new array. Input is
 * preserved. Handles the root-path edge case (`/foo.txt` \u2192 `/`).
 * Deduplicates parents in first-seen order so the dispatch stays
 * small even when the input has many files in the same folder.
 */
export function parentDirectoriesOf(paths: string[]): string[] {
  const seen = new Set<string>();
  const parents: string[] = [];
  for (const p of paths) {
    const parent = p.replace(/\/[^/]+$/, "") || "/";
    if (!seen.has(parent)) {
      seen.add(parent);
      parents.push(parent);
    }
  }
  return parents;
}

/** Normalise a path for comparison: strip trailing slashes but
 *  preserve the single leading slash so the root path stays "/". */
function normalise(path: string): string {
  if (path === "/" || path === "") return "/";
  return path.replace(/\/+$/, "");
}

/**
 * Returns true when a pane viewing `viewedPath` needs to refresh
 * given that an operation affected `affectedPaths`. A pane needs to
 * refresh when:
 *
 *   - `affectedPaths` is empty (unknown = safe refresh),
 *   - any affected path equals the viewed path (its contents changed),
 *   - any affected path is a direct child of the viewed path (a file
 *     inside our directory changed), or
 *   - any affected path is a deeper descendant of the viewed path
 *     (conservative: a nested change might matter if we start
 *     showing recursive state; today this is the same as direct-child).
 *
 * Intentionally does NOT match when the viewed path is a descendant
 * of an affected path \u2014 e.g. if we\u2019re viewing /home/src and the
 * affected path is /home, we should NOT refresh /home/src because
 * /home changing doesn\u2019t affect /home/src\u2019s contents.
 */
export function shouldRefreshPane(
  viewedPath: string,
  affectedPaths: string[],
): boolean {
  if (affectedPaths.length === 0) return true;
  const viewed = normalise(viewedPath);
  for (const raw of affectedPaths) {
    const affected = normalise(raw);
    if (affected === viewed) return true;
    // Affected path is INSIDE the viewed directory.
    // Handle root specially so "/" + "/foo" works.
    const prefix = viewed === "/" ? "/" : viewed + "/";
    if (affected.startsWith(prefix)) return true;
  }
  return false;
}
