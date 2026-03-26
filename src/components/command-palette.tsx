import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useFileManagerStore } from "@/stores/file-manager-store";
import { cn } from "@ufop/ui-components";
import {
  Search,
  FolderPlus,
  FilePlus,
  Copy,
  Scissors,
  Trash2,
  Pencil,
  Columns2,
  PanelLeftClose,
  Settings,
  Sun,
  Moon,
  RefreshCw,
  Star,
  Undo2,
  Layout,
  List,
  Grid3X3,
  AlignJustify,
  CopyPlus,
  Eye,
  Clipboard,
  Globe,
  FileEdit,
  Zap,
  Layers,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

export interface CommandItem {
  id: string;
  label: string;
  description?: string;
  icon?: React.ReactNode;
  shortcut?: string;
  category: string;
  action: () => void;
  keywords?: string[];
}

interface CommandPaletteProps {
  commands: CommandItem[];
  isOpen: boolean;
  onClose: () => void;
}

// ──────────────────────────────────────────────
// Fuzzy search
// ──────────────────────────────────────────────

function fuzzyMatch(query: string, text: string): { match: boolean; score: number } {
  if (!query) return { match: true, score: 0 };

  const queryLower = query.toLowerCase();
  const textLower = text.toLowerCase();

  // Exact substring match gets highest score
  if (textLower.includes(queryLower)) {
    const index = textLower.indexOf(queryLower);
    return { match: true, score: 100 - index };
  }

  // Fuzzy character matching
  let qi = 0;
  let score = 0;
  let lastMatchPos = -1;

  for (let ti = 0; ti < textLower.length && qi < queryLower.length; ti++) {
    if (textLower[ti] === queryLower[qi]) {
      qi++;
      // Consecutive matches score higher
      if (lastMatchPos === ti - 1) {
        score += 10;
      } else {
        score += 5;
      }
      // Word boundary matches score higher
      if (ti === 0 || textLower[ti - 1] === " " || textLower[ti - 1] === "-" || textLower[ti - 1] === "_") {
        score += 15;
      }
      lastMatchPos = ti;
    }
  }

  return { match: qi === queryLower.length, score };
}

// ──────────────────────────────────────────────
// CommandPalette component
// ──────────────────────────────────────────────

export function CommandPalette({ commands, isOpen, onClose }: CommandPaletteProps) {
  const [query, setQuery] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);

  // Filter and sort by fuzzy match score
  const filteredCommands = useMemo(() => {
    if (!query.trim()) return commands;

    const scored = commands
      .map((cmd) => {
        const labelMatch = fuzzyMatch(query, cmd.label);
        const descMatch = fuzzyMatch(query, cmd.description || "");
        const categoryMatch = fuzzyMatch(query, cmd.category);
        const keywordScores = (cmd.keywords || []).map((k) => fuzzyMatch(query, k));

        const bestKeyword = keywordScores.reduce(
          (best, s) => (s.score > best.score ? s : best),
          { match: false, score: 0 },
        );

        const matched = labelMatch.match || descMatch.match || categoryMatch.match || bestKeyword.match;
        const score = Math.max(labelMatch.score, descMatch.score * 0.7, categoryMatch.score * 0.5, bestKeyword.score * 0.8);

        return { cmd, matched, score };
      })
      .filter((s) => s.matched)
      .sort((a, b) => b.score - a.score);

    return scored.map((s) => s.cmd);
  }, [commands, query]);

  // Reset selection when query changes
  useEffect(() => {
    setSelectedIndex(0);
  }, [query]);

  // Focus input when opened
  useEffect(() => {
    if (isOpen) {
      setQuery("");
      setSelectedIndex(0);
      // Small delay to ensure DOM is ready
      requestAnimationFrame(() => {
        inputRef.current?.focus();
      });
    }
  }, [isOpen]);

  // Global shortcut
  useEffect(() => {
    function handleKeyDown(e: KeyboardEvent) {
      // Cmd+K or Cmd+P to open
      if ((e.metaKey || e.ctrlKey) && (e.key === "k" || e.key === "p")) {
        e.preventDefault();
        if (isOpen) {
          onClose();
        } else {
          useFileManagerStore.getState().setCommandPaletteOpen(true);
        }
      }
    }
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isOpen, onClose]);

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          setSelectedIndex((i) => Math.min(i + 1, filteredCommands.length - 1));
          break;
        case "ArrowUp":
          e.preventDefault();
          setSelectedIndex((i) => Math.max(i - 1, 0));
          break;
        case "Enter": {
          e.preventDefault();
          const cmd = filteredCommands[selectedIndex];
          if (cmd) {
            cmd.action();
            onClose();
          }
          break;
        }
        case "Escape":
          e.preventDefault();
          onClose();
          break;
      }
    },
    [filteredCommands, selectedIndex, onClose],
  );

  // Scroll selected item into view
  useEffect(() => {
    const item = listRef.current?.children[selectedIndex] as HTMLElement;
    if (item && typeof item.scrollIntoView === "function") {
      item.scrollIntoView({ block: "nearest" });
    }
  }, [selectedIndex]);

  if (!isOpen) return null;

  // Group commands by category
  const grouped = new Map<string, CommandItem[]>();
  for (const cmd of filteredCommands) {
    const existing = grouped.get(cmd.category) || [];
    existing.push(cmd);
    grouped.set(cmd.category, existing);
  }

  let flatIndex = 0;

  return (
    <>
      {/* Backdrop */}
      <div
        className="fixed inset-0 z-50 bg-black/40"
        onClick={onClose}
        aria-hidden="true"
        data-testid="command-palette-backdrop"
      />

      {/* Palette */}
      <div
        className={cn(
          "fixed z-50 top-[15%] left-1/2 -translate-x-1/2",
          "w-[90vw] max-w-[560px]",
          "bg-[var(--color-bg-elevated)] border border-[var(--color-border)]",
          "rounded-[var(--radius-lg)] shadow-xl",
          "animate-scale-in overflow-hidden",
        )}
        role="dialog"
        aria-label="Command palette"
        data-testid="command-palette"
      >
        {/* Search input */}
        <div className="flex items-center gap-2 px-4 border-b border-[var(--color-border)]">
          <Search className="h-4 w-4 text-[color:var(--color-text-tertiary)] shrink-0" aria-hidden="true" />
          <input
            ref={inputRef}
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={handleKeyDown}
            placeholder="Type a command..."
            className={cn(
              "flex-1 h-12 bg-transparent border-none outline-none",
              "text-[length:var(--font-size-md)] text-[color:var(--color-text)]",
              "placeholder:text-[color:var(--color-text-tertiary)]",
            )}
            aria-label="Search commands"
            data-testid="command-palette-input"
          />
          <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
            ESC
          </span>
        </div>

        {/* Command list */}
        <div
          ref={listRef}
          className="max-h-[60vh] overflow-auto py-2"
          role="listbox"
          aria-label="Commands"
        >
          {filteredCommands.length === 0 ? (
            <div className="px-4 py-8 text-center text-[length:var(--font-size-sm)] text-[color:var(--color-text-tertiary)]">
              No matching commands found
            </div>
          ) : (
            Array.from(grouped.entries()).map(([category, cmds]) => (
              <div key={category}>
                <div className="px-4 py-1 text-[length:var(--font-size-xs)] font-semibold text-[color:var(--color-text-tertiary)] uppercase tracking-wider">
                  {category}
                </div>
                {cmds.map((cmd) => {
                  const currentIndex = flatIndex++;
                  const isSelected = currentIndex === selectedIndex;

                  return (
                    <button
                      key={cmd.id}
                      role="option"
                      aria-selected={isSelected}
                      className={cn(
                        "flex items-center gap-3 w-full px-4 py-2 text-left",
                        "transition-theme",
                        isSelected
                          ? "bg-[var(--color-selection-bg)] text-[color:var(--color-selection-text)]"
                          : "text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
                      )}
                      onClick={() => {
                        cmd.action();
                        onClose();
                      }}
                      onMouseEnter={() => setSelectedIndex(currentIndex)}
                      data-testid={`command-${cmd.id}`}
                    >
                      {cmd.icon && (
                        <span className="w-5 h-5 flex items-center justify-center shrink-0">
                          {cmd.icon}
                        </span>
                      )}
                      <div className="flex-1 min-w-0">
                        <span className="text-[length:var(--font-size-sm)] block truncate">
                          {cmd.label}
                        </span>
                        {cmd.description && (
                          <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)] block truncate">
                            {cmd.description}
                          </span>
                        )}
                      </div>
                      {cmd.shortcut && (
                        <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] shrink-0">
                          {cmd.shortcut}
                        </span>
                      )}
                    </button>
                  );
                })}
              </div>
            ))
          )}
        </div>

        {/* Footer */}
        <div className="flex items-center justify-between px-4 py-2 border-t border-[var(--color-border)] text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
          <span>
            {filteredCommands.length} command{filteredCommands.length !== 1 ? "s" : ""}
          </span>
          <div className="flex items-center gap-3">
            <span>Navigate</span>
            <span>Enter to run</span>
          </div>
        </div>
      </div>
    </>
  );
}

// ──────────────────────────────────────────────
// Default commands builder
// ──────────────────────────────────────────────

export function getDefaultCommands(actions: {
  onNewFolder?: () => void;
  onNewFile?: () => void;
  onCopy?: () => void;
  onCut?: () => void;
  onPaste?: () => void;
  onDelete?: () => void;
  onRename?: () => void;
  onDuplicate?: () => void;
  onUndo?: () => void;
  onRefresh?: () => void;
  onToggleDualPane?: () => void;
  onToggleSidebar?: () => void;
  onSetViewMode?: (mode: string) => void;
  onSetTheme?: (theme: string) => void;
  onAddToFavorites?: () => void;
  onTogglePreview?: () => void;
  onSelectAll?: () => void;
  onInvertSelection?: () => void;
  onSaveWorkspace?: () => void;
  onLoadWorkspace?: () => void;
  onSetPermissions?: () => void;
  onSetOwner?: () => void;
  onCreateSymlink?: () => void;
  onCopyAsScript?: () => void;
  onCopyPath?: () => void;
  onCopyRemotePath?: () => void;
  onCopyUrl?: () => void;
  onCopyRelativePath?: () => void;
  onOpenInEditor?: () => void;
  onOpenSettings?: () => void;
  onToggleAutomations?: () => void;
  onCreateAutomation?: () => void;
  onCreateSmartSpace?: () => void;
}): CommandItem[] {
  const isMac = typeof navigator !== "undefined" && navigator.platform.startsWith("Mac");
  const cmd = isMac ? "\u2318" : "Ctrl+";

  const commands: CommandItem[] = [
    // File operations
    {
      id: "new-folder",
      label: "New Folder",
      icon: <FolderPlus className="h-4 w-4" />,
      shortcut: `${cmd}Shift+N`,
      category: "File",
      action: actions.onNewFolder || (() => {}),
      keywords: ["create", "directory", "mkdir"],
    },
    {
      id: "new-file",
      label: "New File",
      icon: <FilePlus className="h-4 w-4" />,
      category: "File",
      action: actions.onNewFile || (() => {}),
      keywords: ["create", "touch"],
    },
    {
      id: "copy",
      label: "Copy",
      icon: <Copy className="h-4 w-4" />,
      shortcut: `${cmd}C`,
      category: "Edit",
      action: actions.onCopy || (() => {}),
    },
    {
      id: "cut",
      label: "Cut",
      icon: <Scissors className="h-4 w-4" />,
      shortcut: `${cmd}X`,
      category: "Edit",
      action: actions.onCut || (() => {}),
    },
    {
      id: "paste",
      label: "Paste",
      shortcut: `${cmd}V`,
      category: "Edit",
      action: actions.onPaste || (() => {}),
    },
    {
      id: "duplicate",
      label: "Duplicate",
      icon: <CopyPlus className="h-4 w-4" />,
      shortcut: `${cmd}D`,
      category: "Edit",
      action: actions.onDuplicate || (() => {}),
    },
    {
      id: "rename",
      label: "Rename",
      icon: <Pencil className="h-4 w-4" />,
      shortcut: "F2",
      category: "Edit",
      action: actions.onRename || (() => {}),
    },
    {
      id: "delete",
      label: "Move to Trash",
      icon: <Trash2 className="h-4 w-4" />,
      shortcut: `${cmd}Backspace`,
      category: "Edit",
      action: actions.onDelete || (() => {}),
      keywords: ["remove", "trash"],
    },
    {
      id: "undo",
      label: "Undo",
      icon: <Undo2 className="h-4 w-4" />,
      shortcut: `${cmd}Z`,
      category: "Edit",
      action: actions.onUndo || (() => {}),
    },

    // Selection
    {
      id: "select-all",
      label: "Select All",
      shortcut: `${cmd}A`,
      category: "Selection",
      action: actions.onSelectAll || (() => {}),
    },
    {
      id: "invert-selection",
      label: "Invert Selection",
      category: "Selection",
      action: actions.onInvertSelection || (() => {}),
    },

    // View
    {
      id: "view-list",
      label: "List View",
      icon: <List className="h-4 w-4" />,
      category: "View",
      action: () => actions.onSetViewMode?.("list"),
      keywords: ["list", "view"],
    },
    {
      id: "view-detail",
      label: "Detail View",
      icon: <AlignJustify className="h-4 w-4" />,
      category: "View",
      action: () => actions.onSetViewMode?.("detail"),
      keywords: ["detail", "columns", "view"],
    },
    {
      id: "view-grid",
      label: "Grid View",
      icon: <Grid3X3 className="h-4 w-4" />,
      category: "View",
      action: () => actions.onSetViewMode?.("grid"),
      keywords: ["grid", "icons", "view"],
    },
    {
      id: "view-compact",
      label: "Compact View",
      icon: <Layout className="h-4 w-4" />,
      category: "View",
      action: () => actions.onSetViewMode?.("compact"),
      keywords: ["compact", "dense", "view"],
    },
    {
      id: "toggle-dual-pane",
      label: "Toggle Dual Pane",
      icon: <Columns2 className="h-4 w-4" />,
      shortcut: `${cmd}Shift+D`,
      category: "View",
      action: actions.onToggleDualPane || (() => {}),
      keywords: ["split", "dual", "pane"],
    },
    {
      id: "toggle-sidebar",
      label: "Toggle Sidebar",
      icon: <PanelLeftClose className="h-4 w-4" />,
      shortcut: `${cmd}B`,
      category: "View",
      action: actions.onToggleSidebar || (() => {}),
    },
    {
      id: "toggle-preview",
      label: "Toggle Preview Panel",
      icon: <Eye className="h-4 w-4" />,
      category: "View",
      action: actions.onTogglePreview || (() => {}),
      keywords: ["preview", "panel"],
    },
    {
      id: "refresh",
      label: "Refresh",
      icon: <RefreshCw className="h-4 w-4" />,
      shortcut: `${cmd}R`,
      category: "View",
      action: actions.onRefresh || (() => {}),
      keywords: ["reload"],
    },

    // Bookmarks
    {
      id: "add-favorite",
      label: "Add to Favorites",
      icon: <Star className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onAddToFavorites || (() => {}),
      keywords: ["bookmark", "favorite", "star"],
    },

    // Theme
    {
      id: "theme-light",
      label: "Light Theme",
      icon: <Sun className="h-4 w-4" />,
      category: "Appearance",
      action: () => actions.onSetTheme?.("light"),
      keywords: ["theme", "light"],
    },
    {
      id: "theme-dark",
      label: "Dark Theme",
      icon: <Moon className="h-4 w-4" />,
      category: "Appearance",
      action: () => actions.onSetTheme?.("dark"),
      keywords: ["theme", "dark"],
    },
    {
      id: "theme-system",
      label: "System Theme",
      icon: <Settings className="h-4 w-4" />,
      category: "Appearance",
      action: () => actions.onSetTheme?.("system"),
      keywords: ["theme", "system", "auto"],
    },
    // Workspace commands
    {
      id: "save-workspace",
      label: "Save Workspace",
      shortcut: `${cmd}Shift+S`,
      category: "File",
      action: actions.onSaveWorkspace || (() => {}),
      keywords: ["workspace", "save", "layout", "tabs"],
    },
    {
      id: "load-workspace",
      label: "Load Workspace",
      category: "File",
      action: actions.onLoadWorkspace || (() => {}),
      keywords: ["workspace", "restore", "open", "layout"],
    },
    // Permission commands
    {
      id: "set-permissions",
      label: "Set Permissions (chmod)",
      category: "File",
      action: actions.onSetPermissions || (() => {}),
      keywords: ["chmod", "permissions", "mode", "755", "644"],
    },
    {
      id: "set-owner",
      label: "Set Owner (chown)",
      category: "File",
      action: actions.onSetOwner || (() => {}),
      keywords: ["chown", "owner", "group", "uid", "gid"],
    },
    {
      id: "create-symlink",
      label: "Create Symbolic Link",
      category: "File",
      action: actions.onCreateSymlink || (() => {}),
      keywords: ["symlink", "link", "symbolic"],
    },
    // Script generation
    {
      id: "copy-as-script",
      label: "Copy as Script",
      category: "Edit",
      action: actions.onCopyAsScript || (() => {}),
      keywords: ["script", "bash", "shell", "generate", "automation"],
    },
    // Copy Path commands (Transmit parity)
    {
      id: "copy-path",
      label: "Copy Path",
      icon: <Clipboard className="h-4 w-4" />,
      shortcut: `${cmd}Shift+C`,
      category: "Copy Path",
      action: actions.onCopyPath || (() => {}),
      keywords: ["path", "copy", "absolute", "filepath"],
    },
    {
      id: "copy-remote-path",
      label: "Copy Remote Path",
      icon: <Clipboard className="h-4 w-4" />,
      category: "Copy Path",
      action: actions.onCopyRemotePath || (() => {}),
      keywords: ["path", "copy", "remote", "server"],
    },
    {
      id: "copy-url",
      label: "Copy URL",
      icon: <Globe className="h-4 w-4" />,
      category: "Copy Path",
      action: actions.onCopyUrl || (() => {}),
      keywords: ["url", "copy", "link", "protocol"],
    },
    {
      id: "copy-relative-path",
      label: "Copy Relative Path",
      icon: <Clipboard className="h-4 w-4" />,
      category: "Copy Path",
      action: actions.onCopyRelativePath || (() => {}),
      keywords: ["path", "copy", "relative"],
    },
    // Editor
    {
      id: "open-in-editor",
      label: "Open in External Editor",
      icon: <FileEdit className="h-4 w-4" />,
      category: "File",
      action: actions.onOpenInEditor || (() => {}),
      keywords: ["editor", "external", "vscode", "open", "edit"],
    },
    // Settings
    {
      id: "open-settings",
      label: "Open Settings",
      icon: <Settings className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onOpenSettings || (() => {}),
      keywords: ["settings", "preferences", "config"],
    },
    // Automations (Quickflows)
    {
      id: "toggle-automations",
      label: "Toggle Quickflows Panel",
      icon: <Zap className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onToggleAutomations || (() => {}),
      keywords: ["automation", "quickflow", "rules", "workflow", "watcher"],
    },
    {
      id: "create-automation",
      label: "Create Automation Rule",
      icon: <Zap className="h-4 w-4" />,
      category: "File",
      action: actions.onCreateAutomation || (() => {}),
      keywords: ["automation", "quickflow", "rule", "new", "create", "watcher", "trigger"],
    },
    // Smart Spaces
    {
      id: "create-smart-space",
      label: "Create Smart Space...",
      icon: <Layers className="h-4 w-4" />,
      category: "File",
      action: actions.onCreateSmartSpace || (() => {}),
      keywords: ["space", "workspace", "folder", "connection", "sync", "bundle"],
    },
  ];

  return commands;
}
