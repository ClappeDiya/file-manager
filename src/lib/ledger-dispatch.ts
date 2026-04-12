/**
 * ledger-dispatch — smart-open routing shared between every surface
 * that hands the user a ledger-sourced path and wants the "right
 * thing" to happen (Instant Jump → Enter, FilePane → double-click,
 * iter 42's Cmd+Shift+L "Jump to Last Touched", future palette
 * actions …).
 *
 * The routing logic used to live inline in PathJumpDialog.navigateTo
 * and FilePane.handleOpen in parallel. Iter 38 shared the extension
 * predicates at module scope; iter 42 lifts the entire dispatch core
 * so any new surface can route by calling one function and passing
 * the same five optional callbacks — archive, text, preview, nav,
 * and the three predicate functions. DRY, no duplication, no drift
 * between call sites.
 *
 * Routing rules (in order):
 *   1. metadata.is_dir        → onNavigateDir(path)
 *   2. archive extension      → onOpenArchive(path)
 *   3. text extension ≤1 MB   → onOpenFile(path, size)
 *   4. previewable extension  → onOpenPreview(path)
 *   5. otherwise (any file)   → onNavigateDir(parent)   // reveal
 *   6. metadata failed AND no extension hint → onNavigateDir(path)
 *
 * Metadata IPC failure is fail-soft: any rule that needs a size
 * falls back to the extension predicate, and when nothing matches
 * we preserve the pre-iter-37 behaviour of handing the literal
 * path to `onNavigateDir`.
 */
import { tauriInvokeSafe } from "@/hooks/use-tauri";

interface MetaPartial {
  is_dir: boolean;
  size: number;
}

export interface LedgerDispatchHandlers {
  isArchivePath?: (path: string) => boolean;
  isPlainTextPath?: (path: string) => boolean;
  isPreviewablePath?: (path: string) => boolean;
  onOpenArchive?: (path: string) => void;
  onOpenFile?: (path: string, size: number) => void;
  onOpenPreview?: (path: string) => void;
  onNavigateDir: (path: string) => void;
}

/** Matches the iter-32 TEXT_EDITOR_MAX_BYTES constant in the text
 *  editor modal and in fs_commands.rs. Kept in sync by doc-comment. */
const TEXT_EDITOR_MAX_BYTES = 1024 * 1024;

/** Last path segment of a forward- or backslash-separated path.
 *  Shared with PathJumpDialog.doNavigateTab — both surfaces want
 *  the same breadcrumb label when calling file-manager-store
 *  `navigateTab`. */
export function deriveLabel(path: string): string {
  const parts = path.split(/[/\\]/).filter(Boolean);
  return parts[parts.length - 1] ?? path;
}

/** Iter 44: humanize a ledger `kind` column into a short verb phrase
 *  plus a Tailwind tone class. Originally lived in path-jump-dialog
 *  (iter 40) as the label source for the Instant Jump kind badges.
 *  Iter 44 lifts it into the shared ledger-dispatch module so the
 *  file-manager's Jump Ring "operation preview toast" can use the
 *  same verb vocabulary — one source of truth across every surface
 *  that translates kinds for humans. Unknown kinds pass through
 *  with underscores replaced by spaces, so any future backend-
 *  introduced kind still renders without a regression. */
export function labelForKind(kind: string): { label: string; tone: string } {
  switch (kind) {
    case "edit_text":
      return { label: "edited", tone: "bg-amber-500/15 text-amber-400" };
    case "copy":
      return { label: "copied", tone: "bg-sky-500/15 text-sky-400" };
    case "move":
      return { label: "moved", tone: "bg-violet-500/15 text-violet-400" };
    case "rename":
      return { label: "renamed", tone: "bg-violet-500/15 text-violet-400" };
    case "duplicate":
      return { label: "duplicated", tone: "bg-sky-500/15 text-sky-400" };
    case "delete":
      return { label: "deleted", tone: "bg-rose-500/15 text-rose-400" };
    case "create_file":
    case "create_folder":
      return { label: "created", tone: "bg-emerald-500/15 text-emerald-400" };
    case "transfer_ok":
    case "transfer":
      return { label: "transferred", tone: "bg-cyan-500/15 text-cyan-400" };
    case "sync_ok":
    case "sync":
      return { label: "synced", tone: "bg-cyan-500/15 text-cyan-400" };
    default:
      return {
        label: kind.replace(/_/g, " "),
        tone: "bg-white/10 text-[color:var(--color-text-muted)]",
      };
  }
}

export type RelativeTimeStyle =
  | "compact"
  | "withSuffix"
  | "withJustNow"
  | "prose";

export function formatRelativeTime(
  iso: string,
  now?: number,
  style: RelativeTimeStyle = "compact",
): string {
  const resolvedNow = now ?? Date.now();
  const d = new Date(iso.includes("T") ? iso : `${iso.replace(" ", "T")}Z`);
  if (Number.isNaN(d.getTime())) return iso;
  const diffMs = resolvedNow - d.getTime();
  const diffSec = Math.round(diffMs / 1000);

  if (style === "prose") {
    if (diffSec < 60) return "Just now";
    const mins = Math.floor(diffSec / 60);
    if (mins < 60)
      return `${mins} minute${mins !== 1 ? "s" : ""} ago`;
    const hours = Math.floor(mins / 60);
    if (hours < 24)
      return `${hours} hour${hours !== 1 ? "s" : ""} ago`;
    const days = Math.floor(hours / 24);
    if (days === 1) return "Yesterday";
    if (days < 7) return `${days} days ago`;
    return d.toLocaleDateString();
  }

  if (style === "withJustNow" && diffSec < 5) return "just now";

  const suffix = style === "compact" ? "" : " ago";

  if (diffSec < 60) return `${diffSec}s${suffix}`;
  const diffMin = Math.round(diffSec / 60);
  if (diffMin < 60) return `${diffMin}m${suffix}`;
  const diffHr = Math.round(diffMin / 60);
  if (diffHr < 24) return `${diffHr}h${suffix}`;
  const diffDay = Math.round(diffHr / 24);
  return `${diffDay}d${suffix}`;
}

export async function dispatchLedgerPath(
  path: string,
  handlers: LedgerDispatchHandlers,
): Promise<void> {
  const looksLikeArchive = handlers.isArchivePath?.(path) ?? false;
  const looksLikeText = handlers.isPlainTextPath?.(path) ?? false;
  const looksLikePreviewable = handlers.isPreviewablePath?.(path) ?? false;

  let meta: MetaPartial | null = null;
  try {
    meta = await tauriInvokeSafe<MetaPartial | null>(
      "get_file_metadata",
      { path },
      null,
    );
  } catch {
    meta = null;
  }

  if (meta && meta.is_dir) {
    handlers.onNavigateDir(path);
    return;
  }

  if (meta !== null || looksLikeArchive || looksLikeText || looksLikePreviewable) {
    if (looksLikeArchive && handlers.onOpenArchive) {
      handlers.onOpenArchive(path);
      return;
    }
    if (
      looksLikeText &&
      handlers.onOpenFile &&
      (meta?.size ?? 0) <= TEXT_EDITOR_MAX_BYTES
    ) {
      handlers.onOpenFile(path, meta?.size ?? 0);
      return;
    }
    if (looksLikePreviewable && handlers.onOpenPreview) {
      handlers.onOpenPreview(path);
      return;
    }
    // Fallback: reveal the file by navigating to its parent.
    const parent = path.replace(/\/[^/]+$/, "") || "/";
    handlers.onNavigateDir(parent);
    return;
  }

  // Metadata IPC failed AND extension gave no hint — preserve
  // pre-iter-37 behaviour so nothing worse than the legacy path
  // is possible.
  handlers.onNavigateDir(path);
}

// ──────────────────────────────────────────────────────────────
// Iter 45: Jump Ring decision core
// ──────────────────────────────────────────────────────────────

/** Mutable state of a Jump Ring. The host owns one of these and
 *  passes it into `pickJumpRingTarget` on each press. The function
 *  returns a fresh `nextRing` snapshot; the caller commits it by
 *  mutating its own copy in place. Decoupling state from logic
 *  this way keeps `pickJumpRingTarget` pure + testable. */
export interface JumpRingState {
  index: number;
  lastPressTime: number;
  distinctPaths: string[];
}

export type JumpRingDirection = "forward" | "reverse";

export interface JumpRingOutcome {
  /** Chosen target path; empty string when `hits` is empty (callers
   *  must guard against empty hits before invoking this function). */
  target: string;
  /** 1-indexed position of the target within `ringSize`. */
  ringPosition: number;
  ringSize: number;
  nextRing: JumpRingState;
}

/** Cycling window in milliseconds. Rapid re-presses within this
 *  window advance the ring; presses after it reset to the head.
 *  Shared constant so tests can reference the exact value. */
export const JUMP_RING_CYCLE_WINDOW_MS = 2500;

/** Pure, deterministic ring-picker. Given the current ledger hit
 *  list, the user's current folder path, the previous ring state,
 *  and a direction, returns the target to dispatch and the next
 *  ring state. Side-effect free: the caller commits `nextRing`.
 *
 *  Extracted from file-manager.tsx's `handleJumpLastTouched` in
 *  iter 45 so the ring logic is:
 *    1. Testable in isolation (no React, no Tauri, no DOM).
 *    2. Reusable across forward + reverse callers.
 *    3. Unambiguous about what state transitions are legal.
 *
 *  Decision flow (in order):
 *    - **No hits**         → empty target, ring reset (caller
 *                             should show "no recent paths" toast).
 *    - **No distinct**     → every top-N hit equals `currentPath`;
 *                             fall back to the raw first hit so
 *                             the keystroke is never silent.
 *    - **Same-ring cycle** → if `now - lastPressTime` is within
 *                             the cycle window AND the new distinct
 *                             list still starts with the same head
 *                             as the previous ring (no stale ledger
 *                             event has reshuffled the top), advance
 *                             (or retreat) the index with a modular
 *                             wrap. Direction "reverse" subtracts.
 *    - **Reset**           → fresh first press; return `distinct[0]`
 *                             at position 1.
 *
 *  The `now` and `cycleWindowMs` parameters are injectable so tests
 *  can pin the clock and test boundary behaviour without timers. */
export function pickJumpRingTarget(
  hits: readonly { path: string }[],
  currentPath: string,
  ring: Readonly<JumpRingState>,
  direction: JumpRingDirection = "forward",
  now: number = Date.now(),
  cycleWindowMs: number = JUMP_RING_CYCLE_WINDOW_MS,
): JumpRingOutcome {
  if (hits.length === 0) {
    return {
      target: "",
      ringPosition: 0,
      ringSize: 0,
      nextRing: { index: 0, lastPressTime: now, distinctPaths: [] },
    };
  }

  // Build distinct-by-path hits, excluding the current folder
  // path. A Set dedupes so a file touched N times back-to-back
  // only occupies one ring slot.
  const distinct: string[] = [];
  const seen = new Set<string>();
  for (const h of hits) {
    if (h.path === currentPath) continue;
    if (seen.has(h.path)) continue;
    seen.add(h.path);
    distinct.push(h.path);
  }

  // Fallback: every hit is the current path (extremely rare —
  // requires 25 consecutive same-path events). Dispatch the
  // literal first hit to preserve iter 42's pre-ring semantics.
  if (distinct.length === 0) {
    return {
      target: hits[0].path,
      ringPosition: 1,
      ringSize: 1,
      nextRing: { index: 0, lastPressTime: now, distinctPaths: [] },
    };
  }

  const sameRing =
    ring.distinctPaths.length > 0 &&
    ring.distinctPaths[0] === distinct[0];

  if (now - ring.lastPressTime < cycleWindowMs && sameRing) {
    const delta = direction === "reverse" ? -1 : 1;
    // Modular wrap. Adding `distinct.length` keeps the result
    // non-negative even when `delta === -1 && ring.index === 0`.
    const nextIdx = (ring.index + delta + distinct.length) % distinct.length;
    return {
      target: distinct[nextIdx],
      ringPosition: nextIdx + 1,
      ringSize: distinct.length,
      nextRing: {
        index: nextIdx,
        lastPressTime: now,
        distinctPaths: distinct,
      },
    };
  }

  // Fresh press: reset to the head.
  return {
    target: distinct[0],
    ringPosition: 1,
    ringSize: distinct.length,
    nextRing: {
      index: 0,
      lastPressTime: now,
      distinctPaths: distinct,
    },
  };
}
