/**
 * Mounts: every saved and running mount with its state (mounted, unmounted,
 * error) and the actions that apply: mount, unmount, open, edit, remove,
 * and unmount all. "Mount drive…" opens the mount dialog; "Edit" opens it
 * filled in with the saved settings. Shown from the toolbar, the command
 * palette and the tray; reloads when a mount changes anywhere (including the
 * tray's own mount and unmount items).
 */
import { useCallback, useEffect, useState } from "react";
import {
  HardDrive,
  Plug,
  PlugZap,
  RefreshCw,
  AlertCircle,
  CheckCircle2,
  Circle,
  FolderOpen,
  Pencil,
  Trash2,
} from "lucide-react";
import {
  isTauriAvailable,
  tauriInvoke,
  tauriInvokeSafe,
} from "@/hooks/use-tauri";
import { MountOfflineActions } from "@/components/mount-options";
import {
  isCloudMount,
  type MountInfo,
  type MountOptions,
  type MountResult,
  type UnmountAllReport,
} from "@/lib/mount-options";
import { errorText } from "@/lib/error-text";
import { useMountsUi } from "@/stores/mounts-ui-store";

export const MOUNTS_CHANGED_EVENT = "ufop://mounts-changed";

export function StatusBadge({ status }: { status: MountInfo["status"] }) {
  switch (status) {
    case "mounted":
      return (
        <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400">
          <CheckCircle2 size={12} />
          Mounted
        </span>
      );
    case "unmounted":
      return (
        <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400">
          <Circle size={12} />
          Unmounted
        </span>
      );
    case "error":
      return (
        <span className="inline-flex items-center gap-1 text-xs px-2 py-0.5 rounded-full bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400">
          <AlertCircle size={12} />
          Error
        </span>
      );
  }
}

const iconButton =
  "inline-flex items-center gap-1 text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50 disabled:cursor-not-allowed";

function MountRow({
  mount,
  busy,
  onMount,
  onUnmount,
  onOpen,
  onEdit,
  onRemove,
  error,
}: {
  mount: MountInfo;
  busy: boolean;
  onMount: (m: MountInfo) => void;
  onUnmount: (m: MountInfo) => void;
  onOpen: (m: MountInfo) => void;
  onEdit: (m: MountInfo) => void;
  onRemove: (m: MountInfo) => void;
  error: string | null;
}) {
  const [confirming, setConfirming] = useState(false);
  const mounted = mount.status === "mounted";
  return (
    <li
      className="px-3 py-2 border-b border-gray-200 dark:border-gray-700 last:border-b-0"
      data-testid={`mount-row-${mount.id}`}
    >
      <div className="flex items-start justify-between gap-2">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2">
            <HardDrive
              size={14}
              className="text-gray-500 flex-shrink-0"
              aria-hidden="true"
            />
            <span className="text-sm font-medium truncate">
              {mount.connection_name}
            </span>
            <StatusBadge status={mount.status} />
          </div>
          <p
            className="text-xs text-gray-500 mt-0.5 truncate pl-5"
            title={mount.mount_point}
          >
            {mount.mount_point}
          </p>
          <p className="text-xs text-gray-400 pl-5">
            {mount.protocol.toUpperCase()}
            {mount.options?.subfolder
              ? ` | Folder: ${mount.options.subfolder}`
              : ""}
            {mount.auto_mount ? " | Auto-mount" : ""}
          </p>
          {mount.error_message && (
            <p className="text-xs text-red-500 pl-5 mt-0.5">
              {mount.error_message}
            </p>
          )}
          {!!mount.policy_notes?.length && (
            <p className="text-xs text-gray-400 pl-5">
              Profile: {mount.policy_notes.join(", ")}
            </p>
          )}
          {error && (
            <p className="text-xs text-red-500 pl-5 mt-0.5" role="alert">
              {error}
            </p>
          )}
          {isCloudMount(mount.protocol) && mount.options && (
            <MountOfflineActions
              mountId={mount.id}
              mounted={mounted}
              options={mount.options}
            />
          )}
        </div>
        <div className="flex flex-wrap justify-end items-center gap-1 flex-shrink-0">
          {mounted ? (
            <>
              <button
                type="button"
                className={iconButton}
                disabled={busy}
                onClick={() => onOpen(mount)}
                aria-label={`Open ${mount.connection_name}`}
              >
                <FolderOpen size={12} aria-hidden="true" />
                Open
              </button>
              <button
                type="button"
                className={iconButton}
                disabled={busy}
                onClick={() => onUnmount(mount)}
                aria-label={`Unmount ${mount.connection_name}`}
              >
                <PlugZap size={12} aria-hidden="true" />
                Unmount
              </button>
            </>
          ) : (
            <button
              type="button"
              className={iconButton}
              disabled={busy}
              onClick={() => onMount(mount)}
              aria-label={`Mount ${mount.connection_name}`}
            >
              <Plug size={12} aria-hidden="true" />
              Mount
            </button>
          )}
          <button
            type="button"
            className={iconButton}
            disabled={busy || mounted}
            title={mounted ? "Unmount the drive to edit it" : "Edit this mount"}
            onClick={() => onEdit(mount)}
            aria-label={`Edit ${mount.connection_name}`}
          >
            <Pencil size={12} aria-hidden="true" />
            Edit
          </button>
          {confirming ? (
            <span className="inline-flex items-center gap-1 text-xs">
              {mounted ? "Unmount and remove?" : "Remove?"}
              <button
                type="button"
                className={`${iconButton} text-red-600`}
                disabled={busy}
                onClick={() => {
                  setConfirming(false);
                  onRemove(mount);
                }}
                aria-label={`Confirm remove ${mount.connection_name}`}
              >
                Yes
              </button>
              <button
                type="button"
                className={iconButton}
                onClick={() => setConfirming(false)}
              >
                No
              </button>
            </span>
          ) : (
            <button
              type="button"
              className={`${iconButton} text-red-600`}
              disabled={busy}
              onClick={() => setConfirming(true)}
              aria-label={`Remove ${mount.connection_name}`}
            >
              <Trash2 size={12} aria-hidden="true" />
              Remove
            </button>
          )}
        </div>
      </div>
    </li>
  );
}

export function MountsPanel() {
  const [mounts, setMounts] = useState<MountInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [busyId, setBusyId] = useState<string | null>(null);
  const [rowErrors, setRowErrors] = useState<Record<string, string>>({});
  const [note, setNote] = useState<string | null>(null);
  const [allBusy, setAllBusy] = useState(false);
  const refreshTick = useMountsUi((s) => s.refreshTick);
  const openDialog = useMountsUi((s) => s.openDialog);
  const bump = useMountsUi((s) => s.bump);

  const load = useCallback(async () => {
    setMounts(await tauriInvokeSafe<MountInfo[]>("list_mounts", undefined, []));
    setLoading(false);
  }, []);

  useEffect(() => {
    void load();
  }, [load, refreshTick]);

  // The tray (and auto-mount) change mounts outside this window's own actions.
  useEffect(() => {
    if (!isTauriAvailable()) return;
    let unlisten: (() => void) | undefined;
    let gone = false;
    void import("@tauri-apps/api/event").then(({ listen }) =>
      listen(MOUNTS_CHANGED_EVENT, () => void load()).then((u) => {
        if (gone) u();
        else unlisten = u;
      }),
    );
    return () => {
      gone = true;
      unlisten?.();
    };
  }, [load]);

  const act = async (m: MountInfo, f: () => Promise<string | void>) => {
    setBusyId(m.id);
    setNote(null);
    setRowErrors((e) => ({ ...e, [m.id]: "" }));
    try {
      const msg = await f();
      if (msg) setNote(msg);
    } catch (err) {
      setRowErrors((e) => ({ ...e, [m.id]: errorText(err) }));
    } finally {
      setBusyId(null);
      // Reloads this list and any other open one.
      bump();
    }
  };

  const mount = (m: MountInfo) =>
    act(
      m,
      async () =>
        (
          await tauriInvoke<MountResult>("mount_saved_config", {
            mountId: m.id,
          })
        ).message,
    );
  const unmount = (m: MountInfo) =>
    act(m, async () => {
      await tauriInvoke<void>("unmount_remote", { mountId: m.id });
    });
  const open = (m: MountInfo) =>
    act(m, async () => {
      await tauriInvoke<void>("open_mount_point", { mountId: m.id });
    });
  const remove = (m: MountInfo) =>
    act(m, async () => {
      await tauriInvoke<void>("delete_mount_config", { mountId: m.id });
    });

  const unmountAll = async () => {
    setAllBusy(true);
    setNote(null);
    try {
      const r = await tauriInvoke<UnmountAllReport>("unmount_all_mounts");
      setNote(
        r.failed.length
          ? `Unmounted ${r.unmounted}; could not unmount: ${r.failed.join("; ")}`
          : `Unmounted ${r.unmounted} drive${r.unmounted === 1 ? "" : "s"}.`,
      );
    } catch (err) {
      setNote(errorText(err));
    } finally {
      setAllBusy(false);
      // Reloads this list and any other open one.
      bump();
    }
  };

  // Edit starts from what the user saved, not from the options the organization's
  // profile forced onto the running mount.
  const edit = async (m: MountInfo) => {
    const saved = await tauriInvokeSafe<MountOptions | null>("saved_mount_options", { mountId: m.id }, null);
    openDialog({ editing: saved ? { ...m, options: saved } : m });
  };

  const anyMounted = mounts.some((m) => m.status === "mounted");

  return (
    <div className="flex h-full flex-col" data-testid="mounts-panel">
      <div className="flex items-center justify-between gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        <h2 className="text-base font-semibold flex items-center gap-2">
          <HardDrive size={18} className="text-blue-500" aria-hidden="true" />
          Mounts
        </h2>
        <div className="flex items-center gap-2 pr-8">
          <button
            type="button"
            className={iconButton}
            onClick={() => openDialog()}
          >
            <Plug size={12} aria-hidden="true" />
            Mount drive…
          </button>
          <button
            type="button"
            className={iconButton}
            disabled={!anyMounted || allBusy}
            onClick={() => void unmountAll()}
          >
            <PlugZap size={12} aria-hidden="true" />
            Unmount all
          </button>
          <button
            type="button"
            className="p-1 rounded text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
            onClick={() => void load()}
            title="Refresh"
            aria-label="Refresh mounts"
          >
            <RefreshCw size={14} className={loading ? "animate-spin" : ""} />
          </button>
        </div>
      </div>
      {note && (
        <p
          className="px-4 py-2 text-xs text-gray-600 dark:text-gray-300 border-b border-gray-200 dark:border-gray-700"
          role="status"
        >
          {note}
        </p>
      )}
      <div className="flex-1 overflow-y-auto">
        {!loading && mounts.length === 0 ? (
          <p className="text-sm text-gray-500 text-center py-10 px-4">
            No mounts yet. Choose “Mount drive…” to open a connection as a
            folder on this computer.
          </p>
        ) : (
          <ul>
            {mounts.map((m) => (
              <MountRow
                key={m.id}
                mount={m}
                busy={busyId === m.id}
                error={rowErrors[m.id] || null}
                onMount={mount}
                onUnmount={unmount}
                onOpen={open}
                onRemove={remove}
                onEdit={edit}
              />
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

export default MountsPanel;
