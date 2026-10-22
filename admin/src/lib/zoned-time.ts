/**
 * Wall-clock times in an IANA time zone, without a date library: the UTC
 * instant of "HH:MM on this local date" (DST-aware) and the most recent
 * weekly deadlines for expected-arrival alerts.
 */

export function isTimeZone(tz: string): boolean {
  try {
    new Intl.DateTimeFormat('en-US', { timeZone: tz });
    return true;
  } catch {
    return false;
  }
}

/** Local calendar parts of `at` in `tz` (weekday: 0 = Sunday). */
export function localParts(at: Date, tz: string): { y: number; m: number; d: number; hh: number; mm: number; weekday: number } {
  const f = new Intl.DateTimeFormat('en-US', { timeZone: tz, year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hourCycle: 'h23', weekday: 'short' });
  const p = Object.fromEntries(f.formatToParts(at).map((x) => [x.type, x.value]));
  return { y: Number(p.year), m: Number(p.month), d: Number(p.day), hh: Number(p.hour), mm: Number(p.minute), weekday: ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'].indexOf(p.weekday) };
}

/** Offset of `tz` from UTC at `at`, in milliseconds. */
function offsetMs(at: Date, tz: string): number {
  const p = localParts(at, tz);
  return Date.UTC(p.y, p.m - 1, p.d, p.hh, p.mm) - Math.floor(at.getTime() / 60_000) * 60_000;
}

/** The instant of local `hh:mm` on local date y-m-d in `tz`. */
export function zonedToUtc(y: number, m: number, d: number, hh: number, mm: number, tz: string): Date {
  const guess = Date.UTC(y, m - 1, d, hh, mm);
  let t = guess - offsetMs(new Date(guess), tz);
  // Second pass across a DST change.
  t = guess - offsetMs(new Date(t), tz);
  return new Date(t);
}

export interface WeeklyDeadline {
  /** `HH:MM` local time. */
  expectedBy: string;
  /** 0 = Sunday … 6 = Saturday; empty = every day. */
  days: number[];
  timeZone: string;
}

/** The `count` most recent deadlines at or before `at`, newest first. */
export function recentDeadlines(at: Date, w: WeeklyDeadline, count = 2): Date[] {
  const [hh, mm] = w.expectedBy.split(':').map(Number);
  const out: Date[] = [];
  // Walk back over local dates (at most 3 weeks).
  for (let i = 0; i < 22 && out.length < count; i++) {
    const p = localParts(new Date(at.getTime() - i * 86_400_000), w.timeZone);
    const weekday = new Date(Date.UTC(p.y, p.m - 1, p.d)).getUTCDay();
    if (w.days.length && !w.days.includes(weekday)) continue;
    const t = zonedToUtc(p.y, p.m, p.d, hh, mm, w.timeZone);
    if (t.getTime() <= at.getTime() && !out.some((x) => x.getTime() === t.getTime())) out.push(t);
  }
  return out.sort((a, b) => b.getTime() - a.getTime()).slice(0, count);
}
