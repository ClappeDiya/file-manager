/**
 * Location provider: one place that routes pane operations to the local
 * file-system commands or to the connector commands of a saved connection,
 * by the kind of pane path (see `location.ts`). Callers pass pane paths and
 * get pane paths back, whichever side they are on.
 *
 *   list / stat / mkdir / rename / delete      local fs  |  connector_*
 *   local copy for previews and "open"         the file  |  connector_fetch_preview
 *   copy / move between any two locations      (callers keep copy_files/move_files
 *                                               for local↔local)  |  enqueue_location_transfer
 */
import { tauriInvoke } from "@/hooks/use-tauri";
import type { FileEntryData } from "@/components/file-list";
import { useConnectionsStore, type PaneConnection } from "@/stores/connections-store";
import {
  isRemotePath,
  joinLocation,
  parentLocation,
  parseLocation,
  rememberRemoteNames,
  remoteLocation,
  remoteName,
  sameLocationRoot,
} from "@/lib/location";

/** Tooltip of actions that only work on this computer's folders. */
export const LOCAL_ONLY_REASON = "Not available in connection folders — copy the item to this computer first";

/** A pane path names a connection that is not saved (deleted, or a typo). */
export class UnknownConnectionError extends Error {
  constructor(public connectionId: string) {
    super("This connection is no longer saved. Open Connections to add it again.");
  }
}

/** The saved connection behind a remote pane path. */
export async function connectionFor(path: string): Promise<PaneConnection> {
  const loc = parseLocation(path);
  if (loc.kind !== "remote") throw new Error(`${path} is not a connection location`);
  const store = useConnectionsStore.getState();
  await store.ensureLoaded();
  let conn = useConnectionsStore.getState().byId(loc.connectionId);
  if (!conn) {
    // Saved since the list was loaded?
    await useConnectionsStore.getState().load();
    conn = useConnectionsStore.getState().byId(loc.connectionId);
  }
  if (!conn) throw new UnknownConnectionError(loc.connectionId);
  return conn;
}

/** Connector arguments for a remote pane path. */
async function remoteArgs(path: string) {
  const loc = parseLocation(path) as Extract<ReturnType<typeof parseLocation>, { kind: "remote" }>;
  const conn = await connectionFor(path);
  return { conn, protocol: conn.protocol, connectionId: conn.id, remotePath: loc.path };
}

/** Rewrite a connector entry's path into a pane path. */
function toPaneEntry(connectionId: string, e: FileEntryData): FileEntryData {
  return { ...e, path: remoteLocation(connectionId, e.path) };
}

/** List a folder. */
export async function listDirectory(path: string): Promise<FileEntryData[]> {
  if (!isRemotePath(path)) return tauriInvoke<FileEntryData[]>("list_directory", { path });
  const { protocol, connectionId, remotePath } = await remoteArgs(path);
  const entries = await tauriInvoke<FileEntryData[]>("connector_list_remote", { protocol, path: remotePath, connectionId });
  const out = (entries ?? []).map((e) => toPaneEntry(connectionId, e));
  rememberRemoteNames(out);
  return out;
}

/**
 * The entry at `path`, or null when there is nothing there. A connection
 * entry's `path` is where the item really is: a path typed by names
 * (`/Reports/Q1`) comes back as the connection's own path for it (Google
 * Drive: folder ids), which is what a pane must open.
 */
export async function statLocation(path: string): Promise<FileEntryData | null> {
  if (!isRemotePath(path)) {
    try {
      return await tauriInvoke<FileEntryData>("get_file_metadata", { path });
    } catch {
      return null;
    }
  }
  const { protocol, connectionId, remotePath } = await remoteArgs(path);
  const e = await tauriInvoke<FileEntryData | null>("connector_stat", { protocol, path: remotePath, connectionId });
  return e ? toPaneEntry(connectionId, e) : null;
}

/** Create a folder (`path` is the new folder). */
export async function makeDirectory(path: string): Promise<void> {
  if (!isRemotePath(path)) {
    await tauriInvoke("create_directory", { path });
    return;
  }
  const { protocol, connectionId, remotePath } = await remoteArgs(path);
  await tauriInvoke("connector_make_directory", { protocol, remotePath, connectionId });
}

/**
 * Rename an item in place; returns its new pane path. For a connection the
 * backend says where the item is now: the new name's path, or (Google
 * Drive, which addresses items by id) its unchanged id path.
 */
export async function renameEntry(path: string, newName: string): Promise<string> {
  if (!isRemotePath(path)) {
    await tauriInvoke("rename_file", { path, newName });
    return path.replace(/[^\\/]+$/, newName);
  }
  const { protocol, connectionId, remotePath } = await remoteArgs(path);
  const parent = parseLocation(parentLocation(path)).path;
  const toPath = parent.endsWith("/") ? `${parent}${newName}` : `${parent}/${newName}`;
  const now = await tauriInvoke<string | null>("connector_rename", { protocol, fromPath: remotePath, toPath, connectionId });
  return remoteLocation(connectionId, typeof now === "string" && now ? now : toPath);
}

/**
 * Delete items. Local items go to the Trash unless `permanent`; connection
 * items are deleted on the server (cloud drives keep them in their own
 * trash). A remote folder the server will not delete while it has
 * contents is emptied first.
 */
export async function removeEntries(items: ReadonlyArray<{ path: string; is_dir: boolean }>, permanent: boolean): Promise<void> {
  const local = items.filter((i) => !isRemotePath(i.path)).map((i) => i.path);
  if (local.length > 0) await tauriInvoke("delete_files", { paths: local, permanent });
  for (const item of items.filter((i) => isRemotePath(i.path))) await removeRemote(item.path, item.is_dir);
}

async function removeRemote(path: string, isDir: boolean): Promise<void> {
  const { protocol, connectionId, remotePath } = await remoteArgs(path);
  const remove = () => tauriInvoke("connector_remove", { protocol, remotePath, isDir, connectionId });
  if (!isDir) {
    await remove();
    return;
  }
  try {
    await remove();
  } catch (first) {
    const children = await listDirectory(path);
    if (children.length === 0) throw first;
    for (const c of children) await removeRemote(c.path, c.is_dir);
    await remove();
  }
}

/**
 * A local file with the item's contents: the file itself locally; for a
 * connection item, a copy downloaded into the preview cache (size-capped by
 * the backend).
 */
export async function localCopyOf(entry: Pick<FileEntryData, "path" | "name" | "size">): Promise<string> {
  if (!isRemotePath(entry.path)) return entry.path;
  const { protocol, connectionId, remotePath } = await remoteArgs(entry.path);
  return tauriInvoke<string>("connector_fetch_preview", { protocol, remotePath, name: entry.name, size: entry.size, maxBytes: null, connectionId });
}

/** Mirrors `LocationTransferOutcome` (transfer_commands.rs). */
export interface LocationTransferOutcome {
  jobs: { id: string; source_path: string; dest_path: string; total_bytes: number; status: string }[];
  moved: { from: string; to: string }[];
  folders_created: number;
}

export type TransferMode = "copy" | "move";

/**
 * Copy or move `items` into `destDir` when either end is a connection:
 * files become transfer-queue jobs (upload, download, server-side copy or
 * relay through this computer); a move inside one connection is a
 * server-side rename. Between two local folders keep using copy_files /
 * move_files (this refuses them).
 */
export async function transferItems(
  items: ReadonlyArray<Pick<FileEntryData, "path" | "name" | "is_dir" | "size" | "extension">>,
  destDir: string,
  mode: TransferMode,
): Promise<LocationTransferOutcome> {
  if (isRemotePath(destDir)) await connectionFor(destDir);
  return tauriInvoke<LocationTransferOutcome>("enqueue_location_transfer", {
    items: items.map((i) => ({ path: i.path, name: i.name, is_dir: i.is_dir, size: i.size ?? 0, extension: i.extension ?? null })),
    destDir,
    mode,
  });
}

/**
 * Listing entries for `paths` (to transfer or delete them): from `known`
 * (the pane's own listing) when there, else from their folders' listings.
 * A path no listing has is treated as a file of unknown size.
 */
export async function resolveItems(paths: ReadonlyArray<string>, known: ReadonlyArray<FileEntryData> = []): Promise<FileEntryData[]> {
  const byPath = new Map(known.map((e) => [e.path, e]));
  const parents = [...new Set(paths.filter((p) => !byPath.has(p)).map(parentLocation))];
  for (const dir of parents) {
    try {
      for (const e of await listDirectory(dir)) byPath.set(e.path, e);
    } catch {
      // Unlisted: falls back to a file entry below.
    }
  }
  return paths.map(
    (p) =>
      byPath.get(p) ?? {
        name: isRemotePath(p) ? remoteName(p) : p.split(/[\\/]/).pop() || p,
        path: p,
        is_dir: false,
        is_symlink: false,
        size: 0,
        modified: null,
        created: null,
        is_hidden: false,
        extension: null,
        permissions: null,
      },
  );
}

/** Whether a copy/move between these locations needs the connection route. */
export function needsLocationTransfer(sources: ReadonlyArray<string>, destDir: string): boolean {
  return isRemotePath(destDir) || sources.some(isRemotePath);
}

/**
 * Drag-and-drop semantics: same location (this computer, or the same
 * connection) ⇒ move (Alt ⇒ copy, as before); across connections or
 * between this computer and a connection ⇒ copy.
 */
export function dragDropMode(sources: ReadonlyArray<string>, destDir: string, altKey: boolean): TransferMode {
  if (sources.some((s) => !sameLocationRoot(s, destDir))) return "copy";
  return altKey ? "copy" : "move";
}

/** Mirrors `FolderSize`. */
export interface RemoteFolderSize {
  bytes: number;
  files: number;
  folders: number | null;
  complete: boolean;
}

/** Total size of a connection folder (server-side where available). */
export async function remoteFolderSize(path: string, jobId: string): Promise<RemoteFolderSize> {
  const { protocol, connectionId, remotePath } = await remoteArgs(path);
  return tauriInvoke<RemoteFolderSize>("connector_folder_size", { protocol, path: remotePath, jobId, connectionId });
}

/** Mirrors `RemoteQuota`. */
export interface RemoteQuotaInfo {
  total: number | null;
  used: number | null;
  free: number | null;
  trashed: number | null;
}

/** Storage of the account behind a connection; null when it has none. */
export async function connectionQuota(path: string): Promise<RemoteQuotaInfo | null> {
  const { protocol, connectionId } = await remoteArgs(path);
  return tauriInvoke<RemoteQuotaInfo | null>("connector_quota", { protocol, connectionId }, null);
}

/** The connector path of a remote pane path (share links take it). */
export function connectorPath(path: string): string {
  return parseLocation(path).path;
}

export { joinLocation, parentLocation };
