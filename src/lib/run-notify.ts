/**
 * Where a finished sync run or automation run is reported: a notification
 * profile (set up once under Backup → Settings) plus which results send.
 * Mirrors `ufop_core::run_notify::RunNotify`, which sync pairs
 * (`run_options`) and automation rules (`options`) both embed.
 */

/** Results a notification can be limited to. */
export type RunResult = "success" | "warning" | "error";

export const RUN_RESULTS: { id: RunResult; label: string }[] = [
  { id: "success", label: "Success" },
  { id: "warning", label: "Warnings" },
  { id: "error", label: "Errors" },
];

export interface RunNotify {
  notify_profile_id?: string | null;
  /** Results that send; empty = whatever the profile itself sends. */
  notify_on?: RunResult[];
  /** Skip successful runs that moved fewer bytes than this (errors always notify). */
  min_bytes?: number | null;
}

const MB = 1024 * 1024;

export const bytesToMb = (bytes: number | null | undefined): string => (bytes == null ? "" : String(Math.round((bytes / MB) * 100) / 100));

/** MB typed by the user → bytes; empty or invalid = no threshold. */
export function mbToBytes(text: string): number | null {
  const mb = Number(text);
  return text.trim() === "" || !Number.isFinite(mb) || mb < 0 ? null : Math.round(mb * MB);
}
