/**
 * Schedule time zones (K123), mirroring `backup_engine::schedule::ScheduleZone`:
 * `null` / "local" (this computer's zone, with daylight saving), "UTC", or a
 * fixed offset such as "+02:00". Named zones are not supported by the engine.
 */

const pad = (n: number) => String(n).padStart(2, "0");

/** "+02:00" / "-05:30" for an offset in minutes. */
export function offsetName(minutes: number): string {
  const sign = minutes < 0 ? "-" : "+";
  const m = Math.abs(minutes);
  return `${sign}${pad(Math.floor(m / 60))}:${pad(m % 60)}`;
}

/** Offsets real zones use (whole hours −12…+14 and the common half and quarter hours). */
const OFFSETS: number[] = [
  ...Array.from({ length: 27 }, (_, i) => (i - 12) * 60),
  -570, -210, 210, 270, 330, 345, 390, 525, 570, 630, 765,
].sort((a, b) => a - b);

/** Picker choices: local first, then UTC, then the offsets. */
export const ZONE_CHOICES: readonly { value: string; label: string }[] = [
  { value: "local", label: "This computer's time zone (follows daylight saving)" },
  { value: "UTC", label: "UTC" },
  ...OFFSETS.filter((m) => m !== 0).map((m) => ({ value: offsetName(m), label: `UTC${offsetName(m)}` })),
];

/** Whether a stored zone means the local zone. */
export function isLocalZone(tz: string | null | undefined): boolean {
  return !tz || !tz.trim() || tz.trim().toLowerCase() === "local";
}

/** The picker value of a stored zone. */
export function zoneValue(tz: string | null | undefined): string {
  return isLocalZone(tz) ? "local" : tz!.trim();
}

/** Offset in minutes of an RFC 3339 time ("…+02:00", "…Z"); null when absent. */
export function offsetMinutesOf(rfc3339: string): number | null {
  const m = /(Z|[+-]\d{2}:\d{2})$/.exec(rfc3339.trim());
  if (!m) return null;
  if (m[1] === "Z") return 0;
  const sign = m[1][0] === "-" ? -1 : 1;
  const [h, min] = m[1].slice(1).split(":").map(Number);
  return sign * (h * 60 + min);
}

/** The wall-clock time of `zoned` (RFC 3339 with the zone's offset) in that
 *  zone, e.g. "Tue, Mar 17, 02:30 (UTC+02:00)". */
export function zoneWallClock(zoned: string, locale?: string): string {
  const off = offsetMinutesOf(zoned) ?? 0;
  const t = Date.parse(zoned);
  if (Number.isNaN(t)) return zoned;
  const shifted = new Date(t + off * 60_000);
  const text = shifted.toLocaleString(locale, { timeZone: "UTC", weekday: "short", year: "numeric", month: "short", day: "numeric", hour: "2-digit", minute: "2-digit" });
  return `${text} (${off === 0 ? "UTC" : `UTC${offsetName(off)}`})`;
}
