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
