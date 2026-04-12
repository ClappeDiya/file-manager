/**
 * Iter-17 tests: KeyboardCheatSheet grouping helper + render smoke
 * test. Exercises the pure `groupShortcutCommandsByCategory`
 * function so the grouping/filter logic is covered independently
 * of React mounting, plus a minimal render assertion that guards
 * the "no shortcut rows when there are no shortcut commands" path.
 */
import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import { KeyboardCheatSheet } from "../components/keyboard-cheat-sheet";
import { groupShortcutCommandsByCategory } from "../lib/keyboard-shortcut-groups";
import type { CommandItem } from "../components/command-palette";

function cmd(
  id: string,
  category: string,
  shortcut: string | undefined,
  label = id,
): CommandItem {
  return {
    id,
    label,
    category,
    shortcut,
    action: () => {},
  };
}

/** Iter 48 helper: build a CommandItem with secondary shortcuts. */
function cmdWithExtra(
  id: string,
  category: string,
  shortcut: string,
  extras: Array<{ keys: string; hint: string }>,
  label = id,
): CommandItem {
  return {
    id,
    label,
    category,
    shortcut,
    additionalShortcuts: extras,
    action: () => {},
  };
}

describe("groupShortcutCommandsByCategory", () => {
  it("drops commands without a `shortcut` field", () => {
    const groups = groupShortcutCommandsByCategory([
      cmd("no-shortcut", "Navigation", undefined),
      cmd("with-shortcut", "Navigation", "\u2318\u21E7E"),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].items).toHaveLength(1);
    expect(groups[0].items[0].id).toBe("with-shortcut");
  });

  it("preserves category order by first-seen insertion", () => {
    const groups = groupShortcutCommandsByCategory([
      cmd("a", "Navigation", "\u2318A"),
      cmd("b", "File Operations", "\u2318B"),
      cmd("c", "Navigation", "\u2318C"),
      cmd("d", "View", "\u2318D"),
    ]);
    expect(groups.map((g) => g.category)).toEqual([
      "Navigation",
      "File Operations",
      "View",
    ]);
  });

  it("preserves command order within each category", () => {
    const groups = groupShortcutCommandsByCategory([
      cmd("first", "Navigation", "\u23181"),
      cmd("second", "Navigation", "\u23182"),
      cmd("third", "Navigation", "\u23183"),
    ]);
    expect(groups[0].items.map((i) => i.id)).toEqual([
      "first",
      "second",
      "third",
    ]);
  });

  it("returns an empty array when no commands have shortcuts", () => {
    const groups = groupShortcutCommandsByCategory([
      cmd("x", "Navigation", undefined),
      cmd("y", "View", undefined),
    ]);
    expect(groups).toEqual([]);
  });

  it("returns an empty array for an empty input", () => {
    expect(groupShortcutCommandsByCategory([])).toEqual([]);
  });

  it("multiple commands in the same category share one bucket", () => {
    const groups = groupShortcutCommandsByCategory([
      cmd("a", "Navigation", "\u2318A"),
      cmd("b", "Navigation", "\u2318B"),
      cmd("c", "Navigation", "\u2318C"),
    ]);
    expect(groups).toHaveLength(1);
    expect(groups[0].items).toHaveLength(3);
  });
});

describe("KeyboardCheatSheet rendering", () => {
  it("renders nothing when `open` is false", () => {
    const { container } = render(
      <KeyboardCheatSheet
        commands={[cmd("a", "Navigation", "\u2318A")]}
        open={false}
        onClose={() => {}}
      />,
    );
    expect(container.firstChild).toBeNull();
  });

  it("renders the empty-state message when no commands have shortcuts", () => {
    render(
      <KeyboardCheatSheet
        commands={[cmd("no-shortcut", "Navigation", undefined)]}
        open={true}
        onClose={() => {}}
      />,
    );
    expect(
      screen.getByText(/no keyboard shortcuts are registered/i),
    ).toBeInTheDocument();
  });

  it("renders a shortcut badge for each shortcut command", () => {
    render(
      <KeyboardCheatSheet
        commands={[
          cmd("open", "Navigation", "\u2318\u21E7E", "Open in Other Pane"),
          cmd("swap", "Navigation", "\u2318\u21E7X", "Swap Panes"),
        ]}
        open={true}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("Open in Other Pane")).toBeInTheDocument();
    expect(screen.getByText("Swap Panes")).toBeInTheDocument();
    // The kbd elements render the shortcut strings literally.
    expect(screen.getByText("\u2318\u21E7E")).toBeInTheDocument();
    expect(screen.getByText("\u2318\u21E7X")).toBeInTheDocument();
  });

  it("renders category headers for each distinct category", () => {
    render(
      <KeyboardCheatSheet
        commands={[
          cmd("a", "Navigation", "\u2318A"),
          cmd("b", "File Operations", "\u2318B"),
        ]}
        open={true}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("Navigation")).toBeInTheDocument();
    expect(screen.getByText("File Operations")).toBeInTheDocument();
  });
});

// ──────────────────────────────────────────────────────────────
// Iter 48 — Additional shortcut rendering
// Jump Ring reverse (⌘⇧⌥L) and any future multi-gesture features
// are surfaced via `additionalShortcuts` on the primary palette
// entry, rendered in the cheat sheet below the description.
// ──────────────────────────────────────────────────────────────

describe("KeyboardCheatSheet additionalShortcuts", () => {
  it("renders a secondary shortcut badge + hint below the primary", () => {
    render(
      <KeyboardCheatSheet
        commands={[
          cmdWithExtra(
            "jump",
            "Navigation",
            "\u2318\u21E7L",
            [
              {
                keys: "\u2318\u21E7\u2325L",
                hint: "Cycle back through the ring",
              },
            ],
            "Jump to Last Touched",
          ),
        ]}
        open={true}
        onClose={() => {}}
      />,
    );
    // Primary shortcut still visible.
    expect(screen.getByText("\u2318\u21E7L")).toBeInTheDocument();
    // Secondary shortcut key string visible.
    expect(screen.getByText("\u2318\u21E7\u2325L")).toBeInTheDocument();
    // Secondary hint text visible.
    expect(
      screen.getByText("Cycle back through the ring"),
    ).toBeInTheDocument();
  });

  it("renders multiple secondary shortcuts in order", () => {
    render(
      <KeyboardCheatSheet
        commands={[
          cmdWithExtra(
            "multi",
            "Navigation",
            "\u2318A",
            [
              { keys: "\u2318B", hint: "second variant" },
              { keys: "\u2318C", hint: "third variant" },
            ],
            "Multi-gesture command",
          ),
        ]}
        open={true}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("\u2318B")).toBeInTheDocument();
    expect(screen.getByText("\u2318C")).toBeInTheDocument();
    expect(screen.getByText("second variant")).toBeInTheDocument();
    expect(screen.getByText("third variant")).toBeInTheDocument();
  });

  it("commands without additionalShortcuts render unchanged", () => {
    // Regression guard: iter 48's field is strictly additive.
    render(
      <KeyboardCheatSheet
        commands={[cmd("plain", "Navigation", "\u2318P", "Plain command")]}
        open={true}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("Plain command")).toBeInTheDocument();
    expect(screen.getByText("\u2318P")).toBeInTheDocument();
  });

  it("commands with an empty additionalShortcuts array behave as unchanged", () => {
    render(
      <KeyboardCheatSheet
        commands={[
          cmdWithExtra(
            "empty",
            "Navigation",
            "\u2318Q",
            [],
            "Empty extras command",
          ),
        ]}
        open={true}
        onClose={() => {}}
      />,
    );
    expect(screen.getByText("Empty extras command")).toBeInTheDocument();
    expect(screen.getByText("\u2318Q")).toBeInTheDocument();
  });
});
