/**
 * Settings › Advanced › Sync defaults: the copy options every new sync pair
 * starts from. A pair stores only the options it changes; the rest follow
 * these defaults on every run. Pairs created before defaults existed keep
 * all their own options.
 */
import { useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { DEFAULT_COPY_OPTIONS, countChangedOptions, type CopyOptions } from "@/lib/copy-options";
import { RobocopyOptionsSection } from "@/components/robocopy-options";

export function SyncDefaultsSection() {
  const [options, setOptions] = useState<CopyOptions | null>(null);
  const [status, setStatus] = useState<{ ok: boolean; text: string } | null>(null);

  useEffect(() => {
    let live = true;
    void tauriInvokeSafe<CopyOptions>("get_sync_default_options", undefined, DEFAULT_COPY_OPTIONS).then(
      (o) => live && setOptions({ ...DEFAULT_COPY_OPTIONS, ...o }),
    );
    return () => {
      live = false;
    };
  }, []);

  if (!options) return null;

  const save = async () => {
    setStatus(null);
    try {
      const saved = await tauriInvoke<CopyOptions>("set_sync_default_options", { options });
      setOptions({ ...DEFAULT_COPY_OPTIONS, ...saved });
      setStatus({ ok: true, text: "Saved. Pairs that follow the defaults use them from their next run." });
    } catch (e) {
      setStatus({ ok: false, text: errorText(e) });
    }
  };

  return (
    <div className="space-y-2" data-testid="sync-defaults">
      <p className="text-xs text-gray-500 dark:text-gray-400">
        New sync pairs start from these copy options and keep following them for every option they don't change.
        {countChangedOptions(options) > 0 && ` ${countChangedOptions(options)} changed from the built-in defaults.`}
      </p>
      <RobocopyOptionsSection options={options} onChange={setOptions} mode="any" />
      <div className="flex items-center gap-2">
        <button type="button" onClick={() => void save()} className="text-xs px-3 py-1 rounded bg-blue-500 text-white hover:bg-blue-600">
          Save sync defaults
        </button>
        <button type="button" onClick={() => setOptions(DEFAULT_COPY_OPTIONS)} className="text-xs px-3 py-1 rounded border border-gray-300 dark:border-gray-600">
          Built-in defaults
        </button>
        {status && <span className={`text-xs ${status.ok ? "text-green-600" : "text-red-600"}`}>{status.text}</span>}
      </div>
    </div>
  );
}
