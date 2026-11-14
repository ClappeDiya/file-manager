/**
 * Pickers for the schedule library: a named schedule for a cron field
 * (sync pairs, automation rules: stored as `schedule:<id>`), and holiday
 * calendars for backup schedules.
 */
import { useEffect } from "react";
import { refId, scheduleRef, useScheduleStore } from "@/stores/schedule-store";

function useLibrary() {
  const s = useScheduleStore();
  useEffect(() => {
    if (!s.loaded) void s.load().catch(() => undefined);
  }, [s]);
  return s;
}

/**
 * Choose a named schedule instead of typing cron. `value` is the cron field;
 * choosing "Custom" hands back `customCron` (the last typed expression).
 */
export function NamedSchedulePicker({ value, onChange, customCron, className }: { value: string; onChange: (cron: string) => void; customCron: string; className?: string }) {
  const { schedules } = useLibrary();
  const current = refId(value);
  if (schedules.length === 0 && !current) return null;
  return (
    <select aria-label="Named schedule" className={className} value={current ?? ""} onChange={(e) => onChange(e.target.value ? scheduleRef(e.target.value) : customCron)}>
      <option value="">Custom cron expression</option>
      {schedules.map((s) => (
        <option key={s.id} value={s.id}>
          {s.name} ({s.cron_expr})
        </option>
      ))}
      {current && !schedules.some((s) => s.id === current) && <option value={current}>Missing schedule ({current})</option>}
    </select>
  );
}

export function HolidayCalendarPicker({ value, onChange }: { value: string[]; onChange: (ids: string[]) => void }) {
  const { calendars } = useLibrary();
  if (calendars.length === 0) return <p className="text-[11px] text-zinc-500">Holiday calendars (Schedules & holiday calendars, Advanced mode) can skip runs on days off.</p>;
  return (
    <fieldset className="text-xs" aria-label="Skip days off in">
      <legend className="mb-1">Skip runs on the days off in</legend>
      {calendars.map((c) => (
        <label key={c.id} className="mr-3 inline-flex items-center gap-1">
          <input type="checkbox" checked={value.includes(c.id)} onChange={(e) => onChange(e.target.checked ? [...value, c.id] : value.filter((x) => x !== c.id))} />
          {c.name}
        </label>
      ))}
    </fieldset>
  );
}
