/**
 * Everyday sync options shown in the main pair form (not only in the
 * Robocopy section): what happens to the source after copying, the
 * still-being-written guard, a completion marker and version names.
 * Maps onto `copy_options.move_files` and `SyncRunOptions`.
 */
import { useEffect, useState } from "react";
import { tauriInvoke, isTauriAvailable } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import type { CopyOptions } from "@/lib/copy-options";
import { RunNotifyPicker } from "@/components/run-notify-picker";
import { useUIStore } from "@/stores/ui-store";
import { DEFAULT_VERSION_TEMPLATE, type SyncRunOptions } from "@/stores/sync-store";


const field = "mt-1 block w-full px-2 py-1 text-xs border rounded dark:bg-zinc-800 dark:border-zinc-600";
const labelCls = "block text-xs text-zinc-600 dark:text-zinc-400";

function VersionPreview({ template }: { template: string }) {
  const [out, setOut] = useState<{ ok: boolean; text: string } | null>(null);
  useEffect(() => {
    if (!isTauriAvailable() || !template.trim()) return setOut(null);
    let live = true;
    const t = setTimeout(() => {
      tauriInvoke<string>("naming_preview", { template, samplePath: "report.pdf", matchRegex: null, asPath: false })
        .then((text) => live && setOut({ ok: true, text }))
        .catch((e) => live && setOut({ ok: false, text: errorText(e) }));
    }, 250);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [template]);
  if (!out) return null;
  return (
    <p className={`text-[10px] font-mono ${out.ok ? "text-zinc-400" : "text-red-500"}`} data-testid="version-preview">
      {out.ok ? `report.pdf → ${out.text}` : out.text}
    </p>
  );
}

export function SyncRunOptionsSection({
  mode,
  runOptions,
  copyOptions,
  onChange,
}: {
  mode: string;
  runOptions: SyncRunOptions;
  copyOptions: CopyOptions;
  onChange: (runOptions: SyncRunOptions, copyOptions: CopyOptions) => void;
}) {
  const set = (patch: Partial<SyncRunOptions>) => onChange({ ...runOptions, ...patch }, copyOptions);
  const twoWay = mode === "two_way";
  const advanced = useUIStore((s) => s.appMode === "advanced");
  return (
    <>
    <fieldset className="space-y-2 rounded border border-zinc-200 dark:border-zinc-700 p-2">
      <legend className="px-1 text-xs font-medium text-zinc-600 dark:text-zinc-400">After copying</legend>
      {!twoWay && (
        <label className={labelCls}>
          Source files
          <select
            className={field}
            value={copyOptions.move_files ? "move" : "keep"}
            onChange={(e) => onChange(runOptions, { ...copyOptions, move_files: e.target.value === "move" })}
          >
            <option value="keep">Keep them (copy)</option>
            <option value="move">Delete them once copied (move)</option>
          </select>
        </label>
      )}
      <label className={labelCls}>
        Skip files changed in the last (seconds)
        <input
          type="number"
          min={0}
          className={field}
          value={runOptions.settle_secs}
          onChange={(e) => set({ settle_secs: Math.max(0, Number(e.target.value) || 0) })}
        />
        <span className="text-[10px] text-zinc-400">Files still being written are left for the next run. 0 = copy everything.</span>
      </label>
      <label className={labelCls}>
        Completion marker file
        <input
          className={field}
          placeholder="none — e.g. _SYNC_COMPLETE"
          value={runOptions.completion_marker ?? ""}
          onChange={(e) => set({ completion_marker: e.target.value.trim() ? e.target.value : null })}
        />
        <span className="text-[10px] text-zinc-400">Written at the destination after each run without errors.</span>
      </label>
      {mode === "versioned_backup" && (
        <label className={labelCls}>
          Version names
          <input
            className={`${field} font-mono`}
            placeholder={DEFAULT_VERSION_TEMPLATE}
            value={runOptions.version_template ?? ""}
            onChange={(e) => set({ version_template: e.target.value.trim() ? e.target.value : null })}
          />
          <VersionPreview template={runOptions.version_template ?? DEFAULT_VERSION_TEMPLATE} />
        </label>
      )}
    </fieldset>
    <RunNotifyPicker value={runOptions} onChange={set} advanced={advanced} />
    </>
  );
}
