import { GUIDED_FLOW_IDS, GUIDED_FLOW_META } from "@/components/guided-flows/flow-meta";
import { FolderPlus, FilePlus, Copy, Scissors, Trash2, Pencil, Columns2, PanelLeftClose, Settings, Sun, Moon, RefreshCw, Star, Undo2, Layout, List, Grid3X3, AlignJustify, CopyPlus, Eye, Clipboard, Globe, FileEdit, Zap, Activity, Layers, RotateCcw, Bookmark, BookmarkPlus, Sparkles, ArrowRight, ArrowLeftRight, FolderUp, AppWindow, Lock, Shield, Filter, FolderSync, DatabaseBackup, HardDrive, CalendarClock, CircleHelp, ScrollText, MonitorSmartphone, History, ShieldCheck, SlidersHorizontal } from "lucide-react";


// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

/** Iter 48: secondary shortcut entry. Rendered in the keyboard
 *  cheat sheet below the primary `shortcut` so multi-gesture
 *  features (Jump Ring forward/reverse, future copy/move
 *  modifier pairs, …) can surface every binding without needing
 *  separate palette entries that would clutter ⌘K. Not shown in
 *  the ⌘K palette itself — that surface stays one-row-per-action. */
export interface AdditionalShortcut {
  /** Keyboard glyph string, e.g. "\u2318\u21E7\u2325L". */
  keys: string;
  /** Short hint (≤40 chars) describing what the variant does. */
  hint: string;
}


export interface CommandItem {
  id: string;
  label: string;
  description?: string;
  icon?: React.ReactNode;
  shortcut?: string;
  /** Iter 48: secondary shortcuts surfaced in the keyboard cheat
   *  sheet only. Undefined or empty array = single shortcut,
   *  which is the pre-iter-48 behaviour. */
  additionalShortcuts?: AdditionalShortcut[];
  category: string;
  action: () => void;
  keywords?: string[];
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
  onRedo?: () => void;
  onRefresh?: () => void;
  onToggleDualPane?: () => void;
  onToggleSidebar?: () => void;
  onSetViewMode?: (mode: string) => void;
  onSetTheme?: (theme: string) => void;
  /** Text size one step larger (1), smaller (-1) or default (0). */
  onTextSize?: (step: -1 | 0 | 1) => void;
  onAddToFavorites?: () => void;
  onTogglePreview?: () => void;
  /** Open the Lineage / "Show File History" panel for the first
   *  selected file. Mirrors the existing right-click "Show File
   *  History" + activity-dot click flows; surfaced as a keyboard
   *  gesture (Cmd+Shift+G) and a command-palette entry so users
   *  who prefer keyboard or palette discovery reach the same
   *  ledger-derived surface. */
  onShowFileHistory?: () => void;
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
  onToggleOperations?: () => void;
  onCreateAutomation?: () => void;
  onCreateSmartSpace?: () => void;
  onToggleActivityTimeline?: () => void;
  onReopenClosedTab?: () => void;
  onStashSelection?: () => void;
  onRecallLastStash?: () => void;
  /** Toggle a bookmark on the active pane's current directory. */
  onToggleBookmarkCurrentDir?: () => void;
  /** Live snapshot of the favorites ring so the palette can surface
   *  each bookmark as its own jump entry. Kept optional for callers
   *  that build the palette without the file-manager store wired in. */
  bookmarks?: Array<{ id: string; name: string; path: string }>;
  /** Jump the active pane to a bookmark by its slot index. */
  onJumpToBookmark?: (index: number) => void;
  /** Open the active pane's current path as a fresh tab in the OTHER
   *  pane and focus it. Auto-exits single-pane mode. */
  onOpenInOtherPane?: () => void;
  /** Swap the contents of the two panes \u2014 left becomes right and
   *  vice versa, taking selections, tabs, and per-pane history along
   *  with them. Auto-exits single-pane mode. */
  onSwapPanes?: () => void;
  /** Copy the active pane's filter text to the other pane so both
   *  sides filter for the same query. No-op when the source filter
   *  is empty. */
  onEchoFilterToOtherPane?: () => void;
  /** Copy the active pane's full view configuration (viewMode,
   *  sortBy, sortAsc, groupBy) to the other pane in one keystroke.
   *  Idempotent: silent no-op when both panes already match. */
  onMirrorViewToOtherPane?: () => void;
  /** Duplicate the active tab as a fresh tab in the SAME pane.
   *  The clone gets its own back/forward history starting from
   *  the current path. Pairs with Open in Other Pane (which
   *  targets the inactive pane) to give the user both same-pane
   *  and cross-pane "fork this view" gestures. */
  onDuplicateActiveTab?: () => void;
  /** Open the Batch Rename dialog on the active pane's current
   *  selection. Iter 25 surfaces the long-orphaned rename engine
   *  (token substitution, find/replace, regex, sequential
   *  numbering, case transformation, undo). No-op with a toast
   *  when the selection is empty. */
  onBatchRename?: () => void;
  /** Enqueue the selected files as transfer-engine jobs to a picked
   *  destination — the only user-facing path that feeds the Transfer
   *  Queue (previously the engine queue had zero producers). */
  onQueueCopyTo?: () => void;
  /** Compare the left and right panes' folders (standalone folder compare). */
  onComparePanes?: () => void;
  /** Compare the active pane's folder with a folder picked next. */
  onCompareFolderWith?: () => void;
  /** Open the Connection Manager panel \u2014 saved profiles,
   *  quick-connect, connection test, third-party import, and
   *  the 17-protocol registry (SFTP, S3, Google Drive, WebDAV,
   *  SMB, FTP, Azure Blob, GCS, Dropbox, Box, OneDrive, iCloud
   *  Drive, Mega, pCloud, Nextcloud, OpenStack Swift, MinIO). */
  onOpenConnections?: () => void;
  /** Turn the active pane's breadcrumb into the editable address field. */
  onEditAddress?: () => void;
  onGoToParentFolder?: () => void;
  /** Open another app window at the active pane's folder. */
  onNewWindow?: () => void;
  /** Show the lock screen in every window (when the app lock is on). */
  onLockApp?: () => void;
  /** Start a guided wizard (`GUIDED_FLOW_IDS`). */
  onOpenGuidedFlow?: (id: string) => void;
  /** Open the Connection Manager on its rclone remote manager. */
  onOpenRcloneRemotes?: () => void;
  /** Open Settings expanded and scrolled to its rclone section. */
  onOpenRcloneSettings?: () => void;
  /** Open Settings at one section (`startup`, `app-lock`, `reset`…). */
  onOpenSettingsSection?: (section: string) => void;
  /** Open the Sync Panel \u2014 sync-pair health indicators,
   *  create-pair wizard, dry-run preview, conflict resolution,
   *  reports, and rollback. Iter 28 surfaces the long-orphaned
   *  1454-LOC component. */
  onOpenSync?: () => void;
  /** Open the Backups panel — versioned, deduplicated, encrypted
   *  backup jobs with restore, maintenance and run history. */
  onOpenBackups?: () => void;
  /** Advanced mode only (undefined hides the entry). */
  onOpenSchedules?: () => void;
  /** Advanced mode only (undefined hides the entry). */
  onOpenLogViewer?: () => void;
  /** Help > Send feedback. */
  onSendFeedback?: () => void;
  /** Open Nearby computers (peer discovery and transfers). */
  onOpenNearby?: () => void;
  /** Open the Mounts list (saved and running mounts, with mount/unmount/edit/remove). */
  onOpenMounts?: () => void;
  /** Open the mount dialog to mount a connection as a drive or folder. */
  onMountDrive?: () => void;
  /** Send the selection to another computer (undefined hides the entry). */
  onSendToComputer?: () => void;
  /** Open the Transfer History (Journal) panel \u2014 searchable
   *  table of every historical transfer with status, checksum
   *  verification, retry count, duration, and speed; CSV/JSON
   *  export; retention cleanup. Iter 30 surfaces the long-
   *  orphaned 284-LOC component that complements the transfer
   *  engine's journal + crash-recovery layer. */
  onOpenTransferHistory?: () => void;
  /** Open the Encrypted Vault panel (vault-panel.tsx — another
   *  zero-importer orphan until now). */
  onOpenVault?: () => void;
  /** Open the Migration Wizard (migration-wizard.tsx — orphaned,
   *  self-modal overlay). */
  onOpenMigration?: () => void;
  /** Toggle the Transfer Queue dock (transfer-panel.tsx — the engine
   *  queue UI, orphaned until now). */
  onToggleTransferQueue?: () => void;
  /** Toggle the toolbar customizer popover — the ui-store had the
   *  toggle and the popover had the markup, but no control anywhere
   *  called it, so the customizer was unreachable. */
  onCustomizeToolbar?: () => void;
  /** Open the Integrity Tools panel — four tabs covering
   *  checksum computation (MD5 / SHA-1 / SHA-256), checksum
   *  verification, duplicate detection, tags and labels, and
   *  Smart Folder saved queries. Iter 36 surfaces the last
   *  clean 1071-LOC orphan (T-063) with five pre-registered
   *  backend IPCs. */
  onOpenIntegrityTools?: () => void;
  /** Open the Preview Pane on the active pane's first selected
   *  file. Supports images (EXIF), PDFs, syntax-highlighted
   *  code (30+ languages), rendered Markdown, audio waveforms,
   *  video thumbnails, archive contents, and plain metadata
   *  for large files. Sandboxed \u2014 no script / network / write.
   *  Iter 29 surfaces the long-orphaned 844-LOC component.
   *  Shows a structured-error toast when selection is empty. */
  onOpenPreview?: () => void;
  /** Copy every selected path (one per line) to the system
   *  clipboard, or the current directory path when nothing is
   *  selected. Different from the single-path `onCopyPath` entry,
   *  which silently drops every path but the first from a
   *  multi-file selection. */
  onCopyAllPaths?: () => void;
  /** Iter 42: "Jump to Last Touched" — teleport directly to the
   *  most recently touched ledger path without opening Instant
   *  Jump. Routes through the same shared dispatch helper as
   *  Instant Jump, so archives/text/media/folders all land on
   *  the right surface. No-op when the ledger is empty. */
  onJumpLastTouched?: () => void;
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
    {
      id: "redo",
      label: "Redo",
      icon: <Undo2 className="h-4 w-4 -scale-x-100" />,
      shortcut: `${cmd}⇧Z`,
      category: "Edit",
      action: actions.onRedo || (() => {}),
      keywords: ["repeat", "reapply"],
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
      id: "show-file-history",
      label: "Show File History",
      icon: <History className="h-4 w-4" />,
      shortcut: `${cmd}⇧G`,
      category: "View",
      action: actions.onShowFileHistory || (() => {}),
      keywords: ["lineage", "provenance", "get history"],
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
    // Text size (the keys are bound by useTextSizeShortcuts)
    {
      id: "text-size-larger",
      label: "Larger Text",
      shortcut: `${cmd}+`,
      category: "Appearance",
      action: () => actions.onTextSize?.(1),
      keywords: ["text size", "font", "bigger", "zoom in", "accessibility"],
    },
    {
      id: "text-size-smaller",
      label: "Smaller Text",
      shortcut: `${cmd}-`,
      category: "Appearance",
      action: () => actions.onTextSize?.(-1),
      keywords: ["text size", "font", "smaller", "zoom out"],
    },
    {
      id: "text-size-default",
      label: "Default Text Size",
      shortcut: `${cmd}0`,
      category: "Appearance",
      action: () => actions.onTextSize?.(0),
      keywords: ["text size", "font", "reset", "actual size"],
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
    // Preview \u2014 iter 29 surfaced the 844-LOC orphaned
    // PreviewPane. Images (with EXIF), PDFs, syntax-highlighted
    // code (30+ languages), rendered Markdown, audio waveforms,
    // video thumbnails, archive contents, plain metadata for
    // huge files. All sandboxed. 4 IPC commands (preview_file,
    // preview_get_exif, set_permissions, set_owner) were
    // pre-registered in the Rust backend; the component just
    // had zero importers until iter 29.
    {
      id: "open-preview",
      label: "Preview File\u2026",
      description:
        "Open a sandboxed preview (images, PDF, code, Markdown, audio, video, archive, EXIF) for the selected file",
      icon: <Eye className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7P`,
      category: "File Operations",
      action: actions.onOpenPreview || (() => {}),
      keywords: [
        "preview",
        "peek",
        "quick look",
        "view",
        "show",
        "inspect",
        "file preview",
        "pdf",
        "image",
        "exif",
        "markdown",
        "syntax",
        "code",
        "thumbnail",
        "metadata",
      ],
    },
    // Sync Panel \u2014 iter 28 surfaced the 1454-LOC orphaned
    // sync-pair manager. Health indicators, create-pair
    // wizard, dry-run preview, conflict resolution, reports,
    // and rollback. All 14 sync IPC commands were already
    // registered in the Rust backend; the component just had
    // zero importers until iter 28.
    {
      id: "open-backups",
      label: "Backups\u2026",
      description:
        "Versioned, deduplicated, encrypted backups \u2014 create jobs, run, restore old versions, verify, repair, schedule",
      icon: <DatabaseBackup className="h-4 w-4" />,
      shortcut: `${cmd}\u2325B`,
      category: "Navigation",
      action: actions.onOpenBackups || (() => {}),
      keywords: [
        "backup",
        "backups",
        "restore",
        "version",
        "versions",
        "snapshot",
        "retention",
        "deduplication",
        "encrypted backup",
        "verify backup",
        "duplicati",
        "time machine",
      ],
    },
    {
      id: "open-nearby",
      label: "Nearby Computers\u2026",
      description: "Find other computers running UFOP on this network, accept incoming files, connect by address",
      icon: <MonitorSmartphone className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onOpenNearby || (() => {}),
      keywords: ["peer", "peers", "nearby", "airdrop", "lan", "local network", "computer", "device", "mdns", "bonjour", "receive"],
    },
    {
      id: "mount-drive",
      label: "Mount Drive\u2026",
      description: "Mount a saved connection (cloud, SFTP, SMB, WebDAV, NFS) as a folder or drive on this computer",
      icon: <HardDrive className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onMountDrive || (() => {}),
      keywords: ["mount", "drive", "disk", "volume", "fuse", "rclone", "remote drive", "network drive", "map drive", "sshfs"],
    },
    {
      id: "open-mounts",
      label: "Show Mounts",
      description: "See saved and mounted drives; mount, unmount, edit or remove them",
      icon: <HardDrive className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onOpenMounts || (() => {}),
      keywords: ["mounts", "mounted", "drives", "unmount", "eject", "remote drives", "network drives"],
    },
    ...(actions.onSendToComputer
      ? [
          {
            id: "send-to-computer",
            label: "Send Selection to Another Computer\u2026",
            description: "Send the selected files and folders over an encrypted connection; the other computer approves first",
            icon: <MonitorSmartphone className="h-4 w-4" />,
            category: "File",
            action: actions.onSendToComputer,
            keywords: ["send", "share", "peer", "airdrop", "transfer", "computer", "nearby"],
          },
        ]
      : []),
    ...(actions.onSendFeedback
      ? [
          {
            id: "send-feedback",
            label: "Help: Send Feedback\u2026",
            description: "Tell us what went wrong or what you would like; diagnostics are included only if you tick the box",
            icon: <CircleHelp className="h-4 w-4" />,
            category: "Navigation",
            action: actions.onSendFeedback,
            keywords: ["help", "feedback", "bug", "report", "suggest", "idea", "support", "contact"],
          },
        ]
      : []),
    ...(actions.onOpenLogViewer
      ? [
          {
            id: "open-log-viewer",
            label: "Open Log Viewer\u2026",
            description: "Read the app's own log: filter by level, module, time and text, follow it live, copy lines",
            icon: <ScrollText className="h-4 w-4" />,
            category: "Navigation",
            action: actions.onOpenLogViewer,
            keywords: ["log", "logs", "logging", "viewer", "debug", "errors", "tail", "diagnostics"],
          },
        ]
      : []),
    ...(actions.onOpenSchedules
      ? [
          {
            id: "open-schedules",
            label: "Schedules & holiday calendars\u2026",
            description: "Named schedules for sync pairs and rules; holiday calendars that skip scheduled runs",
            icon: <CalendarClock className="h-4 w-4" />,
            category: "Navigation",
            action: actions.onOpenSchedules,
            keywords: ["schedule", "schedules", "cron", "holiday", "holidays", "calendar", "business days", "bank holiday"],
          },
        ]
      : []),
    {
      id: "open-sync",
      label: "Sync Pairs\u2026",
      description:
        "Manage sync pairs \u2014 Robocopy-style copy jobs, health indicators, dry-run preview, conflict resolution, reports, rollback",
      icon: <FolderSync className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7H`,
      category: "Navigation",
      action: actions.onOpenSync || (() => {}),
      keywords: [
        "sync",
        "sync pair",
        "sync pairs",
        "mirror",
        "replicate",
        "dry run",
        "preview",
        "conflict",
        "rollback",
        "health",
        "scheduled",
        "one-way",
        "two-way",
        "backup",
        "robocopy",
        "copy job",
        "scheduled copy",
      ],
    },
    // Transfer History (Journal) \u2014 iter 30 surfaced the 284-LOC
    // `TransferHistoryPanel` orphan that complements the transfer
    // engine's journal + crash-recovery layer. Shows every
    // historical transfer with status, checksum verification,
    // retry count, duration, speed; supports CSV/JSON export and
    // retention cleanup. Three backend IPCs (search_transfer_history,
    // export_transfer_history, cleanup_transfer_history) were
    // already registered; the component just had zero importers
    // until iter 30. Cmd+Shift+J mnemonic = "Journal" \u2014 the only
    // still-free letter that carried semantic meaning after the
    // iter 12\u201329 accumulation.
    {
      id: "open-transfer-history",
      label: "Transfer History\u2026",
      description:
        "Searchable journal of every transfer \u2014 status, checksum, retries, speed, duration; CSV/JSON export",
      icon: <History className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7J`,
      category: "Navigation",
      action: actions.onOpenTransferHistory || (() => {}),
      keywords: [
        "transfer",
        "transfers",
        "history",
        "journal",
        "log",
        "completed",
        "failed",
        "retry",
        "retries",
        "checksum",
        "verified",
        "speed",
        "bandwidth",
        "throughput",
        "duration",
        "export",
        "csv",
        "json",
        "audit",
        "report",
        "past",
        "old",
      ],
    },
    // Integrity Tools — iter 36 surfaced the last clean 1071-LOC
    // orphan (T-063). Four tabs covering MD5/SHA-1/SHA-256
    // checksums, duplicate detection, tags/labels, and Smart
    // Folder saved queries. Five pre-registered backend IPCs
    // (integrity_checksum, integrity_verify, compute_checksum,
    // integrity_find_duplicates, integrity_get_file_info).
    // Cmd+Shift+I mnemonic = "Integrity".
    {
      id: "open-integrity-tools",
      label: "Integrity Tools…",
      description:
        "Checksums (MD5/SHA-1/SHA-256), duplicate detection, tags and labels, Smart Folder saved queries",
      icon: <ShieldCheck className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7I`,
      category: "Navigation",
      action: actions.onOpenIntegrityTools || (() => {}),
      keywords: [
        "integrity",
        "checksum",
        "hash",
        "md5",
        "sha1",
        "sha256",
        "verify",
        "duplicate",
        "duplicates",
        "dedupe",
        "tag",
        "tags",
        "label",
        "labels",
        "smart folder",
        "smart folders",
        "saved query",
        "filter",
      ],
    },
    // Iter 42/43/45: Jump to Last Touched. One-keystroke teleport
    // to a recently touched ledger path. Complements Instant Jump
    // (⌘⇧O) — that one opens a picker, this one is zero-UI: it
    // fetches top `ledger_recent_paths` rows and routes through
    // the shared dispatch helper so archives land in the browser,
    // text in the editor, media in the preview pane, folders in
    // the active tab. Iter 43 upgraded the handler to a "Jump
    // Ring" (rapid re-press cycles forward, the current tab path
    // is always filtered out). Iter 45 added reverse cycling on
    // ⌘⇧⌥L and extracted the decision logic into a pure,
    // unit-tested `pickJumpRingTarget` function. "L" mnemonic = "Last".
    {
      id: "jump-last-touched",
      label: "Jump to Last Touched",
      description:
        "Teleport to a recently touched ledger path — \u2318\u21E7L cycles forward, \u2318\u21E7\u2325L cycles back, the current folder is always skipped",
      icon: <History className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7U`,
      // Iter 48: surface the reverse cycling gesture in the
      // keyboard cheat sheet (?). Secondary shortcuts don't
      // show up in ⌘K itself — the palette stays one row per
      // action — but they DO appear in the cheat sheet so
      // power users discover the Alt modifier convention.
      additionalShortcuts: [
        {
          keys: `${cmd}\u21E7\u2325L`,
          hint: "Cycle back through the ring",
        },
      ],
      category: "Navigation",
      action: actions.onJumpLastTouched || (() => {}),
      keywords: [
        "jump",
        "last",
        "recent",
        "most recent",
        "teleport",
        "back",
        "where",
        "left off",
        "resume",
        "return",
        "previous",
        "ledger",
        "history",
        "ring",
        "cycle",
        "alt tab",
      ],
    },
    // Connection Manager \u2014 iter 27 surfaced the 1941-LOC
    // `ConnectionPanel` orphan that manages profiles across the
    // 17 connector protocols. All 17 Tauri commands it uses
    // were already registered in the Rust backend; the
    // component just had zero importers until iter 27.
    {
      id: "open-connections",
      label: "Connection Manager\u2026",
      description:
        "Manage saved connections (SFTP, S3, Google Drive, WebDAV, SMB, FTP, Azure, GCS, Dropbox, \u2026)",
      icon: <Globe className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7U`,
      category: "Navigation",
      action: actions.onOpenConnections || (() => {}),
      keywords: [
        "connection",
        "connections",
        "connect",
        "remote",
        "server",
        "sftp",
        "s3",
        "google drive",
        "webdav",
        "smb",
        "ftp",
        "azure",
        "gcs",
        "dropbox",
        "protocol",
        "manager",
        "new connection",
      ],
    },
    {
      id: "edit-address",
      label: "Go to Address\u2026",
      description: "Type a folder path, conn://\u2026 or <connection name>:path in the active pane",
      icon: <Globe className="h-4 w-4" />,
      shortcut: `${cmd}L`,
      category: "Navigation",
      action: actions.onEditAddress || (() => {}),
      keywords: ["address", "path", "go to", "location", "breadcrumb", "type path", "cloud", "connection"],
    },
    {
      id: "new-window",
      label: "New Window",
      description: "Open another UFOP window at the active pane's folder (each window has its own panes and tabs)",
      icon: <AppWindow className="h-4 w-4" />,
      shortcut: `${cmd}N`,
      category: "Navigation",
      action: actions.onNewWindow || (() => {}),
      keywords: ["window", "new window", "another window", "second window", "multiple windows", "open window"],
    },
    {
      id: "go-to-parent-folder",
      label: "Go to Parent Folder",
      description: "Open the folder that contains the active pane's folder",
      icon: <FolderUp className="h-4 w-4" />,
      shortcut: isMac ? `${cmd}\u2191` : "Alt+\u2191",
      category: "Navigation",
      action: actions.onGoToParentFolder || (() => {}),
      keywords: ["up", "parent", "back up", "folder up", "..", "backspace", "go up"],
    },
    {
      id: "open-rclone-remotes",
      label: "rclone Remotes\u2026",
      description: "Create, edit, test and delete rclone remotes (Box, pCloud, MEGA, S3, SFTP, 70+ providers)",
      icon: <Globe className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onOpenRcloneRemotes || (() => {}),
      keywords: ["rclone", "remote", "remotes", "rclone config", "box", "pcloud", "mega", "crypt", "provider", "cloud"],
    },
    {
      id: "startup-tray-settings",
      label: "Startup & Tray Settings\u2026",
      description: "Start at login, start minimised, minimise or close to the tray, show the tray icon",
      icon: <Settings className="h-4 w-4" />,
      category: "Navigation",
      action: () => (actions.onOpenSettingsSection ? actions.onOpenSettingsSection("startup") : actions.onOpenSettings?.()),
      keywords: ["startup", "login", "autostart", "tray", "minimise", "minimize", "close", "background", "menu bar"],
    },
    {
      id: "lock-app",
      label: "Lock UFOP",
      description: "Show the lock screen in every window until the master password is entered (turn it on in Settings \u2192 App Lock)",
      icon: <Lock className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onLockApp || (() => actions.onOpenSettingsSection?.("app-lock")),
      keywords: ["lock", "app lock", "password", "privacy", "away", "screen lock"],
    },
    {
      id: "reset-settings",
      label: "Reset Settings\u2026",
      description: "Put appearance, startup and tray, and sync defaults back to the defaults (data is kept)",
      icon: <RotateCcw className="h-4 w-4" />,
      category: "Navigation",
      action: () => (actions.onOpenSettingsSection ? actions.onOpenSettingsSection("reset") : actions.onOpenSettings?.()),
      keywords: ["reset", "defaults", "factory", "restore settings", "start over", "preferences"],
    },
    {
      id: "rclone-settings",
      label: "rclone Program & Config\u2026",
      description: "Choose the rclone binary and rclone.conf, check the version, install the managed copy (Settings \u2192 rclone)",
      icon: <Globe className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onOpenRcloneSettings || actions.onOpenSettings || (() => {}),
      keywords: ["rclone", "binary", "version", "rclone.conf", "config", "update", "install"],
    },
    {
      id: "open-vault",
      label: "Encrypted Vault…",
      description:
        "Manage encrypted vaults — create, unlock, mount, lock",
      icon: <Shield className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onOpenVault || (() => {}),
      keywords: [
        "vault",
        "encrypt",
        "encrypted",
        "secure",
        "lock",
        "unlock",
        "password",
      ],
    },
    {
      id: "open-migration",
      label: "Migration Wizard…",
      description:
        "Move files and settings from another computer or drive",
      icon: <ArrowRight className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onOpenMigration || (() => {}),
      keywords: [
        "migration",
        "migrate",
        "wizard",
        "move",
        "transfer computer",
        "new mac",
        "restore",
      ],
    },
    {
      id: "toggle-transfer-queue",
      label: "Transfer Queue",
      description:
        "Show the transfer engine queue — pause, resume, cancel, retry, throttle",
      icon: <ArrowRight className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onToggleTransferQueue || (() => {}),
      keywords: [
        "transfer",
        "queue",
        "jobs",
        "pause",
        "resume",
        "cancel",
        "retry",
        "throttle",
      ],
    },
    {
      id: "customize-toolbar",
      label: "Customize Toolbar…",
      description: "Choose which buttons appear in the toolbar",
      icon: <SlidersHorizontal className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onCustomizeToolbar || (() => {}),
      keywords: ["toolbar", "customize", "buttons", "layout", "quickflows"],
    },
    // Settings \u2014 iter 26 wired the long-orphaned `SettingsPanel`
    // (1354 LOC with 9+ backend commands already registered) to
    // this entry. The palette stub existed from a prior iter but
    // `actions.onOpenSettings` was never passed, so the entry did
    // nothing until now. Iter 26 enriches the entry with the
    // universal Cmd+, shortcut + richer keywords.
    {
      id: "open-settings",
      label: "Settings\u2026",
      description:
        "Export/import, sync across installations, notification prefs, privacy, logs",
      icon: <Settings className="h-4 w-4" />,
      shortcut: `${cmd},`,
      category: "Navigation",
      action: actions.onOpenSettings || (() => {}),
      keywords: [
        "settings",
        "preferences",
        "options",
        "config",
        "configure",
        "export",
        "import",
        "notifications",
        "privacy",
        "logs",
      ],
    },
    {
      id: "toggle-operations",
      label: "Show Operations (Transfers, Syncs, Backups)",
      icon: <Zap className="h-4 w-4" />,
      category: "Navigation",
      action: actions.onToggleOperations || (() => {}),
      keywords: ["progress", "running", "queue", "activity", "jobs", "cancel"],
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
    // Selection Stash — capture the active pane's selection into the
    // global persistent stash ring. Pairs with "Recall Last Stash" so
    // the user can navigate away, do something else, and come back to
    // the same working set without rebuilding it manually.
    {
      id: "stash-selection",
      label: "Stash Selection",
      description: "Save the current selection so you can recall it later",
      icon: <Bookmark className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7M`,
      category: "Selection",
      action: actions.onStashSelection || (() => {}),
      keywords: [
        "stash",
        "save",
        "selection",
        "mark",
        "remember",
        "bookmark selection",
        "memory",
        "set aside",
      ],
    },
    {
      id: "recall-last-stash",
      label: "Recall Last Stash",
      description:
        "Restore the most recent stashed selection into the active pane",
      icon: <Bookmark className="h-4 w-4" />,
      category: "Selection",
      action: actions.onRecallLastStash || (() => {}),
      keywords: [
        "recall",
        "restore",
        "stash",
        "last",
        "selection",
        "bring back",
        "resume",
      ],
    },
    // Reopen the most recently closed tab in the active pane —
    // browser-muscle-memory shortcut. Fully no-op when the ring buffer
    // is empty, so the entry is always safe to surface.
    {
      id: "reopen-closed-tab",
      label: "Reopen Closed Tab",
      description: "Revive the most recently closed tab in the active pane",
      icon: <RotateCcw className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7T`,
      category: "Navigation",
      action: actions.onReopenClosedTab || (() => {}),
      keywords: [
        "reopen",
        "restore",
        "tab",
        "closed",
        "undo close",
        "bring back",
      ],
    },
    // Activity Timeline (unified ledger viewer)
    {
      id: "toggle-activity-timeline",
      label: "Toggle Activity Timeline",
      description: "View recent activity across all engines",
      icon: <Activity className="h-4 w-4" />,
      category: "Navigation",
      shortcut: `${cmd}\u21E7Y`,
      action: actions.onToggleActivityTimeline || (() => {}),
      keywords: [
        "activity",
        "timeline",
        "history",
        "audit",
        "ledger",
        "recent",
        "events",
        "what happened",
      ],
    },
    // Open in Other Pane — opens the active pane's current path as a
    // fresh tab in the OTHER pane and focuses it. Auto-exits single-pane
    // mode (revealing the second pane is part of the gesture).
    {
      id: "open-in-other-pane",
      label: "Open in Other Pane",
      description:
        "Mirror the current location into the other pane as a new tab",
      icon: <Columns2 className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7E`,
      category: "Navigation",
      action: actions.onOpenInOtherPane || (() => {}),
      keywords: [
        "pane",
        "split",
        "side by side",
        "mirror",
        "echo",
        "twin",
        "compare",
        "open here",
        "dual",
      ],
    },
    // Copy All Paths — copies every selected path to the clipboard
    // (one per line), or the current directory path when nothing
    // is selected. The existing "Copy Path" entry silently drops
    // everything but the first path, so this entry fills the
    // multi-select gap and also provides a keyboard shortcut.
    {
      id: "copy-all-paths",
      label: "Copy Selected Paths",
      description:
        "Copy every selected path to the clipboard \u2014 or the current directory when nothing is selected",
      icon: <Clipboard className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7C`,
      category: "File Operations",
      action: actions.onCopyAllPaths || (() => {}),
      keywords: [
        "copy path",
        "copy paths",
        "clipboard",
        "multi copy",
        "paste",
        "absolute",
        "all paths",
        "selection paths",
        "export paths",
      ],
    },
    // Batch Rename \u2014 opens the rename engine on the active pane's
    // current selection. Iter 25 surfaced this long-orphaned
    // feature: token substitution ({name}, {ext}, {num}, {date},
    // {parent}, {counter}), find/replace with optional regex,
    // sequential numbering, case transformation, live preview,
    // undo, and compatibility-warning integration \u2014 all of it
    // was already built and tested in Rust + React, just never
    // wired into the UI.
    {
      id: "batch-rename",
      label: "Batch Rename\u2026",
      description:
        "Open the batch rename engine on the selected files (patterns, regex, numbering, case)",
      icon: <FileEdit className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7R`,
      category: "File Operations",
      action: actions.onBatchRename || (() => {}),
      keywords: [
        "batch rename",
        "bulk rename",
        "rename",
        "regex rename",
        "find replace",
        "pattern rename",
        "case",
        "numbering",
        "multi rename",
        "mass rename",
      ],
    },
    {
      id: "compare-panes",
      label: "Compare Panes",
      description: "Compare the left and right folders recursively, then copy or delete the differences",
      icon: <FolderSync className="h-4 w-4" />,
      category: "File Operations",
      action: actions.onComparePanes || (() => {}),
      keywords: ["compare", "compare folders", "diff folders", "folder diff", "reconcile", "differences", "check"],
    },
    {
      id: "compare-folder-with",
      label: "Compare Folder With…",
      description: "Compare the active pane's folder with another folder you pick",
      icon: <FolderSync className="h-4 w-4" />,
      category: "File Operations",
      action: actions.onCompareFolderWith || (() => {}),
      keywords: ["compare", "compare with", "diff folders", "folder diff", "reconcile"],
    },
    {
      id: "queue-copy",
      label: "Queue Copy to Destination…",
      description:
        "Enqueue selected files as transfer-queue jobs (verified, resumable, pauseable)",
      icon: <ArrowRight className="h-4 w-4" />,
      category: "File Operations",
      action: actions.onQueueCopyTo || (() => {}),
      keywords: [
        "queue copy",
        "transfer queue",
        "enqueue",
        "copy files",
        "copy to",
        "background copy",
        "verified copy",
      ],
    },
    // Duplicate Tab \u2014 clones the active tab as a fresh tab in the
    // SAME pane and focuses it. Different from Open in Other Pane
    // (which targets the inactive pane). The clone gets its own
    // back/forward history so navigating it does not disturb the
    // original.
    {
      id: "duplicate-active-tab",
      label: "Duplicate Tab",
      description:
        "Clone the active tab as a fresh tab in the same pane with its own history",
      icon: <CopyPlus className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7K`,
      category: "Navigation",
      action: actions.onDuplicateActiveTab || (() => {}),
      keywords: [
        "duplicate",
        "clone",
        "fork",
        "tab",
        "copy tab",
        "split tab",
        "twin tab",
        "new tab here",
        "open here",
      ],
    },
    // Mirror View to Other Pane — copies the active pane's view
    // configuration (viewMode, sortBy, sortAsc, groupBy) to the
    // other pane in one keystroke. Replaces 3-4 menu clicks needed
    // to align two panes for side-by-side comparison. Idempotent
    // (silent no-op when both panes already match).
    {
      id: "mirror-view-to-other-pane",
      label: "Mirror View to Other Pane",
      description:
        "Copy view mode, sort column, sort direction, and grouping to the other pane",
      icon: <Layout className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7V`,
      category: "Navigation",
      action: actions.onMirrorViewToOtherPane || (() => {}),
      keywords: [
        "view",
        "mirror view",
        "equalize",
        "align",
        "sort echo",
        "match view",
        "sync view",
        "compare layout",
        "twin view",
        "same sort",
      ],
    },
    // Echo Filter to Other Pane — copies the active pane's filter
    // text to the inactive pane so both sides filter for the same
    // query. No-op when the source filter is empty (silent: avoids
    // wiping the other pane's filter by surprise).
    {
      id: "echo-filter-to-other-pane",
      label: "Echo Filter to Other Pane",
      description:
        "Copy the active pane's filter text to the other pane so both sides filter for the same query",
      icon: <Filter className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7F`,
      category: "Navigation",
      action: actions.onEchoFilterToOtherPane || (() => {}),
      keywords: [
        "filter",
        "echo",
        "mirror filter",
        "same query",
        "both panes",
        "sync filter",
        "copy filter",
        "twin filter",
        "apply filter",
      ],
    },
    // Swap Panes — flips the two panes' contents, taking tabs,
    // selections, and per-pane history with them. Auto-exits single-
    // pane mode. Pairs with Open in Other Pane: mirror, then swap if
    // the source/target sides are wrong.
    {
      id: "swap-panes",
      label: "Swap Panes",
      description:
        "Flip the left and right panes \u2014 contents, tabs, selections, and history all travel together",
      icon: <ArrowLeftRight className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7X`,
      category: "Navigation",
      action: actions.onSwapPanes || (() => {}),
      keywords: [
        "swap",
        "flip",
        "exchange",
        "switch",
        "sides",
        "left right",
        "rotate",
        "trade",
        "panes",
        "reverse",
      ],
    },
    // Bookmark Current Folder — toggles the active pane's directory in
    // the same `favorites` ring the sidebar drag-drop populates. Single
    // entry, two outcomes (add / remove): the action itself decides
    // based on whether the path is already bookmarked.
    {
      id: "bookmark-current-folder",
      label: "Bookmark Current Folder",
      description:
        "Add (or remove) the active pane's directory from your bookmarks",
      icon: <BookmarkPlus className="h-4 w-4" />,
      shortcut: `${cmd}\u21E7B`,
      category: "Navigation",
      action: actions.onToggleBookmarkCurrentDir || (() => {}),
      keywords: [
        "bookmark",
        "favorite",
        "star",
        "save",
        "pin",
        "remember",
        "add to favorites",
      ],
    },
  ];

  // Every guided wizard, by name (Simple mode also lists them).
  if (actions.onOpenGuidedFlow) {
    const open = actions.onOpenGuidedFlow;
    for (const id of GUIDED_FLOW_IDS) {
      const meta = GUIDED_FLOW_META[id];
      commands.push({
        id: `guided-${id}`,
        label: `${meta.label} (guided)`,
        description: meta.description,
        icon: <Sparkles className="h-4 w-4" />,
        category: "Guided",
        action: () => open(id),
        keywords: ["wizard", "guided", "step by step", "help me", meta.category.toLowerCase()],
      });
    }
  }

  // Inject the first nine bookmarks as their own palette entries so the
  // user can fuzzy-search them by name without remembering slot numbers.
  // Slot 1..9 maps directly to ⌘1..⌘9 muscle memory.
  const bookmarks = actions.bookmarks ?? [];
  const onJump = actions.onJumpToBookmark;
  for (let i = 0; i < Math.min(bookmarks.length, 9); i += 1) {
    const fav = bookmarks[i];
    commands.push({
      id: `jump-bookmark-${fav.id}`,
      label: `Jump to ${fav.name}`,
      description: fav.path,
      icon: <Bookmark className="h-4 w-4" />,
      shortcut: `${cmd}${i + 1}`,
      category: "Bookmarks",
      action: onJump ? () => onJump(i) : () => {},
      keywords: ["bookmark", "favorite", "jump", "navigate", fav.name],
    });
  }

  return commands;
}
