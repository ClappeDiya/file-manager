/** Settings → Managed by Console: what the organization currently manages. */
import { useEffect, useState } from "react";
import { describeManaged, loadManagedSettings, type ManagedSettings } from "@/lib/managed-settings";

export function ManagedSettingsNotice() {
  const [managed, setManaged] = useState<ManagedSettings | null>(null);

  useEffect(() => {
    let live = true;
    void loadManagedSettings().then((m) => live && setManaged(m));
    return () => {
      live = false;
    };
  }, []);

  const lines = describeManaged(managed);
  if (lines.length === 0) return null;
  return (
    <div className="mt-3 rounded border border-amber-300 dark:border-amber-700 bg-amber-50 dark:bg-amber-950/30 p-2 text-xs" data-testid="managed-settings-notice">
      <p className="font-medium mb-1">Set by your organization</p>
      <ul className="list-disc pl-4 space-y-0.5">
        {lines.map((l) => (
          <li key={l}>{l}</li>
        ))}
      </ul>
    </div>
  );
}
