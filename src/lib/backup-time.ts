/**
 * "Restore as it was at …": local date-time input values and the version a
 * time picks. `pickVersionAsOf` follows `ufop_core::backup::select`: the
 * newest complete version made at or before the time (versions are listed
 * newest first), partial versions only when allowed.
 */
import type { Fileset } from "@/stores/backup-store";

const pad = (n: number) => String(n).padStart(2, "0");

/** `YYYY-MM-DDTHH:MM` in local time, for `<input type="datetime-local">`. */
export function toLocalInput(d: Date): string {
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}T${pad(d.getHours())}:${pad(d.getMinutes())}`;
}

/** An input value (local time) as an RFC 3339 instant; null when empty or invalid. */
export function localInputToIso(v: string): string | null {
  if (!v.trim()) return null;
  const d = new Date(v);
  return Number.isNaN(d.getTime()) ? null : d.toISOString();
}

/** Index (0 = newest) of the version a time picks, or null when none is old enough. */
export function pickVersionAsOf(versions: Fileset[], iso: string, allowPartial = false): number | null {
  const t = Date.parse(iso);
  if (Number.isNaN(t)) return null;
  const i = versions.findIndex((v) => (allowPartial || !v.partial) && Date.parse(v.timestamp) <= t);
  return i < 0 ? null : i;
}

/** Time between two RFC 3339 instants, as "1 h 2 min" / "3 min" / "12 s". */
export function runDuration(start: string, end: string | null): string {
  if (!end) return "—";
  const s = Math.max(0, Math.round((Date.parse(end) - Date.parse(start)) / 1000));
  if (!Number.isFinite(s)) return "—";
  if (s >= 3600) return `${Math.floor(s / 3600)} h ${Math.floor((s % 3600) / 60)} min`;
  if (s >= 60) return `${Math.floor(s / 60)} min`;
  return `${s} s`;
}

/** Where backup timestamps are shown (Settings → Appearance, K123). The UI
 *  store sets it; `formatBackupTime` (and so `formatDate`) follows it. */
let timeDisplay: "local" | "utc" = "local";

export function setDefaultTimeDisplay(t: "local" | "utc"): void {
  timeDisplay = t === "utc" ? "utc" : "local";
}

/** A stored timestamp for display: this computer's time zone, or UTC with a
 *  "UTC" suffix when the setting asks for it; "—" when empty, the text itself
 *  when it is not a time. */
export function formatBackupTime(s: string | null | undefined, display: "local" | "utc" = timeDisplay): string {
  if (!s) return "—";
  const d = new Date(s);
  if (Number.isNaN(d.getTime())) return s;
  return display === "utc" ? `${d.toLocaleString(undefined, { timeZone: "UTC" })} UTC` : d.toLocaleString();
}
