import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import {
  ContextMenu,
  getFileContextMenuItems,
  getTabContextMenuItems,
  type ContextMenuItem,
} from "@/components/context-menu";

describe("T-013: ContextMenu", () => {
  const mockItems: ContextMenuItem[] = [
    { id: "open", label: "Open", action: vi.fn() },
    { id: "sep1", label: "", separator: true },
    { id: "copy", label: "Copy", shortcut: "Cmd+C", action: vi.fn() },
    { id: "delete", label: "Delete", danger: true, action: vi.fn() },
    { id: "disabled", label: "Disabled", disabled: true, action: vi.fn() },
  ];

  it("should not render when position is null", () => {
    render(
      <ContextMenu items={mockItems} position={null} onClose={vi.fn()} />,
    );
    expect(screen.queryByTestId("context-menu")).not.toBeInTheDocument();
  });

  it("should render at specified position", () => {
    render(
      <ContextMenu items={mockItems} position={{ x: 100, y: 200 }} onClose={vi.fn()} />,
    );
    const menu = screen.getByTestId("context-menu");
    expect(menu).toBeInTheDocument();
    expect(menu.style.left).toBe("100px");
    expect(menu.style.top).toBe("200px");
  });

  it("should render menu items", () => {
    render(
      <ContextMenu items={mockItems} position={{ x: 0, y: 0 }} onClose={vi.fn()} />,
    );
    expect(screen.getByTestId("context-menu-item-open")).toBeInTheDocument();
    expect(screen.getByTestId("context-menu-item-copy")).toBeInTheDocument();
    expect(screen.getByTestId("context-menu-item-delete")).toBeInTheDocument();
  });

  it("should render separators", () => {
    render(
      <ContextMenu items={mockItems} position={{ x: 0, y: 0 }} onClose={vi.fn()} />,
    );
    const separators = screen.getAllByRole("separator");
    expect(separators.length).toBeGreaterThan(0);
  });

  it("should execute action on click", () => {
    const onClose = vi.fn();
    render(
      <ContextMenu items={mockItems} position={{ x: 0, y: 0 }} onClose={onClose} />,
    );
    fireEvent.click(screen.getByTestId("context-menu-item-copy"));
    expect(mockItems[2].action).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("should not execute disabled items", () => {
    render(
      <ContextMenu items={mockItems} position={{ x: 0, y: 0 }} onClose={vi.fn()} />,
    );
    const disabledItem = screen.getByTestId("context-menu-item-disabled");
    expect(disabledItem).toBeDisabled();
  });

  it("should show shortcut text", () => {
    render(
      <ContextMenu items={mockItems} position={{ x: 0, y: 0 }} onClose={vi.fn()} />,
    );
    expect(screen.getByText("Cmd+C")).toBeInTheDocument();
  });

  it("should have accessible menu role", () => {
    render(
      <ContextMenu items={mockItems} position={{ x: 0, y: 0 }} onClose={vi.fn()} />,
    );
    expect(screen.getByRole("menu")).toBeInTheDocument();
  });

  it("should close on Escape key", () => {
    const onClose = vi.fn();
    render(
      <ContextMenu items={mockItems} position={{ x: 0, y: 0 }} onClose={onClose} />,
    );
    fireEvent.keyDown(document, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });
});

describe("T-013: getFileContextMenuItems", () => {
  it("should return context menu items", () => {
    const items = getFileContextMenuItems({
      hasSelection: true,
      isDirectory: false,
      selectionCount: 1,
      onCopy: vi.fn(),
      onCut: vi.fn(),
      onPaste: vi.fn(),
      onDelete: vi.fn(),
      onRename: vi.fn(),
      onDuplicate: vi.fn(),
      onNewFolder: vi.fn(),
      onNewFile: vi.fn(),
      onAddToFavorites: vi.fn(),
      onGetInfo: vi.fn(),
      onOpen: vi.fn(),
      onRefresh: vi.fn(),
      onSelectAll: vi.fn(),
      onInvertSelection: vi.fn(),
    });

    expect(items.length).toBeGreaterThan(5);
    expect(items.find((i) => i.id === "copy")).toBeTruthy();
    expect(items.find((i) => i.id === "delete")).toBeTruthy();
    expect(items.find((i) => i.id === "rename")).toBeTruthy();
  });

  it("should disable rename for multi-selection", () => {
    const items = getFileContextMenuItems({
      hasSelection: true,
      isDirectory: false,
      selectionCount: 3,
      onCopy: vi.fn(),
      onCut: vi.fn(),
      onPaste: vi.fn(),
      onDelete: vi.fn(),
      onRename: vi.fn(),
      onDuplicate: vi.fn(),
      onNewFolder: vi.fn(),
      onNewFile: vi.fn(),
      onAddToFavorites: vi.fn(),
      onGetInfo: vi.fn(),
      onOpen: vi.fn(),
      onRefresh: vi.fn(),
      onSelectAll: vi.fn(),
      onInvertSelection: vi.fn(),
    });

    const rename = items.find((i) => i.id === "rename");
    expect(rename?.disabled).toBe(true);
  });

  it("should include only paste when no selection", () => {
    const items = getFileContextMenuItems({
      hasSelection: false,
      isDirectory: false,
      selectionCount: 0,
      onCopy: vi.fn(),
      onCut: vi.fn(),
      onPaste: vi.fn(),
      onDelete: vi.fn(),
      onRename: vi.fn(),
      onDuplicate: vi.fn(),
      onNewFolder: vi.fn(),
      onNewFile: vi.fn(),
      onAddToFavorites: vi.fn(),
      onGetInfo: vi.fn(),
      onOpen: vi.fn(),
      onRefresh: vi.fn(),
      onSelectAll: vi.fn(),
      onInvertSelection: vi.fn(),
    });

    expect(items.find((i) => i.id === "paste")).toBeTruthy();
    expect(items.find((i) => i.id === "copy")).toBeFalsy();
  });

  it("should mark advanced-only items", () => {
    const items = getFileContextMenuItems({
      hasSelection: true,
      isDirectory: false,
      selectionCount: 1,
      onCopy: vi.fn(),
      onCut: vi.fn(),
      onPaste: vi.fn(),
      onDelete: vi.fn(),
      onRename: vi.fn(),
      onDuplicate: vi.fn(),
      onNewFolder: vi.fn(),
      onNewFile: vi.fn(),
      onAddToFavorites: vi.fn(),
      onGetInfo: vi.fn(),
      onOpen: vi.fn(),
      onRefresh: vi.fn(),
      onSelectAll: vi.fn(),
      onInvertSelection: vi.fn(),
    });

    const getInfo = items.find((i) => i.id === "get-info");
    expect(getInfo?.advancedOnly).toBe(true);
  });

  // Reveal-in-OS — staple "Show in Finder / Show in Explorer"
  // power-user action. The label adapts to the current platform
  // so the menu text matches what the user is about to see.
  describe("reveal-in-os entry", () => {
    function buildItemsWithReveal(onRevealInOs = vi.fn()) {
      return getFileContextMenuItems({
        hasSelection: true,
        isDirectory: false,
        selectionCount: 1,
        onCopy: vi.fn(),
        onCut: vi.fn(),
        onPaste: vi.fn(),
        onDelete: vi.fn(),
        onRename: vi.fn(),
        onDuplicate: vi.fn(),
        onNewFolder: vi.fn(),
        onNewFile: vi.fn(),
        onAddToFavorites: vi.fn(),
        onGetInfo: vi.fn(),
        onOpen: vi.fn(),
        onRefresh: vi.fn(),
        onSelectAll: vi.fn(),
        onInvertSelection: vi.fn(),
        onRevealInOs,
      });
    }

    it("renders no entry when onRevealInOs is omitted", () => {
      const items = getFileContextMenuItems({
        hasSelection: true,
        isDirectory: false,
        selectionCount: 1,
        onCopy: vi.fn(),
        onCut: vi.fn(),
        onPaste: vi.fn(),
        onDelete: vi.fn(),
        onRename: vi.fn(),
        onDuplicate: vi.fn(),
        onNewFolder: vi.fn(),
        onNewFile: vi.fn(),
        onAddToFavorites: vi.fn(),
        onGetInfo: vi.fn(),
        onOpen: vi.fn(),
        onRefresh: vi.fn(),
        onSelectAll: vi.fn(),
        onInvertSelection: vi.fn(),
      });
      expect(items.find((i) => i.id === "reveal-in-os")).toBeUndefined();
    });

    it("renders the entry when onRevealInOs is provided", () => {
      const items = buildItemsWithReveal();
      const entry = items.find((i) => i.id === "reveal-in-os");
      expect(entry).toBeTruthy();
      // Label is one of three platform-aware strings, never
      // generic — every supported platform gets a meaningful name.
      expect(entry?.label).toMatch(
        /^Show in (Finder|Explorer|File Manager)$/,
      );
    });

    it("places the entry immediately after Open in the items array", () => {
      const items = buildItemsWithReveal();
      const openIdx = items.findIndex((i) => i.id === "open");
      const revealIdx = items.findIndex((i) => i.id === "reveal-in-os");
      expect(openIdx).toBeGreaterThanOrEqual(0);
      expect(revealIdx).toBe(openIdx + 1);
    });

    it("invokes the provided handler when the entry's action fires", () => {
      const onRevealInOs = vi.fn();
      const items = buildItemsWithReveal(onRevealInOs);
      const entry = items.find((i) => i.id === "reveal-in-os");
      entry?.action?.();
      expect(onRevealInOs).toHaveBeenCalledTimes(1);
    });
  });

  // Compress — surfaces the long-orphaned CreateArchiveDialog
  // (full backend via archive_create IPC, full UI in
  // archive-browser.tsx, just no entry point until now). Tests
  // lock the placement and selection-count label.
  describe("compress entry", () => {
    function buildItemsWithCompress(
      onCompress = vi.fn(),
      selectionCount = 1,
    ) {
      return getFileContextMenuItems({
        hasSelection: true,
        isDirectory: false,
        selectionCount,
        onCopy: vi.fn(),
        onCut: vi.fn(),
        onPaste: vi.fn(),
        onDelete: vi.fn(),
        onRename: vi.fn(),
        onDuplicate: vi.fn(),
        onNewFolder: vi.fn(),
        onNewFile: vi.fn(),
        onAddToFavorites: vi.fn(),
        onGetInfo: vi.fn(),
        onOpen: vi.fn(),
        onRefresh: vi.fn(),
        onSelectAll: vi.fn(),
        onInvertSelection: vi.fn(),
        onCompress,
      });
    }

    it("renders no entry when onCompress is omitted", () => {
      const items = getFileContextMenuItems({
        hasSelection: true,
        isDirectory: false,
        selectionCount: 1,
        onCopy: vi.fn(),
        onCut: vi.fn(),
        onPaste: vi.fn(),
        onDelete: vi.fn(),
        onRename: vi.fn(),
        onDuplicate: vi.fn(),
        onNewFolder: vi.fn(),
        onNewFile: vi.fn(),
        onAddToFavorites: vi.fn(),
        onGetInfo: vi.fn(),
        onOpen: vi.fn(),
        onRefresh: vi.fn(),
        onSelectAll: vi.fn(),
        onInvertSelection: vi.fn(),
      });
      expect(items.find((i) => i.id === "compress")).toBeUndefined();
    });

    it("renders the entry when onCompress is provided", () => {
      const items = buildItemsWithCompress();
      const entry = items.find((i) => i.id === "compress");
      expect(entry).toBeTruthy();
    });

    it("labels with the singular form when one item is selected", () => {
      const items = buildItemsWithCompress(vi.fn(), 1);
      const entry = items.find((i) => i.id === "compress");
      expect(entry?.label).toBe("Compress…");
    });

    it("labels with the plural item count when multiple are selected", () => {
      const items = buildItemsWithCompress(vi.fn(), 4);
      const entry = items.find((i) => i.id === "compress");
      expect(entry?.label).toBe("Compress 4 Items…");
    });

    it("places the entry immediately after Duplicate", () => {
      const items = buildItemsWithCompress();
      const dupIdx = items.findIndex((i) => i.id === "duplicate");
      const compressIdx = items.findIndex((i) => i.id === "compress");
      expect(dupIdx).toBeGreaterThanOrEqual(0);
      expect(compressIdx).toBe(dupIdx + 1);
    });

    it("invokes the provided handler when the entry's action fires", () => {
      const onCompress = vi.fn();
      const items = buildItemsWithCompress(onCompress);
      const entry = items.find((i) => i.id === "compress");
      entry?.action?.();
      expect(onCompress).toHaveBeenCalledTimes(1);
    });
  });

  // Extract Here — mirror of Compress. Only wired by FilePane when
  // the right-clicked entry is an archive file (the gating is the
  // caller's job, matching onEditRemote / onCompareFiles). Tests
  // lock the label, placement (right after reveal-in-os when
  // present), and handler wiring.
  describe("extract-here entry", () => {
    function buildItemsWithExtract(onExtract = vi.fn()) {
      return getFileContextMenuItems({
        hasSelection: true,
        isDirectory: false,
        selectionCount: 1,
        onCopy: vi.fn(),
        onCut: vi.fn(),
        onPaste: vi.fn(),
        onDelete: vi.fn(),
        onRename: vi.fn(),
        onDuplicate: vi.fn(),
        onNewFolder: vi.fn(),
        onNewFile: vi.fn(),
        onAddToFavorites: vi.fn(),
        onGetInfo: vi.fn(),
        onOpen: vi.fn(),
        onRefresh: vi.fn(),
        onSelectAll: vi.fn(),
        onInvertSelection: vi.fn(),
        onRevealInOs: vi.fn(),
        onExtract,
      });
    }

    it("renders no entry when onExtract is omitted", () => {
      const items = getFileContextMenuItems({
        hasSelection: true,
        isDirectory: false,
        selectionCount: 1,
        onCopy: vi.fn(),
        onCut: vi.fn(),
        onPaste: vi.fn(),
        onDelete: vi.fn(),
        onRename: vi.fn(),
        onDuplicate: vi.fn(),
        onNewFolder: vi.fn(),
        onNewFile: vi.fn(),
        onAddToFavorites: vi.fn(),
        onGetInfo: vi.fn(),
        onOpen: vi.fn(),
        onRefresh: vi.fn(),
        onSelectAll: vi.fn(),
        onInvertSelection: vi.fn(),
      });
      expect(items.find((i) => i.id === "extract-here")).toBeUndefined();
    });

    it("renders the entry with label 'Extract Here' when onExtract is provided", () => {
      const items = buildItemsWithExtract();
      const entry = items.find((i) => i.id === "extract-here");
      expect(entry).toBeTruthy();
      expect(entry?.label).toBe("Extract Here");
    });

    it("places the entry immediately after Show in Finder/Explorer when both are present", () => {
      const items = buildItemsWithExtract();
      const revealIdx = items.findIndex((i) => i.id === "reveal-in-os");
      const extractIdx = items.findIndex((i) => i.id === "extract-here");
      expect(revealIdx).toBeGreaterThanOrEqual(0);
      expect(extractIdx).toBe(revealIdx + 1);
    });

    it("invokes the provided handler when the entry's action fires", () => {
      const onExtract = vi.fn();
      const items = buildItemsWithExtract(onExtract);
      const entry = items.find((i) => i.id === "extract-here");
      entry?.action?.();
      expect(onExtract).toHaveBeenCalledTimes(1);
    });
  });
});

// ──────────────────────────────────────────────
// Iter 20: getTabContextMenuItems — browser-parity tab right-click
// menu. Pure data transform; verified by inspecting the returned
// item list shape, ordering, and disabled / hidden gates.
// ──────────────────────────────────────────────
describe("Iter 20: getTabContextMenuItems", () => {
  function build(opts: Partial<Parameters<typeof getTabContextMenuItems>[0]> = {}) {
    return getTabContextMenuItems({
      isPinned: false,
      totalTabs: 3,
      closableOthers: 2,
      closableToRight: 1,
      onClose: vi.fn(),
      onCloseOthers: vi.fn(),
      onCloseToRight: vi.fn(),
      onPinToggle: vi.fn(),
      onDuplicate: vi.fn(),
      onMoveToOtherPane: vi.fn(),
      ...opts,
    });
  }

  it("returns the full browser-parity menu when all conditions allow", () => {
    const items = build();
    const ids = items.filter((i) => !i.separator).map((i) => i.id);
    expect(ids).toEqual([
      "close-tab",
      "close-other-tabs",
      "close-tabs-to-right",
      "pin-toggle",
      "duplicate-tab",
      "move-to-other-pane",
    ]);
  });

  it("disables Close Tab when the tab is pinned", () => {
    const items = build({ isPinned: true });
    const closeEntry = items.find((i) => i.id === "close-tab");
    expect(closeEntry?.disabled).toBe(true);
  });

  it("disables Close Tab when it's the last tab in the pane", () => {
    const items = build({ totalTabs: 1, closableOthers: 0, closableToRight: 0 });
    const closeEntry = items.find((i) => i.id === "close-tab");
    expect(closeEntry?.disabled).toBe(true);
  });

  it("disables Close Other Tabs when no other tabs are closable", () => {
    const items = build({ closableOthers: 0 });
    const entry = items.find((i) => i.id === "close-other-tabs");
    expect(entry?.disabled).toBe(true);
  });

  it("disables Close Tabs to the Right when nothing sits to the right", () => {
    const items = build({ closableToRight: 0 });
    const entry = items.find((i) => i.id === "close-tabs-to-right");
    expect(entry?.disabled).toBe(true);
  });

  it("switches the pin entry label to 'Unpin Tab' when the tab is pinned", () => {
    const pinned = build({ isPinned: true });
    expect(pinned.find((i) => i.id === "pin-toggle")?.label).toBe("Unpin Tab");
    const unpinned = build({ isPinned: false });
    expect(unpinned.find((i) => i.id === "pin-toggle")?.label).toBe("Pin Tab");
  });

  it("omits Move to Other Pane when the source pane has only one tab", () => {
    const items = build({ totalTabs: 1, closableOthers: 0, closableToRight: 0 });
    expect(items.find((i) => i.id === "move-to-other-pane")).toBeUndefined();
  });

  it("omits Move to Other Pane for pinned tabs", () => {
    const items = build({ isPinned: true });
    expect(items.find((i) => i.id === "move-to-other-pane")).toBeUndefined();
  });

  it("omits Move to Other Pane when the caller doesn't supply onMoveToOtherPane", () => {
    const items = build({ onMoveToOtherPane: undefined });
    expect(items.find((i) => i.id === "move-to-other-pane")).toBeUndefined();
  });

  it("routes the close-tab action to the supplied handler", () => {
    const onClose = vi.fn();
    const items = build({ onClose });
    items.find((i) => i.id === "close-tab")?.action?.();
    expect(onClose).toHaveBeenCalledTimes(1);
  });

  it("routes close-others / close-right / duplicate / move handlers correctly", () => {
    const onCloseOthers = vi.fn();
    const onCloseToRight = vi.fn();
    const onDuplicate = vi.fn();
    const onMoveToOtherPane = vi.fn();
    const items = build({
      onCloseOthers,
      onCloseToRight,
      onDuplicate,
      onMoveToOtherPane,
    });
    items.find((i) => i.id === "close-other-tabs")?.action?.();
    items.find((i) => i.id === "close-tabs-to-right")?.action?.();
    items.find((i) => i.id === "duplicate-tab")?.action?.();
    items.find((i) => i.id === "move-to-other-pane")?.action?.();
    expect(onCloseOthers).toHaveBeenCalledTimes(1);
    expect(onCloseToRight).toHaveBeenCalledTimes(1);
    expect(onDuplicate).toHaveBeenCalledTimes(1);
    expect(onMoveToOtherPane).toHaveBeenCalledTimes(1);
  });
});
