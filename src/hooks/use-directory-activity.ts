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
import { tauriInvokeSafe } from "./use-tauri";

/** Wire-format mirror of `LedgerPathHit` in `src-tauri/src/ledger/mod.rs`. */
interface LedgerPathHit {
  path: string;
  last_seen: string;
  hit_count: number;
}

/** Age bucket frozen at the moment the directory was fetched. Frozen
 *  deliberately so the dot component can be a pure function of its
 *  props — React-compiler-friendly and eliminates per-row `Date.now()`
 *  during render. The user re-navigating the directory re-fetches and
 *  re-buckets naturally. */
export type ActivityAgeBucket = "recent" | "today" | "week";

export interface FileActivityInfo {
  /** ISO-like timestamp (`YYYY-MM-DD HH:MM:SS` or RFC-3339). */
  lastSeen: string;
  /** Number of ledger rows that touched this path. */
  hitCount: number;
  /** Pre-bucketed age computed when the directory was fetched. */
  ageBucket: ActivityAgeBucket;
  /** Pre-formatted relative-age label (e.g. "12m", "3h", "2d"). */
  ageLabel: string;
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

/** Compute the age bucket + label for a ledger timestamp, anchored to a
 *  caller-supplied `now`. Keeps the bucketing deterministic relative to
 *  a single fetch snapshot so downstream components don't need to call
 *  `Date.now()` during render. */
function bucketAge(
  lastSeen: string,
  now: number,
): { ageBucket: ActivityAgeBucket; ageLabel: string } {
  const parsed = new Date(
    lastSeen.includes("T") ? lastSeen : `${lastSeen.replace(" ", "T")}Z`,
  );
  const validMs = Number.isNaN(parsed.getTime()) ? now : parsed.getTime();
  const ageMin = Math.max(Math.round((now - validMs) / 60000), 0);
  if (ageMin < 60) {
    return {
      ageBucket: "recent",
      ageLabel: ageMin <= 1 ? "just now" : `${ageMin}m ago`,
    };
  }
  if (ageMin < 24 * 60) {
    return { ageBucket: "today", ageLabel: `${Math.round(ageMin / 60)}h ago` };
  }
  return {
    ageBucket: "week",
    ageLabel: `${Math.round(ageMin / (60 * 24))}d ago`,
  };
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
