/**
 * Command palette MRU / frecency (iter 24).
 *
 * Tracks usage of palette commands in localStorage so the empty-query
 * list can surface the user's favorites at the top instead of the
 * declaration-order list the palette shipped with. After 17 iterations
 * the palette has 80+ entries; a user whose muscle-memory maps
 * ⌘K → "Swap Panes" shouldn't have to scroll past 40 commands to
 * reach it.
 *
 * Design:
 *
 * - **Frecency over pure recency**: sorting by `(useCount DESC,
 *   lastUsedAt DESC)` protects against one-off clicks promoting
 *   a rarely-used command to the top. Frequent commands stay pinned;
 *   recency only breaks ties between commands with the same count.
 *   This matches the `ledger_frecent_paths` Rust helper's philosophy
 *   (commit 4d7793e) — different storage, same principle.
 *
 * - **Pure helpers + localStorage**: no Zustand slice, no new IPC,
 *   no new persistence infrastructure. Browsers + Tauri both expose
 *   `localStorage`, so the store works in every environment the
 *   palette renders in.
 *
 * - **Fail-soft**: any localStorage error (quota, disabled, parse
 *   failure) returns an empty list. The palette falls back to its
 *   original declaration-order behavior — worst case, the user
 *   gets iter-23 behavior.
 *
 * - **Bounded**: cap at `MAX_TRACKED_COMMANDS` (50) to keep the
 *   localStorage footprint trivial and evict stale commands that
 *   the user never touches again.
 *
 * - **Pure functions**: `applyMruOrdering` takes the command list
 *   and the usage record as inputs and returns a new sorted list.
 *   Unit-testable without mocking React or Tauri.
 */
import type { CommandItem } from "@/components/command-palette";

/** Storage key for the MRU record. Namespaced so the `ufop-` prefix
 *  matches the project's other localStorage conventions. */
export const MRU_STORAGE_KEY = "ufop-command-mru-v1";

/** Maximum number of commands to track. 50 covers the user's
 *  entire working set comfortably; beyond that, the least-recent
 *  entry is evicted on write. */
export const MAX_TRACKED_COMMANDS = 50;

/** Usage record for a single command. Stored as the shape of each
 *  entry in the localStorage-persisted `commandId → usage` map. */
export interface CommandUsage {
  /** The palette command's `id` field, used as the map key. */
  commandId: string;
  /** Monotonically increasing use counter. Drives the primary
   *  sort dimension so frequent commands win over one-off uses. */
  useCount: number;
  /** Millisecond timestamp (`Date.now()`) of the most recent use.
   *  Drives the tiebreaker sort dimension. */
  lastUsedAt: number;
}

/** Shape persisted to localStorage: a plain map from commandId to
 *  `CommandUsage` record. Map form (rather than array) makes
 *  `recordCommandUsage` O(1) for the common "already tracked" case. */
type CommandUsageMap = Record<string, CommandUsage>;

/**
 * Read the persisted usage map from localStorage. Returns an empty
 * map on any failure (missing key, parse error, quota issue,
 * localStorage disabled in a sandboxed context). The palette then
 * falls back to its original declaration-order behavior \u2014 iter-24
 * degrades cleanly into iter-23.
 */
export function readCommandUsage(): CommandUsageMap {
  try {
    if (typeof localStorage === "undefined") return {};
    const raw = localStorage.getItem(MRU_STORAGE_KEY);
    if (!raw) return {};
    const parsed = JSON.parse(raw);
    // Defensive: ensure the parsed value is a plain object whose
    // entries conform to the expected shape. A corrupt entry from
    // a future schema change should not crash the palette.
    if (typeof parsed !== "object" || parsed === null) return {};
    const clean: CommandUsageMap = {};
    for (const [id, entry] of Object.entries(parsed)) {
      if (
        entry &&
        typeof entry === "object" &&
        typeof (entry as CommandUsage).useCount === "number" &&
        typeof (entry as CommandUsage).lastUsedAt === "number"
      ) {
        clean[id] = {
          commandId: id,
          useCount: (entry as CommandUsage).useCount,
          lastUsedAt: (entry as CommandUsage).lastUsedAt,
        };
      }
    }
    return clean;
  } catch {
    return {};
  }
}

/**
 * Record that `commandId` was just executed. Increments its
 * `useCount`, updates `lastUsedAt` to the given `now`, and evicts
 * the least-recently-used entry if the map would exceed
 * `MAX_TRACKED_COMMANDS`. Fail-soft on any localStorage error.
 *
 * `now` is parameterised so tests can supply a deterministic clock.
 */
export function recordCommandUsage(
  commandId: string,
  now: number = Date.now(),
): void {
  try {
    if (typeof localStorage === "undefined") return;
    const map = readCommandUsage();
    const existing = map[commandId];
    map[commandId] = {
      commandId,
      useCount: (existing?.useCount ?? 0) + 1,
      lastUsedAt: now,
    };
    // If we're over the cap, evict the single LEAST-recently-used
    // entry. We compare on `lastUsedAt` rather than `useCount` so a
    // dormant-but-once-popular command doesn't evict a fresh one.
    const entries = Object.values(map);
    if (entries.length > MAX_TRACKED_COMMANDS) {
      entries.sort((a, b) => a.lastUsedAt - b.lastUsedAt);
      const evictCount = entries.length - MAX_TRACKED_COMMANDS;
      for (let i = 0; i < evictCount; i++) {
        delete map[entries[i].commandId];
      }
    }
    localStorage.setItem(MRU_STORAGE_KEY, JSON.stringify(map));
  } catch {
    // Silent failure: MRU is an enhancement, never a requirement.
  }
}

/**
 * Re-order `commands` so that previously-used commands (present in
 * `usage`) appear at the top, sorted by frecency: useCount DESC,
 * then lastUsedAt DESC. Unused commands follow in their original
 * declaration order.
 *
 * Pure function: no localStorage access, no mutation of the input.
 * Testable in isolation from `readCommandUsage` / `recordCommandUsage`.
 *
 * Returns a NEW array so React's referential equality comparisons
 * correctly re-render the palette when the usage map changes.
 */
export function applyMruOrdering(
  commands: CommandItem[],
  usage: CommandUsageMap,
): CommandItem[] {
  // Nothing to do when the usage map is empty \u2014 preserve iter-23
  // behavior exactly (declaration order).
  if (Object.keys(usage).length === 0) {
    return commands;
  }
  const known: CommandItem[] = [];
  const unknown: CommandItem[] = [];
  for (const cmd of commands) {
    if (usage[cmd.id]) {
      known.push(cmd);
    } else {
      unknown.push(cmd);
    }
  }
  // Sort the known bucket by frecency: frequency primary, recency
  // as the tiebreaker. Ties further broken by stable input order.
  known.sort((a, b) => {
    const ua = usage[a.id]!;
    const ub = usage[b.id]!;
    if (ub.useCount !== ua.useCount) return ub.useCount - ua.useCount;
    return ub.lastUsedAt - ua.lastUsedAt;
  });
  return [...known, ...unknown];
}
