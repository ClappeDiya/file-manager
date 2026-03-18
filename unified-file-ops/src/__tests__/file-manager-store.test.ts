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
