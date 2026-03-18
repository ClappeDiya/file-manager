import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { ContextMenu, getFileContextMenuItems, type ContextMenuItem } from "@/components/context-menu";

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
});
