/**
 * Cache and read-only options of cloud (rclone) mounts, the organization's
 * mount profile that constrains them, and per-mount offline actions:
 * "make available offline" (full cache), cache usage and clearing.
 * Mirrors `mount_engine::{MountOptions, PinReport}` and
 * `ufop_core::agent::ManagedMounts`.
 */
import { useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import {
  CACHE_MODES,
  human,
  type CacheMode,
  type ManagedMounts,
  type MountOptions,
  type PinReport,
} from "@/lib/mount-options";

const field =
  "w-full text-sm px-2 py-1 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900 disabled:opacity-60";

/** Cache options for a new cloud mount; fields the profile forces are locked. */
export function MountCacheFields({
  value,
  onChange,
  policy,
}: {
  value: MountOptions;
  onChange: (o: MountOptions) => void;
  policy: ManagedMounts | null;
}) {
  const num = (v: string) =>
    v.trim() === "" ? null : Math.max(1, Math.floor(Number(v)));
  return (
    <fieldset
      className="space-y-2 rounded border border-gray-200 dark:border-gray-700 p-2"
      aria-label="Cache options"
    >
      <legend className="text-xs font-medium px-1">Cache</legend>
      <label className="block text-xs">
        Cache mode
        <select
          aria-label="Cache mode"
          className={field}
          value={policy?.cacheMode ?? value.cache_mode}
          disabled={!!policy?.cacheMode}
          onChange={(e) =>
            onChange({ ...value, cache_mode: e.target.value as CacheMode })
          }
        >
          {CACHE_MODES.map(([m, l]) => (
            <option key={m} value={m}>
              {l}
            </option>
          ))}
        </select>
      </label>
      <div className="grid grid-cols-2 gap-2">
        <label className="block text-xs">
          Largest cache (MB)
          <input
            aria-label="Largest cache"
            type="number"
            min={1}
            max={policy?.maxCacheMb ?? undefined}
            className={field}
            placeholder={
              policy?.maxCacheMb ? `≤ ${policy.maxCacheMb}` : "Unlimited"
            }
            value={value.max_cache_mb ?? ""}
            onChange={(e) =>
              onChange({ ...value, max_cache_mb: num(e.target.value) })
            }
          />
        </label>
        <label className="block text-xs">
          Keep cached files (hours)
          <input
            aria-label="Keep cached files"
            type="number"
            min={1}
            max={policy?.maxCacheAgeHours ?? undefined}
            className={field}
            placeholder={
              policy?.maxCacheAgeHours ? `≤ ${policy.maxCacheAgeHours}` : "1"
            }
            value={value.max_cache_age_hours ?? ""}
            onChange={(e) =>
              onChange({ ...value, max_cache_age_hours: num(e.target.value) })
            }
          />
        </label>
      </div>
      <label className="flex items-center gap-2 text-xs">
        <input
          type="checkbox"
          aria-label="Read-only"
          checked={policy?.readOnly ? true : value.read_only}
          disabled={!!policy?.readOnly}
          onChange={(e) => onChange({ ...value, read_only: e.target.checked })}
        />
        Read-only
      </label>
      {(policy?.cacheMode ?? value.cache_mode) === "full" && (
        <p className="text-xs text-gray-500">
          Files you open, and folders you make available offline, stay readable
          without a connection.
        </p>
      )}
    </fieldset>
  );
}

/** Offline and cache actions of one cloud mount. */
export function MountOfflineActions({
  mountId,
  mounted,
  options,
}: {
  mountId: string;
  mounted: boolean;
  options: MountOptions;
}) {
  const [usage, setUsage] = useState<number | null>(null);
  const [folder, setFolder] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const refresh = () =>
    void tauriInvokeSafe<number>("mount_cache_usage", { mountId }, 0).then(
      setUsage,
    );
  useEffect(refresh, [mountId]);

  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    setNote(null);
    try {
      await f();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
      refresh();
    }
  };

  return (
    <div
      className="pl-5 mt-1 space-y-1 text-xs"
      data-testid="mount-offline-actions"
    >
      <p className="text-gray-500">
        Cache: {options.cache_mode}
        {options.read_only && " · read-only"}
        {usage !== null && ` · ${human(usage)} on this computer`}
      </p>
      {mounted && options.cache_mode === "full" && (
        <div className="flex gap-1">
          <input
            aria-label="Folder to keep offline"
            className={field}
            placeholder="Folder in the drive (empty = everything)"
            value={folder}
            onChange={(e) => setFolder(e.target.value)}
          />
          <button
            type="button"
            disabled={busy}
            className="shrink-0 px-2 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50"
            onClick={() =>
              void run(async () => {
                const r = await tauriInvoke<PinReport>("mount_pin_offline", {
                  mountId,
                  path: folder.trim() || null,
                });
                setNote(
                  `${r.files} file${r.files === 1 ? "" : "s"} (${human(r.bytes)}) available offline${r.failed ? `, ${r.failed} could not be read` : ""}${r.truncated ? " — the cache limit was reached" : ""}.`,
                );
              })
            }
          >
            {busy ? "Downloading…" : "Make available offline"}
          </button>
        </div>
      )}
      {!mounted && !!usage && (
        <button
          type="button"
          disabled={busy}
          className="underline text-gray-600 dark:text-gray-400"
          onClick={() =>
            void run(async () => {
              const freed = await tauriInvoke<number>("mount_clear_cache", {
                mountId,
              });
              setNote(`Freed ${human(freed)}.`);
            })
          }
        >
          Clear cache
        </button>
      )}
      {note && (
        <p className="text-green-600 dark:text-green-400" role="status">
          {note}
        </p>
      )}
      {error && <p className="text-red-500">{error}</p>}
    </div>
  );
}
