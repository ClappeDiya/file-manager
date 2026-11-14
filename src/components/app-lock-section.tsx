/**
 * Settings → App Lock: lock the windows with the master password at start,
 * after some idle minutes, or now. Changing the lock while it is on needs
 * the master password; turning it on the first time creates one.
 */
import { useEffect, useState } from "react";
import { isTauriAvailable } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { IDLE_CHOICES, useAppLockStore, type AppLockSettings } from "@/stores/app-lock-store";

const idleLabel = (m: number) => (m === 0 ? "Never" : m === 60 ? "After 1 hour" : `After ${m} minute${m === 1 ? "" : "s"}`);

export function AppLockSection() {
  const status = useAppLockStore((s) => s.status);
  const save = useAppLockStore((s) => s.save);
  const lockNow = useAppLockStore((s) => s.lockNow);
  const [draft, setDraft] = useState<AppLockSettings | null>(null);
  const [password, setPassword] = useState("");
  const [newPassword, setNewPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(null);
  const desktop = isTauriAvailable();

  useEffect(() => {
    if (desktop && !status) void useAppLockStore.getState().load().catch((e) => setMsg({ ok: false, text: errorText(e) }));
  }, [desktop, status]);
  useEffect(() => {
    if (status) setDraft({ enabled: status.enabled, lock_on_start: status.lock_on_start, idle_minutes: status.idle_minutes });
  }, [status]);

  if (!desktop) return <p className="text-xs text-gray-500 pt-3">Available in the desktop app.</p>;
  if (!status || !draft) return msg ? <p className="text-xs text-red-600 pt-3">{msg.text}</p> : null;

  const creating = draft.enabled && !status.has_password;
  const needsPassword = (status.enabled || draft.enabled) && status.has_password;
  const changed = draft.enabled !== status.enabled || draft.lock_on_start !== status.lock_on_start || draft.idle_minutes !== status.idle_minutes;
  const problem = creating
    ? newPassword.length < 8
      ? "Choose a master password of at least 8 characters."
      : newPassword !== confirm
        ? "The two passwords are different."
        : null
    : needsPassword && !password
      ? "Enter the master password to change the app lock."
      : null;

  const submit = async () => {
    setBusy(true);
    setMsg(null);
    try {
      await save(draft, needsPassword ? password : null, creating ? newPassword : null);
      setMsg({ ok: true, text: draft.enabled ? "App lock saved." : "App lock turned off." });
      setPassword("");
      setNewPassword("");
      setConfirm("");
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };

  const field = "w-full px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-transparent text-xs";
  return (
    <div className="space-y-2 pt-3" data-testid="app-lock-settings">
      <label className="flex items-start gap-2 text-xs">
        <input type="checkbox" className="mt-0.5" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} data-testid="app-lock-enabled" />
        <span>
          Lock UFOP with the master password
          <span className="block text-[11px] text-gray-500">The windows show a lock screen; scheduled backups, syncs and transfers keep running.</span>
        </span>
      </label>
      <div className={`pl-5 space-y-2 ${draft.enabled ? "" : "opacity-50"}`}>
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={draft.lock_on_start} disabled={!draft.enabled} onChange={(e) => setDraft({ ...draft, lock_on_start: e.target.checked })} data-testid="app-lock-on-start" />
          Lock when UFOP starts
        </label>
        <label className="flex items-center gap-2 text-xs">
          Lock when not used
          <select
            value={draft.idle_minutes}
            disabled={!draft.enabled}
            onChange={(e) => setDraft({ ...draft, idle_minutes: Number(e.target.value) })}
            className="px-2 py-0.5 rounded border border-gray-300 dark:border-gray-600 bg-transparent text-xs"
            data-testid="app-lock-idle"
          >
            {IDLE_CHOICES.map((m) => (
              <option key={m} value={m}>
                {idleLabel(m)}
              </option>
            ))}
          </select>
        </label>
      </div>
      {changed && creating && (
        <div className="space-y-1">
          <p className="text-[11px] text-gray-500">There is no master password yet. Choose one: it unlocks the app and protects saved credentials. It cannot be recovered if you forget it.</p>
          <input type="password" autoComplete="new-password" placeholder="New master password" aria-label="New master password" value={newPassword} onChange={(e) => setNewPassword(e.target.value)} className={field} data-testid="app-lock-new-password" />
          <input type="password" autoComplete="new-password" placeholder="Repeat it" aria-label="Repeat the new master password" value={confirm} onChange={(e) => setConfirm(e.target.value)} className={field} data-testid="app-lock-confirm" />
        </div>
      )}
      {changed && needsPassword && (
        <input type="password" autoComplete="current-password" placeholder="Master password" aria-label="Master password" value={password} onChange={(e) => setPassword(e.target.value)} className={field} data-testid="app-lock-password-field" />
      )}
      <div className="flex items-center gap-2">
        {changed && (
          <button
            type="button"
            disabled={busy || !!problem}
            title={problem ?? undefined}
            onClick={() => void submit()}
            className="px-3 py-1 text-xs rounded bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
            data-testid="app-lock-save"
          >
            {busy ? "Saving…" : "Save"}
          </button>
        )}
        {status.enabled && !changed && (
          <button
            type="button"
            onClick={() => void lockNow().catch((e) => setMsg({ ok: false, text: errorText(e) }))}
            className="px-3 py-1 text-xs rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-50 dark:hover:bg-gray-800"
            data-testid="app-lock-now"
          >
            Lock now
          </button>
        )}
        {changed && problem && <span className="text-[11px] text-gray-500">{problem}</span>}
      </div>
      {msg && <p className={`text-xs ${msg.ok ? "text-green-700 dark:text-green-400" : "text-red-600"}`}>{msg.text}</p>}
    </div>
  );
}
