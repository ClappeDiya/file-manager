/**
 * "Tell me when it finishes" for sync pairs and automation rules: pick one of
 * the notification profiles from Backup → Settings (the same channels backup
 * jobs use), which results send, and, in Advanced, a size threshold that
 * mutes small successful runs. The profile list and the missing-profile
 * check are shared with the backup job editor's picker.
 */
import { useState } from "react";
import { useProfileChoices } from "@/components/backup/backup-notification-profiles";
import { channelLabel } from "@/stores/backup-notify";
import { RUN_RESULTS, bytesToMb, mbToBytes, type RunNotify, type RunResult } from "@/lib/run-notify";

const field = "mt-1 block w-full px-2 py-1 text-xs border rounded bg-transparent dark:bg-zinc-800 dark:border-zinc-600";
const labelCls = "block text-xs text-zinc-600 dark:text-zinc-400";

export function RunNotifyPicker({ value, onChange, advanced }: { value: RunNotify; onChange: (patch: RunNotify) => void; advanced: boolean }) {
  const chosen = value.notify_profile_id ?? "";
  const { profiles, missing } = useProfileChoices(chosen ? [chosen] : []);
  const on = value.notify_on ?? [];
  // Keep what is being typed ("0." must not turn into "0").
  const [mbText, setMbText] = useState(bytesToMb(value.min_bytes));
  const toggle = (r: RunResult, checked: boolean) => onChange({ notify_on: checked ? [...on, r] : on.filter((x) => x !== r) });
  return (
    <fieldset className="space-y-2 rounded border border-zinc-200 dark:border-zinc-700 p-2" data-testid="run-notify">
      <legend className="px-1 text-xs font-medium text-zinc-600 dark:text-zinc-400">Notify when a run finishes</legend>
      <label className={labelCls}>
        Send to
        <select
          className={field}
          value={chosen}
          onChange={(e) => onChange({ notify_profile_id: e.target.value || null })}
          data-testid="run-notify-profile"
        >
          <option value="">Nobody</option>
          {profiles.map((p) => (
            <option key={p.id} value={p.id}>
              {p.name} ({channelLabel(p.notification)})
            </option>
          ))}
          {missing.map((id) => (
            <option key={id} value={id}>
              (deleted profile)
            </option>
          ))}
        </select>
        {profiles.length === 0 && <span className="text-[10px] text-zinc-400">No profiles yet; create one under Backup → Settings → Notification profiles.</span>}
        {missing.length > 0 && <span className="text-[10px] text-amber-600">This profile no longer exists, so nothing is sent. Choose another or “Nobody”.</span>}
      </label>
      {chosen && (
        <>
          <div className="flex flex-wrap gap-3" role="group" aria-label="Send on">
            {RUN_RESULTS.map((r) => (
              <label key={r.id} className="flex items-center gap-1 text-xs">
                <input type="checkbox" checked={on.includes(r.id)} onChange={(e) => toggle(r.id, e.target.checked)} />
                {r.label}
              </label>
            ))}
          </div>
          <p className="text-[10px] text-zinc-400">None ticked = whatever the profile itself sends.</p>
          {advanced && (
            <label className={labelCls}>
              Don’t report successful runs below (MB)
              <input
                type="number"
                min={0}
                step="any"
                className={field}
                placeholder="no limit"
                value={mbText}
                onChange={(e) => {
                  setMbText(e.target.value);
                  onChange({ min_bytes: mbToBytes(e.target.value) });
                }}
              />
              <span className="text-[10px] text-zinc-400">Warnings and errors are always reported.</span>
            </label>
          )}
        </>
      )}
    </fieldset>
  );
}
