import { useCallback, useEffect, useRef, useState } from "react";
import { cn } from "@ufop/ui-components";
import { useUIStore } from "@/stores/ui-store";
import {
  Copy,
  Scissors,
  ClipboardPaste,
  Trash2,
  Pencil,
  FolderPlus,
  FilePlus,
  Star,
  Info,
  ArrowUpRight,
  RefreshCw,
  CheckSquare,
  XSquare,
  CopyPlus,
  Bot,
  Terminal,
  Calculator,
  ExternalLink,
  ArrowLeftRight,
  GitBranch,
  Lock,
  Link,
  FileCode,
  Clipboard,
  Globe,
  FolderOpen,
  FileEdit,
} from "lucide-react";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

export interface ContextMenuItem {
  id: string;
  label: string;
  icon?: React.ReactNode;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  separator?: boolean;
  advancedOnly?: boolean;
  action?: () => void;
}

interface ContextMenuProps {
  items: ContextMenuItem[];
  position: { x: number; y: number } | null;
  onClose: () => void;
}

// ──────────────────────────────────────────────
// ContextMenu component
// ──────────────────────────────────────────────

export function ContextMenu({ items, position, onClose }: ContextMenuProps) {
  const menuRef = useRef<HTMLDivElement>(null);
  const [focusedIndex, setFocusedIndex] = useState(0);
  const appMode = useUIStore((s) => s.appMode);

  // Filter items based on mode
  const visibleItems = items.filter(
    (item) => !item.advancedOnly || appMode === "advanced",
  );

  // Close on click outside
  useEffect(() => {
    if (!position) return;

    function handleClick(e: MouseEvent) {
      if (menuRef.current && !menuRef.current.contains(e.target as Node)) {
        onClose();
      }
    }

    function handleEscape(e: KeyboardEvent) {
      if (e.key === "Escape") {
        onClose();
      }
    }

    document.addEventListener("mousedown", handleClick);
    document.addEventListener("keydown", handleEscape);
    return () => {
      document.removeEventListener("mousedown", handleClick);
      document.removeEventListener("keydown", handleEscape);
    };
  }, [position, onClose]);

  // Focus menu when it opens
  useEffect(() => {
    if (position && menuRef.current) {
      menuRef.current.focus();
      setFocusedIndex(0);
    }
  }, [position]);

  // Keyboard navigation
  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      switch (e.key) {
        case "ArrowDown": {
          e.preventDefault();
          let next = focusedIndex;
          do {
            next = (next + 1) % visibleItems.length;
          } while (visibleItems[next]?.separator);
          setFocusedIndex(next);
          break;
        }
        case "ArrowUp": {
          e.preventDefault();
          let prev = focusedIndex;
          do {
            prev = (prev - 1 + visibleItems.length) % visibleItems.length;
          } while (visibleItems[prev]?.separator);
          setFocusedIndex(prev);
          break;
        }
        case "Enter":
        case " ": {
          e.preventDefault();
          const item = visibleItems[focusedIndex];
          if (item && !item.disabled && !item.separator) {
            item.action?.();
            onClose();
          }
          break;
        }
        case "Home": {
          e.preventDefault();
          const first = visibleItems.findIndex((i) => !i.separator);
          if (first !== -1) setFocusedIndex(first);
          break;
        }
        case "End": {
          e.preventDefault();
          for (let i = visibleItems.length - 1; i >= 0; i--) {
            if (!visibleItems[i].separator) {
              setFocusedIndex(i);
              break;
            }
          }
          break;
        }
      }
    },
    [focusedIndex, visibleItems, onClose],
  );

  if (!position) return null;

  // Adjust position to stay within viewport
  const adjustedPosition = { ...position };
  if (typeof window !== "undefined") {
    const menuWidth = 220;
    const menuHeight = visibleItems.length * 32;
    if (adjustedPosition.x + menuWidth > window.innerWidth) {
      adjustedPosition.x = window.innerWidth - menuWidth - 8;
    }
    if (adjustedPosition.y + menuHeight > window.innerHeight) {
      adjustedPosition.y = window.innerHeight - menuHeight - 8;
    }
  }

  return (
    <div
      ref={menuRef}
      className={cn(
        "fixed z-50 min-w-[200px] max-w-[280px]",
        "bg-[var(--color-bg-elevated)] border border-[var(--color-border)]",
        "rounded-[var(--radius-md)] shadow-lg",
        "py-1 animate-scale-in",
        "focus:outline-none",
      )}
      style={{
        left: adjustedPosition.x,
        top: adjustedPosition.y,
      }}
      role="menu"
      aria-label="Context menu"
      tabIndex={-1}
      onKeyDown={handleKeyDown}
      data-testid="context-menu"
    >
      {visibleItems.map((item, index) => {
        if (item.separator) {
          return (
            <div
              key={item.id}
              className="h-px mx-2 my-1 bg-[var(--color-border)]"
              role="separator"
            />
          );
        }

        return (
          <button
            key={item.id}
            role="menuitem"
            className={cn(
              "flex items-center gap-2 w-full px-3 py-1.5 text-left",
              "text-[length:var(--font-size-xs)]",
              "transition-theme",
              "focus-visible:outline-none",
              item.disabled
                ? "text-[color:var(--color-text-disabled)] cursor-not-allowed"
                : item.danger
                  ? "text-[color:var(--color-error)] hover:bg-[var(--color-error-bg)]"
                  : "text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
              index === focusedIndex && !item.disabled && "bg-[var(--color-hover-bg)]",
            )}
            disabled={item.disabled}
            onClick={() => {
              if (!item.disabled) {
                item.action?.();
                onClose();
              }
            }}
            data-testid={`context-menu-item-${item.id}`}
          >
            {item.icon && <span className="w-4 h-4 flex items-center justify-center shrink-0">{item.icon}</span>}
            <span className="flex-1 truncate">{item.label}</span>
            {item.shortcut && (
              <span className="text-[color:var(--color-text-tertiary)] text-[length:var(--font-size-xs)] shrink-0 ml-2">
                {item.shortcut}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}

// ──────────────────────────────────────────────
// Pre-built context menu item sets
// ──────────────────────────────────────────────

export function getFileContextMenuItems(options: {
  hasSelection: boolean;
  isDirectory: boolean;
  selectionCount: number;
  onCopy: () => void;
  onCut: () => void;
  onPaste: () => void;
  onDelete: () => void;
  onRename: () => void;
  onDuplicate: () => void;
  onNewFolder: () => void;
  onNewFile: () => void;
  onAddToFavorites: () => void;
  onGetInfo: () => void;
  onOpen: () => void;
  onRefresh: () => void;
  onSelectAll: () => void;
  onInvertSelection: () => void;
  onDeletePermanently?: () => void;
  onCalculateSize?: () => void;
  onEditRemote?: () => void;
  onCompareFiles?: () => void;
  onExplainError?: () => void;
  onOpenInTerminal?: () => void;
  onGitAdd?: () => void;
  onGitCommit?: () => void;
  onSetPermissions?: () => void;
  onCreateSymlink?: () => void;
  onCopyAsScript?: () => void;
  onCopyPath?: () => void;
  onCopyRemotePath?: () => void;
  onCopyUrl?: () => void;
  onCopyRelativePath?: () => void;
  onOpenWith?: () => void;
  onOpenInEditor?: () => void;
}): ContextMenuItem[] {
  const items: ContextMenuItem[] = [];
  const isMac = typeof navigator !== "undefined" && navigator.platform.startsWith("Mac");
  const cmdKey = isMac ? "\u2318" : "Ctrl+";

  if (options.hasSelection) {
    items.push({
      id: "open",
      label: options.isDirectory ? "Open" : "Open File",
      icon: <ArrowUpRight className="h-3.5 w-3.5" />,
      shortcut: "Enter",
      action: options.onOpen,
    });

    // Feature 2: Edit & Auto-Upload for remote files (#44)
    if (options.onEditRemote && options.hasSelection && !options.isDirectory) {
      items.push({
        id: "edit-remote",
        label: "Edit & Auto-Upload",
        icon: <ExternalLink className="h-3.5 w-3.5" />,
        action: options.onEditRemote,
        advancedOnly: true,
      });
    }

    // Feature 3: Compare two selected files (#47)
    if (options.onCompareFiles && options.selectionCount === 2 && options.hasSelection) {
      items.push({
        id: "compare-files",
        label: "Compare Selected",
        icon: <ArrowLeftRight className="h-3.5 w-3.5" />,
        shortcut: `${cmdKey}=`,
        action: options.onCompareFiles,
      });
    }

    if (options.onOpenWith && !options.isDirectory) {
      items.push({
        id: "open-with",
        label: "Open With...",
        icon: <FolderOpen className="h-3.5 w-3.5" />,
        action: options.onOpenWith,
      });
    }

    if (options.onOpenInEditor && !options.isDirectory) {
      items.push({
        id: "open-in-editor",
        label: "Open in Editor",
        icon: <FileEdit className="h-3.5 w-3.5" />,
        action: options.onOpenInEditor,
      });
    }

    items.push({ id: "sep1", label: "", separator: true });

    items.push({
      id: "copy",
      label: "Copy",
      icon: <Copy className="h-3.5 w-3.5" />,
      shortcut: `${cmdKey}C`,
      action: options.onCopy,
    });

    // Copy Path submenu (Transmit parity)
    if (options.onCopyPath) {
      items.push({ id: "sep-copy-path", label: "", separator: true });
      items.push({
        id: "copy-path",
        label: "Copy Path",
        icon: <Clipboard className="h-3.5 w-3.5" />,
        shortcut: `${cmdKey}Shift+C`,
        action: options.onCopyPath,
      });
      if (options.onCopyRemotePath) {
        items.push({
          id: "copy-remote-path",
          label: "Copy Remote Path",
          icon: <Clipboard className="h-3.5 w-3.5" />,
          action: options.onCopyRemotePath,
        });
      }
      if (options.onCopyUrl) {
        items.push({
          id: "copy-url",
          label: "Copy URL",
          icon: <Globe className="h-3.5 w-3.5" />,
          action: options.onCopyUrl,
        });
      }
      if (options.onCopyRelativePath) {
        items.push({
          id: "copy-relative-path",
          label: "Copy Relative Path",
          icon: <Clipboard className="h-3.5 w-3.5" />,
          action: options.onCopyRelativePath,
        });
      }
    }

    items.push({
      id: "cut",
      label: "Cut",
      icon: <Scissors className="h-3.5 w-3.5" />,
      shortcut: `${cmdKey}X`,
      action: options.onCut,
    });
  }

  items.push({
    id: "paste",
    label: "Paste",
    icon: <ClipboardPaste className="h-3.5 w-3.5" />,
    shortcut: `${cmdKey}V`,
    action: options.onPaste,
  });

  if (options.hasSelection) {
    items.push({
      id: "duplicate",
      label: "Duplicate",
      icon: <CopyPlus className="h-3.5 w-3.5" />,
      shortcut: `${cmdKey}D`,
      action: options.onDuplicate,
    });

    items.push({ id: "sep2", label: "", separator: true });

    items.push({
      id: "rename",
      label: "Rename",
      icon: <Pencil className="h-3.5 w-3.5" />,
      shortcut: "F2",
      action: options.onRename,
      disabled: options.selectionCount > 1,
    });

    items.push({
      id: "delete",
      label: "Move to Trash",
      icon: <Trash2 className="h-3.5 w-3.5" />,
      shortcut: `${cmdKey}Backspace`,
      danger: true,
      action: options.onDelete,
    });

    if (options.onDeletePermanently) {
      items.push({
        id: "delete-permanently",
        label: "Delete Permanently",
        icon: <Trash2 className="h-3.5 w-3.5 text-red-500" />,
        shortcut: "\u21E7\u232B",
        danger: true,
        advancedOnly: true,
        action: options.onDeletePermanently,
      });
    }

    items.push({ id: "sep3", label: "", separator: true });

    items.push({
      id: "add-favorite",
      label: "Add to Favorites",
      icon: <Star className="h-3.5 w-3.5" />,
      action: options.onAddToFavorites,
    });

    items.push({
      id: "get-info",
      label: "Get Info",
      icon: <Info className="h-3.5 w-3.5" />,
      shortcut: `${cmdKey}I`,
      action: options.onGetInfo,
      advancedOnly: true,
    });

    if (options.isDirectory && options.onCalculateSize) {
      items.push({
        id: "calculate-size",
        label: "Calculate Folder Size",
        icon: <Calculator className="h-3.5 w-3.5" />,
        action: options.onCalculateSize,
      });
    }
  }

  items.push({ id: "sep4", label: "", separator: true });

  items.push({
    id: "new-folder",
    label: "New Folder",
    icon: <FolderPlus className="h-3.5 w-3.5" />,
    shortcut: `${cmdKey}Shift+N`,
    action: options.onNewFolder,
  });

  items.push({
    id: "new-file",
    label: "New File",
    icon: <FilePlus className="h-3.5 w-3.5" />,
    action: options.onNewFile,
  });

  items.push({ id: "sep5", label: "", separator: true });

  items.push({
    id: "select-all",
    label: "Select All",
    icon: <CheckSquare className="h-3.5 w-3.5" />,
    shortcut: `${cmdKey}A`,
    action: options.onSelectAll,
  });

  items.push({
    id: "invert-selection",
    label: "Invert Selection",
    icon: <XSquare className="h-3.5 w-3.5" />,
    action: options.onInvertSelection,
    advancedOnly: true,
  });

  // Permissions, symlink, and script generation (WinSCP parity)
  if (options.onSetPermissions || options.onCreateSymlink || options.onCopyAsScript) {
    items.push({ id: "sep-perms", label: "", separator: true });

    if (options.onSetPermissions && options.hasSelection) {
      items.push({
        id: "set-permissions",
        label: "Set Permissions...",
        icon: <Lock className="h-3.5 w-3.5" />,
        action: options.onSetPermissions,
        advancedOnly: false,
      });
    }

    if (options.onCreateSymlink && options.hasSelection && options.selectionCount === 1) {
      items.push({
        id: "create-symlink",
        label: "Create Symbolic Link...",
        icon: <Link className="h-3.5 w-3.5" />,
        action: options.onCreateSymlink,
        advancedOnly: true,
      });
    }

    if (options.onCopyAsScript && options.hasSelection) {
      items.push({
        id: "copy-as-script",
        label: "Copy as Script",
        icon: <FileCode className="h-3.5 w-3.5" />,
        action: options.onCopyAsScript,
        advancedOnly: true,
      });
    }
  }

  // AI and Terminal actions (T-043, T-046)
  if (options.onExplainError || options.onOpenInTerminal) {
    items.push({ id: "sep-ai", label: "", separator: true });

    if (options.onExplainError) {
      items.push({
        id: "explain-error",
        label: "Explain this error",
        icon: <Bot className="h-3.5 w-3.5" />,
        action: options.onExplainError,
        advancedOnly: false,
      });
    }

    if (options.onOpenInTerminal && options.hasSelection && options.isDirectory) {
      items.push({
        id: "open-in-terminal",
        label: "Open in Terminal",
        icon: <Terminal className="h-3.5 w-3.5" />,
        action: options.onOpenInTerminal,
      });
    }
  }

  // Git actions (#46)
  if (options.onGitAdd || options.onGitCommit) {
    items.push({ id: "sep-git", label: "", separator: true });

    if (options.onGitAdd) {
      items.push({
        id: "git-add",
        label: "Git: Stage File",
        icon: <GitBranch className="h-3.5 w-3.5" />,
        action: options.onGitAdd,
        advancedOnly: true,
      });
    }

    if (options.onGitCommit) {
      items.push({
        id: "git-commit",
        label: "Git: Commit",
        icon: <GitBranch className="h-3.5 w-3.5" />,
        action: options.onGitCommit,
        advancedOnly: true,
      });
    }
  }

  items.push({ id: "sep6", label: "", separator: true });

  items.push({
    id: "refresh",
    label: "Refresh",
    icon: <RefreshCw className="h-3.5 w-3.5" />,
    shortcut: `${cmdKey}R`,
    action: options.onRefresh,
  });

  return items;
}
