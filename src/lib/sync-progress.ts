/**
 * Live sync progress — types and pure helpers.
 * Mirrors `SyncProgress` in src-tauri/src/sync_engine/executor.rs.
 */

export interface SyncProgress {
  pair_id: string;
  run_id: string;
  /** "planning" | "copying" | "finishing" */
  phase: string;
  files_done: number;
  files_total: number;
  bytes_done: number;
  bytes_total: number;
  current_file: string | null;
  started_at: string;
  cancel_requested: boolean;
}

/** Percent complete (0–100), by bytes when there are any, else by files. */
export function progressPercent(p: SyncProgress): number {
  const [done, total] =
    p.bytes_total > 0 ? [p.bytes_done, p.bytes_total] : [p.files_done, p.files_total];
  if (total <= 0) return p.phase === "finishing" ? 100 : 0;
  return Math.min(100, Math.round((done / total) * 100));
}

/** Rough time left, or null until there's enough signal to estimate. */
export function estimateSecondsLeft(p: SyncProgress, now: number = Date.now()): number | null {
  const elapsed = (now - new Date(p.started_at).getTime()) / 1000;
  const pct = progressPercent(p);
  if (p.phase !== "copying" || elapsed < 2 || pct <= 0 || pct >= 100) return null;
  return Math.round((elapsed * (100 - pct)) / pct);
}

export function formatSecondsLeft(s: number | null): string {
  if (s === null) return "";
  if (s < 60) return `about ${Math.max(1, s)}s left`;
  if (s < 3600) return `about ${Math.round(s / 60)} min left`;
  return `about ${(s / 3600).toFixed(1)} h left`;
}

export function phaseLabel(p: SyncProgress): string {
  if (p.cancel_requested) return "Stopping after the current file…";
  if (p.phase === "planning") return "Comparing folders…";
  if (p.phase === "finishing") return "Finishing up…";
  return `Copying ${p.files_done} of ${p.files_total}`;
}
