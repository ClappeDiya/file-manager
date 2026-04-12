/**
 * useDirectoryActivity
 *
 * Fetches the set of recent ledger activity for files under a given
 * directory, and returns it as a Map keyed by full path. Powers the
 * inline **activity dots** rendered in the file list — so users can
 * see at a glance which files have recent ledger events without
 * opening the Activity Timeline panel.
 *
 * Design principles applied here:
 *
 * - **Pull, not push**: fires once when the directory changes. No
 *   polling, no subscriptions, no background work. If the user sits in
 *   the same directory, nothing refetches.
 * - **Cheap**: one `ledger_directory_activity` IPC call per directory
 *   navigation. The backend returns up to 1000 distinct paths via a
 *   single SQL scan — bounded by the ledger's own 30-day retention.
 * - **Fail-soft**: outside Tauri (pnpm dev preview) returns an empty
 *   map via `tauriInvokeSafe`, so the file list simply renders without
 *   dots. Never crashes the pane.
 * - **Time-bounded**: requests only events from the last 7 days so the
 *   dots reflect *recent* activity, not ancient history. The backend
 *   still caps at its own 30-day retention.
 */
import { useEffect, useState } from "react";
import { isTauriAvailable, tauriInvokeSafe } from "./use-tauri";
import { formatRelativeTime } from "@/lib/ledger-dispatch";

/** Wire-format mirror of `LedgerPathHit` in `src-tauri/src/ledger/mod.rs`. */
interface LedgerPathHit {
  path: string;
  last_seen: string;
  hit_count: number;
  last_engine: string | null;
}

/** Age bucket frozen at the moment the directory was fetched. Frozen
 *  deliberately so the dot component can be a pure function of its
 *  props — React-compiler-friendly and eliminates per-row `Date.now()`
 *  during render. The user re-navigating the directory re-fetches and
 *  re-buckets naturally. */
export type ActivityAgeBucket = "recent" | "today" | "week";

/** Engine that most recently touched a file — matches `LedgerEngine`
 *  string identifiers in the Rust backend. */
export type LedgerEngineKind =
  | "fs"
  | "transfer"
  | "sync"
  | "automation"
  | "mount"
  | "spaces"
  | "ai"
  | "vault"
  | "compat"
  | "connector"
  | "system";

export interface FileActivityInfo {
  /** ISO-like timestamp (`YYYY-MM-DD HH:MM:SS` or RFC-3339). */
  lastSeen: string;
  /** Number of ledger rows that touched this path. */
  hitCount: number;
  /** Pre-bucketed age computed when the directory was fetched. */
  ageBucket: ActivityAgeBucket;
  /** Pre-formatted relative-age label (e.g. "12m", "3h", "2d"). */
  ageLabel: string;
  /** Engine that most recently touched this path (e.g. "sync", "transfer"). */
  lastEngine: LedgerEngineKind | null;
}

/** Activity lookup — zero allocation per-row if the map is empty. */
export type FileActivityMap = Record<string, FileActivityInfo>;

/** How far back to include events. 7 days is enough to surface "recent"
 *  activity without drowning the UI in month-old events, and well below
 *  the ledger's 30-day retention ceiling. */
const SINCE_DAYS = 7;

function sevenDaysAgoIso(): string {
  return new Date(Date.now() - SINCE_DAYS * 24 * 60 * 60 * 1000).toISOString();
}

function bucketAge(
  lastSeen: string,
  now: number,
): { ageBucket: ActivityAgeBucket; ageLabel: string } {
  const parsed = new Date(
    lastSeen.includes("T") ? lastSeen : `${lastSeen.replace(" ", "T")}Z`,
  );
  const validMs = Number.isNaN(parsed.getTime()) ? now : parsed.getTime();
  const ageMin = Math.max(Math.round((now - validMs) / 60000), 0);
  const ageLabel =
    ageMin <= 1
      ? "just now"
      : formatRelativeTime(lastSeen, now, "withSuffix");
  if (ageMin < 60) return { ageBucket: "recent", ageLabel };
  if (ageMin < 24 * 60) return { ageBucket: "today", ageLabel };
  return { ageBucket: "week", ageLabel };
}

/** Number of days the **Stale Files Filter** chip considers when
 *  deciding whether a file has been "forgotten". 30 days strikes the
 *  right balance: long enough that genuinely-active files never get
 *  flagged, short enough that the user sees a meaningful cleanup
 *  opportunity in heavily-used folders. Lives next to `SINCE_DAYS` so
 *  the two windows are visually adjacent and easy to keep in sync. */
export const STALE_SINCE_DAYS = 30;

function nDaysAgoIso(days: number): string {
  return new Date(Date.now() - days * 24 * 60 * 60 * 1000).toISOString();
}

/** Result returned by [`useRecentlyTouchedSet`]. The `loaded` flag is
 *  the critical safety bit: it discriminates "no paths touched"
 *  (fetch completed and ledger really is empty) from "fetch hasn't
 *  finished yet" / "running outside Tauri". Consumers MUST gate any
 *  inverse computation (e.g. "X files are stale") on `loaded` —
 *  otherwise every file flashes as stale during the brief loading
 *  window or in browser preview mode. */
export interface RecentlyTouchedResult {
  /** Paths that the ledger has seen in the staleness window. Empty
   *  set when `loaded === false`. */
  touched: Set<string>;
  /** True once the backend has answered at least once for this
   *  directory. False outside Tauri (the fetch fail-soft path never
   *  resolves to "loaded with real data") and during the initial
   *  fetch. */
  loaded: boolean;
}

/**
 * Returns the Set of paths under `dirPath` that have at least one
 * ledger event within the last `sinceDays` days. Powers the
 * **Stale Files Filter** chip in the FilterBar — the consumer flips
 * this set to compute the *inverse* (files with no recent activity).
 *
 * Why a separate hook from `useDirectoryActivity`?
 *
 * - **Different time window**: 30 days vs 7 days. The 7-day signal
 *   drives the inline activity dots; the 30-day signal drives the
 *   cleanup chip. Conflating them would force a single window that's
 *   wrong for at least one consumer.
 * - **Different shape**: callers only need membership-test ("is this
 *   path stale?"), not the full per-path metadata. Returning a `Set`
 *   keeps the consumer side O(1) per file with zero allocation noise.
 * - **DRY**: shares the existing `ledger_directory_activity` Tauri
 *   command — no new IPC, no new SQL, no new infrastructure.
 *
 * The returned [`RecentlyTouchedResult`] carries a `loaded` flag so
 * consumers can suppress the chip count entirely until the first
 * fetch completes (avoiding the "every file flashes as stale during
 * loading" pitfall). Outside Tauri the hook stays in `loaded: false`
 * forever, which is what we want — without the ledger we genuinely
 * cannot tell what's stale.
 */
export function useRecentlyTouchedSet(
  dirPath: string | null,
  sinceDays: number = STALE_SINCE_DAYS,
): RecentlyTouchedResult {
  const [result, setResult] = useState<RecentlyTouchedResult>(() => ({
    touched: new Set<string>(),
    loaded: false,
  }));

  useEffect(() => {
    if (!dirPath) {
      setResult({ touched: new Set(), loaded: false });
      return;
    }
    // Reset to "loading" on every directory change so the chip
    // disappears immediately and reappears only after the new
    // directory's data arrives.
    setResult({ touched: new Set(), loaded: false });
    let cancelled = false;
    (async () => {
      // Only consider the result authoritative when we're inside
      // Tauri AND the call returned successfully. Outside Tauri,
      // `tauriInvokeSafe` resolves to the fallback ([]) immediately
      // — but that "empty list" is meaningless for staleness, so we
      // refuse to mark the result as loaded.
      if (!isTauriAvailable()) {
        return;
      }
      const hits = await tauriInvokeSafe<LedgerPathHit[]>(
        "ledger_directory_activity",
        {
          dirPath,
          sinceIso: nDaysAgoIso(sinceDays),
          limit: 1000,
        },
        [],
      );
      if (cancelled) return;
      const next = new Set<string>();
      for (const hit of hits) next.add(hit.path);
      setResult({ touched: next, loaded: true });
    })();
    return () => {
      cancelled = true;
    };
  }, [dirPath, sinceDays]);

  return result;
}

/**
 * Returns a Record<path, FileActivityInfo> covering recent ledger
 * activity under `dirPath`. Returns an empty object outside Tauri or
 * while the initial fetch is in flight, so callers can spread-safely.
 */
export function useDirectoryActivity(dirPath: string | null): FileActivityMap {
  const [map, setMap] = useState<FileActivityMap>({});

  useEffect(() => {
    if (!dirPath) {
      setMap({});
      return;
    }
    let cancelled = false;
    (async () => {
      const hits = await tauriInvokeSafe<LedgerPathHit[]>(
        "ledger_directory_activity",
        {
          dirPath,
          sinceIso: sevenDaysAgoIso(),
          limit: 1000,
        },
        [],
      );
      if (cancelled) return;
      // Snapshot `now` exactly once per fetch so every entry's age
      // bucket is computed against the same anchor. Passing the anchor
      // here — rather than calling `Date.now()` in the render path —
      // keeps the consuming ActivityDot component pure.
      const now = Date.now();
      const next: FileActivityMap = {};
      for (const hit of hits) {
        const { ageBucket, ageLabel } = bucketAge(hit.last_seen, now);
        next[hit.path] = {
          lastSeen: hit.last_seen,
          hitCount: hit.hit_count,
          ageBucket,
          ageLabel,
          lastEngine: (hit.last_engine as LedgerEngineKind) ?? null,
        };
      }
      setMap(next);
    })();
    return () => {
      cancelled = true;
    };
  }, [dirPath]);

  return map;
}
