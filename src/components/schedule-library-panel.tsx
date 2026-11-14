/**
 * Schedules & holiday calendars, defined once and used by sync pairs and
 * automation rules (named schedules) and by backup jobs (calendars).
 * A schedule previews its next runs with the days its calendars skip.
 */
import { useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { pickFiles } from "@/lib/folder-picker";
import { newCalendar, newSchedule, useScheduleStore, type HolidayCalendar, type NamedSchedule, type PreviewRun } from "@/stores/schedule-store";

const field = "w-full px-2 py-1 text-xs rounded border border-[var(--color-border)] bg-[var(--color-surface)]";
const btn = "px-2 py-1 text-xs rounded border border-[var(--color-border)] hover:bg-[var(--color-hover)] disabled:opacity-50";
const primary = "px-2 py-1 text-xs rounded bg-blue-600 text-white disabled:opacity-50";
const lines = (v: string) => v.split(/[\s,]+/).map((s) => s.trim()).filter(Boolean);

function ScheduleEditor({ value, calendars, onDone }: { value: NamedSchedule; calendars: HolidayCalendar[]; onDone: () => void }) {
  const { saveSchedule, preview } = useScheduleStore();
  const [s, setS] = useState(value);
  const [runs, setRuns] = useState<PreviewRun[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const t = setTimeout(() => {
      preview(s.cron_expr, s.calendar_ids, 8)
        .then((r) => {
          setRuns(r);
          setError(null);
        })
        .catch((e) => {
          setRuns(null);
          setError(errorText(e));
        });
    }, 250);
    return () => clearTimeout(t);
  }, [s.cron_expr, s.calendar_ids, preview]);

  return (
    <div className="space-y-2 rounded border border-[var(--color-border)] p-2" data-testid="schedule-editor">
      <div className="grid grid-cols-2 gap-2">
        <label className="text-xs">
          Name
          <input aria-label="Schedule name" className={field} value={s.name} onChange={(e) => setS({ ...s, name: e.target.value })} />
        </label>
        <label className="text-xs">
          Cron (UTC: sec min hour day month weekday)
          <input aria-label="Cron expression" className={`${field} font-mono`} value={s.cron_expr} onChange={(e) => setS({ ...s, cron_expr: e.target.value })} />
        </label>
      </div>
      <label className="text-xs block">
        Description
        <input aria-label="Schedule description" className={field} value={s.description} onChange={(e) => setS({ ...s, description: e.target.value })} />
      </label>
      <fieldset className="text-xs">
        <legend>Skip the days off in</legend>
        {calendars.length === 0 && <p className="text-[var(--color-text-muted)]">No holiday calendars yet.</p>}
        {calendars.map((c) => (
          <label key={c.id} className="mr-3 inline-flex items-center gap-1">
            <input
              type="checkbox"
              checked={s.calendar_ids.includes(c.id)}
              onChange={(e) => setS({ ...s, calendar_ids: e.target.checked ? [...s.calendar_ids, c.id] : s.calendar_ids.filter((x) => x !== c.id) })}
            />
            {c.name}
          </label>
        ))}
      </fieldset>
      {runs && (
        <ul className="text-[11px] font-mono" aria-label="Next runs">
          {runs.map((r) => (
            <li key={r.at} className={r.skipped_by ? "line-through text-[var(--color-text-muted)]" : ""}>
              {new Date(r.at).toLocaleString()}
              {r.skipped_by && <span className="no-underline"> — skipped ({r.skipped_by})</span>}
            </li>
          ))}
        </ul>
      )}
      {error && (
        <p className="text-xs text-red-600" role="alert">
          {error}
        </p>
      )}
      <div className="flex gap-2">
        <button
          type="button"
          className={primary}
          disabled={!s.name.trim()}
          onClick={async () => {
            try {
              await saveSchedule(s);
              onDone();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          Save schedule
        </button>
        <button type="button" className={btn} onClick={onDone}>
          Cancel
        </button>
      </div>
    </div>
  );
}

function CalendarEditor({ value, onDone }: { value: HolidayCalendar; onDone: () => void }) {
  const { saveCalendar, importIcs } = useScheduleStore();
  const [c, setC] = useState(value);
  const [dates, setDates] = useState(value.dates.join("\n"));
  const [annual, setAnnual] = useState(value.annual.join("\n"));
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const draft = (): HolidayCalendar => ({ ...c, dates: lines(dates), annual: lines(annual) });

  return (
    <div className="space-y-2 rounded border border-[var(--color-border)] p-2" data-testid="calendar-editor">
      <label className="text-xs block">
        Name
        <input aria-label="Calendar name" className={field} value={c.name} onChange={(e) => setC({ ...c, name: e.target.value })} />
      </label>
      <div className="grid grid-cols-2 gap-2">
        <label className="text-xs">
          Dates (YYYY-MM-DD, one per line)
          <textarea aria-label="Dates" rows={4} className={`${field} font-mono`} value={dates} onChange={(e) => setDates(e.target.value)} />
        </label>
        <label className="text-xs">
          Every year (MM-DD)
          <textarea aria-label="Yearly dates" rows={4} className={`${field} font-mono`} value={annual} onChange={(e) => setAnnual(e.target.value)} />
        </label>
      </div>
      <label className="text-xs flex items-center gap-1">
        <input type="checkbox" checked={c.skip_weekends} onChange={(e) => setC({ ...c, skip_weekends: e.target.checked })} />
        Weekends are days off too
      </label>
      {error && (
        <p className="text-xs text-red-600" role="alert">
          {error}
        </p>
      )}
      {note && (
        <p className="text-xs text-green-600" role="status">
          {note}
        </p>
      )}
      <div className="flex gap-2">
        <button
          type="button"
          className={primary}
          disabled={!c.name.trim()}
          onClick={async () => {
            try {
              await saveCalendar(draft());
              onDone();
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          Save calendar
        </button>
        <button
          type="button"
          className={btn}
          disabled={!c.name.trim()}
          title="Add the all-day events of an iCalendar (.ics) file"
          onClick={async () => {
            const [path] = await pickFiles("Import holidays (.ics)");
            if (!path) return;
            try {
              const [saved, added] = await importIcs(c.id || null, c.name, path);
              setC(saved);
              setDates(saved.dates.join("\n"));
              setAnnual(saved.annual.join("\n"));
              setNote(`Imported ${added} date(s).`);
            } catch (e) {
              setError(errorText(e));
            }
          }}
        >
          Import .ics…
        </button>
        <button type="button" className={btn} onClick={onDone}>
          Cancel
        </button>
      </div>
    </div>
  );
}

export function ScheduleLibraryPanel() {
  const { schedules, calendars, load, deleteSchedule, deleteCalendar } = useScheduleStore();
  const [editSchedule, setEditSchedule] = useState<NamedSchedule | null>(null);
  const [editCalendar, setEditCalendar] = useState<HolidayCalendar | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    load().catch((e) => setError(errorText(e)));
  }, [load]);

  const remove = (f: () => Promise<void>) => f().then(() => setError(null), (e) => setError(errorText(e)));
  const calName = (id: string) => calendars.find((c) => c.id === id)?.name ?? id;

  return (
    <div className="p-3 space-y-4 text-sm" data-testid="schedule-library">
      <section className="space-y-2">
        <div className="flex items-center gap-2">
          <h2 className="font-semibold">Schedules</h2>
          <button type="button" className={`${btn} ml-auto`} onClick={() => setEditSchedule(newSchedule())}>
            New schedule
          </button>
        </div>
        <p className="text-xs text-[var(--color-text-muted)]">Pick a schedule in a sync pair or automation rule; changing it here changes them all.</p>
        {editSchedule && <ScheduleEditor value={editSchedule} calendars={calendars} onDone={() => setEditSchedule(null)} />}
        <ul className="space-y-1">
          {schedules.map((s) => (
            <li key={s.id} className="flex items-center gap-2 text-xs">
              <b>{s.name}</b>
              <code>{s.cron_expr}</code>
              {s.calendar_ids.length > 0 && <span className="text-[var(--color-text-muted)]">skips {s.calendar_ids.map(calName).join(", ")}</span>}
              <button type="button" className={`${btn} ml-auto`} onClick={() => setEditSchedule(s)}>
                Edit
              </button>
              <button type="button" className={btn} aria-label={`Delete ${s.name}`} onClick={() => confirm(`Delete the schedule "${s.name}"?`) && void remove(() => deleteSchedule(s.id))}>
                Delete
              </button>
            </li>
          ))}
        </ul>
      </section>
      <section className="space-y-2">
        <div className="flex items-center gap-2">
          <h2 className="font-semibold">Holiday calendars</h2>
          <button type="button" className={`${btn} ml-auto`} onClick={() => setEditCalendar(newCalendar())}>
            New calendar
          </button>
        </div>
        <p className="text-xs text-[var(--color-text-muted)]">Scheduled runs that fall on these days are skipped (sync, automation, and backup jobs that use the calendar).</p>
        {editCalendar && <CalendarEditor value={editCalendar} onDone={() => setEditCalendar(null)} />}
        <ul className="space-y-1">
          {calendars.map((c) => (
            <li key={c.id} className="flex items-center gap-2 text-xs">
              <b>{c.name}</b>
              <span className="text-[var(--color-text-muted)]">
                {c.dates.length} date(s), {c.annual.length} yearly{c.skip_weekends ? ", weekends" : ""}
              </span>
              <button type="button" className={`${btn} ml-auto`} onClick={() => setEditCalendar(c)}>
                Edit
              </button>
              <button type="button" className={btn} aria-label={`Delete ${c.name}`} onClick={() => confirm(`Delete the calendar "${c.name}"?`) && void remove(() => deleteCalendar(c.id))}>
                Delete
              </button>
            </li>
          ))}
        </ul>
      </section>
      {error && (
        <p className="text-xs text-red-600" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
