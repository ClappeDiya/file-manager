/**
 * path-recall
 *
 * Pure helper that turns a `FileLineage` payload into a single
 * actionable hint for the user when they navigate to a path that
 * no longer exists. Answers the universal file-manager question:
 *
 *   "Where did my file go?"
 *
 * # Design
 *
 * - **Pure**: takes a `FileLineage` (already a backend-fetched value)
 *   and the originally-requested path, returns either a hint or
 *   `null` if nothing useful can be said. No side effects, no IPC,
 *   no DOM access. The consuming component renders the hint.
 * - **DRY**: reuses the existing `FileLineage` type from
 *   `@/stores/lineage-store` and the already-registered
 *   `get_file_lineage` IPC. No new backend, no new schema.
 * - **Composable**: the same helper can power the failing-pane
 *   banner today and (without modification) a path-not-found toast,
 *   a CLI suggestion, or an admin-console hint tomorrow.
 *
 * # What counts as a useful hint
 *
 * A hint is produced only when:
 *   - The lineage has at least one event whose subject_path or
 *     target_path matches (case-insensitively) the original path —
 *     proving the ledger has actually seen this path before.
 *   - That event has a kind we know how to phrase (`move`, `rename`,
 *     `delete`, `copy`).
 *
 * The `alternateLocation`, when set, is the path the file now lives
 * at according to the *latest* move/rename event whose source was
 * the original path. It is `null` when the file was deleted, or when
 * the lineage shows only operations that don't relocate (e.g. copies
 * away to other places, leaving the original where it was).
 */
import { FS_INTENT_KIND } from "@/lib/fs-kinds";
import type { FileLineage, LineageEvent } from "@/stores/lineage-store";

export interface PathRecallInfo {
  /** ISO timestamp of the most recent lineage event touching this path. */
  lastSeenIso: string;
  /** The lineage event that produced this hint — used by the renderer
   *  to label the action ("moved", "renamed", "deleted", "copied"). */
  lastEvent: LineageEvent;
  /** Where the file *is now* if the latest event indicates it was
   *  relocated. `null` when the path was deleted, or when no event
   *  shows an alternate destination (e.g. the original was only the
   *  target of copies away from it). */
  alternateLocation: string | null;
  /** True when the most recent event suggests the file was destroyed
   *  (delete / purge). Helps the UI choose between "Jump to current
   *  location" and "File was deleted" copy. */
  wasDeleted: boolean;
}

/** Event kinds whose semantics we know how to phrase to the user.
 *  Anything else is reported by its raw kind. Kept narrow to avoid
 *  inventing wrong-sounding language for engine-specific kinds. */
const RELOCATION_KINDS: ReadonlySet<string> = new Set([
  FS_INTENT_KIND.move,
  "rename",
]);
const DELETION_KINDS: ReadonlySet<string> = new Set([
  FS_INTENT_KIND.deleteTrash,
  FS_INTENT_KIND.deletePermanent,
]);

/** True when the event represents a *successful* state change on
 *  disk that we can confidently report. Failed/cancelled events are
 *  history, not state. */
function isStateChange(event: LineageEvent): boolean {
  return event.status === "ok";
}

/** Case-insensitive equality. Paths from the ledger come pre-
 *  normalised, so this is mostly defensive; macOS HFS+ folds case
 *  by default and Windows is case-insensitive everywhere. */
function samePath(a: string | null | undefined, b: string): boolean {
  if (!a) return false;
  return a.toLowerCase() === b.toLowerCase();
}

/**
 * Compute a single PathRecallInfo for the given path from the
 * lineage payload, or return `null` when no useful hint exists.
 *
 * The lineage is expected to be the result of a `get_file_lineage`
 * IPC call rooted at the same path. Events are assumed newest-first
 * (as the backend produces them via `OperationLedger::query`).
 */
export function inferPathRecall(
  lineage: FileLineage | null,
  originalPath: string,
): PathRecallInfo | null {
  if (!lineage || lineage.events.length === 0) return null;

  // The most recent state-change event that names the original path
  // as either subject or target. We rely on the backend's DESC sort.
  const lastTouchingEvent = lineage.events.find(
    (e) =>
      isStateChange(e) &&
      (samePath(e.subject_path, originalPath) ||
        samePath(e.target_path, originalPath)),
  );
  if (!lastTouchingEvent) return null;

  const wasDeleted =
    DELETION_KINDS.has(lastTouchingEvent.kind) &&
    samePath(lastTouchingEvent.subject_path, originalPath);

  // For relocation events, "alternate location" is the target IF the
  // event moved/renamed FROM the original path. If the most recent
  // event TARGETED the original path (e.g. a copy that put a file
  // here that later vanished), there's no alternate to suggest.
  let alternateLocation: string | null = null;
  if (
    RELOCATION_KINDS.has(lastTouchingEvent.kind) &&
    samePath(lastTouchingEvent.subject_path, originalPath) &&
    lastTouchingEvent.target_path &&
    !samePath(lastTouchingEvent.target_path, originalPath)
  ) {
    alternateLocation = lastTouchingEvent.target_path;
  }

  return {
    lastSeenIso: lastTouchingEvent.occurred_at,
    lastEvent: lastTouchingEvent,
    alternateLocation,
    wasDeleted,
  };
}

/** Produce a short human-readable phrase for the hint, e.g.
 *  "moved to /Users/x/Documents". Kept here (not in the component) so
 *  every consumer phrases the same situation identically. */
export function describePathRecall(info: PathRecallInfo): string {
  const kind = info.lastEvent.kind;
  // Trash and permanent delete are separate kinds precisely because one is
  // recoverable and the other is not; that is the single most useful thing to
  // tell someone hunting for a file, so don't flatten it to "was deleted".
  if (info.wasDeleted) {
    return kind === FS_INTENT_KIND.deleteTrash
      ? "was moved to the Trash"
      : "was permanently deleted";
  }
  if (RELOCATION_KINDS.has(kind) && info.alternateLocation) {
    const verb = kind === "rename" ? "renamed to" : "moved to";
    return `was ${verb} ${info.alternateLocation}`;
  }
  if (kind === "copy") return "was last copied from here";
  return `last ${kind} event recorded`;
}
