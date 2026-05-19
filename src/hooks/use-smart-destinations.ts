/**
 * useSmartDestinations
 *
 * Tiny extension → top-N destination directories cache that powers the
 * **Smart Send-To** entries in the file context menu. When the user
 * right-clicks a file, we look up the most-frecency-ranked destination
 * directories where files of that same extension have historically been
 * moved or copied. Click any entry → fires the existing `move_files`
 * IPC. The result: one click instead of a folder-picker tree walk.
 *
 * Design principles:
 *
 * - **DRY**: backed by the existing `ledger_extension_destinations`
 *   Tauri command. No new IPC, no new tables, no new background work.
 * - **Pull, not push**: the cache is populated lazily on first lookup.
 *   Subsequent right-clicks on files of the same extension are instant.
 * - **Bounded**: extensions are bounded by user behavior; the cache is
 *   a flat module-level Map, no LRU eviction needed.
 * - **Fail-soft**: outside Tauri (`pnpm dev` browser preview) returns an
 *   empty list via `tauriInvokeSafe`, so the menu just renders without
 *   the Send-To entries. Never crashes the right-click flow.
 */
import { tauriInvokeSafe } from "./use-tauri";

/** Wire-format mirror of `FrecencyHit` in `src-tauri/src/ledger/mod.rs`.
 *  Reused here rather than re-imported because the FrecencyHit shape
 *  is small enough that re-stating the contract locally avoids a
 *  cross-store import. */
export interface SmartDestination {
  path: string;
  last_seen: string;
  hit_count: number;
  score: number;
}

/** Module-level cache. Lives for the session; cleared on app reload.
 *  Keyed by lowercased, dot-stripped extension. */
const cache: Map<string, SmartDestination[]> = new Map();

/** In-flight requests, so two simultaneous lookups for the same
 *  extension don't fire two backend queries. */
const inflight: Map<string, Promise<SmartDestination[]>> = new Map();

/**
 * Fetch (or read from cache) the top destination directories for files
 * with the given extension. The leading dot is optional; case is
 * ignored. Returns an empty array when:
 *
 * - the extension is empty,
 * - the ledger has no matching events, or
 * - the call is made outside Tauri.
 *
 * `limit` defaults to 3 (matching the visual budget of an inline
 * context-menu Send-To section).
 */
export async function loadSmartDestinations(
  extension: string,
  limit: number = 3,
): Promise<SmartDestination[]> {
  const key = extension.trim().replace(/^\./, "").toLowerCase();
  if (key === "") return [];

  const cached = cache.get(key);
  if (cached !== undefined) return cached;

  const existing = inflight.get(key);
  if (existing !== undefined) return existing;

  const promise = (async () => {
    const hits = await tauriInvokeSafe<SmartDestination[]>(
      "ledger_extension_destinations",
      { extension: key, limit },
      [],
    );
    cache.set(key, hits);
    inflight.delete(key);
    return hits;
  })();

  inflight.set(key, promise);
  return promise;
}

/**
 * Drop the cache. Used by tests and (in the future) by a "refresh"
 * affordance if we ever surface one. Today the session-scoped cache is
 * fine — the data only changes after the user does a fresh move/copy,
 * at which point right-clicking the *same* extension a moment later
 * still produces a useful (slightly stale) suggestion list.
 */
export function clearSmartDestinationsCache(): void {
  cache.clear();
  inflight.clear();
}

/**
 * Extract the lowercased, dot-stripped extension of a path. Returns the
 * empty string for paths with no extension or for paths whose final
 * segment is dot-prefixed (e.g., `.gitignore` is treated as having no
 * suggestable extension — it's a dotfile name, not a content type).
 */
export function fileExtension(path: string): string {
  const lastSlash = Math.max(path.lastIndexOf("/"), path.lastIndexOf("\\"));
  const name = lastSlash >= 0 ? path.slice(lastSlash + 1) : path;
  const dot = name.lastIndexOf(".");
  if (dot <= 0) return "";
  return name.slice(dot + 1).toLowerCase();
}
