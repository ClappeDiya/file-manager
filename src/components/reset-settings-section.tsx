/**
 * Settings → Reset: choose which preferences go back to their defaults,
 * confirm, reset. Data and integrations are listed as kept.
 */
import { useState } from "react";
import { isTauriAvailable } from "@/hooks/use-tauri";
import { confirmText } from "@/lib/prompt-dialog";
import { errorText } from "@/lib/error-text";
import { RESET_KEEPS, RESET_SCOPES, resetSettings, type ResetScope } from "@/lib/reset-settings";

const DONE = { ok: true, text: "Settings reset to the defaults." };

/** `justReset`: the panel remounted after a reset (say it worked). */
export function ResetSettingsSection({ onReset, justReset = false }: { onReset?: () => void; justReset?: boolean }) {
  const desktop = isTauriAvailable();
  const offered = RESET_SCOPES.filter((s) => desktop || !s.desktopOnly);
  const [chosen, setChosen] = useState<Set<ResetScope>>(() => new Set(offered.filter((s) => s.id !== "tips").map((s) => s.id)));
  const [busy, setBusy] = useState(false);
  const [msg, setMsg] = useState<{ ok: boolean; text: string } | null>(justReset ? DONE : null);

  const toggle = (id: ResetScope, on: boolean) =>
    setChosen((prev) => {
      const next = new Set(prev);
      if (on) next.add(id);
      else next.delete(id);
      return next;
    });

  const run = async () => {
    const scopes = offered.map((s) => s.id).filter((id) => chosen.has(id));
    const names = offered.filter((s) => chosen.has(s.id)).map((s) => s.label.toLowerCase());
    const ok = await confirmText(`Reset ${names.join(", ")} to the defaults? Kept: ${RESET_KEEPS}`, "Reset", true);
    if (!ok) return;
    setBusy(true);
    setMsg(null);
    try {
      await resetSettings(scopes);
      setMsg(DONE);
      onReset?.();
    } catch (e) {
      setMsg({ ok: false, text: errorText(e) });
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="space-y-2 pt-3" data-testid="reset-settings">
      {offered.map((s) => (
        <label key={s.id} className="flex items-start gap-2 text-xs">
          <input type="checkbox" className="mt-0.5" checked={chosen.has(s.id)} onChange={(e) => toggle(s.id, e.target.checked)} data-testid={`reset-scope-${s.id}`} />
          <span>
            {s.label}
            <span className="block text-[11px] text-gray-500">{s.covers}</span>
          </span>
        </label>
      ))}
      <p className="text-[11px] text-gray-500">Always kept: {RESET_KEEPS}</p>
      <button
        type="button"
        disabled={busy || chosen.size === 0}
        onClick={() => void run()}
        className="px-3 py-1 text-xs rounded bg-red-600 text-white hover:bg-red-700 disabled:opacity-50"
        data-testid="reset-settings-button"
      >
        {busy ? "Resetting…" : "Reset to defaults…"}
      </button>
      {msg && <p className={`text-xs ${msg.ok ? "text-green-700 dark:text-green-400" : "text-red-600"}`}>{msg.text}</p>}
    </div>
  );
}
