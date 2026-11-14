/**
 * Date ranges of the history views (transfer history, sync run history,
 * backup run history): the chips Today / Yesterday / Last 7 days / Last 30
 * days / Custom, turned into the RFC 3339 instants the backend filters on.
 * Days are the user's own (local midnight to local midnight), not UTC days.
 */

export type RangePreset = "all" | "today" | "yesterday" | "7d" | "30d" | "custom";

export interface HistoryRange {
  preset: RangePreset;
  /** Custom range, as `YYYY-MM-DD` input values (local days, both inclusive). */
  from: string;
  to: string;
}

export const ALL_TIME: HistoryRange = { preset: "all", from: "", to: "" };

export const RANGE_CHIPS: { preset: Exclude<RangePreset, "all">; label: string }[] = [
  { preset: "today", label: "Today" },
  { preset: "yesterday", label: "Yesterday" },
  { preset: "7d", label: "Last 7 days" },
  { preset: "30d", label: "Last 30 days" },
  { preset: "custom", label: "Custom" },
];

const startOfDay = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate());
const addDays = (d: Date, n: number) => new Date(d.getFullYear(), d.getMonth(), d.getDate() + n);

/** A `YYYY-MM-DD` input value as that local day's midnight; null when empty or invalid. */
function localDay(v: string): Date | null {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v.trim());
  if (!m) return null;
  const d = new Date(Number(m[1]), Number(m[2]) - 1, Number(m[3]));
  return Number.isNaN(d.getTime()) ? null : d;
}

/** The instants a range covers (`null` = open on that side). "Last 7 days" is
 *  today and the six days before it; "Last 30 days" likewise. */
export function rangeBounds(range: HistoryRange, now: Date = new Date()): { from: string | null; to: string | null } {
  const today = startOfDay(now);
  const iso = (d: Date) => d.toISOString();
  // The last millisecond of a local day.
  const endOf = (d: Date) => new Date(addDays(d, 1).getTime() - 1);
  switch (range.preset) {
    case "today":
      return { from: iso(today), to: null };
    case "yesterday":
      return { from: iso(addDays(today, -1)), to: iso(endOf(addDays(today, -1))) };
    case "7d":
      return { from: iso(addDays(today, -6)), to: null };
    case "30d":
      return { from: iso(addDays(today, -29)), to: null };
    case "custom": {
      const from = localDay(range.from);
      const to = localDay(range.to);
      return { from: from ? iso(from) : null, to: to ? iso(endOf(to)) : null };
    }
    default:
      return { from: null, to: null };
  }
}

/** The `dateFrom` / `dateTo` command arguments; absent when the range is open,
 *  so "all time" calls look exactly as they did before ranges existed. */
export function rangeArgs(range: HistoryRange, now: Date = new Date()): { dateFrom?: string; dateTo?: string } {
  const { from, to } = rangeBounds(range, now);
  return { ...(from ? { dateFrom: from } : {}), ...(to ? { dateTo: to } : {}) };
}

/** Whether a custom range is the wrong way round (nothing could match). */
export function rangeInverted(range: HistoryRange): boolean {
  if (range.preset !== "custom") return false;
  const { from, to } = rangeBounds(range);
  return !!from && !!to && Date.parse(to) < Date.parse(from);
}

/** A stored instant in this computer's time zone; the text itself when it is not a time. */
export function formatLocalTime(iso: string | null | undefined): string {
  if (!iso) return "—";
  const d = new Date(iso);
  return Number.isNaN(d.getTime()) ? iso : d.toLocaleString();
}
