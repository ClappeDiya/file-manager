/**
 * How long, and how much, failed transfers are kept for retry. Failed
 * transfers come back (still failed, with their error) after a restart; the
 * ones older than the age, and the oldest beyond the size, are removed when
 * the history is cleaned up and on the next start.
 */
import { useEffect, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";

interface FailedRetention {
  max_age_days: number;
  max_total_kb: number;
}

const DEFAULTS: FailedRetention = { max_age_days: 30, max_total_kb: 4096 };
const field = "w-16 text-xs px-2 py-0.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900";

export function FailedRetentionSettings() {
  const [days, setDays] = useState(String(DEFAULTS.max_age_days));
  const [mb, setMb] = useState(String(DEFAULTS.max_total_kb / 1024));
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    tauriInvoke<FailedRetention>("get_failed_transfer_retention", undefined, DEFAULTS)
      .then((r) => {
        if (!live) return;
        setDays(String(r.max_age_days));
        setMb(String(Math.max(1, Math.round(r.max_total_kb / 1024))));
      })
      .catch(() => undefined);
    return () => {
      live = false;
    };
  }, []);

  const save = async () => {
    setMessage(null);
    setError(null);
    const d = Math.max(1, Math.floor(Number(days)) || DEFAULTS.max_age_days);
    const m = Math.max(1, Math.floor(Number(mb)) || DEFAULTS.max_total_kb / 1024);
    try {
      const saved = await tauriInvoke<FailedRetention>("set_failed_transfer_retention", { maxAgeDays: d, maxTotalKb: m * 1024 });
      setDays(String(saved.max_age_days));
      setMb(String(Math.max(1, Math.round(saved.max_total_kb / 1024))));
      setMessage("Saved. Failed transfers past these limits were removed.");
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <div className="mt-2" data-testid="failed-retention">
      <div className="text-xs text-gray-600 dark:text-gray-400 mb-1">Failed transfers stay available to retry after a restart, up to:</div>
      <div className="flex flex-wrap items-center gap-1.5 text-xs text-gray-500 dark:text-gray-400">
        <input type="number" min={1} max={3650} value={days} onChange={(e) => setDays(e.target.value)} className={field} aria-label="Keep failed transfers for days" />
        days old, and
        <input type="number" min={1} max={1024} value={mb} onChange={(e) => setMb(e.target.value)} className={field} aria-label="Keep failed transfers up to MB" />
        MB of saved data
        <button type="button" onClick={() => void save()} className="px-2 py-0.5 rounded bg-blue-500 text-white hover:bg-blue-600">
          Save
        </button>
      </div>
      {message && (
        <div className="mt-1 text-[11px] text-gray-600 dark:text-gray-400" role="status">
          {message}
        </div>
      )}
      {error && (
        <div className="mt-1 text-[11px] text-red-600" role="alert">
          {error}
        </div>
      )}
    </div>
  );
}
