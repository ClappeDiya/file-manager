/**
 * useDriveSpace (iter 16) — resolve the drive that owns a given path
 * and return its current free / total space.
 *
 * Design constraints:
 *   - **Cheap**: `detect_drives` walks the system mount table once
 *     per fetch. We cache the result at module scope with a 30s TTL
 *     so rapid navigation (Cmd+J, tab switching, breadcrumb clicks)
 *     doesn't fire one IPC per click.
 *   - **Single in-flight**: concurrent callers share the same promise
 *     so even if two FilePanes mount simultaneously, only one IPC
 *     round-trip happens.
 *   - **Fail-soft**: outside Tauri, in the demo path, or on IPC
 *     failure the hook resolves to `null` — the status-bar render
 *     hides the indicator instead of showing stale or fake data.
 *   - **No-op when path is empty**: the demo path occasionally
 *     hydrates with a "/" or empty string before the active tab's
 *     real path is resolved; we treat both as no-match.
 */
import { useEffect, useState } from "react";
import { isTauriAvailable, tauriInvokeSafe } from "./use-tauri";
import { findMountForPath, type DriveInfoLike } from "@/lib/drive-space";

const CACHE_TTL_MS = 30_000;

interface CacheEntry {
  drives: DriveInfoLike[];
  fetchedAt: number;
}

let cache: CacheEntry | null = null;
let inflight: Promise<DriveInfoLike[]> | null = null;

/** Cache generation. Bumped on every `invalidateDriveCache` so a
 *  fetch that started before an invalidation can detect on resolve
 *  that its result is stale, and skip writing the cache. Without
 *  this guard, the eject button would race against the 30s-TTL
 *  background fetch: the older fetch's result could clobber the
 *  just-cleared cache and put the ejected drive back into the
 *  sidebar until the next external trigger.
 *
 *  Numeric — monotonically increasing for the life of the module.
 *  Wraparound is irrelevant on practical timescales (2^53 ejects). */
let cacheGeneration = 0;

/** Mounted hooks that should re-run their fetch effect after a cache
 *  invalidation. Module-level mutation isn't React-observable on its
 *  own, so each `useCachedDrives` consumer registers a small "wake"
 *  callback at mount; `invalidateDriveCache` calls them so the next
 *  render finds the cache empty and re-fetches. */
const invalidationSubscribers = new Set<() => void>();

/** Reset the module-level cache. Test-only escape hatch — production
 *  code should never need this. Exported so vitest cases can stage a
 *  clean slate between assertions. */
export function __resetDriveSpaceCacheForTests(): void {
  cache = null;
  inflight = null;
  cacheGeneration = 0;
  invalidationSubscribers.clear();
}

/** Test-only accessor for the current cache state. Used by the
 *  iter-17 race-fix test to assert the cache stays null after an
 *  invalidation even when an older fetch resolves afterwards. */
export function __peekCacheForTests(): {
  drives: ReadonlyArray<DriveInfoLike> | null;
  fetchedAt: number | null;
} {
  return cache === null
    ? { drives: null, fetchedAt: null }
    : { drives: cache.drives, fetchedAt: cache.fetchedAt };
}

/** Test-only seeder used by the iter-18 `refreshDriveCacheIfStale`
 *  test to assert the "fresh cache skips" half of the contract.
 *  Production code never seeds the cache directly — fetches happen
 *  only through `fetchDrives` so the in-flight + generation guards
 *  apply. */
export function __seedCacheForTests(
  drives: ReadonlyArray<DriveInfoLike>,
  fetchedAt: number,
): void {
  cache = { drives: drives.slice(), fetchedAt };
}

/** Force every mounted `useAllDrives` / `useDriveSpace` consumer to
 *  re-run the `detect_drives` IPC on the next render. Call this after
 *  an action that mutates the mount table — currently the sidebar's
 *  eject-drive button (iter 17), which would otherwise leave the
 *  just-ejected volume in the cached list for up to 30s.
 *
 *  Also clears the in-flight pointer (so any subsequent fetch starts
 *  a new IPC instead of awaiting the stale one) and bumps the cache
 *  generation (so the stale in-flight, when it resolves, knows to
 *  skip writing the cache). */
export function invalidateDriveCache(): void {
  cache = null;
  inflight = null;
  cacheGeneration += 1;
  for (const wake of invalidationSubscribers) wake();
}

/** Iter 18: opportunistic refresh trigger for attention-pull events
 *  (window focus, visibility change). Skips the invalidate when the
 *  cache is younger than `maxAgeMs` — so rapid Cmd+Tab toggling
 *  doesn't thrash the IPC. Returns `true` if an invalidate ran,
 *  `false` if it was skipped.
 *
 *  Distinct from `invalidateDriveCache` (which is unconditional)
 *  because the eject path and the focus path have different
 *  semantics: eject MUST refresh now (the mount table changed);
 *  focus only refreshes if the data is plausibly stale. Keeping
 *  the functions separate makes the call sites self-documenting. */
export function refreshDriveCacheIfStale(maxAgeMs: number): boolean {
  if (cache !== null && Date.now() - cache.fetchedAt < maxAgeMs) {
    return false;
  }
  invalidateDriveCache();
  return true;
}

async function fetchDrives(): Promise<DriveInfoLike[]> {
  if (!isTauriAvailable()) return [];
  // Coalesce concurrent fetches — the second caller awaits the first
  // caller's in-flight promise instead of spawning a duplicate IPC.
  if (inflight) return inflight;
  const startGeneration = cacheGeneration;
  const promise = tauriInvokeSafe<DriveInfoLike[]>("detect_drives", undefined, []);
  inflight = promise;
  try {
    const drives = await promise;
    // Only commit to cache if no invalidation happened during the fetch.
    // If `cacheGeneration` advanced, this fetch started before the most
    // recent invalidate and its result is stale — return it to the
    // immediate caller (their cancelled flag in the useEffect cleanup
    // will guard the setState) but don't poison the shared cache.
    if (cacheGeneration === startGeneration) {
      cache = { drives, fetchedAt: Date.now() };
    }
    return drives;
  } finally {
    // Only clear the in-flight pointer if we're still the active fetch
    // — invalidate may have reset `inflight` and a newer fetch may have
    // taken its place. Without this guard the older fetch's finally
    // would clear the newer fetch's pointer, breaking coalescing for
    // any caller that arrives between the clear and the next set.
    if (inflight === promise) inflight = null;
  }
}

/**
 * Resolve the drive that owns `path`. Returns `null` while loading,
 * if no drive matches, or outside Tauri.
 */
export function useDriveSpace(path: string): DriveInfoLike | null {
  const drives = useCachedDrives([path]);
  if (!path || drives.length === 0) return null;
  return findMountForPath(drives, path);
}

/**
 * Return the full cached drive list. Same TTL / in-flight semantics
 * as `useDriveSpace` — one IPC per 30 seconds across the whole app
 * regardless of how many components subscribe. Used by the sidebar
 * Devices section (iter 17) to render every mount with its used /
 * free bar.
 *
 * The hook also re-fetches when the cache is explicitly invalidated
 * via `invalidateDriveCache()` (e.g. after the user ejects a removable
 * drive), so the sidebar updates the moment the mount table changes.
 */
export function useAllDrives(): DriveInfoLike[] {
  return useCachedDrives([]);
}

/** Shared implementation behind `useDriveSpace` and `useAllDrives`.
 *  Hoisted so both hooks share the same TTL gate, the same in-flight
 *  guard, and (most importantly) the same cache slot — there is one
 *  drive list in the app, not two.
 *
 *  Re-fetches happen on three triggers: (a) the consumer's own deps
 *  change (path navigation for `useDriveSpace`), (b) cache invalidation
 *  via `invalidateDriveCache()` (eject button), or (c) initial mount
 *  with a stale / empty cache. A consumer-local `refetchKey` lifts (b)
 *  into React state so the fetch effect re-runs without piggybacking
 *  on render-driven re-execution. */
function useCachedDrives(extraDeps: ReadonlyArray<unknown>): DriveInfoLike[] {
  const [drives, setDrives] = useState<DriveInfoLike[]>(() => cache?.drives ?? []);
  const [refetchKey, setRefetchKey] = useState(0);

  useEffect(() => {
    const wake = () => setRefetchKey((n) => n + 1);
    invalidationSubscribers.add(wake);
    return () => {
      invalidationSubscribers.delete(wake);
    };
  }, []);

  useEffect(() => {
    let cancelled = false;
    // Snapshot the cache pointer into a local so TS narrowing
    // survives between the freshness check and the subsequent read.
    const snapshot = cache;
    if (snapshot !== null && Date.now() - snapshot.fetchedAt < CACHE_TTL_MS) {
      setDrives(snapshot.drives);
      return;
    }
    fetchDrives().then((next) => {
      if (cancelled) return;
      setDrives(next);
    });
    return () => {
      cancelled = true;
    };
    // `refetchKey` participates so an external cache invalidation wakes
    // the effect. `useDriveSpace` also passes `[path]` so cross-mount
    // navigation re-runs the fetch; `useAllDrives` passes [] and only
    // re-fetches on mount or invalidation.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [...extraDeps, refetchKey]);

  return drives;
}
