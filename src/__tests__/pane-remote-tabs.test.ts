/**
 * Remote locations in the pane store: tabs, history and persistence keep
 * `conn://` paths exactly like local ones, and labels come from the listed
 * names / the connection's name.
 */
import { describe, it, expect, beforeEach } from "vitest";
import { paneLabel, useFileManagerStore } from "@/stores/file-manager-store";
import { useConnectionsStore } from "@/stores/connections-store";
import { clearRemoteNames, rememberRemoteNames } from "@/lib/location";

const ID = "6f1c2b0a-1111-4222-8333-944455556666";
const ROOT = `conn://${ID}/`;

function resetPanes() {
  const pane = (i: number) => ({
    id: `pane-${i}`,
    tabs: [{ id: `tab-${i}`, path: "/home", label: "home", pinned: false, pathHistory: ["/home"], historyIndex: 0 }],
    activeTabId: `tab-${i}`,
    viewMode: "detail" as const,
    sortBy: "name",
    sortAsc: true,
    filterText: "",
    groupBy: null,
  });
  useFileManagerStore.setState({ panes: [pane(0), pane(1)], activePaneIndex: 0, recentlyClosedTabs: { 0: [], 1: [] }, syncBrowsing: false });
}

describe("remote locations in pane tabs", () => {
  beforeEach(() => {
    resetPanes();
    clearRemoteNames();
    useConnectionsStore.setState({ connections: [{ id: ID, name: "Work Drive", protocol: "google_drive", remote_path: "/" }], statuses: {}, loaded: true });
  });

  it("labels remote paths by listed name, the root by connection name; local labels unchanged", () => {
    rememberRemoteNames([{ path: `conn://${ID}/f1`, name: "Reports" }]);
    expect(paneLabel(ROOT)).toBe("Work Drive");
    expect(paneLabel(`conn://${ID}/f1`)).toBe("Reports");
    expect(paneLabel("/")).toBe("Root");
    expect(paneLabel("/home/me")).toBe("me");
  });

  it("navigates, goes back and forward across local and remote folders", () => {
    rememberRemoteNames([{ path: `conn://${ID}/f1`, name: "Reports" }]);
    const s = useFileManagerStore.getState();
    s.navigateTab(0, "tab-0", ROOT, paneLabel(ROOT));
    s.navigateTab(0, "tab-0", `conn://${ID}/f1`, paneLabel(`conn://${ID}/f1`));
    expect(useFileManagerStore.getState().getActivePath(0)).toBe(`conn://${ID}/f1`);
    useFileManagerStore.getState().navigateBack(0);
    let tab = useFileManagerStore.getState().getActiveTab(0)!;
    expect([tab.path, tab.label]).toEqual([ROOT, "Work Drive"]);
    useFileManagerStore.getState().navigateBack(0);
    tab = useFileManagerStore.getState().getActiveTab(0)!;
    expect([tab.path, tab.label]).toEqual(["/home", "home"]);
    useFileManagerStore.getState().navigateForward(0);
    useFileManagerStore.getState().navigateForward(0);
    tab = useFileManagerStore.getState().getActiveTab(0)!;
    expect([tab.path, tab.label]).toEqual([`conn://${ID}/f1`, "Reports"]);
  });

  it("persists remote tabs and history, and a reload restores them", () => {
    const s = useFileManagerStore.getState();
    s.addTab(1, ROOT, "Work Drive");
    s.navigateTab(0, "tab-0", `conn://${ID}/f1`, "Reports");
    const persisted = useFileManagerStore.persist.getOptions().partialize!(useFileManagerStore.getState()) as {
      panes: { tabs: { path: string; pathHistory: string[] }[] }[];
    };
    const blob = JSON.parse(JSON.stringify(persisted));
    expect(blob.panes[1].tabs.map((t: { path: string }) => t.path)).toContain(ROOT);
    expect(blob.panes[0].tabs[0].pathHistory).toEqual(["/home", `conn://${ID}/f1`]);
    // "Reload": wipe, then merge the stored blob back in.
    resetPanes();
    useFileManagerStore.setState(blob);
    expect(useFileManagerStore.getState().getActivePath(0)).toBe(`conn://${ID}/f1`);
    expect(useFileManagerStore.getState().panes[1].tabs.some((t) => t.path === ROOT)).toBe(true);
  });

  it("moves and duplicates remote tabs like local ones; closed remote tabs reopen", () => {
    const s = useFileManagerStore.getState();
    s.addTab(0, ROOT, "Work Drive");
    const tabId = useFileManagerStore.getState().panes[0].activeTabId;
    expect(useFileManagerStore.getState().duplicateTab(0, tabId)?.path).toBe(ROOT);
    useFileManagerStore.getState().closeTab(0, useFileManagerStore.getState().panes[0].activeTabId);
    expect(useFileManagerStore.getState().recentlyClosedTabs[0][0].path).toBe(ROOT);
    const moved = useFileManagerStore.getState().moveTabToOtherPane(0, tabId);
    expect(moved?.path).toBe(ROOT);
    expect(useFileManagerStore.getState().panes[1].tabs.some((t) => t.path === ROOT)).toBe(true);
  });
});
