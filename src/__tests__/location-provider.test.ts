import { describe, it, expect, vi, beforeEach } from "vitest";

vi.mock("@/hooks/use-tauri", async () => {
  const actual = await vi.importActual<typeof import("@/hooks/use-tauri")>("@/hooks/use-tauri");
  return { ...actual, isTauriAvailable: () => true, tauriInvoke: vi.fn(), tauriInvokeSafe: vi.fn() };
});

import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { useConnectionsStore } from "@/stores/connections-store";
import { clearRemoteNames, remoteName } from "@/lib/location";
import {
  connectionQuota,
  dragDropMode,
  listDirectory,
  localCopyOf,
  makeDirectory,
  measureLocations,
  needsLocationTransfer,
  removeEntries,
  renameEntry,
  statLocation,
  transferItems,
  UnknownConnectionError,
} from "@/lib/location-provider";

const ID = "6f1c2b0a-1111-4222-8333-944455556666";
const invoke = tauriInvoke as unknown as ReturnType<typeof vi.fn>;
const entry = (path: string, name: string, is_dir = false) => ({
  name, path, is_dir, is_symlink: false, size: 3, modified: null, created: null, is_hidden: false, extension: null, permissions: null,
});

describe("location provider routing", () => {
  beforeEach(() => {
    invoke.mockReset();
    clearRemoteNames();
    useConnectionsStore.setState({
      connections: [{ id: ID, name: "Work Drive", protocol: "google_drive", remote_path: "/" }],
      statuses: {},
      loaded: true,
    });
  });

  it("lists local folders with list_directory, unchanged", async () => {
    invoke.mockResolvedValue([entry("/home/a.txt", "a.txt")]);
    const out = await listDirectory("/home");
    expect(invoke).toHaveBeenCalledWith("list_directory", { path: "/home" });
    expect(out[0].path).toBe("/home/a.txt");
  });

  it("lists connection folders with connector_list_remote and returns pane paths", async () => {
    invoke.mockResolvedValue([entry("/f1", "Reports", true)]);
    const out = await listDirectory(`conn://${ID}/`);
    expect(invoke).toHaveBeenCalledWith("connector_list_remote", { protocol: "google_drive", path: "/", connectionId: ID });
    expect(out[0].path).toBe(`conn://${ID}/f1`);
    expect(remoteName(`conn://${ID}/f1`)).toBe("Reports");
  });

  it("refuses a connection that is not saved", async () => {
    (tauriInvokeSafe as unknown as ReturnType<typeof vi.fn>).mockResolvedValue([]);
    await expect(listDirectory("conn://00000000-0000-4000-8000-000000000000/")).rejects.toBeInstanceOf(UnknownConnectionError);
  });

  it("routes stat, mkdir and rename", async () => {
    invoke.mockResolvedValue(null);
    await makeDirectory("/home/new");
    expect(invoke).toHaveBeenLastCalledWith("create_directory", { path: "/home/new" });
    await makeDirectory(`conn://${ID}/f1/new`);
    expect(invoke).toHaveBeenLastCalledWith("connector_make_directory", { protocol: "google_drive", remotePath: "/f1/new", connectionId: ID });

    expect(await renameEntry("/home/a.txt", "b.txt")).toBe("/home/b.txt");
    expect(invoke).toHaveBeenLastCalledWith("rename_file", { path: "/home/a.txt", newName: "b.txt" });
    // Google Drive keeps the item's id: the backend says where it is now.
    invoke.mockResolvedValueOnce("/f1/x9");
    expect(await renameEntry(`conn://${ID}/f1/x9`, "b.txt")).toBe(`conn://${ID}/f1/x9`);
    expect(invoke).toHaveBeenLastCalledWith("connector_rename", { protocol: "google_drive", fromPath: "/f1/x9", toPath: "/f1/b.txt", connectionId: ID });
    // Path-based connections: the new name's path.
    invoke.mockResolvedValueOnce("/f1/b.txt");
    expect(await renameEntry(`conn://${ID}/f1/a.txt`, "b.txt")).toBe(`conn://${ID}/f1/b.txt`);

    invoke.mockResolvedValue(entry("/f1/x9", "a.txt"));
    expect((await statLocation(`conn://${ID}/f1/x9`))?.path).toBe(`conn://${ID}/f1/x9`);
    expect(invoke).toHaveBeenLastCalledWith("connector_stat", { protocol: "google_drive", path: "/f1/x9", connectionId: ID });
    invoke.mockRejectedValue(new Error("missing"));
    expect(await statLocation("/nope")).toBeNull();
  });

  it("deletes local items to the Trash and remote ones on the server, emptying a refusing folder first", async () => {
    invoke.mockImplementation(async (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "connector_remove" && args.remotePath === "/d" && args.isDir && !removedChild) throw new Error("not empty");
      if (cmd === "connector_remove" && args.remotePath === "/d/c.txt") removedChild = true;
      if (cmd === "connector_list_remote") return [entry("/d/c.txt", "c.txt")];
      return null;
    });
    let removedChild = false;
    await removeEntries([{ path: "/home/a", is_dir: false }, { path: `conn://${ID}/d`, is_dir: true }], false);
    expect(invoke).toHaveBeenCalledWith("delete_files", { paths: ["/home/a"], permanent: false });
    expect(invoke).toHaveBeenCalledWith("connector_remove", { protocol: "google_drive", remotePath: "/d/c.txt", isDir: false, connectionId: ID });
    expect(invoke.mock.calls.filter((c) => c[0] === "connector_remove" && c[1].remotePath === "/d")).toHaveLength(2);
  });

  it("gives local copies for previews: the file itself, or a fetched cache copy", async () => {
    expect(await localCopyOf({ path: "/home/a.png", name: "a.png", size: 1 })).toBe("/home/a.png");
    expect(invoke).not.toHaveBeenCalled();
    invoke.mockResolvedValue("/tmp/ufop-remote-preview/x/a.png");
    expect(await localCopyOf({ path: `conn://${ID}/f1/x9`, name: "a.png", size: 10 })).toBe("/tmp/ufop-remote-preview/x/a.png");
    expect(invoke).toHaveBeenCalledWith("connector_fetch_preview", { protocol: "google_drive", remotePath: "/f1/x9", name: "a.png", size: 10, maxBytes: null, connectionId: ID });
  });

  it("transfers through enqueue_location_transfer and picks drag-drop semantics", async () => {
    invoke.mockResolvedValue({ jobs: [], moved: [], folders_created: 0 });
    await transferItems([entry("/home/a.txt", "a.txt")], `conn://${ID}/f1`, "copy");
    expect(invoke).toHaveBeenCalledWith("enqueue_location_transfer", {
      items: [{ path: "/home/a.txt", name: "a.txt", is_dir: false, size: 3, extension: null }],
      destDir: `conn://${ID}/f1`,
      mode: "copy",
    });
    expect(needsLocationTransfer(["/a"], "/b")).toBe(false);
    expect(needsLocationTransfer([`conn://${ID}/a`], "/b")).toBe(true);
    expect(needsLocationTransfer(["/a"], `conn://${ID}/`)).toBe(true);
    // Same location ⇒ move (Alt ⇒ copy); across ⇒ copy.
    expect(dragDropMode(["/a"], "/b", false)).toBe("move");
    expect(dragDropMode(["/a"], "/b", true)).toBe("copy");
    expect(dragDropMode([`conn://${ID}/a`], `conn://${ID}/b`, false)).toBe("move");
    expect(dragDropMode(["/a"], `conn://${ID}/b`, false)).toBe("copy");
    expect(dragDropMode([`conn://${ID}/a`], "/b", false)).toBe("copy");
  });

  it("reads the connection's quota", async () => {
    invoke.mockResolvedValue({ total: 100, used: 40, free: 60, trashed: null });
    expect(await connectionQuota(`conn://${ID}/f1`)).toEqual({ total: 100, used: 40, free: 60, trashed: null });
    expect(invoke).toHaveBeenCalledWith("connector_quota", { protocol: "google_drive", connectionId: ID }, null);
  });

  it("measures what a transfer affects, folders counted through, on either side", async () => {
    const safe = tauriInvokeSafe as unknown as ReturnType<typeof vi.fn>;
    safe.mockReset().mockResolvedValue({ files: 120, bytes: 5000, capped: false });
    invoke.mockImplementation(async (cmd: string) => {
      if (cmd === "connector_list_remote") return [entry("/f1", "Reports", true), entry("/a.txt", "a.txt")];
      if (cmd === "connector_folder_size") return { bytes: 9000, files: 300, folders: 4, complete: true };
      return null;
    });
    const out = await measureLocations(["/home/big", `conn://${ID}/f1`, `conn://${ID}/a.txt`]);
    expect(safe).toHaveBeenCalledWith("count_affected_files", { paths: ["/home/big"] }, expect.anything());
    expect(invoke).toHaveBeenCalledWith("connector_folder_size", expect.objectContaining({ protocol: "google_drive", path: "/f1", connectionId: ID }));
    // 120 local + 300 in the remote folder + 1 remote file (3 bytes).
    expect(out).toEqual({ files: 421, bytes: 14003 });
  });
});
