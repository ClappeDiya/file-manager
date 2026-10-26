/**
 * Settings → Startup & Tray: start at login, how the window starts, and what
 * minimising and closing do. "Keep running in the tray" is the same setting
 * as on the Backups screen. Without the tray icon nothing hides the window
 * (the app would be unreachable), so those options are turned off here.
 */
import { useEffect, useState } from "react";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";

/** Mirrors `startup::StartupSettings`. */
export interface StartupSettings {
  start_at_login: boolean;
  start_minimized: boolean;
  start_hidden_at_login: boolean;
  close_to_tray: boolean;
  minimize_to_tray: boolean;
  show_tray_icon: boolean;
}

function Toggle({
  label,
  hint,
  checked,
  disabled,
  onChange,
  testId,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  disabled?: boolean;
  onChange: (v: boolean) => void;
  testId: string;
}) {
  return (
    <label className={`flex items-start gap-2 text-xs ${disabled ? "opacity-50" : ""}`}>
      <input type="checkbox" className="mt-0.5" checked={checked} disabled={disabled} onChange={(e) => onChange(e.target.checked)} data-testid={testId} />
      <span>
        {label}
        {hint && <span className="block text-[11px] text-gray-500">{hint}</span>}
      </span>
    </label>
  );
}

export function StartupTraySection() {
  const [s, setS] = useState<StartupSettings | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const desktop = isTauriAvailable();

  useEffect(() => {
    if (!desktop) return;
    tauriInvoke<StartupSettings>("get_startup_settings").then(setS).catch((e) => setErr(errorText(e)));
  }, [desktop]);

  if (!desktop) return <p className="text-xs text-gray-500 pt-3">Available in the desktop app.</p>;
  if (!s) return err ? <p className="text-xs text-red-600 pt-3">{err}</p> : null;

  const update = (patch: Partial<StartupSettings>) => {
    const next = { ...s, ...patch };
    // No tray icon: nothing may hide the window.
    if (!next.show_tray_icon) Object.assign(next, { close_to_tray: false, minimize_to_tray: false });
    setSaving(true);
    setErr(null);
    tauriInvoke<StartupSettings>("set_startup_settings", { settings: next })
      .then(setS)
      .catch((e) => setErr(errorText(e)))
      .finally(() => setSaving(false));
  };
  const noTray = !s.show_tray_icon;

  return (
    <div className="space-y-2 pt-3" data-testid="startup-tray-settings">
      <Toggle label="Start UFOP when I log in" checked={s.start_at_login} disabled={saving} onChange={(v) => update({ start_at_login: v })} testId="startup-at-login" />
      <div className="pl-5">
        <Toggle
          label="…in the tray, without opening the window"
          checked={s.start_hidden_at_login}
          disabled={saving || !s.start_at_login}
          onChange={(v) => update({ start_hidden_at_login: v })}
          testId="startup-hidden-at-login"
        />
      </div>
      <Toggle
        label="Start minimised"
        hint={noTray ? "Without the tray icon the window starts minimised in the taskbar." : "Every start, the window stays in the tray until you open it."}
        checked={s.start_minimized}
        disabled={saving}
        onChange={(v) => update({ start_minimized: v })}
        testId="startup-minimized"
      />
      <Toggle
        label="Keep running in the tray when the window is closed"
        hint="Scheduled backups, syncs and automations keep running. Quit from the tray menu."
        checked={s.close_to_tray}
        disabled={saving || noTray}
        onChange={(v) => update({ close_to_tray: v })}
        testId="startup-close-to-tray"
      />
      <Toggle label="Minimise to the tray" checked={s.minimize_to_tray} disabled={saving || noTray} onChange={(v) => update({ minimize_to_tray: v })} testId="startup-minimize-to-tray" />
      <Toggle
        label="Show the tray icon"
        hint={noTray ? "Turn it on to keep UFOP in the tray." : undefined}
        checked={s.show_tray_icon}
        disabled={saving}
        onChange={(v) => update({ show_tray_icon: v })}
        testId="startup-show-tray"
      />
      {err && <p className="text-xs text-red-600">{err}</p>}
    </div>
  );
}
