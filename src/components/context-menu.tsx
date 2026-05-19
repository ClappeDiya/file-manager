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
  History,
  SendToBack,
  ShieldCheck,
  FileArchive,
  PackageOpen,
  Pin,
  PinOff,
  X as CloseIcon,
  ChevronsRight,
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

/**
 * Compress a directory path into a short label for use in menu items.
 * Prefers the trailing folder name (e.g., `/Users/md/Documents/Receipts`
 * → `Receipts`); falls back to the full path for short / unusual inputs.
 */
function shortenFolderName(path: string): string {
  const trimmed = path.replace(/[/\\]+$/, "");
  if (trimmed === "" || trimmed === "/") return path;
  const lastSlash = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  if (lastSlash < 0) return trimmed;
  const tail = trimmed.slice(lastSlash + 1);
  return tail.length > 0 ? tail : trimmed;
}

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
  onShowHistory?: () => void;
  /** **Show this folder's activity** — open the Activity Timeline panel
   *  pre-filtered to events whose `subject_path` or `target_path` starts
   *  with this folder's path. Symmetric per-folder mirror of
   *  `onShowHistory` (which is per-file). Wired on folder rows only —
   *  see the `isDirectory` gate at the call site. */
  onShowFolderActivity?: () => void;
  /** **Compress to archive** — open the existing `CreateArchiveDialog`
   *  with the right-clicked selection pre-filled. The dialog is the
   *  orphaned tail of the iter 31 archive integration: backend
   *  `archive_create` IPC has been wired since T-062 with full format /
   *  password / compression-level support, but the UI never had an
   *  entry point. Every mainstream file manager has "Compress N items"
   *  in the right-click menu; this entry closes that gap with zero new
   *  infrastructure. Hidden when the caller omits the prop so the
   *  pane-only-selection (Paste-only) menu stays untouched. */
  onCompress?: () => void;
  /** **Extract Here** — mirror of iter 14's `onCompress`. Pre-wired
   *  `archive_extract` IPC already supports zip / tar / tar.gz / 7z
   *  with optional password. The caller (FilePane) sets this prop only
   *  when the right-clicked entry has an archive extension, so the
   *  menu doesn't need to gate on file type itself. Direct action (no
   *  dialog) — extracts into `${parent}/${stem}/` next to the archive.
   *  Same singleton-action contract as `onRevealInOs`. */
  onExtract?: () => void;
  /** Reveal the selected entry in the OS's native file manager
   *  (Finder on macOS / Explorer on Windows / xdg-open the parent
   *  on Linux). Mirrors the "Show in Finder" / "Show in Explorer"
   *  staple that every mainstream file manager has. */
  onRevealInOs?: () => void;
  /** Iter 39: open Integrity Tools with the right-clicked
   *  selection preset into the Checksums tab. Lets users jump
   *  from "these files" to "hash them / verify them / find
   *  duplicates of them" without manually reselecting inside
   *  the modal the way iter 36 required. The handler receives
   *  no args — the FilePane closure is expected to snapshot the
   *  selection + current directory at menu-build time. */
  onVerifyIntegrity?: () => void;
  /** **Smart Send-To** — frecency-ranked destination directories the
   *  ledger has learned for the right-clicked file's extension. When
   *  non-empty, each entry is rendered inline as a one-click
   *  "Send to {dir}" action that fires the existing `move_files` IPC.
   *  Pass an empty array (or omit) to hide the section entirely. */
  smartDestinations?: Array<{ path: string; hit_count: number }>;
  /** Handler invoked when the user clicks a Send-To entry. Receives
   *  the destination directory the user picked. */
  onSendTo?: (destDir: string) => void;
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

    // Reveal in the native OS file manager — Finder on macOS,
    // Explorer on Windows, parent-directory open on Linux. Staple
    // power-user action present in every mainstream file manager.
    // Shown right under Open so users find it at the position
    // muscle memory expects. Platform-aware label so the menu
    // text matches what the user is about to see ("Show in
    // Finder" vs "Show in Explorer" vs the generic fallback).
    if (options.onRevealInOs) {
      const isWin =
        typeof navigator !== "undefined" &&
        navigator.platform.toLowerCase().includes("win");
      items.push({
        id: "reveal-in-os",
        label: isMac
          ? "Show in Finder"
          : isWin
            ? "Show in Explorer"
            : "Show in File Manager",
        icon: <FolderOpen className="h-3.5 w-3.5" />,
        action: options.onRevealInOs,
      });
    }

    // Iter 15: Extract Here — direct action for archive files
    // (.zip / .tar / .tar.gz / .7z). Sits right after the
    // platform-aware "Show in OS" entry because extraction IS the
    // primary intent for an archive — exposing it at the top of
    // the menu saves the user a double-click into ArchiveBrowser
    // when they already know they want everything extracted. The
    // mirror entry is iter 14's "Compress…" further down, which
    // *creates* archives from arbitrary selections.
    if (options.onExtract) {
      items.push({
        id: "extract-here",
        label: "Extract Here",
        icon: <PackageOpen className="h-3.5 w-3.5" />,
        action: options.onExtract,
      });
    }

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

    // Iter 39: Verify integrity on the right-clicked selection.
    // Available for both files AND directories — checksums for
    // files, duplicate-scan / smart-folder base path for
    // directories. The single menu entry routes to whichever
    // tab makes sense because the IntegrityTools modal adapts
    // its default tab based on the selection shape.
    if (options.onVerifyIntegrity) {
      items.push({
        id: "verify-integrity",
        label: "Verify Integrity…",
        icon: <ShieldCheck className="h-3.5 w-3.5" />,
        shortcut: `${cmdKey}Shift+I`,
        action: options.onVerifyIntegrity,
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

    // Smart Send-To — ledger-derived frecency-ranked destinations for
    // files with this extension. Hidden entirely when the ledger has
    // no relevant history (no clutter for new users). Each entry is
    // a one-click move via the existing fs.move_files IPC.
    if (
      options.smartDestinations &&
      options.smartDestinations.length > 0 &&
      options.onSendTo
    ) {
      items.push({ id: "sep-send-to", label: "", separator: true });
      for (const dest of options.smartDestinations) {
        const shortName = shortenFolderName(dest.path);
        items.push({
          id: `send-to-${dest.path}`,
          label: `Send to ${shortName}`,
          icon: <SendToBack className="h-3.5 w-3.5" />,
          action: () => options.onSendTo!(dest.path),
        });
      }
    }
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

    // Compress — sits right after Duplicate because they're the two
    // "make a new file from the selection" actions. The trailing
    // ellipsis signals that a dialog will open (format / compression
    // level / optional password). Backend has full support via the
    // pre-wired `archive_create` IPC; this entry surfaces the
    // `CreateArchiveDialog` that was already built but never mounted.
    if (options.onCompress) {
      items.push({
        id: "compress",
        label: options.selectionCount > 1
          ? `Compress ${options.selectionCount} Items…`
          : "Compress…",
        icon: <FileArchive className="h-3.5 w-3.5" />,
        action: options.onCompress,
      });
    }

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

    // Show File History — opens the lineage panel, which walks the unified
    // operation ledger to surface every rename / move / copy / sync /
    // transfer / automation fire that ever touched this file, even under
    // prior names. Zero cost until clicked — no background work.
    if (options.onShowHistory) {
      items.push({
        id: "show-history",
        label: "Show File History",
        icon: <History className="h-3.5 w-3.5" />,
        action: options.onShowHistory,
        advancedOnly: true,
      });
    }

    // Show Folder Activity — opens the Activity Timeline pre-filtered to
    // events under this folder. Folder-only mirror of "Show File History":
    // same ledger source, broader scope. Surfaces "what happened in this
    // folder?" with zero new IPC — the panel already pulls
    // `ledger_recent(200)` and adds one client-side predicate.
    if (options.isDirectory && options.onShowFolderActivity) {
      items.push({
        id: "show-folder-activity",
        label: "Show Folder Activity",
        icon: <History className="h-3.5 w-3.5" />,
        action: options.onShowFolderActivity,
        advancedOnly: true,
      });
    }

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

// ──────────────────────────────────────────────
// Tab context menu (iter 20) — browser-parity right-click on tabs.
// Reuses the same `ContextMenu` component as the file context menu;
// the items list lives here next to `getFileContextMenuItems` so both
// menus share imports, icon conventions, and the disabled / danger
// styling rules.
// ──────────────────────────────────────────────

/** Build the right-click menu for a tab. Every action operates on the
 *  *target* tab (the one right-clicked) — never the active tab — so
 *  the gesture matches the browser convention of "do this thing to
 *  THIS tab" regardless of which tab is currently focused.
 *
 *  Several actions are conditionally enabled rather than hidden:
 *    - Close Tab: disabled if the tab is pinned or it's the last tab.
 *    - Close Other Tabs: disabled if nothing would actually close.
 *    - Close Tabs to the Right: disabled if no closable tab sits to the right.
 *    - Move to Other Pane: hidden in single-tab panes (the move would
 *      leave the source pane empty); also hidden for pinned tabs since
 *      moving them would contradict the pin contract.
 *
 *  The caller is responsible for passing the counts / flags it
 *  already knows (no IPC, no store reads from inside this builder)
 *  so this function stays a pure data transform — easily tested. */
export function getTabContextMenuItems(options: {
  isPinned: boolean;
  /** Total tab count in the pane (post-pin sort doesn't matter — the
   *  count is invariant under sorting). Used to gate "Close Tab" and
   *  "Move to Other Pane". */
  totalTabs: number;
  /** Count of non-pinned tabs other than the target tab. When zero,
   *  "Close Other Tabs" is disabled because nothing would close. */
  closableOthers: number;
  /** Count of non-pinned tabs whose visual position is to the right
   *  of the target. When zero, "Close Tabs to the Right" is disabled. */
  closableToRight: number;
  onClose: () => void;
  onCloseOthers: () => void;
  onCloseToRight: () => void;
  onPinToggle: () => void;
  onDuplicate: () => void;
  /** Optional — omit when single-pane mode + only one tab make the move
   *  meaningless. Iter 20 hides the entry entirely in those cases. */
  onMoveToOtherPane?: () => void;
}): ContextMenuItem[] {
  const isMac =
    typeof navigator !== "undefined" && navigator.platform.startsWith("Mac");
  const cmdKey = isMac ? "⌘" : "Ctrl+";

  const items: ContextMenuItem[] = [];

  // Close Tab — disabled for pinned and last-tab cases, matching the
  // safety checks inside `closeTab` in the store. Disabling rather than
  // hiding keeps the menu shape stable so users build muscle memory.
  items.push({
    id: "close-tab",
    label: "Close Tab",
    icon: <CloseIcon className="h-3.5 w-3.5" />,
    shortcut: `${cmdKey}W`,
    action: options.onClose,
    disabled: options.isPinned || options.totalTabs <= 1,
    danger: true,
  });

  items.push({
    id: "close-other-tabs",
    label: "Close Other Tabs",
    icon: <XSquare className="h-3.5 w-3.5" />,
    action: options.onCloseOthers,
    disabled: options.closableOthers === 0,
    danger: true,
  });

  items.push({
    id: "close-tabs-to-right",
    label: "Close Tabs to the Right",
    icon: <ChevronsRight className="h-3.5 w-3.5" />,
    action: options.onCloseToRight,
    disabled: options.closableToRight === 0,
    danger: true,
  });

  items.push({ id: "tab-sep1", label: "", separator: true });

  items.push({
    id: "pin-toggle",
    label: options.isPinned ? "Unpin Tab" : "Pin Tab",
    icon: options.isPinned ? (
      <PinOff className="h-3.5 w-3.5" />
    ) : (
      <Pin className="h-3.5 w-3.5" />
    ),
    action: options.onPinToggle,
  });

  items.push({
    id: "duplicate-tab",
    label: "Duplicate Tab",
    icon: <CopyPlus className="h-3.5 w-3.5" />,
    action: options.onDuplicate,
  });

  // Move to Other Pane — only show when the move is actually possible.
  // Pinned tabs and single-tab panes hide the entry entirely rather
  // than disable: this action is for power users and showing it
  // dimmed in a single-tab pane would just clutter the menu.
  if (options.onMoveToOtherPane && !options.isPinned && options.totalTabs > 1) {
    items.push({ id: "tab-sep2", label: "", separator: true });
    items.push({
      id: "move-to-other-pane",
      label: "Move to Other Pane",
      icon: <ArrowLeftRight className="h-3.5 w-3.5" />,
      action: options.onMoveToOtherPane,
    });
  }

  return items;
}
