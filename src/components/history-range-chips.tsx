/**
 * Date range chips of the history views: Today, Yesterday, Last 7 days,
 * Last 30 days and Custom (two date inputs). Choosing the active chip again
 * clears it. The parent turns the range into command arguments with
 * `rangeArgs`.
 */
import { ALL_TIME, RANGE_CHIPS, rangeInverted, type HistoryRange } from "@/lib/history-range";

const chip = "px-2 py-0.5 text-xs rounded-full border";
const on = "bg-blue-500 border-blue-500 text-white";
const off = "border-zinc-300 dark:border-zinc-600 text-zinc-600 dark:text-zinc-300 hover:bg-zinc-100 dark:hover:bg-zinc-800";
const dateInput = "text-xs px-1.5 py-0.5 rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-900";

export function HistoryRangeChips({ value, onChange, label = "Date range" }: { value: HistoryRange; onChange: (r: HistoryRange) => void; label?: string }) {
  return (
    <div className="flex items-center gap-1 flex-wrap" role="group" aria-label={label}>
      {RANGE_CHIPS.map((c) => (
        <button
          key={c.preset}
          type="button"
          className={`${chip} ${value.preset === c.preset ? on : off}`}
          aria-pressed={value.preset === c.preset}
          onClick={() => onChange(value.preset === c.preset ? ALL_TIME : { ...value, preset: c.preset })}
        >
          {c.label}
        </button>
      ))}
      {value.preset === "custom" && (
        <span className="flex items-center gap-1 text-xs text-zinc-500">
          <input type="date" className={dateInput} value={value.from} max={value.to || undefined} aria-label={`${label} from`} onChange={(e) => onChange({ ...value, from: e.target.value })} />
          to
          <input type="date" className={dateInput} value={value.to} min={value.from || undefined} aria-label={`${label} to`} onChange={(e) => onChange({ ...value, to: e.target.value })} />
          {rangeInverted(value) && (
            <span role="alert" className="text-red-600">
              The end is before the start.
            </span>
          )}
        </span>
      )}
    </div>
  );
}
