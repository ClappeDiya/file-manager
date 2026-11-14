/**
 * "Next run" of scheduled sync pairs and automation rules. One backend call
 * (`upcoming_runs`) lists every armed schedule with its next time, days off
 * already skipped; the panels pick their own entries out by id.
 */
import { useCallback, useEffect, useState } from "react";
import { tauriInvokeSafe } from "@/hooks/use-tauri";

/** `schedule_library::upcoming::UpcomingRun`: one armed schedule and its next run. */
export interface UpcomingRun {
  kind: "sync" | "automation";
  id: string;
  name: string;
  /** RFC 3339 */
  next_run_at: string;
}

export async function fetchUpcomingRuns(): Promise<UpcomingRun[]> {
  return (await tauriInvokeSafe<UpcomingRun[]>("upcoming_runs", undefined, [])) ?? [];
}

/** Next runs of one kind by id. */
export function nextRunsById(runs: UpcomingRun[], kind: UpcomingRun["kind"]): Record<string, string> {
  return Object.fromEntries(runs.filter((r) => r.kind === kind).map((r) => [r.id, r.next_run_at]));
}

/** "in 5 min" within the hour, else "Mon, 24 Oct, 02:00". Empty for an unreadable time. */
export function formatNextRun(iso: string, now: Date = new Date()): string {
  const at = new Date(iso);
  if (Number.isNaN(at.getTime())) return "";
  const mins = Math.round((at.getTime() - now.getTime()) / 60_000);
  if (mins <= 0) return "due now";
  if (mins < 60) return `in ${mins} min`;
  return at.toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

/** Next runs of `kind` by id, refreshed while `active` (every minute) and on `refresh()`. */
export function useNextRuns(kind: UpcomingRun["kind"], active = true): { nextRuns: Record<string, string>; refresh: () => Promise<void> } {
  const [nextRuns, setNextRuns] = useState<Record<string, string>>({});
  const refresh = useCallback(async () => setNextRuns(nextRunsById(await fetchUpcomingRuns(), kind)), [kind]);
  useEffect(() => {
    if (!active) return;
    void refresh();
    const t = setInterval(() => void refresh(), 60_000);
    return () => clearInterval(t);
  }, [active, refresh]);
  return { nextRuns, refresh };
}
