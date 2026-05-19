import { describe, it, expect, beforeEach } from "vitest";
import { useFileManagerStore } from "@/stores/file-manager-store";

describe("FileManagerStore", () => {
  beforeEach(() => {
    // Reset store to initial state
    useFileManagerStore.setState({
      panes: [
        {
          id: "pane-0",
          tabs: [{ id: "tab-0", path: "/", label: "Root", pinned: false, pathHistory: ["/"], historyIndex: 0 }],
          activeTabId: "tab-0",
          viewMode: "detail",
          sortBy: "name",
          sortAsc: true,
          filterText: "",
          groupBy: null,
        },
        {
          id: "pane-1",
          tabs: [{ id: "tab-1", path: "/", label: "Root", pinned: false, pathHistory: ["/"], historyIndex: 0 }],
          activeTabId: "tab-1",
          viewMode: "detail",
          sortBy: "name",
          sortAsc: true,
          filterText: "",
          groupBy: null,
        },
      ],
      activePaneIndex: 0,
      singlePaneMode: true,
      paneSplitPercent: 50,
      favorites: [],
      recentLocations: [],
      expandedTreePaths: [],
      selectedPaths: {},
      savedSearches: [],
      undoStack: [],
      commandPaletteOpen: false,
      showHiddenFiles: false,
      paneOrientation: "horizontal",
      syncBrowsing: false,
      folderViewDefaults: {},
      workspaces: [],
      doubleClickBehavior: "open",
      editorMappings: [],
      privacySettings: { autoUpdateCheck: true, crashReportOptIn: false },
      recentlyClosedTabs: { 0: [], 1: [] },
      selectionStashes: [],
    });
  });

  // ── T-006: Dual-Pane Layout ──

  describe("T-006: Dual-Pane Layout", () => {
    it("should start in single-pane mode by default", () => {
      const state = useFileManagerStore.getState();
      expect(state.singlePaneMode).toBe(true);
    });

    it("should toggle between single and dual-pane mode", () => {
      const { toggleSinglePaneMode } = useFileManagerStore.getState();
      toggleSinglePaneMode();
      expect(useFileManagerStore.getState().singlePaneMode).toBe(false);
      toggleSinglePaneMode();
      expect(useFileManagerStore.getState().singlePaneMode).toBe(true);
    });

    it("should switch active pane index", () => {
      const { setActivePaneIndex } = useFileManagerStore.getState();
      expect(useFileManagerStore.getState().activePaneIndex).toBe(0);
      setActivePaneIndex(1);
      expect(useFileManagerStore.getState().activePaneIndex).toBe(1);
    });

    it("should constrain panel split to 20%-80%", () => {
      const { setPaneSplitPercent } = useFileManagerStore.getState();

      setPaneSplitPercent(10);
      expect(useFileManagerStore.getState().paneSplitPercent).toBe(20);

      setPaneSplitPercent(90);
      expect(useFileManagerStore.getState().paneSplitPercent).toBe(80);

      setPaneSplitPercent(50);
      expect(useFileManagerStore.getState().paneSplitPercent).toBe(50);
    });

    it("should have two panes with independent state", () => {
      const state = useFileManagerStore.getState();
      expect(state.panes).toHaveLength(2);
      expect(state.panes[0].id).not.toBe(state.panes[1].id);
    });
  });

  // ── T-008: Tabbed Browsing ──

  describe("T-008: Tabbed Browsing", () => {
    it("should add a new tab", () => {
      const { addTab } = useFileManagerStore.getState();
      addTab(0, "/home", "Home");
      const pane = useFileManagerStore.getState().panes[0];
      expect(pane.tabs).toHaveLength(2);
      expect(pane.tabs[1].path).toBe("/home");
      expect(pane.tabs[1].label).toBe("Home");
      // New tab should be active
      expect(pane.activeTabId).toBe(pane.tabs[1].id);
    });

    it("should close a tab", () => {
      const { addTab, closeTab } = useFileManagerStore.getState();
      addTab(0, "/home", "Home");
      const tabToClose = useFileManagerStore.getState().panes[0].tabs[1].id;
      closeTab(0, tabToClose);
      expect(useFileManagerStore.getState().panes[0].tabs).toHaveLength(1);
    });

    it("should not close the last tab", () => {
      const { closeTab } = useFileManagerStore.getState();
      const lastTabId = useFileManagerStore.getState().panes[0].tabs[0].id;
      closeTab(0, lastTabId);
      expect(useFileManagerStore.getState().panes[0].tabs).toHaveLength(1);
    });

    it("should not close a pinned tab", () => {
      const { addTab, pinTab, closeTab } = useFileManagerStore.getState();
      addTab(0, "/home", "Home");
      const tabId = useFileManagerStore.getState().panes[0].tabs[1].id;
      pinTab(0, tabId);
      closeTab(0, tabId);
      // Tab should still exist
      expect(
        useFileManagerStore.getState().panes[0].tabs.find((t) => t.id === tabId),
      ).toBeTruthy();
    });

    it("should set active tab", () => {
      const { addTab, setActiveTab } = useFileManagerStore.getState();
      addTab(0, "/home", "Home");
      const firstTabId = useFileManagerStore.getState().panes[0].tabs[0].id;
      setActiveTab(0, firstTabId);
      expect(useFileManagerStore.getState().panes[0].activeTabId).toBe(firstTabId);
    });

    it("should pin and unpin tabs", () => {
      const { pinTab, unpinTab } = useFileManagerStore.getState();
      const tabId = useFileManagerStore.getState().panes[0].tabs[0].id;

      pinTab(0, tabId);
      expect(
        useFileManagerStore.getState().panes[0].tabs[0].pinned,
      ).toBe(true);

      unpinTab(0, tabId);
      expect(
        useFileManagerStore.getState().panes[0].tabs[0].pinned,
      ).toBe(false);
    });

    it("should reorder tabs via drag", () => {
      const { addTab, reorderTabs } = useFileManagerStore.getState();
      addTab(0, "/a", "A");
      addTab(0, "/b", "B");

      const tabs = useFileManagerStore.getState().panes[0].tabs;
      expect(tabs[0].label).toBe("Root");
      expect(tabs[1].label).toBe("A");
      expect(tabs[2].label).toBe("B");

      reorderTabs(0, 2, 0);
      const reordered = useFileManagerStore.getState().panes[0].tabs;
      expect(reordered[0].label).toBe("B");
      expect(reordered[1].label).toBe("Root");
      expect(reordered[2].label).toBe("A");
    });

    it("should navigate a tab to a new path", () => {
      const { navigateTab } = useFileManagerStore.getState();
      const tabId = useFileManagerStore.getState().panes[0].tabs[0].id;

      navigateTab(0, tabId, "/Documents", "Documents");
      const tab = useFileManagerStore.getState().panes[0].tabs[0];
      expect(tab.path).toBe("/Documents");
      expect(tab.label).toBe("Documents");
    });
  });

  // ── Iter 20: Tab Context Menu Actions ──
  //
  // Right-click on a tab → close-others / close-right / duplicate /
  // move-to-other-pane. Each action operates on the *target* tab
  // (potentially non-active), respects pinned safety, and reuses the
  // canonical addTab / closeTab write sites so closed-tab ring,
  // pathHistory, and active-tab fallbacks all behave identically to a
  // user click on the per-tab × button.
  describe("Iter 20: Tab context-menu actions", () => {
    it("closeOtherTabs closes every non-pinned tab except keepTabId", () => {
      const { addTab, closeOtherTabs } = useFileManagerStore.getState();
      addTab(0, "/a", "A");
      addTab(0, "/b", "B");
      addTab(0, "/c", "C");
      // Pane now has [Root, A, B, C] — 4 tabs.
      const tabs = useFileManagerStore.getState().panes[0].tabs;
      expect(tabs).toHaveLength(4);
      const keep = tabs.find((t) => t.label === "B")!.id;

      closeOtherTabs(0, keep);
      const after = useFileManagerStore.getState().panes[0].tabs;
      expect(after).toHaveLength(1);
      expect(after[0].label).toBe("B");
      expect(useFileManagerStore.getState().panes[0].activeTabId).toBe(keep);
    });

    it("closeOtherTabs preserves pinned tabs", () => {
      const { addTab, pinTab, closeOtherTabs } =
        useFileManagerStore.getState();
      addTab(0, "/a", "A");
      addTab(0, "/b", "B");
      const tabs = useFileManagerStore.getState().panes[0].tabs;
      const pinned = tabs.find((t) => t.label === "A")!.id;
      pinTab(0, pinned);

      const keep = useFileManagerStore
        .getState()
        .panes[0].tabs.find((t) => t.label === "B")!.id;
      closeOtherTabs(0, keep);

      const after = useFileManagerStore.getState().panes[0].tabs;
      // Pinned A survives, B (the kept tab) survives. Root closes.
      expect(after).toHaveLength(2);
      expect(after.some((t) => t.label === "A" && t.pinned)).toBe(true);
      expect(after.some((t) => t.label === "B")).toBe(true);
    });

    it("closeTabsToRight closes tabs to the right of fromTabId in visual order", () => {
      const { addTab, closeTabsToRight } = useFileManagerStore.getState();
      addTab(0, "/a", "A");
      addTab(0, "/b", "B");
      addTab(0, "/c", "C");
      // Visual order: [Root, A, B, C] (no pins).
      const from = useFileManagerStore
        .getState()
        .panes[0].tabs.find((t) => t.label === "A")!.id;

      closeTabsToRight(0, from);
      const after = useFileManagerStore.getState().panes[0].tabs;
      // Root and A survive; B and C are closed.
      expect(after.map((t) => t.label)).toEqual(["Root", "A"]);
    });

    it("closeTabsToRight respects the pinned-first visual order", () => {
      const { addTab, pinTab, closeTabsToRight } =
        useFileManagerStore.getState();
      addTab(0, "/a", "A");
      addTab(0, "/b", "B");
      // Pin B so it sorts before Root/A. Visual order becomes [B, Root, A].
      const bId = useFileManagerStore
        .getState()
        .panes[0].tabs.find((t) => t.label === "B")!.id;
      pinTab(0, bId);

      // Right-click on Root and close tabs to the right → A closes.
      // B is pinned and would be skipped regardless of position.
      const rootId = useFileManagerStore
        .getState()
        .panes[0].tabs.find((t) => t.label === "Root")!.id;
      closeTabsToRight(0, rootId);

      const after = useFileManagerStore.getState().panes[0].tabs;
      expect(after.map((t) => t.label).sort()).toEqual(["B", "Root"].sort());
    });

    it("closeTabsToRight is a no-op for the rightmost tab", () => {
      const { addTab, closeTabsToRight } = useFileManagerStore.getState();
      addTab(0, "/a", "A");
      const aId = useFileManagerStore
        .getState()
        .panes[0].tabs.find((t) => t.label === "A")!.id;

      closeTabsToRight(0, aId);
      const after = useFileManagerStore.getState().panes[0].tabs;
      expect(after).toHaveLength(2);
    });

    it("duplicateTab creates a new tab at the same path (non-active target)", () => {
      const { addTab, setActiveTab, duplicateTab } =
        useFileManagerStore.getState();
      addTab(0, "/a", "A");
      addTab(0, "/b", "B");
      const rootId = useFileManagerStore
        .getState()
        .panes[0].tabs.find((t) => t.label === "Root")!.id;
      // Make B active so the right-click target (Root) is NOT active.
      const bId = useFileManagerStore
        .getState()
        .panes[0].tabs.find((t) => t.label === "B")!.id;
      setActiveTab(0, bId);

      const result = duplicateTab(0, rootId);
      expect(result).not.toBeNull();
      expect(result!.label).toBe("Root");

      const after = useFileManagerStore.getState().panes[0].tabs;
      // Original 3 (Root, A, B) plus one duplicate = 4.
      expect(after).toHaveLength(4);
      expect(after.filter((t) => t.label === "Root")).toHaveLength(2);
    });

    it("moveTabToOtherPane moves the target tab, focuses the other pane", () => {
      const { addTab, moveTabToOtherPane } = useFileManagerStore.getState();
      addTab(0, "/a", "A");
      const aId = useFileManagerStore
        .getState()
        .panes[0].tabs.find((t) => t.label === "A")!.id;

      const result = moveTabToOtherPane(0, aId);
      expect(result).not.toBeNull();
      expect(result!.otherIndex).toBe(1);

      const after = useFileManagerStore.getState();
      // Source pane has Root only (A moved away).
      expect(after.panes[0].tabs.map((t) => t.label)).toEqual(["Root"]);
      // Other pane gained A (plus its own original Root).
      expect(after.panes[1].tabs.some((t) => t.label === "A")).toBe(true);
      // Other pane is now active.
      expect(after.activePaneIndex).toBe(1);
    });

    it("moveTabToOtherPane is a no-op when source pane has only one tab", () => {
      const { moveTabToOtherPane } = useFileManagerStore.getState();
      const onlyId = useFileManagerStore.getState().panes[0].tabs[0].id;

      const result = moveTabToOtherPane(0, onlyId);
      expect(result).toBeNull();
      // Source pane keeps its one tab; active pane unchanged.
      expect(useFileManagerStore.getState().panes[0].tabs).toHaveLength(1);
      expect(useFileManagerStore.getState().activePaneIndex).toBe(0);
    });

    it("moveTabToOtherPane is a no-op for pinned tabs", () => {
      const { addTab, pinTab, moveTabToOtherPane } =
        useFileManagerStore.getState();
      addTab(0, "/a", "A");
      const aId = useFileManagerStore
        .getState()
        .panes[0].tabs.find((t) => t.label === "A")!.id;
      pinTab(0, aId);

      const result = moveTabToOtherPane(0, aId);
      expect(result).toBeNull();
      // Pinned A stays in the source pane.
      expect(
        useFileManagerStore
          .getState()
          .panes[0].tabs.some((t) => t.id === aId && t.pinned),
      ).toBe(true);
    });
  });

  // ── Reopen Closed Tab (Cmd+Shift+T) ──
  //
  // Browser-style closed-tab ring buffer. The closeTab action captures a
  // bit-exact snapshot before removing a tab; reopenClosedTab pops the
  // most recent snapshot from the active pane's ring and revives it.
  describe("Reopen Closed Tab", () => {
    it("captures a snapshot when a non-pinned tab is closed", () => {
      const { addTab, navigateTab, closeTab } = useFileManagerStore.getState();
      addTab(0, "/projects", "Projects");
      const tabId = useFileManagerStore.getState().panes[0].tabs[1].id;
      // Build a small back/forward stack so the snapshot has history.
      navigateTab(0, tabId, "/projects/alpha", "alpha");
      navigateTab(0, tabId, "/projects/alpha/src", "src");
      closeTab(0, tabId);

      const ring = useFileManagerStore.getState().recentlyClosedTabs[0];
      expect(ring).toHaveLength(1);
      expect(ring[0].path).toBe("/projects/alpha/src");
      expect(ring[0].pathHistory.length).toBeGreaterThanOrEqual(2);
    });

    it("does not capture a snapshot for pinned tabs", () => {
      const { addTab, pinTab, closeTab } = useFileManagerStore.getState();
      addTab(0, "/pinned", "Pinned");
      const tabId = useFileManagerStore.getState().panes[0].tabs[1].id;
      pinTab(0, tabId);
      closeTab(0, tabId);
      // Pinned tabs cannot be closed → ring stays empty.
      expect(useFileManagerStore.getState().recentlyClosedTabs[0]).toHaveLength(0);
    });

    it("does not capture a snapshot when refusing to close the last tab", () => {
      const { closeTab } = useFileManagerStore.getState();
      const lastTabId = useFileManagerStore.getState().panes[0].tabs[0].id;
      closeTab(0, lastTabId);
      expect(useFileManagerStore.getState().recentlyClosedTabs[0]).toHaveLength(0);
    });

    it("revives the most recent snapshot bit-exact via LIFO", () => {
      const { addTab, navigateTab, closeTab, reopenClosedTab } =
        useFileManagerStore.getState();
      addTab(0, "/a", "A");
      addTab(0, "/b", "B");
      const aId = useFileManagerStore.getState().panes[0].tabs[1].id;
      const bId = useFileManagerStore.getState().panes[0].tabs[2].id;
      // Give B a back/forward stack so we can verify it survives revival.
      navigateTab(0, bId, "/b/inner", "inner");
      closeTab(0, aId);
      closeTab(0, bId);

      // LIFO: B closed last → comes back first.
      const revivedBId = reopenClosedTab(0);
      expect(revivedBId).toBeTruthy();
      const tabs = useFileManagerStore.getState().panes[0].tabs;
      const revivedB = tabs.find((t) => t.id === revivedBId);
      expect(revivedB?.path).toBe("/b/inner");
      expect(revivedB?.label).toBe("inner");
      expect(revivedB?.pathHistory).toContain("/b");
      expect(revivedB?.pathHistory).toContain("/b/inner");
      // Active tab follows the revival so users can keep working.
      expect(useFileManagerStore.getState().panes[0].activeTabId).toBe(revivedBId);

      // Ring shrinks by one.
      expect(useFileManagerStore.getState().recentlyClosedTabs[0]).toHaveLength(1);
    });

    it("returns null and is a no-op when the ring is empty", () => {
      const { reopenClosedTab } = useFileManagerStore.getState();
      const before = useFileManagerStore.getState().panes[0].tabs.length;
      const result = reopenClosedTab(0);
      expect(result).toBeNull();
      expect(useFileManagerStore.getState().panes[0].tabs).toHaveLength(before);
    });

    it("caps the per-pane ring at 10 entries", () => {
      const { addTab, closeTab } = useFileManagerStore.getState();
      // Open and close 12 tabs in pane 0; oldest two should be evicted.
      for (let i = 0; i < 12; i++) {
        addTab(0, `/tab-${i}`, `Tab ${i}`);
        const tabs = useFileManagerStore.getState().panes[0].tabs;
        const justAdded = tabs[tabs.length - 1].id;
        closeTab(0, justAdded);
      }
      const ring = useFileManagerStore.getState().recentlyClosedTabs[0];
      expect(ring).toHaveLength(10);
      // The freshest entry sits at index 0.
      expect(ring[0].path).toBe("/tab-11");
      // The oldest surviving entry is /tab-2 (0 and 1 fell off the back).
      expect(ring[ring.length - 1].path).toBe("/tab-2");
    });

    it("survives a pre-iteration hydration where recentlyClosedTabs is missing", () => {
      // Simulate the persist middleware merging an old localStorage blob
      // that pre-dates the closed-tab ring. The merged state intentionally
      // strips the new field to prove closeTab/reopenClosedTab tolerate
      // it without throwing — defensive guards inside the actions are the
      // only thing standing between an upgraded user and a crash on the
      // first tab close after launch.
      useFileManagerStore.setState((s) => {
        const next = { ...s };
        delete (next as Partial<typeof s>).recentlyClosedTabs;
        return next;
      });
      const { addTab, closeTab, reopenClosedTab } =
        useFileManagerStore.getState();
      addTab(0, "/legacy", "Legacy");
      const tabId = useFileManagerStore.getState().panes[0].tabs[1].id;
      // Must not throw even though the ring field was absent.
      closeTab(0, tabId);
      const ring = useFileManagerStore.getState().recentlyClosedTabs[0];
      expect(ring).toHaveLength(1);
      expect(ring[0].path).toBe("/legacy");
      // And reopening must round-trip the just-closed tab.
      const revivedId = reopenClosedTab(0);
      expect(revivedId).toBeTruthy();
      expect(
        useFileManagerStore.getState().panes[0].tabs.find((t) => t.id === revivedId)?.path,
      ).toBe("/legacy");
    });

    it("isolates closed-tab rings between panes", () => {
      const { addTab, closeTab, reopenClosedTab } =
        useFileManagerStore.getState();
      addTab(0, "/pane0", "Pane0");
      addTab(1, "/pane1", "Pane1");
      const t0 = useFileManagerStore.getState().panes[0].tabs[1].id;
      const t1 = useFileManagerStore.getState().panes[1].tabs[1].id;
      closeTab(0, t0);
      closeTab(1, t1);

      // Reopening on pane 0 must not touch pane 1's ring or tabs.
      const revived = reopenClosedTab(0);
      expect(revived).toBeTruthy();
      expect(useFileManagerStore.getState().recentlyClosedTabs[0]).toHaveLength(0);
      expect(useFileManagerStore.getState().recentlyClosedTabs[1]).toHaveLength(1);
      expect(
        useFileManagerStore.getState().panes[0].tabs.find((t) => t.path === "/pane0"),
      ).toBeTruthy();
      expect(
        useFileManagerStore.getState().panes[1].tabs.find((t) => t.path === "/pane1"),
      ).toBeFalsy();
    });
  });

  // ── Selection Stash & Recall ──
  //
  // Persistent global ring of explicit selection snapshots. Pairs with
  // the closed-tab ring (Reopen Closed Tab) to give the user a complete
  // "where I left off" recovery story: tabs come back via reopenClosedTab,
  // working selections come back via recallStash.
  describe("Selection Stash & Recall", () => {
    function selectFiles(paneIndex: 0 | 1, paths: string[]) {
      const pane = useFileManagerStore.getState().panes[paneIndex];
      useFileManagerStore.getState().setSelection(pane.id, paths);
    }

    it("captures the active pane's selection into the stash ring", () => {
      const { navigateTab, stashCurrentSelection } =
        useFileManagerStore.getState();
      const tabId = useFileManagerStore.getState().panes[0].tabs[0].id;
      navigateTab(0, tabId, "/work/alpha", "alpha");
      selectFiles(0, ["/work/alpha/a.txt", "/work/alpha/b.txt"]);

      const stash = stashCurrentSelection("My set");
      expect(stash).not.toBeNull();
      expect(stash?.paths).toEqual(["/work/alpha/a.txt", "/work/alpha/b.txt"]);
      expect(stash?.sourceDir).toBe("/work/alpha");
      expect(stash?.label).toBe("My set");
      const ring = useFileManagerStore.getState().selectionStashes;
      expect(ring).toHaveLength(1);
      expect(ring[0].id).toBe(stash?.id);
    });

    it("returns null and creates no entry when nothing is selected", () => {
      const result = useFileManagerStore.getState().stashCurrentSelection();
      expect(result).toBeNull();
      expect(useFileManagerStore.getState().selectionStashes).toHaveLength(0);
    });

    it("snapshots the selection by value (later changes do not bleed in)", () => {
      const { stashCurrentSelection } = useFileManagerStore.getState();
      selectFiles(0, ["/x/1", "/x/2"]);
      const stash = stashCurrentSelection();
      // Mutate the live selection AFTER stashing.
      selectFiles(0, ["/y/9"]);
      expect(stash?.paths).toEqual(["/x/1", "/x/2"]);
      const ring = useFileManagerStore.getState().selectionStashes;
      expect(ring[0].paths).toEqual(["/x/1", "/x/2"]);
    });

    it("recalls a stash by navigating the pane and restoring selection", () => {
      const { navigateTab, stashCurrentSelection, recallStash } =
        useFileManagerStore.getState();
      const tabId = useFileManagerStore.getState().panes[0].tabs[0].id;
      navigateTab(0, tabId, "/projects/spec", "spec");
      selectFiles(0, ["/projects/spec/one.md", "/projects/spec/two.md"]);
      const stash = stashCurrentSelection();
      expect(stash).not.toBeNull();

      // Walk away: navigate elsewhere and clobber the live selection.
      navigateTab(0, tabId, "/elsewhere", "elsewhere");
      selectFiles(0, []);
      const before = useFileManagerStore.getState().panes[0].tabs.find(
        (t) => t.id === tabId,
      );
      expect(before?.path).toBe("/elsewhere");

      recallStash(stash!.id, 0);

      // Pane navigates back to the source dir.
      const after = useFileManagerStore.getState().panes[0].tabs.find(
        (t) => t.id === tabId,
      );
      expect(after?.path).toBe("/projects/spec");
      // Selection is restored on the pane that received the recall.
      const paneId = useFileManagerStore.getState().panes[0].id;
      expect(useFileManagerStore.getState().selectedPaths[paneId]).toEqual([
        "/projects/spec/one.md",
        "/projects/spec/two.md",
      ]);
      // Stash stays in the ring so it can be re-recalled.
      expect(useFileManagerStore.getState().selectionStashes).toHaveLength(1);
    });

    it("can recall the same stash into the other pane", () => {
      const { stashCurrentSelection, recallStash } =
        useFileManagerStore.getState();
      selectFiles(0, ["/shared/file.txt"]);
      const stash = stashCurrentSelection();
      expect(stash).not.toBeNull();

      recallStash(stash!.id, 1);
      const pane1Id = useFileManagerStore.getState().panes[1].id;
      expect(useFileManagerStore.getState().selectedPaths[pane1Id]).toEqual([
        "/shared/file.txt",
      ]);
    });

    it("recall is a no-op for an unknown stash id", () => {
      const { recallStash } = useFileManagerStore.getState();
      recallStash("nonexistent-id", 0);
      const paneId = useFileManagerStore.getState().panes[0].id;
      expect(useFileManagerStore.getState().selectedPaths[paneId]).toBeUndefined();
    });

    it("discards a single stash by id without affecting others", () => {
      const { stashCurrentSelection, discardStash } =
        useFileManagerStore.getState();
      selectFiles(0, ["/a"]);
      const first = stashCurrentSelection();
      selectFiles(0, ["/b"]);
      const second = stashCurrentSelection();
      expect(useFileManagerStore.getState().selectionStashes).toHaveLength(2);

      discardStash(first!.id);
      const remaining = useFileManagerStore.getState().selectionStashes;
      expect(remaining).toHaveLength(1);
      expect(remaining[0].id).toBe(second?.id);
    });

    it("clearStashes wipes the entire ring", () => {
      const { stashCurrentSelection, clearStashes } =
        useFileManagerStore.getState();
      selectFiles(0, ["/a"]);
      stashCurrentSelection();
      selectFiles(0, ["/b"]);
      stashCurrentSelection();
      clearStashes();
      expect(useFileManagerStore.getState().selectionStashes).toHaveLength(0);
    });

    it("caps the stash ring at 20 entries (LIFO eviction)", () => {
      const { stashCurrentSelection } = useFileManagerStore.getState();
      for (let i = 0; i < 25; i++) {
        selectFiles(0, [`/dir-${i}/file.txt`]);
        stashCurrentSelection(`Stash ${i}`);
      }
      const ring = useFileManagerStore.getState().selectionStashes;
      expect(ring).toHaveLength(20);
      // Newest at index 0; oldest surviving entry is the 6th-from-last
      // pushed (indices 5..24 survived, since 0..4 fell off the back).
      expect(ring[0].label).toBe("Stash 24");
      expect(ring[ring.length - 1].label).toBe("Stash 5");
    });

    it("survives a pre-iteration hydration where selectionStashes is missing", () => {
      // Simulate persistence from before this iteration.
      useFileManagerStore.setState((s) => {
        const next = { ...s };
        delete (next as Partial<typeof s>).selectionStashes;
        return next;
      });
      selectFiles(0, ["/legacy/a", "/legacy/b"]);
      // Must not throw and must lazily seed the ring.
      const stash = useFileManagerStore.getState().stashCurrentSelection();
      expect(stash).not.toBeNull();
      expect(useFileManagerStore.getState().selectionStashes).toHaveLength(1);
    });
  });

  // ── T-009: Navigation ──

  describe("T-009: Navigation (Favorites, Recents, Tree)", () => {
    it("should add and remove favorites", () => {
      const { addFavorite, removeFavorite } = useFileManagerStore.getState();

      addFavorite({ id: "fav-1", name: "Home", path: "/home" });
      expect(useFileManagerStore.getState().favorites).toHaveLength(1);
      expect(useFileManagerStore.getState().favorites[0].name).toBe("Home");

      removeFavorite("fav-1");
      expect(useFileManagerStore.getState().favorites).toHaveLength(0);
    });

    it("should track recent locations with deduplication", () => {
      const { addRecentLocation } = useFileManagerStore.getState();

      addRecentLocation("/home");
      addRecentLocation("/documents");
      addRecentLocation("/home"); // Duplicate - should move to top

      const recents = useFileManagerStore.getState().recentLocations;
      expect(recents).toHaveLength(2);
      expect(recents[0].path).toBe("/home"); // Most recent first
      expect(recents[1].path).toBe("/documents");
    });

    it("should limit recent locations to 20", () => {
      const { addRecentLocation } = useFileManagerStore.getState();
      for (let i = 0; i < 25; i++) {
        addRecentLocation(`/path-${i}`);
      }
      expect(useFileManagerStore.getState().recentLocations).toHaveLength(20);
    });

    it("should toggle tree expanded paths", () => {
      const { toggleTreeExpanded } = useFileManagerStore.getState();

      toggleTreeExpanded("/home");
      expect(useFileManagerStore.getState().expandedTreePaths).toContain("/home");

      toggleTreeExpanded("/home");
      expect(useFileManagerStore.getState().expandedTreePaths).not.toContain("/home");
    });
  });

  describe("Bookmark Shortcuts", () => {
    function navigateActive(path: string, label = "Folder") {
      const state = useFileManagerStore.getState();
      const pane = state.panes[state.activePaneIndex];
      state.navigateTab(state.activePaneIndex, pane.activeTabId, path, label);
    }

    it("toggleFavoriteForCurrentDir adds the active dir on first call", () => {
      navigateActive("/home/work", "work");
      const result = useFileManagerStore
        .getState()
        .toggleFavoriteForCurrentDir();
      expect(result).not.toBeNull();
      expect(result?.added).toBe(true);
      expect(result?.item.path).toBe("/home/work");
      expect(result?.item.name).toBe("work");
      expect(useFileManagerStore.getState().favorites).toHaveLength(1);
      expect(useFileManagerStore.getState().favorites[0].path).toBe("/home/work");
    });

    it("toggleFavoriteForCurrentDir removes when already bookmarked", () => {
      navigateActive("/home/work");
      useFileManagerStore.getState().toggleFavoriteForCurrentDir();
      const second = useFileManagerStore
        .getState()
        .toggleFavoriteForCurrentDir();
      expect(second?.added).toBe(false);
      expect(useFileManagerStore.getState().favorites).toHaveLength(0);
    });

    it("toggleFavoriteForCurrentDir derives 'Root' for /", () => {
      const result = useFileManagerStore
        .getState()
        .toggleFavoriteForCurrentDir();
      expect(result?.item.path).toBe("/");
      expect(result?.item.name).toBe("Root");
    });

    it("toggleFavoriteForCurrentDir is independent across panes", () => {
      // Default starts in single-pane mode with active index 0
      navigateActive("/pane0/dir");
      useFileManagerStore.getState().toggleFavoriteForCurrentDir();
      useFileManagerStore.setState({ activePaneIndex: 1 });
      navigateActive("/pane1/other");
      useFileManagerStore.getState().toggleFavoriteForCurrentDir();
      const favs = useFileManagerStore.getState().favorites;
      expect(favs).toHaveLength(2);
      expect(favs.map((f) => f.path)).toEqual([
        "/pane0/dir",
        "/pane1/other",
      ]);
    });

    it("jumpToFavoriteByIndex navigates the active pane to the slot", () => {
      useFileManagerStore.getState().addFavorite({
        id: "fav-a",
        name: "Alpha",
        path: "/var/alpha",
      });
      useFileManagerStore.getState().addFavorite({
        id: "fav-b",
        name: "Beta",
        path: "/var/beta",
      });
      const ok = useFileManagerStore.getState().jumpToFavoriteByIndex(0, 1);
      expect(ok).toBe(true);
      const pane = useFileManagerStore.getState().panes[0];
      const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
      expect(tab?.path).toBe("/var/beta");
      expect(tab?.label).toBe("Beta");
    });

    it("jumpToFavoriteByIndex pushes the destination onto pathHistory", () => {
      useFileManagerStore.getState().addFavorite({
        id: "fav-a",
        name: "Alpha",
        path: "/var/alpha",
      });
      useFileManagerStore.getState().jumpToFavoriteByIndex(0, 0);
      const pane = useFileManagerStore.getState().panes[0];
      const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
      // navigateTab pushes onto the back/forward stack — proves the jump
      // routes through the same pipeline as every other navigation.
      expect(tab?.pathHistory).toContain("/var/alpha");
      expect(tab?.pathHistory[tab.historyIndex]).toBe("/var/alpha");
    });

    it("jumpToFavoriteByIndex is a safe no-op for empty/out-of-range slots", () => {
      // Empty ring
      expect(
        useFileManagerStore.getState().jumpToFavoriteByIndex(0, 0),
      ).toBe(false);
      // Single entry, ask for slot 5
      useFileManagerStore.getState().addFavorite({
        id: "fav-a",
        name: "Alpha",
        path: "/var/alpha",
      });
      expect(
        useFileManagerStore.getState().jumpToFavoriteByIndex(0, 5),
      ).toBe(false);
      const pane = useFileManagerStore.getState().panes[0];
      const tab = pane.tabs.find((t) => t.id === pane.activeTabId);
      // Original tab still at /
      expect(tab?.path).toBe("/");
    });

    it("jumpToFavoriteByIndex routes to the requested pane, not just the active one", () => {
      useFileManagerStore.getState().addFavorite({
        id: "fav-a",
        name: "Alpha",
        path: "/var/alpha",
      });
      // Active pane stays at 0; explicit jump targets pane 1.
      useFileManagerStore.getState().jumpToFavoriteByIndex(1, 0);
      const pane1 = useFileManagerStore.getState().panes[1];
      const tab1 = pane1.tabs.find((t) => t.id === pane1.activeTabId);
      expect(tab1?.path).toBe("/var/alpha");
      // Active pane (0) untouched.
      const pane0 = useFileManagerStore.getState().panes[0];
      const tab0 = pane0.tabs.find((t) => t.id === pane0.activeTabId);
      expect(tab0?.path).toBe("/");
    });

    it("toggleFavoriteForCurrentDir is idempotent when re-bookmarked at a different label", () => {
      navigateActive("/home/work", "work");
      useFileManagerStore.getState().toggleFavoriteForCurrentDir();
      // Re-navigate the same path with a different label, then re-toggle:
      // the matcher uses path, not label, so it removes the original.
      navigateActive("/home/work", "Aliased");
      const second = useFileManagerStore
        .getState()
        .toggleFavoriteForCurrentDir();
      expect(second?.added).toBe(false);
      expect(useFileManagerStore.getState().favorites).toHaveLength(0);
    });
  });

  describe("Open in Other Pane", () => {
    function navigateActive(path: string, label = "Folder") {
      const state = useFileManagerStore.getState();
      const pane = state.panes[state.activePaneIndex];
      state.navigateTab(state.activePaneIndex, pane.activeTabId, path, label);
    }

    it("opens the active path as a new tab in the other pane and focuses it", () => {
      navigateActive("/home/projects", "projects");
      const result = useFileManagerStore.getState().openInOtherPane(0);
      expect(result).not.toBeNull();
      expect(result?.path).toBe("/home/projects");
      expect(result?.label).toBe("projects");
      expect(result?.otherIndex).toBe(1);
      const state = useFileManagerStore.getState();
      // Other pane received a fresh tab pointing at the source path.
      const otherPane = state.panes[1];
      const newTab = otherPane.tabs.find((t) => t.id === otherPane.activeTabId);
      expect(newTab?.path).toBe("/home/projects");
      expect(newTab?.label).toBe("projects");
      // Focus moved to the other pane.
      expect(state.activePaneIndex).toBe(1);
    });

    it("auto-exits single-pane mode so the new tab is visible", () => {
      // Default state is singlePaneMode: true
      expect(useFileManagerStore.getState().singlePaneMode).toBe(true);
      navigateActive("/home/projects");
      useFileManagerStore.getState().openInOtherPane(0);
      expect(useFileManagerStore.getState().singlePaneMode).toBe(false);
    });

    it("creates a fresh tab without disturbing the source pane", () => {
      navigateActive("/home/projects", "projects");
      const beforeSourceTabs = useFileManagerStore
        .getState()
        .panes[0].tabs.map((t) => t.id);
      useFileManagerStore.getState().openInOtherPane(0);
      const afterSourceTabs = useFileManagerStore
        .getState()
        .panes[0].tabs.map((t) => t.id);
      // Source pane's tab list is unchanged.
      expect(afterSourceTabs).toEqual(beforeSourceTabs);
      // The other pane gained a tab.
      expect(useFileManagerStore.getState().panes[1].tabs.length).toBe(2);
    });

    it("new tab in other pane has its own pathHistory initialized to source path", () => {
      navigateActive("/home/projects", "projects");
      useFileManagerStore.getState().openInOtherPane(0);
      const otherPane = useFileManagerStore.getState().panes[1];
      const newTab = otherPane.tabs.find((t) => t.id === otherPane.activeTabId);
      expect(newTab?.pathHistory).toEqual(["/home/projects"]);
      expect(newTab?.historyIndex).toBe(0);
    });

    it("works in either direction (pane 1 → pane 0)", () => {
      // Switch active pane to 1, navigate it, then echo to 0.
      useFileManagerStore.setState({ singlePaneMode: false });
      useFileManagerStore.setState({ activePaneIndex: 1 });
      navigateActive("/right/side");
      const result = useFileManagerStore.getState().openInOtherPane(1);
      expect(result?.otherIndex).toBe(0);
      const state = useFileManagerStore.getState();
      const leftPane = state.panes[0];
      const newTab = leftPane.tabs.find((t) => t.id === leftPane.activeTabId);
      expect(newTab?.path).toBe("/right/side");
      expect(state.activePaneIndex).toBe(0);
    });

    it("does not mutate the source tab's history when echoed", () => {
      navigateActive("/a", "a");
      navigateActive("/b", "b");
      useFileManagerStore.getState().openInOtherPane(0);
      const sourcePane = useFileManagerStore.getState().panes[0];
      const sourceTab = sourcePane.tabs.find(
        (t) => t.id === sourcePane.activeTabId,
      );
      // Source tab still has its own back stack intact.
      expect(sourceTab?.path).toBe("/b");
      expect(sourceTab?.pathHistory).toContain("/a");
      expect(sourceTab?.pathHistory).toContain("/b");
    });
  });

  describe("Swap Panes", () => {
    function setLeftAndRight(leftPath: string, rightPath: string) {
      // Make pane[0] active, populate it, then pane[1].
      useFileManagerStore.setState({ singlePaneMode: false });
      const s = useFileManagerStore.getState();
      const left = s.panes[0];
      const right = s.panes[1];
      s.navigateTab(0, left.activeTabId, leftPath, leftPath);
      s.navigateTab(1, right.activeTabId, rightPath, rightPath);
    }

    it("swaps the contents of the two panes by index", () => {
      setLeftAndRight("/left/here", "/right/here");
      const beforeLeftId = useFileManagerStore.getState().panes[0].id;
      const beforeRightId = useFileManagerStore.getState().panes[1].id;

      const result = useFileManagerStore.getState().swapPanes();
      expect(result).not.toBeNull();

      const after = useFileManagerStore.getState();
      // Pane structure flipped: panes[0] now carries the OLD right pane.
      expect(after.panes[0].id).toBe(beforeRightId);
      expect(after.panes[1].id).toBe(beforeLeftId);
      const leftActive = after.panes[0].tabs.find(
        (t) => t.id === after.panes[0].activeTabId,
      );
      const rightActive = after.panes[1].tabs.find(
        (t) => t.id === after.panes[1].activeTabId,
      );
      expect(leftActive?.path).toBe("/right/here");
      expect(rightActive?.path).toBe("/left/here");
    });

    it("leaves activePaneIndex on the same physical side", () => {
      setLeftAndRight("/left/here", "/right/here");
      // Focus pane 0 explicitly.
      useFileManagerStore.setState({ activePaneIndex: 0 });
      useFileManagerStore.getState().swapPanes();
      // Focus stays on the LEFT physical side; the user now sees the
      // OLD right content there.
      expect(useFileManagerStore.getState().activePaneIndex).toBe(0);
    });

    it("auto-exits single-pane mode so the swap is visible", () => {
      // Default is true; force it on first.
      useFileManagerStore.setState({ singlePaneMode: true });
      setLeftAndRight("/left/here", "/right/here");
      // navigateTab in setLeftAndRight already disabled it via setState
      // above; force it back on for the explicit test.
      useFileManagerStore.setState({ singlePaneMode: true });
      useFileManagerStore.getState().swapPanes();
      expect(useFileManagerStore.getState().singlePaneMode).toBe(false);
    });

    it("selections follow content because they are keyed by pane.id, not index", () => {
      setLeftAndRight("/left/here", "/right/here");
      const before = useFileManagerStore.getState();
      const leftId = before.panes[0].id;
      const rightId = before.panes[1].id;
      // Stamp distinct selections into each pane.
      before.setSelection(leftId, ["/left/here/file-a"]);
      before.setSelection(rightId, ["/right/here/file-b"]);

      useFileManagerStore.getState().swapPanes();

      const after = useFileManagerStore.getState();
      // panes[0] is now the OLD right pane; its selection is still
      // /right/here/file-b because lookups are keyed by pane.id, not
      // by the array index.
      expect(after.panes[0].id).toBe(rightId);
      expect(after.selectedPaths[after.panes[0].id]).toEqual([
        "/right/here/file-b",
      ]);
      expect(after.selectedPaths[after.panes[1].id]).toEqual([
        "/left/here/file-a",
      ]);
    });

    it("is its own inverse \u2014 swapping twice is a no-op", () => {
      setLeftAndRight("/left/here", "/right/here");
      const beforeLeftId = useFileManagerStore.getState().panes[0].id;
      const beforeRightId = useFileManagerStore.getState().panes[1].id;
      useFileManagerStore.getState().swapPanes();
      useFileManagerStore.getState().swapPanes();
      const after = useFileManagerStore.getState();
      expect(after.panes[0].id).toBe(beforeLeftId);
      expect(after.panes[1].id).toBe(beforeRightId);
    });

    it("returns the new active pane's path so the toast can describe it", () => {
      setLeftAndRight("/left/here", "/right/here");
      useFileManagerStore.setState({ activePaneIndex: 0 });
      const result = useFileManagerStore.getState().swapPanes();
      // After swap, panes[0] holds the OLD right content; that is the
      // NEW active pane (because activePaneIndex=0 unchanged), so the
      // returned path is /right/here.
      expect(result?.activePath).toBe("/right/here");
    });
  });

  describe("Collect Clipboard Paths", () => {
    function navigateActive(path: string, label = "Folder") {
      const state = useFileManagerStore.getState();
      const pane = state.panes[state.activePaneIndex];
      state.navigateTab(state.activePaneIndex, pane.activeTabId, path, label);
    }

    it("returns every selected path when there is a multi-file selection", () => {
      navigateActive("/home/projects");
      const state = useFileManagerStore.getState();
      const pane = state.panes[state.activePaneIndex];
      state.setSelection(pane.id, [
        "/home/projects/a.pdf",
        "/home/projects/b.pdf",
        "/home/projects/c.pdf",
      ]);
      const result = useFileManagerStore.getState().collectClipboardPaths();
      expect(result.source).toBe("selection");
      expect(result.paths).toEqual([
        "/home/projects/a.pdf",
        "/home/projects/b.pdf",
        "/home/projects/c.pdf",
      ]);
    });

    it("returns a single-element array (selection) for a single selected path", () => {
      navigateActive("/home/projects");
      const state = useFileManagerStore.getState();
      const pane = state.panes[state.activePaneIndex];
      state.setSelection(pane.id, ["/home/projects/only.txt"]);
      const result = useFileManagerStore.getState().collectClipboardPaths();
      expect(result.source).toBe("selection");
      expect(result.paths).toEqual(["/home/projects/only.txt"]);
    });

    it("falls back to the active pane's current directory when nothing is selected", () => {
      navigateActive("/home/projects/here");
      const result = useFileManagerStore.getState().collectClipboardPaths();
      expect(result.source).toBe("directory");
      expect(result.paths).toEqual(["/home/projects/here"]);
    });

    it("reads from the ACTIVE pane, not pane 0 by default", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      // Populate both panes then activate pane 1.
      const state = useFileManagerStore.getState();
      state.navigateTab(0, state.panes[0].activeTabId, "/left", "left");
      state.navigateTab(1, state.panes[1].activeTabId, "/right", "right");
      useFileManagerStore.setState({ activePaneIndex: 1 });
      const result = useFileManagerStore.getState().collectClipboardPaths();
      expect(result.source).toBe("directory");
      expect(result.paths).toEqual(["/right"]);
    });

    it("returns a defensive copy of the selection (caller can't mutate store state)", () => {
      navigateActive("/home/projects");
      const state = useFileManagerStore.getState();
      const pane = state.panes[state.activePaneIndex];
      state.setSelection(pane.id, [
        "/home/projects/a",
        "/home/projects/b",
      ]);
      const result = useFileManagerStore.getState().collectClipboardPaths();
      result.paths.push("/tamper");
      const storeSelection =
        useFileManagerStore.getState().selectedPaths[pane.id];
      expect(storeSelection).toEqual([
        "/home/projects/a",
        "/home/projects/b",
      ]);
    });

    it("preserves selection order so copy-paste matches on-screen order", () => {
      navigateActive("/home/projects");
      const state = useFileManagerStore.getState();
      const pane = state.panes[state.activePaneIndex];
      state.setSelection(pane.id, ["/z.txt", "/a.txt", "/m.txt"]);
      const result = useFileManagerStore.getState().collectClipboardPaths();
      expect(result.paths).toEqual(["/z.txt", "/a.txt", "/m.txt"]);
    });
  });

  describe("Duplicate Tab", () => {
    function navigateActive(path: string, label = "Folder") {
      const state = useFileManagerStore.getState();
      const pane = state.panes[state.activePaneIndex];
      state.navigateTab(state.activePaneIndex, pane.activeTabId, path, label);
    }

    it("creates a new tab in the SAME pane with the active tab's path", () => {
      navigateActive("/home/projects", "projects");
      const beforeTabCount = useFileManagerStore.getState().panes[0].tabs
        .length;
      const result = useFileManagerStore.getState().duplicateActiveTab(0);
      expect(result).not.toBeNull();
      expect(result?.path).toBe("/home/projects");
      expect(result?.label).toBe("projects");
      const after = useFileManagerStore.getState().panes[0];
      expect(after.tabs.length).toBe(beforeTabCount + 1);
    });

    it("focuses the freshly-cloned tab", () => {
      navigateActive("/home/projects", "projects");
      const beforeActiveId = useFileManagerStore.getState().panes[0]
        .activeTabId;
      useFileManagerStore.getState().duplicateActiveTab(0);
      const afterActiveId = useFileManagerStore.getState().panes[0]
        .activeTabId;
      expect(afterActiveId).not.toBe(beforeActiveId);
    });

    it("clone has its own pathHistory starting at the cloned path", () => {
      navigateActive("/home/projects", "projects");
      useFileManagerStore.getState().duplicateActiveTab(0);
      const pane = useFileManagerStore.getState().panes[0];
      const clone = pane.tabs.find((t) => t.id === pane.activeTabId);
      expect(clone?.pathHistory).toEqual(["/home/projects"]);
      expect(clone?.historyIndex).toBe(0);
    });

    it("does NOT touch the other pane", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      navigateActive("/home/projects", "projects");
      const beforeOtherTabs = useFileManagerStore.getState().panes[1].tabs
        .length;
      useFileManagerStore.getState().duplicateActiveTab(0);
      const afterOtherTabs = useFileManagerStore.getState().panes[1].tabs
        .length;
      expect(afterOtherTabs).toBe(beforeOtherTabs);
    });

    it("does not mutate the source tab's history", () => {
      navigateActive("/a", "a");
      navigateActive("/b", "b");
      useFileManagerStore.getState().duplicateActiveTab(0);
      // The clone is now active. Find the OLD source tab \u2014 it
      // should still have its `/a` -> `/b` back stack intact and
      // still be sitting at `/b`.
      const pane = useFileManagerStore.getState().panes[0];
      const sourceTab = pane.tabs.find(
        (t) => t.id !== pane.activeTabId && t.path === "/b",
      );
      expect(sourceTab).toBeDefined();
      expect(sourceTab?.pathHistory).toContain("/a");
      expect(sourceTab?.pathHistory).toContain("/b");
    });

    it("each duplicate is independent \u2014 navigating the clone leaves the original at its path", () => {
      navigateActive("/origin", "origin");
      useFileManagerStore.getState().duplicateActiveTab(0);
      // Now the clone is active. Navigate it elsewhere.
      const pane = useFileManagerStore.getState().panes[0];
      const cloneId = pane.activeTabId;
      useFileManagerStore.getState().navigateTab(0, cloneId, "/elsewhere", "elsewhere");
      const after = useFileManagerStore.getState().panes[0];
      const original = after.tabs.find(
        (t) => t.id !== cloneId && t.path === "/origin",
      );
      expect(original).toBeDefined();
      expect(original?.path).toBe("/origin");
    });
  });

  describe("Mirror View to Other Pane", () => {
    it("copies viewMode, sort, and groupBy from active to other pane", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      // Configure pane 0 with non-default view.
      useFileManagerStore.getState().setViewMode(0, "grid");
      useFileManagerStore.getState().setSorting(0, "modified", false);
      useFileManagerStore.getState().setGroupBy(0, "type");
      // Pane 1 starts with defaults.
      useFileManagerStore.getState().setViewMode(1, "detail");
      useFileManagerStore.getState().setSorting(1, "name", true);
      useFileManagerStore.getState().setGroupBy(1, null);

      const result = useFileManagerStore
        .getState()
        .mirrorViewToOtherPane(0);

      expect(result).not.toBeNull();
      expect(result?.otherIndex).toBe(1);
      expect(result?.viewMode).toBe("grid");
      expect(result?.sortBy).toBe("modified");
      expect(result?.sortAsc).toBe(false);
      expect(result?.groupBy).toBe("type");

      const after = useFileManagerStore.getState();
      expect(after.panes[1].viewMode).toBe("grid");
      expect(after.panes[1].sortBy).toBe("modified");
      expect(after.panes[1].sortAsc).toBe(false);
      expect(after.panes[1].groupBy).toBe("type");
    });

    it("works in either direction (pane 1 \u2192 pane 0)", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      useFileManagerStore.getState().setViewMode(1, "list");
      useFileManagerStore.getState().setSorting(1, "size", true);
      useFileManagerStore.getState().setGroupBy(1, null);
      useFileManagerStore.getState().setViewMode(0, "detail");
      useFileManagerStore.getState().setSorting(0, "name", true);

      const result = useFileManagerStore
        .getState()
        .mirrorViewToOtherPane(1);

      expect(result?.otherIndex).toBe(0);
      const after = useFileManagerStore.getState();
      expect(after.panes[0].viewMode).toBe("list");
      expect(after.panes[0].sortBy).toBe("size");
    });

    it("returns null and does NOT touch the other pane when both already match", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      // Force both panes identical.
      useFileManagerStore.getState().setViewMode(0, "detail");
      useFileManagerStore.getState().setSorting(0, "name", true);
      useFileManagerStore.getState().setGroupBy(0, null);
      useFileManagerStore.getState().setViewMode(1, "detail");
      useFileManagerStore.getState().setSorting(1, "name", true);
      useFileManagerStore.getState().setGroupBy(1, null);

      const result = useFileManagerStore
        .getState()
        .mirrorViewToOtherPane(0);
      expect(result).toBeNull();
    });

    it("auto-exits single-pane mode so the mirrored view is visible", () => {
      useFileManagerStore.setState({ singlePaneMode: true });
      useFileManagerStore.getState().setViewMode(0, "grid");
      useFileManagerStore.getState().setSorting(0, "modified", false);
      useFileManagerStore.getState().mirrorViewToOtherPane(0);
      expect(useFileManagerStore.getState().singlePaneMode).toBe(false);
    });

    it("does not mutate the source pane's view configuration", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      useFileManagerStore.getState().setViewMode(0, "grid");
      useFileManagerStore.getState().setSorting(0, "modified", false);
      useFileManagerStore.getState().setGroupBy(0, "type");
      useFileManagerStore.getState().mirrorViewToOtherPane(0);
      const after = useFileManagerStore.getState();
      expect(after.panes[0].viewMode).toBe("grid");
      expect(after.panes[0].sortBy).toBe("modified");
      expect(after.panes[0].sortAsc).toBe(false);
      expect(after.panes[0].groupBy).toBe("type");
    });

    it("is idempotent: a second call after a successful mirror returns null", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      useFileManagerStore.getState().setViewMode(0, "grid");
      useFileManagerStore.getState().setSorting(0, "size", false);
      useFileManagerStore.getState().setGroupBy(0, "type");
      useFileManagerStore.getState().setViewMode(1, "detail");

      const first = useFileManagerStore.getState().mirrorViewToOtherPane(0);
      expect(first).not.toBeNull();
      const second = useFileManagerStore.getState().mirrorViewToOtherPane(0);
      // After the first echo, both panes match, so the second call
      // is a no-op and returns null. This is the contract that
      // prevents the toast from spamming on repeated keypresses.
      expect(second).toBeNull();
    });
  });

  describe("Echo Filter to Other Pane", () => {
    it("copies the active pane's filter text onto the other pane", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      useFileManagerStore.getState().setFilterText(0, "*.pdf");
      const result = useFileManagerStore
        .getState()
        .echoFilterToOtherPane(0);
      expect(result).not.toBeNull();
      expect(result?.text).toBe("*.pdf");
      expect(result?.otherIndex).toBe(1);
      expect(useFileManagerStore.getState().panes[1].filterText).toBe(
        "*.pdf",
      );
    });

    it("works in either direction (pane 1 \u2192 pane 0)", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      useFileManagerStore.getState().setFilterText(1, "draft-");
      const result = useFileManagerStore
        .getState()
        .echoFilterToOtherPane(1);
      expect(result?.otherIndex).toBe(0);
      expect(useFileManagerStore.getState().panes[0].filterText).toBe(
        "draft-",
      );
    });

    it("returns null and does NOT wipe the other pane when source filter is empty", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      useFileManagerStore.getState().setFilterText(0, "");
      useFileManagerStore.getState().setFilterText(1, "keep-this");
      const result = useFileManagerStore
        .getState()
        .echoFilterToOtherPane(0);
      expect(result).toBeNull();
      // Defensive: the other pane's filter MUST survive an
      // empty-source echo. This is the whole point of the
      // null-guard \u2014 silently wiping the other side would be a
      // surprise that the user has no easy way to undo.
      expect(useFileManagerStore.getState().panes[1].filterText).toBe(
        "keep-this",
      );
    });

    it("auto-exits single-pane mode so the echoed filter is visible", () => {
      useFileManagerStore.setState({ singlePaneMode: true });
      useFileManagerStore.getState().setFilterText(0, "*.log");
      useFileManagerStore.getState().echoFilterToOtherPane(0);
      expect(useFileManagerStore.getState().singlePaneMode).toBe(false);
    });

    it("does not mutate the source pane's filter", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      useFileManagerStore.getState().setFilterText(0, "report");
      useFileManagerStore.getState().echoFilterToOtherPane(0);
      expect(useFileManagerStore.getState().panes[0].filterText).toBe(
        "report",
      );
    });

    it("overwrites whatever filter the other pane already had", () => {
      useFileManagerStore.setState({ singlePaneMode: false });
      useFileManagerStore.getState().setFilterText(0, "new-query");
      useFileManagerStore.getState().setFilterText(1, "old-query");
      useFileManagerStore.getState().echoFilterToOtherPane(0);
      expect(useFileManagerStore.getState().panes[1].filterText).toBe(
        "new-query",
      );
    });
  });

  // ── T-010: Undo support ──

  describe("T-010: Undo Stack", () => {
    it("should push and pop undo entries", () => {
      const { pushUndo, popUndo } = useFileManagerStore.getState();

      pushUndo({
        id: "u1",
        type: "copy",
        sourcePaths: ["/a"],
        destPaths: ["/b"],
        timestamp: Date.now(),
      });

      expect(useFileManagerStore.getState().undoStack).toHaveLength(1);

      const entry = popUndo();
      expect(entry?.id).toBe("u1");
      expect(useFileManagerStore.getState().undoStack).toHaveLength(0);
    });

    it("should limit undo stack to 10 levels", () => {
      const { pushUndo } = useFileManagerStore.getState();
      for (let i = 0; i < 15; i++) {
        pushUndo({
          id: `u-${i}`,
          type: "copy",
          sourcePaths: [`/a-${i}`],
          destPaths: [`/b-${i}`],
          timestamp: Date.now(),
        });
      }
      expect(useFileManagerStore.getState().undoStack).toHaveLength(10);
      // Most recent should be first
      expect(useFileManagerStore.getState().undoStack[0].id).toBe("u-14");
    });

    it("should return undefined when popping empty stack", () => {
      const { popUndo } = useFileManagerStore.getState();
      expect(popUndo()).toBeUndefined();
    });
  });

  // ── T-011: Multi-Select ──

  describe("T-011: Multi-Selection", () => {
    it("should set selection for a pane", () => {
      const { setSelection } = useFileManagerStore.getState();
      setSelection("pane-0", ["/a", "/b"]);
      expect(useFileManagerStore.getState().selectedPaths["pane-0"]).toEqual(["/a", "/b"]);
    });

    it("should add to selection", () => {
      const { setSelection, addToSelection } = useFileManagerStore.getState();
      setSelection("pane-0", ["/a"]);
      addToSelection("pane-0", "/b");
      expect(useFileManagerStore.getState().selectedPaths["pane-0"]).toEqual(["/a", "/b"]);
    });

    it("should not add duplicates", () => {
      const { setSelection, addToSelection } = useFileManagerStore.getState();
      setSelection("pane-0", ["/a"]);
      addToSelection("pane-0", "/a");
      expect(useFileManagerStore.getState().selectedPaths["pane-0"]).toEqual(["/a"]);
    });

    it("should remove from selection", () => {
      const { setSelection, removeFromSelection } = useFileManagerStore.getState();
      setSelection("pane-0", ["/a", "/b", "/c"]);
      removeFromSelection("pane-0", "/b");
      expect(useFileManagerStore.getState().selectedPaths["pane-0"]).toEqual(["/a", "/c"]);
    });

    it("should clear selection", () => {
      const { setSelection, clearSelection } = useFileManagerStore.getState();
      setSelection("pane-0", ["/a", "/b"]);
      clearSelection("pane-0");
      expect(useFileManagerStore.getState().selectedPaths["pane-0"]).toEqual([]);
    });

    it("should select range", () => {
      const { selectRange } = useFileManagerStore.getState();
      selectRange("pane-0", ["/a", "/b", "/c", "/d"]);
      expect(useFileManagerStore.getState().selectedPaths["pane-0"]).toEqual([
        "/a", "/b", "/c", "/d",
      ]);
    });
  });

  // ── Pane stats — published by FilePane for the global status bar. ──

  describe("setPaneStats", () => {
    const baseStats = {
      totalCount: 5,
      totalBytes: 1024,
      totalHasDir: false,
      selectedCount: 0,
      selectedBytes: 0,
      selectedHasDir: false,
    };

    it("starts with empty paneStats map", () => {
      expect(
        useFileManagerStore.getState().paneStats["pane-0"] ?? null,
      ).toBeNull();
    });

    it("publishes stats under the pane id", () => {
      useFileManagerStore.getState().setPaneStats("pane-0", baseStats);
      expect(useFileManagerStore.getState().paneStats["pane-0"]).toEqual(
        baseStats,
      );
    });

    it("equality-shortcircuits when identical stats are re-published", () => {
      useFileManagerStore.getState().setPaneStats("pane-0", baseStats);
      const ref1 = useFileManagerStore.getState().paneStats;
      useFileManagerStore.getState().setPaneStats("pane-0", { ...baseStats });
      const ref2 = useFileManagerStore.getState().paneStats;
      // The whole `paneStats` slice object should be the SAME reference —
      // the short-circuit returned `state` unchanged, which Zustand
      // treats as a no-op so subscribers don't re-render.
      expect(ref2).toBe(ref1);
    });

    it("updates when any one field differs", () => {
      useFileManagerStore.getState().setPaneStats("pane-0", baseStats);
      useFileManagerStore.getState().setPaneStats("pane-0", {
        ...baseStats,
        selectedCount: 2,
      });
      expect(
        useFileManagerStore.getState().paneStats["pane-0"]?.selectedCount,
      ).toBe(2);
    });

    it("publishes per pane id without cross-contamination", () => {
      useFileManagerStore.getState().setPaneStats("pane-0", baseStats);
      useFileManagerStore.getState().setPaneStats("pane-1", {
        ...baseStats,
        totalCount: 99,
      });
      expect(
        useFileManagerStore.getState().paneStats["pane-0"]?.totalCount,
      ).toBe(5);
      expect(
        useFileManagerStore.getState().paneStats["pane-1"]?.totalCount,
      ).toBe(99);
    });
  });

  // ── T-012: Saved Searches ──

  describe("T-012: Saved Searches", () => {
    it("should add and remove saved searches", () => {
      const { addSavedSearch, removeSavedSearch } = useFileManagerStore.getState();

      addSavedSearch({
        id: "s1",
        name: "TypeScript files",
        query: "*.ts",
        path: "/",
        recursive: true,
      });
      expect(useFileManagerStore.getState().savedSearches).toHaveLength(1);

      removeSavedSearch("s1");
      expect(useFileManagerStore.getState().savedSearches).toHaveLength(0);
    });
  });

  // ── T-013: Command Palette ──

  describe("T-013: Command Palette", () => {
    it("should toggle command palette", () => {
      const { toggleCommandPalette } = useFileManagerStore.getState();
      expect(useFileManagerStore.getState().commandPaletteOpen).toBe(false);
      toggleCommandPalette();
      expect(useFileManagerStore.getState().commandPaletteOpen).toBe(true);
      toggleCommandPalette();
      expect(useFileManagerStore.getState().commandPaletteOpen).toBe(false);
    });

    it("should set command palette open state", () => {
      const { setCommandPaletteOpen } = useFileManagerStore.getState();
      setCommandPaletteOpen(true);
      expect(useFileManagerStore.getState().commandPaletteOpen).toBe(true);
      setCommandPaletteOpen(false);
      expect(useFileManagerStore.getState().commandPaletteOpen).toBe(false);
    });
  });

  // ── View mode ──

  describe("View mode management", () => {
    it("should set view mode per pane", () => {
      const { setViewMode } = useFileManagerStore.getState();
      setViewMode(0, "grid");
      expect(useFileManagerStore.getState().panes[0].viewMode).toBe("grid");
      expect(useFileManagerStore.getState().panes[1].viewMode).toBe("detail"); // Other pane unchanged
    });

    it("should set sorting per pane", () => {
      const { setSorting } = useFileManagerStore.getState();
      setSorting(0, "size", false);
      const pane = useFileManagerStore.getState().panes[0];
      expect(pane.sortBy).toBe("size");
      expect(pane.sortAsc).toBe(false);
    });

    it("should set filter text per pane", () => {
      const { setFilterText } = useFileManagerStore.getState();
      setFilterText(0, "hello");
      expect(useFileManagerStore.getState().panes[0].filterText).toBe("hello");
      expect(useFileManagerStore.getState().panes[1].filterText).toBe(""); // Other pane unchanged
    });
  });

  // ── Hidden files ──

  describe("Hidden files toggle", () => {
    it("should default to hidden files off", () => {
      expect(useFileManagerStore.getState().showHiddenFiles).toBe(false);
    });

    it("should toggle hidden files", () => {
      const { toggleHiddenFiles } = useFileManagerStore.getState();
      toggleHiddenFiles();
      expect(useFileManagerStore.getState().showHiddenFiles).toBe(true);
      toggleHiddenFiles();
      expect(useFileManagerStore.getState().showHiddenFiles).toBe(false);
    });
  });

  // ── Pane orientation & sync browsing ──

  describe("Pane orientation and sync browsing", () => {
    it("should toggle pane orientation", () => {
      useFileManagerStore.setState({ paneOrientation: "horizontal" });
      const { togglePaneOrientation } = useFileManagerStore.getState();
      togglePaneOrientation();
      expect(useFileManagerStore.getState().paneOrientation).toBe("vertical");
      togglePaneOrientation();
      expect(useFileManagerStore.getState().paneOrientation).toBe("horizontal");
    });

    it("should toggle sync browsing", () => {
      useFileManagerStore.setState({ syncBrowsing: false });
      const { toggleSyncBrowsing } = useFileManagerStore.getState();
      toggleSyncBrowsing();
      expect(useFileManagerStore.getState().syncBrowsing).toBe(true);
      toggleSyncBrowsing();
      expect(useFileManagerStore.getState().syncBrowsing).toBe(false);
    });
  });

  // ── Double-click behavior ──

  describe("Double-click behavior", () => {
    it("should default to open", () => {
      expect(useFileManagerStore.getState().doubleClickBehavior).toBe("open");
    });

    it("should set double-click behavior", () => {
      const { setDoubleClickBehavior } = useFileManagerStore.getState();
      setDoubleClickBehavior("edit");
      expect(useFileManagerStore.getState().doubleClickBehavior).toBe("edit");
      setDoubleClickBehavior("preview");
      expect(useFileManagerStore.getState().doubleClickBehavior).toBe("preview");
      setDoubleClickBehavior("transfer");
      expect(useFileManagerStore.getState().doubleClickBehavior).toBe("transfer");
      setDoubleClickBehavior("open");
      expect(useFileManagerStore.getState().doubleClickBehavior).toBe("open");
    });
  });

  // ── Editor mappings ──

  describe("Editor mappings", () => {
    it("should default to empty", () => {
      expect(useFileManagerStore.getState().editorMappings).toEqual([]);
    });

    it("should add an editor mapping", () => {
      const { addEditorMapping } = useFileManagerStore.getState();
      addEditorMapping({
        id: "em-1",
        extension: "py",
        appName: "VS Code",
        appPath: "/usr/bin/code",
        args: ["--wait"],
      });
      expect(useFileManagerStore.getState().editorMappings).toHaveLength(1);
      expect(useFileManagerStore.getState().editorMappings[0].appName).toBe("VS Code");
    });

    it("should update an editor mapping", () => {
      const { addEditorMapping, updateEditorMapping } = useFileManagerStore.getState();
      addEditorMapping({
        id: "em-1",
        extension: "py",
        appName: "VS Code",
        appPath: "/usr/bin/code",
        args: ["--wait"],
      });
      updateEditorMapping({
        id: "em-1",
        extension: "py",
        appName: "PyCharm",
        appPath: "/usr/bin/pycharm",
        args: [],
      });
      expect(useFileManagerStore.getState().editorMappings[0].appName).toBe("PyCharm");
    });

    it("should remove an editor mapping", () => {
      const { addEditorMapping, removeEditorMapping } = useFileManagerStore.getState();
      addEditorMapping({
        id: "em-1",
        extension: "py",
        appName: "VS Code",
        appPath: "/usr/bin/code",
        args: [],
      });
      expect(useFileManagerStore.getState().editorMappings).toHaveLength(1);
      removeEditorMapping("em-1");
      expect(useFileManagerStore.getState().editorMappings).toHaveLength(0);
    });
  });

  // ── Privacy settings ──

  describe("Privacy settings", () => {
    it("should have correct defaults", () => {
      expect(useFileManagerStore.getState().privacySettings).toEqual({
        autoUpdateCheck: true,
        crashReportOptIn: false,
      });
    });

    it("should update partial privacy settings", () => {
      const { setPrivacySettings } = useFileManagerStore.getState();
      setPrivacySettings({ autoUpdateCheck: false });
      expect(useFileManagerStore.getState().privacySettings.autoUpdateCheck).toBe(false);
      expect(useFileManagerStore.getState().privacySettings.crashReportOptIn).toBe(false);
    });

    it("should update crashReportOptIn independently", () => {
      const { setPrivacySettings } = useFileManagerStore.getState();
      setPrivacySettings({ crashReportOptIn: true });
      expect(useFileManagerStore.getState().privacySettings.crashReportOptIn).toBe(true);
      expect(useFileManagerStore.getState().privacySettings.autoUpdateCheck).toBe(true);
    });
  });

  // ── Workspace management ──

  describe("Workspace management", () => {
    it("should start with no workspaces", () => {
      expect(useFileManagerStore.getState().workspaces).toHaveLength(0);
    });

    it("should save a workspace", () => {
      const { saveWorkspace } = useFileManagerStore.getState();
      const ws = saveWorkspace("My Workspace");
      expect(ws.name).toBe("My Workspace");
      expect(ws.id).toBeTruthy();
      expect(useFileManagerStore.getState().workspaces).toHaveLength(1);
    });

    it("should delete a workspace", () => {
      const { saveWorkspace, deleteWorkspace } = useFileManagerStore.getState();
      const ws = saveWorkspace("To Delete");
      expect(useFileManagerStore.getState().workspaces).toHaveLength(1);
      deleteWorkspace(ws.id);
      expect(useFileManagerStore.getState().workspaces).toHaveLength(0);
    });

    it("should list workspaces", () => {
      const { saveWorkspace, listWorkspaces } = useFileManagerStore.getState();
      saveWorkspace("WS 1");
      saveWorkspace("WS 2");
      const list = listWorkspaces();
      expect(list).toHaveLength(2);
    });

    it("should load a workspace and restore state", () => {
      const store = useFileManagerStore.getState();
      store.setPaneSplitPercent(70);
      store.toggleSinglePaneMode(); // now false
      const ws = store.saveWorkspace("Saved State");

      // Change state after saving
      store.setPaneSplitPercent(30);
      store.toggleSinglePaneMode(); // back to true

      store.loadWorkspace(ws.id);
      expect(useFileManagerStore.getState().paneSplitPercent).toBe(70);
      expect(useFileManagerStore.getState().singlePaneMode).toBe(false);
    });
  });

  // ── Per-folder view defaults ──

  describe("Per-folder view defaults", () => {
    it("should save and retrieve folder view defaults", () => {
      const { saveFolderViewDefault, getFolderViewDefault } = useFileManagerStore.getState();
      saveFolderViewDefault("/home", { viewMode: "grid", sortBy: "size", sortAsc: false });
      const defaults = getFolderViewDefault("/home");
      expect(defaults).toEqual({ viewMode: "grid", sortBy: "size", sortAsc: false });
    });

    it("should return undefined for unknown folder", () => {
      const { getFolderViewDefault } = useFileManagerStore.getState();
      expect(getFolderViewDefault("/nonexistent")).toBeUndefined();
    });
  });

  // ── Helper methods ──

  describe("Helper methods", () => {
    it("should get active pane", () => {
      const state = useFileManagerStore.getState();
      const activePane = state.getActivePane();
      expect(activePane.id).toBe(state.panes[0].id);
    });

    it("should get active tab", () => {
      const state = useFileManagerStore.getState();
      const tab = state.getActiveTab(0);
      expect(tab?.id).toBe(state.panes[0].activeTabId);
    });

    it("should get active path", () => {
      const state = useFileManagerStore.getState();
      expect(state.getActivePath(0)).toBe("/");
    });
  });
});
