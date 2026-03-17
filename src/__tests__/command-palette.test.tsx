import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { CommandPalette, getDefaultCommands, type CommandItem } from "@/components/command-palette";

describe("T-013: CommandPalette", () => {
  const mockCommands: CommandItem[] = [
    {
      id: "new-folder",
      label: "New Folder",
      category: "File",
      action: vi.fn(),
      keywords: ["create", "directory"],
    },
    {
      id: "new-file",
      label: "New File",
      category: "File",
      action: vi.fn(),
    },
    {
      id: "copy",
      label: "Copy",
      shortcut: "Cmd+C",
      category: "Edit",
      action: vi.fn(),
    },
    {
      id: "paste",
      label: "Paste",
      shortcut: "Cmd+V",
      category: "Edit",
      action: vi.fn(),
    },
    {
      id: "toggle-dark",
      label: "Dark Theme",
      category: "Appearance",
      action: vi.fn(),
      keywords: ["theme", "dark"],
    },
  ];

  it("should not render when closed", () => {
    render(
      <CommandPalette commands={mockCommands} isOpen={false} onClose={vi.fn()} />,
    );
    expect(screen.queryByTestId("command-palette")).not.toBeInTheDocument();
  });

  it("should render when open", () => {
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={vi.fn()} />,
    );
    expect(screen.getByTestId("command-palette")).toBeInTheDocument();
    expect(screen.getByTestId("command-palette-input")).toBeInTheDocument();
  });

  it("should show all commands when no query", () => {
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={vi.fn()} />,
    );
    expect(screen.getByTestId("command-new-folder")).toBeInTheDocument();
    expect(screen.getByTestId("command-copy")).toBeInTheDocument();
  });

  it("should filter commands by query", () => {
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={vi.fn()} />,
    );
    const input = screen.getByTestId("command-palette-input");
    fireEvent.change(input, { target: { value: "folder" } });

    expect(screen.getByTestId("command-new-folder")).toBeInTheDocument();
    expect(screen.queryByTestId("command-copy")).not.toBeInTheDocument();
  });

  it("should search by keywords", () => {
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={vi.fn()} />,
    );
    const input = screen.getByTestId("command-palette-input");
    fireEvent.change(input, { target: { value: "directory" } });

    expect(screen.getByTestId("command-new-folder")).toBeInTheDocument();
  });

  it("should execute command on click", () => {
    const onClose = vi.fn();
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={onClose} />,
    );
    fireEvent.click(screen.getByTestId("command-copy"));
    expect(mockCommands[2].action).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("should close on Escape key", () => {
    const onClose = vi.fn();
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={onClose} />,
    );
    const input = screen.getByTestId("command-palette-input");
    fireEvent.keyDown(input, { key: "Escape" });
    expect(onClose).toHaveBeenCalled();
  });

  it("should close on backdrop click", () => {
    const onClose = vi.fn();
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={onClose} />,
    );
    fireEvent.click(screen.getByTestId("command-palette-backdrop"));
    expect(onClose).toHaveBeenCalled();
  });

  it("should execute command on Enter key", () => {
    const onClose = vi.fn();
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={onClose} />,
    );
    const input = screen.getByTestId("command-palette-input");
    // First command should be selected
    fireEvent.keyDown(input, { key: "Enter" });
    expect(mockCommands[0].action).toHaveBeenCalled();
    expect(onClose).toHaveBeenCalled();
  });

  it("should navigate with arrow keys", () => {
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={vi.fn()} />,
    );
    const input = screen.getByTestId("command-palette-input");

    // Move down
    fireEvent.keyDown(input, { key: "ArrowDown" });
    // Move down again
    fireEvent.keyDown(input, { key: "ArrowDown" });
    // Move up
    fireEvent.keyDown(input, { key: "ArrowUp" });

    // Just verify it doesn't crash
    expect(screen.getByTestId("command-palette")).toBeInTheDocument();
  });

  it("should show 'no results' for unmatched query", () => {
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={vi.fn()} />,
    );
    const input = screen.getByTestId("command-palette-input");
    fireEvent.change(input, { target: { value: "zzzznonexistent" } });

    expect(screen.getByText(/no matching commands/i)).toBeInTheDocument();
  });

  it("should show command shortcuts", () => {
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={vi.fn()} />,
    );
    expect(screen.getByText("Cmd+C")).toBeInTheDocument();
    expect(screen.getByText("Cmd+V")).toBeInTheDocument();
  });

  it("should have accessible dialog role", () => {
    render(
      <CommandPalette commands={mockCommands} isOpen={true} onClose={vi.fn()} />,
    );
    expect(screen.getByRole("dialog", { name: /command palette/i })).toBeInTheDocument();
  });
});

describe("T-013: getDefaultCommands", () => {
  it("should return a list of commands", () => {
    const commands = getDefaultCommands({});
    expect(commands.length).toBeGreaterThan(10);
  });

  it("should include file, edit, view, and appearance categories", () => {
    const commands = getDefaultCommands({});
    const categories = new Set(commands.map((c) => c.category));
    expect(categories.has("File")).toBe(true);
    expect(categories.has("Edit")).toBe(true);
    expect(categories.has("View")).toBe(true);
    expect(categories.has("Appearance")).toBe(true);
  });

  it("should have unique IDs", () => {
    const commands = getDefaultCommands({});
    const ids = commands.map((c) => c.id);
    expect(new Set(ids).size).toBe(ids.length);
  });

  it("should call provided action handlers", () => {
    const onNewFolder = vi.fn();
    const commands = getDefaultCommands({ onNewFolder });
    const cmd = commands.find((c) => c.id === "new-folder");
    cmd?.action();
    expect(onNewFolder).toHaveBeenCalled();
  });
});
