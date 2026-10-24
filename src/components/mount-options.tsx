/**
 * Cache and read-only options of cloud (rclone) mounts, the organization's
 * mount profile that constrains them, and per-mount offline actions:
 * "make available offline" (full cache), cache usage and clearing.
 * Mirrors `mount_engine::{MountOptions, PinReport}` and
 * `ufop_core::agent::ManagedMounts`.
 */
import { useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { pickFolder } from "@/lib/folder-picker";
import {
  CACHE_MODES,
  human,
  hostOs,
  type CacheMode,
  type HostOs,
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

/**
 * Advanced options of a cloud mount: drive letter, volume label, permissions,
 * cache folder, directory-cache time and extra rclone flags. Options the
 * operating system lacks are not shown (the engine refuses them there too).
 */
export function MountAdvancedFields({
  value,
  onChange,
  policy,
  os = hostOs(),
}: {
  value: MountOptions;
  onChange: (o: MountOptions) => void;
  policy: ManagedMounts | null;
  os?: HostOs;
}) {
  const [letters, setLetters] = useState<string[]>([]);
  const [flagsText, setFlagsText] = useState(
    (value.extra_flags ?? []).join(" "),
  );
  useEffect(() => {
    if (os !== "windows") return;
    let live = true;
    void tauriInvokeSafe<string[]>("free_drive_letters", undefined, []).then(
      (l) => live && setLetters(l),
    );
    return () => {
      live = false;
    };
  }, [os]);
  const text = (v: string) => (v === "" ? null : v);
  const id = (v: string) =>
    v.trim() === "" ? null : Math.max(0, Math.floor(Number(v)));
  const set = (patch: Partial<MountOptions>) =>
    onChange({ ...value, ...patch });
  // The chosen letter stays selectable even when it is no longer listed as free.
  const choices =
    value.drive_letter && !letters.includes(value.drive_letter)
      ? [value.drive_letter, ...letters]
      : letters;
  return (
    <fieldset
      className="space-y-2 rounded border border-gray-200 dark:border-gray-700 p-2"
      aria-label="Advanced mount options"
      data-testid="mount-advanced"
    >
      <legend className="text-xs font-medium px-1">Advanced</legend>
      {os === "windows" && (
        <label className="block text-xs">
          Drive letter
          <select
            aria-label="Drive letter"
            className={field}
            value={value.drive_letter ?? ""}
            disabled={!!policy?.mountRoot}
            onChange={(e) => set({ drive_letter: text(e.target.value) })}
          >
            <option value="">None — mount in a folder</option>
            {choices.map((l) => (
              <option key={l} value={l}>
                {l}:
              </option>
            ))}
          </select>
          {policy?.mountRoot && (
            <span className="text-gray-500">
              Your organization requires mount points inside {policy.mountRoot}.
            </span>
          )}
        </label>
      )}
      {os !== "linux" && (
        <label className="block text-xs">
          Volume label
          <input
            aria-label="Volume label"
            className={field}
            maxLength={32}
            placeholder="Name shown in the file manager"
            value={value.volume_label ?? ""}
            onChange={(e) => set({ volume_label: text(e.target.value) })}
          />
        </label>
      )}
      {os === "windows" && (
        <label className="flex items-center gap-2 text-xs">
          <input
            type="checkbox"
            aria-label="Network drive"
            checked={!!value.network_mode}
            onChange={(e) => set({ network_mode: e.target.checked })}
          />
          Show as a network drive
        </label>
      )}
      {os !== "windows" && (
        <label className="flex items-center gap-2 text-xs">
          <input
            type="checkbox"
            aria-label="Allow other users"
            checked={!!value.allow_other}
            onChange={(e) => set({ allow_other: e.target.checked })}
          />
          Let other users of this computer see the drive (needs user_allow_other
          in /etc/fuse.conf)
        </label>
      )}
      <div className="grid grid-cols-2 gap-2">
        <label className="block text-xs">
          File permissions
          <input
            aria-label="File permissions"
            className={field}
            placeholder="0644"
            value={value.file_perms ?? ""}
            onChange={(e) => set({ file_perms: text(e.target.value) })}
          />
        </label>
        <label className="block text-xs">
          Folder permissions
          <input
            aria-label="Folder permissions"
            className={field}
            placeholder="0755"
            value={value.dir_perms ?? ""}
            onChange={(e) => set({ dir_perms: text(e.target.value) })}
          />
        </label>
        {os !== "windows" && (
          <>
            <label className="block text-xs">
              Owner (user id)
              <input
                aria-label="Owner"
                type="number"
                min={0}
                className={field}
                value={value.uid ?? ""}
                onChange={(e) => set({ uid: id(e.target.value) })}
              />
            </label>
            <label className="block text-xs">
              Group (group id)
              <input
                aria-label="Group"
                type="number"
                min={0}
                className={field}
                value={value.gid ?? ""}
                onChange={(e) => set({ gid: id(e.target.value) })}
              />
            </label>
          </>
        )}
      </div>
      <label className="block text-xs">
        Cache folder
        <span className="flex gap-1">
          <input
            aria-label="Cache folder"
            className={field}
            placeholder="Default: the app's cache folder"
            value={value.cache_dir ?? ""}
            onChange={(e) => set({ cache_dir: text(e.target.value) })}
          />
          <button
            type="button"
            className="shrink-0 px-2 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
            onClick={async () => {
              const dir = await pickFolder("Choose a cache folder");
              if (dir) set({ cache_dir: dir });
            }}
          >
            Browse
          </button>
        </span>
      </label>
      <label className="block text-xs">
        Directory cache
        <input
          aria-label="Directory cache"
          className={field}
          placeholder="30s"
          value={value.dir_cache_time ?? ""}
          onChange={(e) => set({ dir_cache_time: text(e.target.value) })}
        />
        <span className="text-gray-500">
          How long folder listings are trusted, e.g. 30s, 5m, 1h.
        </span>
      </label>
      <label className="block text-xs">
        Extra rclone flags
        <input
          aria-label="Extra rclone flags"
          className={field}
          placeholder="--buffer-size=64M --no-modtime"
          value={flagsText}
          onChange={(e) => {
            setFlagsText(e.target.value);
            set({ extra_flags: e.target.value.split(/\s+/).filter(Boolean) });
          }}
        />
        <span className="text-gray-500">
          Only read and timeout tuning flags are accepted; options with their
          own field above, and anything your organization sets, can't be changed
          here.
        </span>
      </label>
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
