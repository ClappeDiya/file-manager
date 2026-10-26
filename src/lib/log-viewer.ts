/** Types and helpers of the in-app log viewer (Rust `log_query`). */

export type LogLevelName = "ERROR" | "WARN" | "INFO" | "DEBUG" | "TRACE";

export interface LogRecord {
  id: string;
  ts: string;
  level: LogLevelName | string;
  target: string;
  message: string;
  fields: Record<string, unknown> | null;
  spans: string[];
}

export interface LogPage {
  records: LogRecord[];
  next_cursor: string | null;
  sources: string[];
  scan_limited: boolean;
  directory: string;
}

export interface LogFilters {
  level: string;
  source: string;
  text: string;
  /** `datetime-local` input values (this computer's time). */
  since: string;
  until: string;
}

export const NO_FILTERS: LogFilters = { level: "", source: "", text: "", since: "", until: "" };

/** Whether any filter narrows the log. */
export const isFiltered = (f: LogFilters): boolean => !!(f.level || f.source.trim() || f.text.trim() || f.since || f.until);

export const LEVEL_OPTIONS = [
  { value: "", label: "All levels" },
  { value: "debug", label: "Debug and above" },
  { value: "info", label: "Info and above" },
  { value: "warn", label: "Warnings and errors" },
  { value: "error", label: "Errors only" },
];

/** A `datetime-local` value (local time) as an RFC 3339 instant; null when empty or invalid. */
export function localToIso(v: string): string | null {
  if (!v.trim()) return null;
  const d = new Date(v);
  return Number.isNaN(d.getTime()) ? null : d.toISOString();
}

/** The `query_logs` argument for some filters (empty ones are left out). */
export function logQuery(f: LogFilters, extra: Record<string, unknown> = {}): { query: Record<string, unknown> } {
  const q: Record<string, unknown> = { ...extra };
  if (f.level) q.level = f.level;
  if (f.source.trim()) q.source = f.source.trim();
  if (f.text.trim()) q.text = f.text.trim();
  const since = localToIso(f.since);
  const until = localToIso(f.until);
  if (since) q.since = since;
  if (until) q.until = until;
  return { query: q };
}

/** Records oldest first (a page arrives newest first). */
export const oldestFirst = (page: LogRecord[]): LogRecord[] => [...page].reverse();

/** How many pages one live-tail poll reads at most. */
export const TAIL_MAX_PAGES = 20;

/**
 * Everything logged since `since` (newest first): page back through the
 * cursor until a page is not full, so a burst of more lines than one page
 * between two polls leaves no gap. `complete` is false when it stopped at
 * `maxPages` (older lines of the burst were skipped).
 */
export async function fetchTail(
  query: (q: Record<string, unknown>) => Promise<LogPage>,
  filtersQuery: Record<string, unknown>,
  since: string | null,
  limit: number,
  maxPages = TAIL_MAX_PAGES,
): Promise<{ records: LogRecord[]; complete: boolean }> {
  const records: LogRecord[] = [];
  let cursor: string | null = null;
  for (let i = 0; i < maxPages; i++) {
    const page = await query({ ...filtersQuery, limit, ...(since ? { since } : {}), ...(cursor ? { cursor } : {}) });
    records.push(...page.records);
    // Without a starting point (an empty view), one page is enough.
    if (!since || !page.next_cursor || page.records.length < limit) return { records, complete: true };
    cursor = page.next_cursor;
  }
  return { records, complete: false };
}

/** Adds records a live tail fetched, dropping ones already shown (the tail re-asks from the newest record's time). */
export function mergeTail(shown: LogRecord[], fetchedNewestFirst: LogRecord[]): LogRecord[] {
  if (fetchedNewestFirst.length === 0) return shown;
  const known = new Set(shown.map((r) => r.id));
  const fresh = oldestFirst(fetchedNewestFirst.filter((r) => !known.has(r.id)));
  return fresh.length === 0 ? shown : [...shown, ...fresh];
}

/** `HH:MM:SS.mmm` in this computer's time zone (the date too when `withDate`). */
export function clockTime(iso: string, withDate = false): string {
  const d = new Date(iso);
  if (Number.isNaN(d.getTime())) return iso;
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  const time = `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
  return withDate ? `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${time}` : time;
}

/** A record as text for the clipboard: the line a human log would have, with fields and spans after it. */
export function recordText(r: LogRecord): string {
  const spans = r.spans.length ? `${r.spans.join(":")}: ` : "";
  const fields = r.fields && Object.keys(r.fields).length ? ` ${JSON.stringify(r.fields)}` : "";
  return `${clockTime(r.ts, true)} ${r.level.padStart(5)} ${spans}${r.target}: ${r.message}${fields}`;
}

export const LEVEL_TONE: Record<string, string> = {
  ERROR: "text-red-600",
  WARN: "text-amber-600",
  INFO: "text-blue-600",
  DEBUG: "text-zinc-500",
  TRACE: "text-zinc-400",
};
