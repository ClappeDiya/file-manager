/**
 * Settings › Advanced › Bandwidth timetable: windows (weekdays, from, to)
 * with upload and download limits. While a window is active its upload
 * limit is shared by file transfers, sync copies and backups together, and
 * both limits replace the backup machine-wide limits. Outside every window
 * transfers and sync are not limited by the backup caps.
 */
import { useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";

/** Mirrors `BandwidthWindow` in ufop-core throttle.rs. */
export interface BandwidthWindow {
  days: number[];
  start: string;
  end: string;
  upload_bps: number;
  download_bps: number;
}

const DAYS = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
const cls = "px-2 py-1 text-xs border rounded dark:bg-gray-800 dark:border-gray-600";
const kib = (bps: number) => Math.round(bps / 1024);

export function BandwidthTimetableSection() {
  const [windows, setWindows] = useState<BandwidthWindow[] | null>(null);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    let live = true;
    void tauriInvokeSafe<BandwidthWindow[]>("get_bandwidth_schedule", undefined, []).then((w) => live && setWindows(Array.isArray(w) ? w : []));
    return () => {
      live = false;
    };
  }, []);

  if (!windows) return null;
  const patch = (i: number, p: Partial<BandwidthWindow>) => setWindows(windows.map((w, j) => (j === i ? { ...w, ...p } : w)));

  const save = async () => {
    setStatus(null);
    try {
      setWindows(await tauriInvoke<BandwidthWindow[]>("set_bandwidth_schedule", { windows }));
      setStatus({ ok: true, text: "Saved and applied." });
    } catch (e) {
      setStatus({ ok: false, text: errorText(e) });
    }
  };

  return (
    <div className="space-y-2" data-testid="bandwidth-timetable">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        While a window is active, its upload limit (KiB/s) is shared by file transfers, sync copies and backups together; its
        download limit applies to backup restores. Each replaces the backup machine-wide limit for its direction during the
        window; 0 sets no limit of its own, so the machine-wide limit for that direction stays in force.
        The first matching window wins; a window that ends before it starts runs past midnight.
      </p>
      {windows.map((w, i) => (
        <div key={i} className="flex flex-wrap items-center gap-1.5 rounded border border-gray-200 dark:border-gray-700 p-1.5" data-testid="bandwidth-window">
          {DAYS.map((d, n) => (
            <label key={d} className="flex items-center gap-0.5 text-[11px]">
              <input
                type="checkbox"
                checked={w.days.includes(n)}
                onChange={(e) => patch(i, { days: e.target.checked ? [...w.days, n].sort() : w.days.filter((x) => x !== n) })}
              />
              {d}
            </label>
          ))}
          <input aria-label="From" type="time" className={cls} value={w.start} onChange={(e) => patch(i, { start: e.target.value })} />
          <span className="text-xs">to</span>
          <input aria-label="To" type="time" className={cls} value={w.end} onChange={(e) => patch(i, { end: e.target.value })} />
          <label className="text-[11px]">
            Up
            <input aria-label="Upload limit" type="number" min={0} className={`${cls} w-20 ml-1`} value={kib(w.upload_bps)} onChange={(e) => patch(i, { upload_bps: Math.max(0, Number(e.target.value) || 0) * 1024 })} />
          </label>
          <label className="text-[11px]">
            Down
            <input aria-label="Download limit" type="number" min={0} className={`${cls} w-20 ml-1`} value={kib(w.download_bps)} onChange={(e) => patch(i, { download_bps: Math.max(0, Number(e.target.value) || 0) * 1024 })} />
          </label>
          <button type="button" aria-label="Remove window" className="text-xs text-red-600 hover:underline" onClick={() => setWindows(windows.filter((_, j) => j !== i))}>
            Remove
          </button>
        </div>
      ))}
      <div className="flex items-center gap-2">
        <button
          type="button"
          className="text-xs px-3 py-1 rounded border border-gray-300 dark:border-gray-600"
          onClick={() => setWindows([...windows, { days: [0, 1, 2, 3, 4], start: "08:00", end: "18:00", upload_bps: 1024 * 1024, download_bps: 0 }])}
        >
          Add window
        </button>
        <button type="button" onClick={() => void save()} className="text-xs px-3 py-1 rounded bg-blue-500 text-white hover:bg-blue-600">
          Save timetable
        </button>
        {status && <span className={`text-xs ${status.ok ? "text-green-600" : "text-red-600"}`}>{status.text}</span>}
      </div>
    </div>
  );
}
