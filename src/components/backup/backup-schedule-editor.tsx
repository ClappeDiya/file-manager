/**
 * Schedule fields shared by the job's backup schedule and the maintenance
 * schedules: interval mode (start time, repeat, weekdays) or, in Advanced
 * mode, "Custom (cron)" with one expression per line and a preview of the
 * next five runs. Mirrors `backup_engine::schedule::BackupSchedule`.
 */
import { useEffect, useState } from "react";
import { HolidayCalendarPicker } from "@/components/schedule-pickers";
import { isBackendAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { cronLines, inputCls, isCronSchedule } from "@/lib/backup-helpers";
import { errorText, type BackupSchedule } from "@/stores/backup-store";
import { Check, Field } from "./backup-ui";
import { useIsAdvanced } from "./use-is-advanced";

const WEEKDAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const INTERVALS: [number, string][] = [
  [15, "Every 15 minutes"],
  [60, "Hourly"],
  [360, "Every 6 hours"],
  [720, "Every 12 hours"],
  [1440, "Daily"],
  [10080, "Weekly"],
  [43200, "Every 30 days"],
];

/** Mirrors `schedule_library::PreviewRun`. */
export interface ScheduleRun {
  at: string;
  skipped_by: string | null;
}

export function ScheduleEditor({
  value,
  onChange,
  label = "Run automatically",
  showBattery = true,
  testId = "schedule",
}: {
  value: BackupSchedule;
  onChange: (s: BackupSchedule) => void;
  label?: string;
  showBattery?: boolean;
  testId?: string;
}) {
  const advanced = useIsAdvanced();
  const [mode, setMode] = useState<"interval" | "cron">(isCronSchedule(value) ? "cron" : "interval");
  const cron = mode === "cron";
  const [cronText, setCronText] = useState((value.cron ?? []).join("\n"));
  const [preview, setPreview] = useState<ScheduleRun[] | null>(null);
  const [previewError, setPreviewError] = useState<string | null>(null);
  const set = (p: Partial<BackupSchedule>) => onChange({ ...value, ...p });

  // Next five runs of the schedule as it would be saved (debounced).
  useEffect(() => {
    if (!value.enabled || !isBackendAvailable()) {
      setPreview(null);
      setPreviewError(null);
      return;
    }
    const t = setTimeout(() => {
      tauriInvoke<ScheduleRun[]>("backup_schedule_preview", { schedule: value, count: 5 })
        .then((r) => {
          setPreview(r);
          setPreviewError(null);
        })
        .catch((e) => {
          setPreview(null);
          setPreviewError(errorText(e));
        });
    }, 300);
    return () => clearTimeout(t);
  }, [value]);

  return (
    <div className="space-y-2" data-testid={testId}>
      <Check label={label} checked={value.enabled} onChange={(v) => set({ enabled: v })} />
      {(advanced || cron) && (
        <Field label="Schedule type">
          <select
            className={inputCls}
            value={cron ? "cron" : "interval"}
            data-testid={`${testId}-type`}
            onChange={(e) => {
              setMode(e.target.value as "interval" | "cron");
              if (e.target.value === "cron") {
                const lines = cronLines(cronText);
                const next = lines.length ? lines : ["0 2 * * *"];
                setCronText(next.join("\n"));
                set({ cron: next });
              } else {
                set({ cron: [] });
              }
            }}
          >
            <option value="interval">Start time and repeat</option>
            <option value="cron">Custom (cron)</option>
          </select>
        </Field>
      )}
      {cron ? (
        <Field label="Cron expressions (one per line, local time)" hint="min hour day month weekday, e.g. 30 2 * * Mon-Fri. Seconds first and a year last are allowed. The earliest match of any line runs.">
          <textarea
            className={`${inputCls} font-mono`}
            rows={3}
            value={cronText}
            data-testid={`${testId}-cron`}
            onChange={(e) => {
              setCronText(e.target.value);
              set({ cron: cronLines(e.target.value) });
            }}
          />
        </Field>
      ) : (
        <>
          <div className="grid grid-cols-2 gap-2">
            <Field label="Start time">
              <input className={inputCls} type="time" value={value.start_time} onChange={(e) => set({ start_time: e.target.value })} />
            </Field>
            <Field label="Repeat">
              <select className={inputCls} value={value.interval_minutes} onChange={(e) => set({ interval_minutes: Number(e.target.value) })}>
                {INTERVALS.map(([v, l]) => (
                  <option key={v} value={v}>
                    {l}
                  </option>
                ))}
                {!INTERVALS.some(([v]) => v === value.interval_minutes) && <option value={value.interval_minutes}>Every {value.interval_minutes} minutes</option>}
              </select>
            </Field>
          </div>
          <div className="flex gap-2 flex-wrap text-xs">
            {WEEKDAYS.map((d, i) => (
              <label key={d} className="flex items-center gap-1">
                <input
                  type="checkbox"
                  checked={value.weekdays.length === 0 || value.weekdays.includes(i)}
                  onChange={(e) => {
                    const all = value.weekdays.length === 0 ? [0, 1, 2, 3, 4, 5, 6] : value.weekdays;
                    const next = e.target.checked ? [...new Set([...all, i])].sort() : all.filter((x) => x !== i);
                    set({ weekdays: next.length === 7 ? [] : next });
                  }}
                />
                {d}
              </label>
            ))}
          </div>
        </>
      )}
      <HolidayCalendarPicker value={value.calendar_ids ?? []} onChange={(ids) => set({ calendar_ids: ids })} />
      {showBattery && <Check label="Skip scheduled runs while on battery" checked={value.skip_on_battery} onChange={(v) => set({ skip_on_battery: v })} />}
      {value.enabled && (previewError || (preview && preview.length > 0)) && (
        <div className="text-[11px] text-zinc-600 dark:text-zinc-400" data-testid={`${testId}-preview`}>
          {previewError ? (
            <span className="text-red-600">{previewError}</span>
          ) : (
            <>
              <span className="font-medium">Next runs:</span>
              <ul className="ml-3 list-disc">
                {preview!.map((r) => (
                  <li key={r.at} className={r.skipped_by ? "line-through" : undefined}>
                    {new Date(r.at).toLocaleString()}
                    {r.skipped_by && ` (skipped: ${r.skipped_by})`}
                  </li>
                ))}
              </ul>
            </>
          )}
        </div>
      )}
    </div>
  );
}
