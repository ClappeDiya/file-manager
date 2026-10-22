/** Sharing policy (Settings): which share links may be created. */
import { useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { isTauriAvailable } from "@/hooks/use-tauri";
import { DEFAULT_SHARING_POLICY, sharing, type SharingPolicy } from "@/stores/share-store";

export function SharingPolicySection() {
  const [p, setP] = useState<SharingPolicy>(DEFAULT_SHARING_POLICY);
  const [saved, setSaved] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    if (isTauriAvailable()) sharing.policy().then(setP, (e) => setError(errorText(e)));
  }, []);
  const save = async (next: SharingPolicy) => {
    setP(next);
    setSaved(false);
    setError(null);
    try {
      setP(await sharing.setPolicy(next));
      setSaved(true);
    } catch (e) {
      setError(errorText(e));
    }
  };
  const box = (key: keyof SharingPolicy, label: string, disabled = false) => (
    <label className="flex items-center gap-2">
      <input type="checkbox" disabled={disabled} checked={p[key] as boolean} onChange={(e) => save({ ...p, [key]: e.target.checked })} /> {label}
    </label>
  );
  return (
    <div className="space-y-2 pt-3 text-xs">
      <p className="text-gray-500">Applies to links created from cloud connections (S3, Azure, OneDrive, Dropbox, Google Drive).</p>
      {box("enabled", "Allow creating share links")}
      {box("allow_anyone", "Allow links anyone can open (off = organization only)", !p.enabled)}
      {box("require_password", "Links anyone can open need a password", !p.enabled)}
      {box("allow_edit", "Allow links that grant editing", !p.enabled)}
      {box("require_expiry", "Links must expire", !p.enabled)}
      <label className="flex items-center gap-2">
        Longest allowed expiry (days)
        <input
          type="number"
          min={1}
          disabled={!p.enabled}
          className="w-20 px-2 py-0.5 rounded border border-gray-200 dark:border-gray-700 bg-transparent"
          value={p.max_expiry_days ?? ""}
          placeholder="none"
          onChange={(e) => save({ ...p, max_expiry_days: e.target.value ? Math.max(1, Number(e.target.value)) : null })}
          aria-label="Longest allowed expiry (days)"
        />
      </label>
      {saved && <p className="text-green-600">Saved</p>}
      {error && <p className="text-red-500" role="alert">{error}</p>}
    </div>
  );
}
