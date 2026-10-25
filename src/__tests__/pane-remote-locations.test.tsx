/**
 * Cloud locations in the main panes, through the real FileManager:
 * a sidebar connection opens in the active pane (listed through the
 * connector), breadcrumbs name the connection, a failing connection shows
 * Reconnect, local-only actions are disabled with a reason, F5 and drops
 * between this computer and a connection go through the transfer queue,
 * and the drag-and-drop confirmation (Settings › General) asks first.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...a: unknown[]) => mockInvoke(...a),
  convertFileSrc: (p: string) => p,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: async () => () => {},
  emit: async () => {},
}));
// jsdom has no layout, so the list's virtualizer would render no rows:
// render every row.
vi.mock("@tanstack/react-virtual", () => ({
  useVirtualizer: (o: { count: number; estimateSize: (i: number) => number }) => ({
    getVirtualItems: () => Array.from({ length: o.count }, (_, index) => ({ index, key: index, start: index * 28, size: 28, end: (index + 1) * 28, lane: 0 })),
    getTotalSize: () => o.count * 28,
    scrollToIndex: () => {},
    measureElement: () => {},
  }),
}));

import { FileManager } from "@/components/file-manager";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { useUIStore } from "@/stores/ui-store";
import { useConnectionsStore } from "@/stores/connections-store";
import { clearRemoteNames } from "@/lib/location";
import { LOCAL_ONLY_REASON } from "@/lib/location-provider";

const ID = "6f1c2b0a-1111-4222-8333-944455556666";
const ROOT = `conn://${ID}/`;
const entry = (path: string, name: string, is_dir = false, size = 3) => ({
  name, path, is_dir, is_symlink: false, size, modified: null, created: null, is_hidden: false, extension: null, permissions: null,
});

let remoteFails = false;

function setPanes(left: string, right: string, single: boolean) {
  const pane = (i: number, path: string) => ({
    id: `pane-${i}`,
    tabs: [{ id: `tab-${i}`, path, label: "x", pinned: false, pathHistory: [path], historyIndex: 0 }],
    activeTabId: `tab-${i}`,
    viewMode: "detail" as const,
    sortBy: "name",
    sortAsc: true,
    filterText: "",
    groupBy: null,
  });
  useFileManagerStore.setState({ panes: [pane(0, left), pane(1, right)], activePaneIndex: 0, singlePaneMode: single, selectedPaths: {}, commandPaletteOpen: false, syncBrowsing: false });
}

beforeEach(() => {
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  remoteFails = false;
  clearRemoteNames();
  useConnectionsStore.setState({ connections: [], statuses: {}, loaded: false });
  useUIStore.setState({ sidebarOpen: true, confirmDragDrop: false, defaultDownloadDir: null });
  mockInvoke.mockReset().mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
    switch (cmd) {
      case "list_connections":
        return [{ id: ID, name: "Work Drive", protocol: "dropbox", host: "", port: null, username: null, remote_path: "/", created_at: "", last_used: null, group_id: null }];
      case "list_connection_status":
        return remoteFails ? [{ id: ID, starred: false, credentials_rejected_at: "2026-10-01T00:00:00Z", credentials_rejected_reason: "invalid_grant" }] : [];
      case "connector_list_remote":
        if (remoteFails) throw new Error("Dropbox rejected the sign-in");
        return args?.path === "/" ? [entry("/Reports", "Reports", true, 0), entry("/a.txt", "a.txt")] : [];
      case "list_directory":
        return args?.path === "/home" ? [entry("/home/local.txt", "local.txt", false, 5)] : [];
      case "connector_stat":
        if (args?.path === "/Docs/Q1") return entry("/d1/q1", "Q1", true, 0); // an id-path provider resolves names
        return args?.path === "/Reports" ? entry("/Reports", "Reports", true, 0) : null;
      case "enqueue_location_transfer":
        return { jobs: [], moved: [], folders_created: 0 };
      case "connector_quota":
        return { total: 1000, used: 400, free: 600, trashed: null };
      case "safety_assess_intent":
        return { level: "low", reasons: [], baseline_sample_size: 0, baseline_median_files: 0, baseline_median_bytes: 0, intent_hash: "h", requires_confirmation: false };
      case "ledger_since_last_seen":
      case "ledger_get_pulse":
        return null;
      default:
        return [];
    }
  });
});

/** The pane remounts its list once while it settles: find the row text after that. */
async function settled(text: string, root: HTMLElement | Document = document) {
  const q = root === document ? screen : within(root as HTMLElement);
  await q.findByText(text);
  await act(() => new Promise((r) => setTimeout(r, 200)));
  return q.getByText(text);
}

const calls = (cmd: string) => mockInvoke.mock.calls.filter((c) => c[0] === cmd).map((c) => c[1]);

describe("connections in the panes", () => {
  it("opens a sidebar connection in the active pane, listed through the connector", async () => {
    setPanes("/home", "/home", true);
    render(<FileManager />);
    fireEvent.click(await screen.findByTestId(`sidebar-connection-${ID}`));
    await waitFor(() => expect(useFileManagerStore.getState().getActivePath(0)).toBe(ROOT));
    await waitFor(() => expect(screen.getByText("Reports")).toBeInTheDocument());
    expect(calls("connector_list_remote")).toContainEqual({ protocol: "dropbox", path: "/", connectionId: ID });
    // Listed once, not in a loop.
    await new Promise((r) => setTimeout(r, 300));
    expect(calls("connector_list_remote").length).toBeLessThanOrEqual(2);
    // The breadcrumb starts at the connection; the tab is named after it.
    expect(within(screen.getByTestId("pane-0")).getByTestId("breadcrumb-connection")).toHaveTextContent("Work Drive");
    expect(useFileManagerStore.getState().getActiveTab(0)?.label).toBe("Work Drive");
    // The status bar shows the account's storage.
    expect(await screen.findByTestId("status-bar-quota")).toHaveTextContent("600 B free of 1.0 KB");
  });

  it("shows why a connection folder cannot be listed, with Reconnect", async () => {
    remoteFails = true;
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    const state = await screen.findByTestId("remote-error-state");
    expect(state).toHaveTextContent("credentials rejected");
    expect(screen.queryByText("Reports")).not.toBeInTheDocument();
    remoteFails = false;
    fireEvent.click(within(state).getByTestId("remote-reconnect"));
    await waitFor(() => expect(screen.getByText("Reports")).toBeInTheDocument());
    expect(calls("clear_connection_credentials_rejected")).toContainEqual({ connectionId: ID });
    expect(screen.queryByTestId("remote-error-state")).not.toBeInTheDocument();
  });

  it("offers share links and disables this-computer-only actions with a reason", async () => {
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    fireEvent.contextMenu(await settled("a.txt"));
    const menu = await screen.findByTestId("context-menu");
    expect(within(menu).getByTestId("context-menu-item-share-link")).toBeEnabled();
    const dup = within(menu).getByTestId("context-menu-item-duplicate");
    expect(dup).toBeDisabled();
    expect(dup).toHaveAttribute("title", LOCAL_ONLY_REASON);
    expect(within(menu).getByTestId("context-menu-item-delete")).toHaveTextContent("Delete");
    expect(within(menu).getByTestId("context-menu-item-copy")).toBeEnabled();
  });

  it("F5 from this computer to a connection pane queues an upload", async () => {
    setPanes("/home", ROOT, false);
    render(<FileManager />);
    const left = screen.getByTestId("pane-0");
    fireEvent.click(await settled("local.txt", left));
    await act(async () => {
      fireEvent.keyDown(window, { key: "F5" });
    });
    await waitFor(() => expect(calls("enqueue_location_transfer")).toHaveLength(1));
    expect(calls("enqueue_location_transfer")[0]).toEqual({
      items: [{ path: "/home/local.txt", name: "local.txt", is_dir: false, size: 5, extension: null }],
      destDir: ROOT,
      mode: "copy",
    });
  });

  it("asks before a drop when drag-and-drop confirmation is on; local→connection is a copy", async () => {
    useUIStore.setState({ confirmDragDrop: true });
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    await screen.findByText("Reports");
    const target = screen.getByTestId("pane-0").firstElementChild as HTMLElement;
    const data = new Map<string, string>([["application/x-ufop-files", JSON.stringify(["/home/local.txt"])]]);
    fireEvent.drop(target, { dataTransfer: { getData: (t: string) => data.get(t) ?? "", types: [...data.keys()] } });
    const confirm = await screen.findByTestId("drag-drop-confirm");
    expect(within(confirm).getByTestId("drag-drop-summary")).toHaveTextContent("Copy 1 item to Work Drive:/?");
    expect(calls("enqueue_location_transfer")).toHaveLength(0);
    // Confirmable once measured and assessed.
    await waitFor(() => expect(within(confirm).getByTestId("drag-drop-ok")).toBeEnabled());
    fireEvent.click(within(confirm).getByTestId("drag-drop-ok"));
    await waitFor(() => expect(calls("enqueue_location_transfer")).toHaveLength(1));
    expect(calls("enqueue_location_transfer")[0]).toMatchObject({ destDir: ROOT, mode: "copy" });
    // Cancel runs nothing.
    fireEvent.drop(target, { dataTransfer: { getData: (t: string) => data.get(t) ?? "", types: [...data.keys()] } });
    fireEvent.click(within(await screen.findByTestId("drag-drop-confirm")).getByTestId("drag-drop-cancel"));
    expect(screen.queryByTestId("drag-drop-confirm")).not.toBeInTheDocument();
    expect(calls("enqueue_location_transfer")).toHaveLength(1);
  });

  it("without the confirmation a same-location drop moves at once, as before", async () => {
    setPanes("/home", "/home", true);
    render(<FileManager />);
    await screen.findByText("local.txt");
    const target = screen.getByTestId("pane-0").firstElementChild as HTMLElement;
    const data = new Map<string, string>([["application/x-ufop-files", JSON.stringify(["/other/x.txt"])]]);
    fireEvent.drop(target, { dataTransfer: { getData: (t: string) => data.get(t) ?? "", types: [...data.keys()] } });
    await waitFor(() => expect(calls("move_files")).toContainEqual({ sourcePaths: ["/other/x.txt"], destDir: "/home" }));
    expect(screen.queryByTestId("drag-drop-confirm")).not.toBeInTheDocument();
  });

  it("F5 from a connection with no other pane downloads to the default download folder", async () => {
    useUIStore.setState({ defaultDownloadDir: "/home/me/Downloads" });
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    fireEvent.click(await settled("a.txt"));
    await act(async () => {
      fireEvent.keyDown(window, { key: "F5" });
    });
    await waitFor(() => expect(calls("enqueue_location_transfer")).toHaveLength(1));
    expect(calls("enqueue_location_transfer")[0]).toMatchObject({
      items: [{ path: `conn://${ID}/a.txt`, name: "a.txt", is_dir: false, size: 3, extension: null }],
      destDir: "/home/me/Downloads",
      mode: "copy",
    });
  });

  it("renames and deletes connection items on the server", async () => {
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    fireEvent.click(await settled("a.txt"));
    fireEvent.keyDown(window, { key: "F2" });
    const input = await screen.findByLabelText("Enter new name:");
    fireEvent.change(input, { target: { value: "b.txt" } });
    fireEvent.click(screen.getByText("OK"));
    await waitFor(() => expect(calls("connector_rename")).toEqual([{ protocol: "dropbox", fromPath: "/a.txt", toPath: "/b.txt", connectionId: ID }]));
    // Delete: no local Trash — confirmed by name, then removed on the server.
    fireEvent.keyDown(window, { key: "Delete" });
    const dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("Delete 1 item from Work Drive:/?");
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(calls("connector_remove")).toEqual([{ protocol: "dropbox", remotePath: "/a.txt", isDir: false, connectionId: ID }]));
    expect(calls("delete_files")).toHaveLength(0);
  });

  it("assesses deleting a connection folder with what is inside it, not as one empty item", async () => {
    // Regression: the Safety Interlock saw one item of 0 bytes for a
    // folder of thousands of files.
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) =>
      cmd === "connector_folder_size" ? { files: 1200, folders: 30, bytes: 9_000_000_000, complete: true } : base(cmd, args),
    );
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    fireEvent.click(await settled("Reports"));
    fireEvent.keyDown(window, { key: "Delete" });
    const dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(calls("safety_assess_intent")).toHaveLength(1));
    expect(calls("connector_folder_size")).toContainEqual(expect.objectContaining({ protocol: "dropbox", path: "/Reports", connectionId: ID }));
    expect(calls("safety_assess_intent")[0]).toMatchObject({ intent: { affected_files: 1200, total_bytes: 9_000_000_000 } });
    await waitFor(() => expect(calls("connector_remove")).toEqual([{ protocol: "dropbox", remotePath: "/Reports", isDir: true, connectionId: ID }]));
  });

  it("Backspace, Alt+↑ and the Up button go to the parent folder, locally and on a connection, and stop at the top", async () => {
    setPanes(`conn://${ID}/Reports`, "/home", true);
    render(<FileManager />);
    await waitFor(() => expect(calls("connector_list_remote")).toContainEqual(expect.objectContaining({ path: "/Reports" })));
    fireEvent.keyDown(window, { key: "Backspace" });
    await waitFor(() => expect(useFileManagerStore.getState().getActivePath(0)).toBe(ROOT));
    expect(within(screen.getByTestId("pane-0")).getByTestId("breadcrumb-up")).toBeDisabled();
    fireEvent.keyDown(window, { key: "ArrowUp", altKey: true });
    expect(useFileManagerStore.getState().getActivePath(0)).toBe(ROOT);
    act(() => useFileManagerStore.getState().navigateTab(0, "tab-0", "/home/me/docs", "docs"));
    fireEvent.keyDown(window, { key: "ArrowUp", altKey: true });
    await waitFor(() => expect(useFileManagerStore.getState().getActivePath(0)).toBe("/home/me"));
    fireEvent.click(within(screen.getByTestId("pane-0")).getByTestId("breadcrumb-up"));
    await waitFor(() => expect(useFileManagerStore.getState().getActivePath(0)).toBe("/home"));
  });

  it("Backspace in a text field types instead of going up", async () => {
    setPanes("/home", "/home", true);
    render(<FileManager />);
    await settled("local.txt");
    fireEvent.keyDown(window, { key: "l", ctrlKey: true });
    const input = (await screen.findByTestId("address-bar-input")) as HTMLInputElement;
    input.focus();
    fireEvent.keyDown(input, { key: "Backspace" });
    expect(useFileManagerStore.getState().getActivePath(0)).toBe("/home");
  });

  it("Ctrl+L turns the breadcrumb into the address field; <connection>:path goes there", async () => {
    setPanes("/home", "/home", true);
    render(<FileManager />);
    await settled("local.txt");
    fireEvent.keyDown(window, { key: "l", ctrlKey: true });
    const input = (await screen.findByTestId("address-bar-input")) as HTMLInputElement;
    expect(input.value).toBe("/home");
    await waitFor(() => expect(useConnectionsStore.getState().loaded).toBe(true));
    fireEvent.change(input, { target: { value: "Work Drive:/Reports" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(useFileManagerStore.getState().getActivePath(0)).toBe(`conn://${ID}/Reports`));
    expect(screen.queryByTestId("address-bar-input")).not.toBeInTheDocument();
  });

  it("a typed name path opens the folder at the path the connection resolved (Google Drive ids)", async () => {
    setPanes("/home", "/home", true);
    render(<FileManager />);
    await settled("local.txt");
    fireEvent.keyDown(window, { key: "l", ctrlKey: true });
    const input = (await screen.findByTestId("address-bar-input")) as HTMLInputElement;
    await waitFor(() => expect(useConnectionsStore.getState().loaded).toBe(true));
    fireEvent.change(input, { target: { value: "Work Drive:/Docs/Q1" } });
    fireEvent.keyDown(input, { key: "Enter" });
    await waitFor(() => expect(useFileManagerStore.getState().getActivePath(0)).toBe(`conn://${ID}/d1/q1`));
    expect(calls("connector_stat")).toContainEqual({ protocol: "dropbox", path: "/Docs/Q1", connectionId: ID });
  });

  it("never shows a slow listing of a folder already left under the newer folder", async () => {
    // Regression: the listing of a remote folder the user had left could
    // land after the newer folder's and replace its files.
    let releaseSlow: () => void = () => undefined;
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "connector_list_remote" && args?.path === "/Slow") {
        await new Promise<void>((r) => (releaseSlow = r));
        return [entry("/Slow/old.txt", "old.txt")];
      }
      if (cmd === "connector_list_remote" && args?.path === "/Fast") return [entry("/Fast/new.txt", "new.txt")];
      return base(cmd, args);
    });
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    await settled("Reports");
    const fm = useFileManagerStore.getState();
    act(() => fm.navigateTab(0, "tab-0", `conn://${ID}/Slow`, "Slow"));
    await waitFor(() => expect(calls("connector_list_remote")).toContainEqual(expect.objectContaining({ path: "/Slow" })));
    act(() => useFileManagerStore.getState().navigateTab(0, "tab-0", `conn://${ID}/Fast`, "Fast"));
    await screen.findByText("new.txt");
    await act(async () => {
      releaseSlow();
      await new Promise((r) => setTimeout(r, 100));
    });
    expect(screen.getByText("new.txt")).toBeInTheDocument();
    expect(screen.queryByText("old.txt")).not.toBeInTheDocument();
  });

  it("assesses a confirmed drop with what it really affects", async () => {
    useUIStore.setState({ confirmDragDrop: true });
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) =>
      cmd === "count_affected_files" ? { files: 2500, bytes: 7_000_000_000, capped: false } : base(cmd, args),
    );
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    await screen.findByText("Reports");
    const target = screen.getByTestId("pane-0").firstElementChild as HTMLElement;
    const data = new Map<string, string>([["application/x-ufop-files", JSON.stringify(["/home/big"])]]);
    fireEvent.drop(target, { dataTransfer: { getData: (t: string) => data.get(t) ?? "", types: [...data.keys()] } });
    const confirm = await screen.findByTestId("drag-drop-confirm");
    await waitFor(() => expect(calls("safety_assess_intent")).toHaveLength(1));
    expect(calls("count_affected_files")).toContainEqual({ paths: ["/home/big"] });
    expect(calls("safety_assess_intent")[0]).toMatchObject({ intent: { affected_files: 2500, total_bytes: 7_000_000_000 } });
    expect(within(confirm).getByTestId("drag-drop-summary")).toHaveTextContent("(7.0 GB)");
  });

  it("says when a move kept Google documents (exported, not moved)", async () => {
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) =>
      cmd === "enqueue_location_transfer"
        ? { jobs: [], moved: [], folders_created: 0, kept: [`conn://${ID}/a.txt`] }
        : cmd === "count_affected_files"
          ? { files: 1, bytes: 3, capped: false }
          : base(cmd, args),
    );
    useUIStore.setState({ structuredErrors: [] });
    setPanes(ROOT, "/home", false);
    render(<FileManager />);
    fireEvent.click(await settled("a.txt", screen.getByTestId("pane-0")));
    await act(async () => {
      fireEvent.keyDown(window, { key: "F6" });
    });
    await waitFor(() => expect(calls("enqueue_location_transfer")).toContainEqual(expect.objectContaining({ mode: "move" })));
    await waitFor(() => expect(useUIStore.getState().structuredErrors.map((e) => e.what)).toContain("1 Google document was copied, not moved"));
  });

  it("re-lists both panes when a move fails part-way (items may have moved on the server)", async () => {
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) => {
      if (cmd === "enqueue_location_transfer") throw new Error("Listing failed. 1 item(s) had already been moved on the server: a.txt.");
      if (cmd === "count_affected_files") return { files: 1, bytes: 3, capped: false };
      return base(cmd, args);
    });
    setPanes(ROOT, "/home", false);
    render(<FileManager />);
    fireEvent.click(await settled("a.txt", screen.getByTestId("pane-0")));
    const before = { remote: calls("connector_list_remote").length, local: calls("list_directory").filter((a) => a?.path === "/home").length };
    await act(async () => {
      fireEvent.keyDown(window, { key: "F6" });
    });
    await waitFor(() => expect(calls("enqueue_location_transfer")).toContainEqual(expect.objectContaining({ mode: "move" })));
    await waitFor(() => {
      expect(calls("connector_list_remote").length).toBeGreaterThan(before.remote);
      expect(calls("list_directory").filter((a) => a?.path === "/home").length).toBeGreaterThan(before.local);
    });
  });

  it("a drop or cut+paste back into the folder that lists the items moves nothing (Google shared-drive roots)", async () => {
    // Regression: a Drive whose root is a shared drive lists `/` items as
    // `/shared-drives/<id>/<item>`; dropping one back on its own pane (or
    // Cut then Paste there) sent a move, and the item became "a (copy).txt".
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) =>
      cmd === "connector_list_remote" && args?.path === "/" ? [entry("/shared-drives/D/x9", "a.txt")] : base(cmd, args),
    );
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    const row = await settled("a.txt");
    const target = screen.getByTestId("pane-0").firstElementChild as HTMLElement;
    const data = new Map<string, string>([["application/x-ufop-files", JSON.stringify([`conn://${ID}/shared-drives/D/x9`])]]);
    fireEvent.drop(target, { dataTransfer: { getData: (t: string) => data.get(t) ?? "", types: [...data.keys()] } });
    fireEvent.click(row);
    await act(async () => {
      fireEvent.keyDown(window, { key: "x", ctrlKey: true });
    });
    await act(async () => {
      fireEvent.keyDown(window, { key: "v", ctrlKey: true });
    });
    await act(() => new Promise((r) => setTimeout(r, 100)));
    expect(calls("enqueue_location_transfer")).toHaveLength(0);
  });

  it("asks before opening a program or script from a connection", async () => {
    // Regression: a double-click downloaded `x.bat` from a (possibly
    // shared) connection and ran it with no warning.
    const base = mockInvoke.getMockImplementation()!;
    mockInvoke.mockImplementation(async (cmd: string, args?: Record<string, unknown>) =>
      cmd === "connector_list_remote" && args?.path === "/"
        ? [entry("/x.bat", "x.bat"), entry("/scene.blend", "scene.blend")]
        : cmd === "connector_fetch_preview"
          ? `/cache/${args?.name}`
          : base(cmd, args),
    );
    setPanes(ROOT, "/home", true);
    render(<FileManager />);
    fireEvent.doubleClick(await settled("x.bat"));
    let dialog = await screen.findByRole("dialog");
    expect(dialog).toHaveTextContent("x.bat came from Work Drive");
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await act(() => new Promise((r) => setTimeout(r, 50)));
    expect(calls("connector_fetch_preview")).toHaveLength(0);
    expect(calls("open_file_with_default")).toHaveLength(0);
    // Confirmed: downloaded and opened.
    fireEvent.doubleClick(screen.getByText("x.bat"));
    dialog = await screen.findByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Open" }));
    await waitFor(() => expect(calls("open_file_with_default")).toEqual([{ path: "/cache/x.bat" }]));
    // An ordinary document opens without asking.
    fireEvent.doubleClick(screen.getByText("scene.blend"));
    await waitFor(() => expect(calls("open_file_with_default")).toContainEqual({ path: "/cache/scene.blend" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
});

